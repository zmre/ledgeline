//! Cross-report invariant / reconciliation suite (CLEANUP.md RPT-5).
//!
//! Generalises the single cross-report test that existed before
//! (`ledgeline-server/tests/report_endpoints.rs::insights_reconciles_with_income_statement_and_networth`)
//! into a set of relationships that must hold between EVERY report over
//! `fixtures/sample.journal`, at several as-of dates.
//!
//! # Why the core level
//!
//! These are statements about the report ENGINE, not about the wire: they need
//! to vary `depth`, `interval`, `count` and `value_in` freely, to compare exact
//! `MixedAmount`/`Dec` values (never display-rounded text), and to run at half a
//! dozen dates each. Driving that through HTTP would mean a URL per combination
//! and a JSON reparse per assertion for no extra coverage — the wire shape is
//! already pinned by `report_endpoints.rs` and `reports_golden.rs`. The subset
//! that only the wire can break — query parsing, defaulting and JSON encoding
//! moving a number — is restated over HTTP in
//! `ledgeline-server/tests/report_invariants_http.rs`, so the endpoints are
//! known to serve the same figures these tests constrain.
//!
//! # The accounting identity is not `== 0` in a multi-commodity journal
//!
//! `assets + liabilities + equity + revenues + expenses` sums to zero only when
//! every transaction balances in RAW amounts. `sample.journal` balances several
//! transactions AT COST (`10 AAPL @ $220.00` against `$-2,200.00`), so the raw
//! sum is off by exactly the cost residual of every priced posting. hledger says
//! the same thing:
//!
//! ```text
//! $ hledger -f fixtures/sample.journal bal -e 2026-07-01 --depth 1
//!   ... $-10,677.50  19.5000 AAPL  1.500,00 EUR  5.0 GLD  -2.0 TSLA  17.0 VTI
//! $ hledger -f fixtures/sample.journal bal -e 2026-07-01 --depth 1 --cost
//!   ... 0
//! ```
//!
//! So the exact identity asserted here is
//! `Σ(all five type sections) == Σ over priced postings of (amount − cost)`,
//! which degenerates to the textbook `== 0` for a journal without conversions
//! (pinned separately on `fixtures/reports/invariants-basic.journal`).

mod common;

use ledgeline_core::holdings::{HoldingsScope, ScopeMode, compute_holdings};
use ledgeline_core::model::{Commodity, CostKind, Journal, PriceDirective, Transaction};
use ledgeline_core::reports::{
    AccountType, InsightsOpts, Interval, MixedAmount, NetWorthOpts, PeriodReport, PostingFilter,
    PriceDb, ReportRow, Section, SectionedReport, account_decls, account_totals, add_days,
    balance_sheet, bucket_end, bucket_start, cash_flow, cash_predicate, days_between,
    declared_types, income_statement, infer_market_prices, insights, is_account_type, net_worth,
    next_bucket, value_at,
};
use ledgeline_core::{Dec, parse_journal};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Dates under test
// ---------------------------------------------------------------------------

/// As-of dates spanning the interesting shapes: before any activity, month- and
/// year-aligned, mid-month, the equity-transfer day itself, and the journal's
/// last day.
const AS_OF_DATES: [&str; 7] = [
    "2024-06-30", // before the journal's first transaction (2024-07-01)
    "2024-12-31", // month- AND year-aligned
    "2025-02-14", // mid-month
    "2025-08-20", // the GLD equity transfer's own date
    "2026-02-28", // month-aligned, no conversions in that month
    "2026-06-30", // month-aligned, latest full month
    "2026-07-03", // the journal's last transaction
];

/// Depths to sweep. `0` is hledger's "totals only" (no per-account rows), where
/// RPT-4 found the totals reading zero; `sample.journal`'s deepest account is 4
/// segments (`assets:broker:taxable:aapl`), so 5 and 6 exercise "deeper than
/// anything".
const DEPTHS: [usize; 7] = [0, 1, 2, 3, 4, 5, 6];

/// The five mutually exclusive top-level account types. `Cash` folds into
/// `Asset` and `Gain` into `Revenue` (see `account_types::is_account_type`), so
/// these five partition every classifiable account.
const TYPES: [AccountType; 5] = [
    AccountType::Asset,
    AccountType::Liability,
    AccountType::Equity,
    AccountType::Revenue,
    AccountType::Expense,
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn declared_of(journal: &Journal) -> BTreeMap<String, AccountType> {
    declared_types(&account_decls(journal))
}

/// Parse a fixture by path relative to `fixtures/`. (`common::fixture_journal`
/// only loads `sample.journal`.)
fn journal_fixture(relative: &str) -> Journal {
    let path = common::fixtures_dir().join(relative);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"));
    parse_journal(&text, &path.to_string_lossy())
        .unwrap_or_else(|e| panic!("parse {relative}: {e}"))
}

/// Direct (un-rolled) totals per full account name over `[from, to]`.
fn direct_totals(
    txns: &[Transaction],
    from: Option<&str>,
    to: Option<&str>,
) -> BTreeMap<String, MixedAmount> {
    account_totals(
        txns,
        &PostingFilter {
            from,
            to,
            ..PostingFilter::default()
        },
    )
    .expect("account_totals must not overflow")
}

fn sum_all(totals: &BTreeMap<String, MixedAmount>) -> MixedAmount {
    totals
        .values()
        .try_fold(MixedAmount::new(), |acc, ma| acc.ma_add(ma))
        .expect("summing account totals must not overflow")
}

/// Natural-sign total over every account whose EFFECTIVE type is `ty` — the
/// same membership rule `sections::build_section` and `net_worth` use, computed
/// independently of them.
fn typed_total(
    totals: &BTreeMap<String, MixedAmount>,
    declared: &BTreeMap<String, AccountType>,
    ty: AccountType,
) -> MixedAmount {
    totals
        .iter()
        .filter(|(account, _)| is_account_type(account, declared, ty))
        .try_fold(MixedAmount::new(), |acc, (_, ma)| acc.ma_add(ma))
        .expect("summing a type's totals must not overflow")
}

fn add(a: &MixedAmount, b: &MixedAmount) -> MixedAmount {
    a.ma_add(b).expect("ma_add must not overflow")
}

fn neg(a: &MixedAmount) -> MixedAmount {
    a.ma_neg().expect("ma_neg must not overflow")
}

fn sub(a: &MixedAmount, b: &MixedAmount) -> MixedAmount {
    add(a, &neg(b))
}

/// The cost-equivalent of a priced amount: `quantity × unit price` for `@`, and
/// the written total (signed to follow the amount) for `@@`.
fn cost_total(quantity: Dec, kind: CostKind, price: Dec) -> Dec {
    match kind {
        CostKind::Unit => quantity
            .mul(price)
            .expect("cost multiply must not overflow"),
        CostKind::Total => {
            let magnitude = price.abs().expect("abs must not overflow");
            if quantity.mantissa < 0 {
                magnitude.neg().expect("neg must not overflow")
            } else {
                magnitude
            }
        }
    }
}

/// `Σ over postings in [from, to] of (raw amount − cost-equivalent)`.
///
/// Every transaction balances at COST, so this is exactly the amount by which
/// the raw journal-wide balance departs from zero — see the module docs. Uses
/// the same effective posting date (`posting.date ?? txn.date`) as
/// `account_totals`, so the two are filtered identically.
fn conversion_residual(txns: &[Transaction], from: Option<&str>, to: Option<&str>) -> MixedAmount {
    let mut residual = MixedAmount::new();
    for txn in txns {
        for posting in &txn.postings {
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            if from.is_some_and(|f| date < f) || to.is_some_and(|t| date > t) {
                continue;
            }
            for amount in &posting.amounts {
                let Some(cost) = amount.cost.as_deref() else {
                    continue;
                };
                residual
                    .accumulate(&amount.commodity, amount.quantity)
                    .expect("residual accumulate must not overflow");
                let paid = cost_total(amount.quantity, cost.kind, cost.amount.quantity);
                residual
                    .accumulate(
                        &cost.amount.commodity,
                        paid.neg().expect("neg must not overflow"),
                    )
                    .expect("residual accumulate must not overflow");
            }
        }
    }
    residual.drop_zeros();
    residual
}

/// The price set every valuing report uses: costs inferred from `@`/`@@` first,
/// then the explicit `P` directives (so an explicit price wins a same-date tie).
fn all_prices(journal: &Journal) -> Vec<PriceDirective> {
    let mut all = infer_market_prices(&journal.transactions).expect("inference must not overflow");
    all.extend_from_slice(&journal.prices);
    all
}

fn price_db(journal: &Journal) -> PriceDb {
    PriceDb::build(&all_prices(journal))
}

fn usd() -> Commodity {
    Commodity("$".to_string())
}

/// `inclusive(a) == own(a) + Σ inclusive(direct children of a)` for every row
/// shallower than the report depth (a row AT the depth has had its children
/// clamped away and is skipped).
///
/// Note the `own(a)` term: CLEANUP.md's sketch of this invariant omits it. On
/// `sample.journal` no parent carries direct postings so the two forms agree,
/// but the general identity needs it — `fixtures/reports/invariants-basic.journal`
/// posts directly to a parent precisely to cover that.
fn assert_tree_consistent(
    label: &str,
    rows: &BTreeMap<String, (MixedAmount, MixedAmount)>,
    depth: usize,
) {
    for (account, (own, inclusive)) in rows {
        let account_depth = account.split(':').count();
        if account_depth >= depth {
            continue; // children clamped away — nothing to compare against
        }
        let prefix = format!("{account}:");
        let children = rows
            .iter()
            .filter(|(name, _)| {
                name.starts_with(&prefix) && name.split(':').count() == account_depth + 1
            })
            .try_fold(MixedAmount::new(), |acc, (_, (_, child))| acc.ma_add(child))
            .expect("child sum must not overflow");
        assert_eq!(
            *inclusive,
            add(own, &children),
            "{label}: {account} inclusive != own + Σ children (depth {depth})"
        );
    }
}

/// One SECTION's rows as `account -> (own, inclusive)`.
///
/// Deliberately per-section, never merged across sections: when a parent's
/// declared type disagrees with a child's, hledger clamps WITHIN each section
/// and the same account name appears in BOTH with that section's own subtotal
/// (`fixtures/reports/mixed-subtree.journal` — `assets` shows $1,000.00 under
/// Assets and $300.00 under Liabilities at depth 1). A single map keyed by
/// account name would silently drop one of the two.
fn section_rows(section: &Section) -> BTreeMap<String, (MixedAmount, MixedAmount)> {
    section
        .rows
        .iter()
        .map(
            |ReportRow {
                 account,
                 own,
                 inclusive,
                 ..
             }| (account.clone(), (own.clone(), inclusive.clone())),
        )
        .collect()
}

/// Assert the rollup identity separately in each of a report's sections.
fn assert_sections_roll_up(label: &str, report: &SectionedReport, depth: usize) {
    for section in &report.sections {
        assert_tree_consistent(
            &format!("{label} / {}", section.title),
            &section_rows(section),
            depth,
        );
    }
}

/// The rows of one bucket of a period report, paired with independently
/// recomputed `own` totals for the same account set.
fn period_rows(
    report: &PeriodReport,
    bucket: usize,
    own: &BTreeMap<String, MixedAmount>,
) -> BTreeMap<String, (MixedAmount, MixedAmount)> {
    report
        .rows
        .iter()
        .map(|row| {
            (
                row.account.clone(),
                (
                    own.get(&row.account).cloned().unwrap_or_default(),
                    row.values[bucket].clone(),
                ),
            )
        })
        .collect()
}

// ===========================================================================
// 1. The accounting identity
// ===========================================================================

/// `assets + liabilities + equity + revenues + expenses` (natural signs) equals
/// the conversion residual at every as-of date — the multi-commodity form of
/// "the books balance". Catches any report-wide sign flip or a whole account
/// type going missing.
#[test]
fn accounting_identity_holds_at_every_as_of() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);

    for as_of in AS_OF_DATES {
        let direct = direct_totals(&journal.transactions, None, Some(as_of));
        let five = TYPES
            .iter()
            .map(|ty| typed_total(&direct, &declared, *ty))
            .fold(MixedAmount::new(), |acc, ma| add(&acc, &ma));
        assert_eq!(
            five,
            conversion_residual(&journal.transactions, None, Some(as_of)),
            "accounting identity at {as_of}"
        );
    }
}

/// The five type sections PARTITION the chart of accounts: no account is
/// counted twice and none is dropped. This is the half of the identity that a
/// section-membership bug would break (RPT-1/RPT-2's failure mode), isolated
/// from the conversion residual.
#[test]
fn the_five_type_sections_partition_every_account() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);

    for as_of in AS_OF_DATES {
        let direct = direct_totals(&journal.transactions, None, Some(as_of));

        // Every account lands in exactly one of the five.
        for account in direct.keys() {
            let hits: Vec<&AccountType> = TYPES
                .iter()
                .filter(|ty| is_account_type(account, &declared, **ty))
                .collect();
            assert_eq!(
                hits.len(),
                1,
                "{account} at {as_of} matched {hits:?}, want exactly one type"
            );
        }

        // …and therefore the five totals reassemble the whole journal.
        let five = TYPES
            .iter()
            .map(|ty| typed_total(&direct, &declared, *ty))
            .fold(MixedAmount::new(), |acc, ma| add(&acc, &ma));
        assert_eq!(
            five,
            sum_all(&direct),
            "type totals reassemble all accounts at {as_of}"
        );
    }
}

/// The textbook identity, on a journal with no cost conversions: every type
/// total sums to exactly zero, and a single-commodity balance sheet's
/// `grand_total` is exactly the negated equity + income statement net.
#[test]
fn accounting_identity_is_exactly_zero_without_conversions() {
    let journal = journal_fixture("reports/invariants-basic.journal");
    let declared = declared_of(&journal);

    for as_of in ["2026-01-31", "2026-02-14", "2026-03-31", "2026-12-31"] {
        let direct = direct_totals(&journal.transactions, None, Some(as_of));
        let five = TYPES
            .iter()
            .map(|ty| typed_total(&direct, &declared, *ty))
            .fold(MixedAmount::new(), |acc, ma| add(&acc, &ma));
        assert_eq!(five, MixedAmount::new(), "A+L+E+R+X == 0 at {as_of}");

        // bs.grand_total (= assets + liabilities, natural) must equal
        // −(equity) + is.grand_total (= −(revenues + expenses)).
        let bs = balance_sheet(&journal.transactions, as_of, 9, &declared).expect("bs");
        let is =
            income_statement(&journal.transactions, "0000-01-01", as_of, 9, &declared).expect("is");
        let equity = typed_total(&direct, &declared, AccountType::Equity);
        assert_eq!(
            bs.grand_total,
            add(&neg(&equity), &is.grand_total),
            "bs.grand_total == −equity + is.net at {as_of}"
        );
    }
}

// ===========================================================================
// 2. Section totals agree with independently computed type totals
// ===========================================================================

/// `balance_sheet` and `income_statement` sections must contain exactly the
/// accounts of their type, with hledger's display flip applied to liabilities
/// and revenues only.
#[test]
fn report_sections_equal_independently_typed_totals() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);

    for as_of in AS_OF_DATES {
        let direct = direct_totals(&journal.transactions, None, Some(as_of));
        let bs = balance_sheet(&journal.transactions, as_of, 4, &declared).expect("bs");
        assert_eq!(
            bs.sections[0].total,
            typed_total(&direct, &declared, AccountType::Asset),
            "Assets section total at {as_of}"
        );
        assert_eq!(
            bs.sections[1].total,
            neg(&typed_total(&direct, &declared, AccountType::Liability)),
            "Liabilities section total (displayed flipped) at {as_of}"
        );
        // grand_total = assets(natural) + liabilities(natural).
        assert_eq!(
            bs.grand_total,
            add(
                &typed_total(&direct, &declared, AccountType::Asset),
                &typed_total(&direct, &declared, AccountType::Liability)
            ),
            "bs.grand_total at {as_of}"
        );

        let range = direct_totals(&journal.transactions, Some("2024-01-01"), Some(as_of));
        let is =
            income_statement(&journal.transactions, "2024-01-01", as_of, 4, &declared).expect("is");
        assert_eq!(
            is.sections[0].total,
            neg(&typed_total(&range, &declared, AccountType::Revenue)),
            "Revenues section total (displayed flipped) at {as_of}"
        );
        assert_eq!(
            is.sections[1].total,
            typed_total(&range, &declared, AccountType::Expense),
            "Expenses section total at {as_of}"
        );
        assert_eq!(
            is.grand_total,
            neg(&add(
                &typed_total(&range, &declared, AccountType::Revenue),
                &typed_total(&range, &declared, AccountType::Expense)
            )),
            "is.grand_total == −(revenues + expenses) at {as_of}"
        );
    }
}

// ===========================================================================
// 3. Depth rollup — directly exercises `aggregate::roll_up`
// ===========================================================================

/// Every report's total is depth-independent (hledger's totals do not move with
/// `--depth`), and every parent row equals its own postings plus its children.
#[test]
fn balance_sheet_totals_are_depth_independent_and_rows_roll_up() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);

    for as_of in AS_OF_DATES {
        let baseline = balance_sheet(&journal.transactions, as_of, 1, &declared).expect("bs");
        for depth in DEPTHS {
            let report = balance_sheet(&journal.transactions, as_of, depth, &declared).expect("bs");
            assert_eq!(
                report.grand_total, baseline.grand_total,
                "bs grand_total at {as_of} depth {depth}"
            );
            for (i, section) in report.sections.iter().enumerate() {
                assert_eq!(
                    section.total, baseline.sections[i].total,
                    "bs {} total at {as_of} depth {depth}",
                    section.title
                );
            }
            assert_sections_roll_up(&format!("bs {as_of}"), &report, depth);
        }
    }
}

#[test]
fn income_statement_totals_are_depth_independent_and_rows_roll_up() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);

    for to in AS_OF_DATES {
        let baseline =
            income_statement(&journal.transactions, "2024-01-01", to, 1, &declared).expect("is");
        for depth in DEPTHS {
            let report =
                income_statement(&journal.transactions, "2024-01-01", to, depth, &declared)
                    .expect("is");
            assert_eq!(
                report.grand_total, baseline.grand_total,
                "is grand_total to {to} depth {depth}"
            );
            assert_sections_roll_up(&format!("is {to}"), &report, depth);
        }
    }
}

#[test]
fn net_worth_totals_are_depth_independent_and_rows_roll_up() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);
    let db = price_db(&journal);
    let target = usd();

    for as_of in AS_OF_DATES {
        let opts = |depth: usize| NetWorthOpts {
            end: as_of,
            interval: Interval::Monthly,
            count: 1,
            depth,
            value_in: None,
            declared: &declared,
        };
        let baseline =
            net_worth(&journal.transactions, &journal.prices, &opts(1)).expect("net_worth");

        // `own` for an asset/liability account, valued the same way the report
        // values its rows — so the tree identity can be checked on the valued
        // numbers the caller actually sees.
        let direct = direct_totals(&journal.transactions, None, Some(as_of));
        let own: BTreeMap<String, MixedAmount> = direct
            .iter()
            .filter(|(account, _)| {
                is_account_type(account, &declared, AccountType::Asset)
                    || is_account_type(account, &declared, AccountType::Liability)
            })
            .map(|(account, ma)| {
                let qty = value_at(ma, &target, &db, as_of, None).expect("value_at");
                (account.clone(), MixedAmount::single(target.clone(), qty))
            })
            .collect();

        for depth in DEPTHS {
            let report =
                net_worth(&journal.transactions, &journal.prices, &opts(depth)).expect("net_worth");
            assert_eq!(
                report.totals, baseline.totals,
                "net worth total at {as_of} depth {depth}"
            );
            assert_tree_consistent(
                &format!("net worth {as_of}"),
                &period_rows(&report, 0, &own),
                depth,
            );
        }
    }
}

#[test]
fn cash_flow_totals_are_depth_independent_and_rows_roll_up() {
    let journal = common::fixture_journal();
    let decls = account_decls(&journal);
    let is_cash = cash_predicate(&decls);

    for end in AS_OF_DATES {
        let baseline = cash_flow(
            &journal.transactions,
            end,
            Interval::Monthly,
            3,
            1,
            Some(&is_cash),
        )
        .expect("cash_flow");
        for depth in DEPTHS {
            let report = cash_flow(
                &journal.transactions,
                end,
                Interval::Monthly,
                3,
                depth,
                Some(&is_cash),
            )
            .expect("cash_flow");
            assert_eq!(
                report.totals, baseline.totals,
                "cash flow totals at {end} depth {depth}"
            );
            for (bucket, key) in report.buckets.iter().enumerate() {
                let start = bucket_start(key).expect("bucket_start");
                let bucket_end = bucket_end(key).expect("bucket_end");
                let to = if end < bucket_end.as_str() {
                    end
                } else {
                    bucket_end.as_str()
                };
                let own: BTreeMap<String, MixedAmount> =
                    direct_totals(&journal.transactions, Some(&start), Some(to))
                        .into_iter()
                        .filter(|(account, _)| is_cash(account))
                        .collect();
                assert_tree_consistent(
                    &format!("cash flow {end} bucket {key}"),
                    &period_rows(&report, bucket, &own),
                    depth,
                );
            }
        }
    }
}

/// The same rollup identity where a PARENT carries direct postings of its own —
/// the case `sample.journal` cannot cover and CLEANUP.md's sketch of this
/// invariant would have got wrong.
#[test]
fn rows_roll_up_when_a_parent_has_its_own_postings() {
    let journal = journal_fixture("reports/invariants-basic.journal");
    let declared = declared_of(&journal);

    let report = balance_sheet(&journal.transactions, "2026-03-31", 3, &declared).expect("bs");
    assert_sections_roll_up("parent-postings bs", &report, 3);
    let rows = section_rows(&report.sections[0]);

    // The fixture must actually exercise the case, or the test proves nothing.
    let (own, _) = rows.get("assets:bank").expect("assets:bank row present");
    assert!(
        !own.is_zero(),
        "fixture must post directly to the parent assets:bank"
    );
}

// ===========================================================================
// 4. Net worth reconciles with the balance sheet
// ===========================================================================

/// `net_worth(D).totals[0]` is the balance sheet's `grand_total` valued at the
/// same date. Both are "assets + liabilities in natural signs"; net worth then
/// values it in the price base. Catches a net-worth membership drift away from
/// the balance sheet's Asset/Liability sections.
#[test]
fn net_worth_total_equals_the_valued_balance_sheet_grand_total() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);
    let db = price_db(&journal);
    let target = usd();

    for as_of in AS_OF_DATES {
        let bs = balance_sheet(&journal.transactions, as_of, 1, &declared).expect("bs");
        let nw = net_worth(
            &journal.transactions,
            &journal.prices,
            &NetWorthOpts {
                end: as_of,
                interval: Interval::Monthly,
                count: 1,
                depth: 1,
                value_in: None,
                declared: &declared,
            },
        )
        .expect("net_worth");

        let valued = value_at(&bs.grand_total, &target, &db, as_of, None).expect("value_at");
        let expected = if valued.is_zero() {
            MixedAmount::new()
        } else {
            MixedAmount::single(target.clone(), valued)
        };
        assert_eq!(
            nw.totals[0], expected,
            "net worth == valued bs.grand_total at {as_of}"
        );
    }
}

/// With `value_in` naming a commodity nothing prices INTO, the report must skip
/// rather than guess — and the unvalued balance sheet is untouched. Pins that
/// the two reports disagree only through valuation, never through membership.
#[test]
fn net_worth_and_balance_sheet_agree_before_any_valuation() {
    let journal = journal_fixture("reports/invariants-basic.journal");
    let declared = declared_of(&journal);

    // This fixture declares no prices and uses no `@` costs, so `net_worth` has
    // no valuation target at all and reports raw balances — directly comparable
    // to the balance sheet's `grand_total`.
    for as_of in ["2026-01-31", "2026-02-14", "2026-03-31"] {
        let bs = balance_sheet(&journal.transactions, as_of, 1, &declared).expect("bs");
        let nw = net_worth(
            &journal.transactions,
            &journal.prices,
            &NetWorthOpts {
                end: as_of,
                interval: Interval::Monthly,
                count: 1,
                depth: 1,
                value_in: None,
                declared: &declared,
            },
        )
        .expect("net_worth");
        assert_eq!(
            nw.totals[0], bs.grand_total,
            "unvalued net worth == bs.grand_total at {as_of}"
        );
    }
}

// ===========================================================================
// 5. Income statement net == balance sheet delta
// ===========================================================================

/// Over a window with no equity postings and no cost conversions, the period's
/// net income is exactly the change in `assets + liabilities`.
///
/// Verified against hledger:
/// ```text
/// $ hledger -f fixtures/sample.journal bal type:AL -e 2026-03-01 --depth 1 → $41,188.44
/// $ hledger -f fixtures/sample.journal bal type:AL -e 2026-02-01 --depth 1 → $39,245.09
/// $ hledger -f fixtures/sample.journal is -b 2026-02-01 -e 2026-03-01      → Net: $1,943.35
/// ```
#[test]
fn income_statement_net_equals_balance_sheet_delta_over_a_clean_window() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);

    // (previous close, window start, window end) — each window free of equity
    // postings AND of `@`/`@@` conversions.
    for (before, from, to) in [
        ("2025-11-30", "2025-12-01", "2025-12-31"),
        ("2026-01-31", "2026-02-01", "2026-02-28"),
        ("2026-01-31", "2026-02-01", "2026-02-14"), // mid-month close
    ] {
        // The window really is clean, or the assertion below is vacuous.
        let window = direct_totals(&journal.transactions, Some(from), Some(to));
        assert!(
            typed_total(&window, &declared, AccountType::Equity).is_zero(),
            "no equity postings in {from}..{to}"
        );
        assert!(
            conversion_residual(&journal.transactions, Some(from), Some(to)).is_zero(),
            "no cost conversions in {from}..{to}"
        );

        let opening = balance_sheet(&journal.transactions, before, 1, &declared).expect("bs");
        let closing = balance_sheet(&journal.transactions, to, 1, &declared).expect("bs");
        let is = income_statement(&journal.transactions, from, to, 1, &declared).expect("is");
        assert_eq!(
            sub(&closing.grand_total, &opening.grand_total),
            is.grand_total,
            "bs delta == is net over {from}..{to}"
        );
    }
}

/// The general form, over a window that DOES contain conversions: the balance
/// sheet delta departs from net income by exactly the conversion residual (and
/// by the equity movement, which is zero here).
#[test]
fn balance_sheet_delta_equals_net_income_plus_conversion_residual() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);

    // 2025-09-01 .. 2026-06-30: after the 2025-08-20 GLD equity transfer, so no
    // equity postings; contains the VTI/NVDA/AAPL/EUR/TSLA conversions.
    let (before, from, to) = ("2025-08-31", "2025-09-01", "2026-06-30");
    let window = direct_totals(&journal.transactions, Some(from), Some(to));
    assert!(
        typed_total(&window, &declared, AccountType::Equity).is_zero(),
        "no equity postings in the window"
    );
    let residual = conversion_residual(&journal.transactions, Some(from), Some(to));
    assert!(!residual.is_zero(), "window must contain conversions");

    let opening = balance_sheet(&journal.transactions, before, 1, &declared).expect("bs");
    let closing = balance_sheet(&journal.transactions, to, 1, &declared).expect("bs");
    let is = income_statement(&journal.transactions, from, to, 1, &declared).expect("is");
    assert_eq!(
        sub(&closing.grand_total, &opening.grand_total),
        add(&is.grand_total, &residual),
        "bs delta == is net + conversion residual over {from}..{to}"
    );
}

// ===========================================================================
// 6. Buckets sum to the range they cover
// ===========================================================================

/// The month number of `date` (1-12).
fn month_of(date: &str) -> usize {
    date[5..7].parse().expect("ISO month")
}

/// Finer buckets covering the SAME range sum to the coarser bucket's total —
/// including when `end` falls mid-month, where both the last monthly bucket and
/// the single yearly bucket are truncated at `end`.
#[test]
fn monthly_buckets_sum_to_the_yearly_bucket_over_the_same_range() {
    let journal = common::fixture_journal();
    let decls = account_decls(&journal);
    let is_cash = cash_predicate(&decls);

    for end in [
        "2026-02-28",
        "2026-06-30",
        "2026-02-14",
        "2026-07-03",
        "2024-12-31",
    ] {
        let months = month_of(end); // Jan 1 of `end`'s year through `end`
        let monthly = cash_flow(
            &journal.transactions,
            end,
            Interval::Monthly,
            months,
            1,
            Some(&is_cash),
        )
        .expect("cash_flow monthly");
        let yearly = cash_flow(
            &journal.transactions,
            end,
            Interval::Yearly,
            1,
            1,
            Some(&is_cash),
        )
        .expect("cash_flow yearly");

        let summed = monthly
            .totals
            .iter()
            .fold(MixedAmount::new(), |acc, ma| add(&acc, ma));
        assert_eq!(
            summed, yearly.totals[0],
            "Σ {months} monthly buckets == the yearly bucket at {end}"
        );
    }
}

/// Quarterly is consistent with monthly the same way.
#[test]
fn monthly_buckets_sum_to_the_quarterly_bucket_over_the_same_range() {
    let journal = common::fixture_journal();
    let decls = account_decls(&journal);
    let is_cash = cash_predicate(&decls);

    for (end, months) in [("2026-06-30", 3), ("2026-02-14", 2), ("2026-03-31", 3)] {
        let monthly = cash_flow(
            &journal.transactions,
            end,
            Interval::Monthly,
            months,
            1,
            Some(&is_cash),
        )
        .expect("cash_flow monthly");
        let quarterly = cash_flow(
            &journal.transactions,
            end,
            Interval::Quarterly,
            1,
            1,
            Some(&is_cash),
        )
        .expect("cash_flow quarterly");
        let summed = monthly
            .totals
            .iter()
            .fold(MixedAmount::new(), |acc, ma| add(&acc, ma));
        assert_eq!(
            summed, quarterly.totals[0],
            "Σ {months} monthly buckets == the quarterly bucket at {end}"
        );
    }
}

/// PINNED SEMANTICS, not a bug. CLEANUP.md sketches this invariant as
/// `Σ cash_flow(end, Monthly, 12) == cash_flow(end, Yearly, 1)` and notes it
/// "currently fails for unaligned ends". It does — but because the two calls
/// deliberately cover DIFFERENT ranges, not because of a rollup error:
///
/// - `count` is a number of buckets ending with the bucket containing `end`, so
///   `Monthly, 12` at 2026-06-30 spans 2025-07-01..2026-06-30.
/// - `Yearly, 1` spans only 2026-01-01..2026-06-30 (its bucket start, truncated
///   at `end`).
///
/// They coincide exactly when `end` closes a December. This test pins both
/// facts, so the semantics are locked rather than accidental; the
/// same-range version is `monthly_buckets_sum_to_the_yearly_bucket_over_the_same_range`.
#[test]
fn twelve_monthly_buckets_span_a_rolling_year_not_the_calendar_year() {
    let journal = common::fixture_journal();
    let decls = account_decls(&journal);
    let is_cash = cash_predicate(&decls);

    let sum_monthly = |end: &str, count: usize| {
        cash_flow(
            &journal.transactions,
            end,
            Interval::Monthly,
            count,
            1,
            Some(&is_cash),
        )
        .expect("cash_flow")
        .totals
        .iter()
        .fold(MixedAmount::new(), |acc, ma| add(&acc, ma))
    };
    let yearly = |end: &str| {
        cash_flow(
            &journal.transactions,
            end,
            Interval::Yearly,
            1,
            1,
            Some(&is_cash),
        )
        .expect("cash_flow")
        .totals[0]
            .clone()
    };

    // December end: the twelve monthly buckets ARE the calendar year.
    assert_eq!(
        sum_monthly("2024-12-31", 12),
        yearly("2024-12-31"),
        "at a December close, Monthly×12 == Yearly×1"
    );

    // Mid-year end: they differ, and the difference is exactly the months of the
    // PREVIOUS calendar year that the rolling window reaches back into.
    let rolling = sum_monthly("2026-06-30", 12); // 2025-07-01..2026-06-30
    let calendar = yearly("2026-06-30"); // 2026-01-01..2026-06-30
    assert_ne!(
        rolling, calendar,
        "at a mid-year close the two windows are NOT the same range"
    );
    let prior_half = sum_monthly("2025-12-31", 6); // 2025-07-01..2025-12-31
    assert_eq!(
        rolling,
        add(&calendar, &prior_half),
        "Monthly×12 == Yearly×1 + the previous year's tail"
    );
}

/// Net worth is cumulative, so its buckets do not sum — but each bucket must
/// equal the balance sheet at that bucket's (possibly truncated) close.
#[test]
fn net_worth_buckets_equal_the_balance_sheet_at_each_bucket_close() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);
    let db = price_db(&journal);
    let target = usd();

    for end in ["2026-06-30", "2026-02-14", "2026-07-03"] {
        let report = net_worth(
            &journal.transactions,
            &journal.prices,
            &NetWorthOpts {
                end,
                interval: Interval::Monthly,
                count: 6,
                depth: 1,
                value_in: None,
                declared: &declared,
            },
        )
        .expect("net_worth");

        for (i, key) in report.buckets.iter().enumerate() {
            let close = bucket_end(key).expect("bucket_end");
            let as_of = if end < close.as_str() {
                end
            } else {
                close.as_str()
            };
            let bs = balance_sheet(&journal.transactions, as_of, 1, &declared).expect("bs");
            let valued = value_at(&bs.grand_total, &target, &db, as_of, None).expect("value_at");
            let expected = if valued.is_zero() {
                MixedAmount::new()
            } else {
                MixedAmount::single(target.clone(), valued)
            };
            assert_eq!(
                report.totals[i], expected,
                "net worth bucket {key} (as of {as_of}) == valued bs.grand_total"
            );
        }
    }
}

// ===========================================================================
// 7. Documented divergence: the final bucket truncates at `end`
// ===========================================================================

/// PINNED, DELIBERATE DIVERGENCE (CLEANUP.md "INFO").
///
/// `hledger bal -M -e DATE` WIDENS the report period to whole intervals, so a
/// mid-month `-e` still reports the WHOLE final month. Ledgeline truncates the
/// final bucket at `end` (`cash_flow.rs:50-54`, `net_worth.rs:116-120`,
/// `budget.rs:220-224`) — better for a live dashboard, and what
/// `hledger bal -e` (no interval) already does.
///
/// Concretely, at `end = 2026-02-16`:
/// ```text
/// $ hledger -f fixtures/sample.journal bal type:C -M -e 2026-02-16 --depth 1
///     ... 2026-02 column: $1,923.16        # widened to 2026-02-28
/// $ hledger -f fixtures/sample.journal bal type:C -b 2026-02-01 -e 2026-02-17 --depth 1
///     $-2,276.84                            # truncated at 2026-02-16 — ledgeline
/// ```
/// The $4,200.00 gap is the 2026-02-27 salary deposit, which falls after `end`.
#[test]
fn interval_reports_truncate_the_final_bucket_at_end_unlike_hledger_m_e() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);
    let decls = account_decls(&journal);
    let is_cash = cash_predicate(&decls);

    let cf = cash_flow(
        &journal.transactions,
        "2026-02-16",
        Interval::Monthly,
        2,
        1,
        Some(&is_cash),
    )
    .expect("cash_flow");
    assert_eq!(cf.buckets, ["2026-01", "2026-02"]);

    // Ledgeline: 2026-02-01..2026-02-16 only.
    let truncated = MixedAmount::single(usd(), Dec::new(-227_684, 2));
    assert_eq!(
        cf.totals[1], truncated,
        "final cash-flow bucket truncates at end (hledger -M -e would widen to $1,923.16)"
    );

    // The widened month hledger would report, for contrast.
    let widened = cash_flow(
        &journal.transactions,
        "2026-02-28",
        Interval::Monthly,
        1,
        1,
        Some(&is_cash),
    )
    .expect("cash_flow");
    assert_eq!(
        widened.totals[0],
        MixedAmount::single(usd(), Dec::new(192_316, 2)),
        "the whole month is $1,923.16 — the $4,200.00 salary lands 2026-02-27"
    );

    // Net worth truncates identically: its final bucket is the balance as of
    // `end`, not as of the month end.
    let nw = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-02-16",
            interval: Interval::Monthly,
            count: 2,
            depth: 1,
            value_in: None,
            declared: &declared,
        },
    )
    .expect("net_worth");
    let as_of_end = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-02-16",
            interval: Interval::Daily,
            count: 1,
            depth: 1,
            value_in: None,
            declared: &declared,
        },
    )
    .expect("net_worth");
    assert_eq!(
        nw.totals[1], as_of_end.totals[0],
        "net worth's final monthly bucket is the balance at `end`, not at month end"
    );

    let month_end = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-02-28",
            interval: Interval::Monthly,
            count: 1,
            depth: 1,
            value_in: None,
            declared: &declared,
        },
    )
    .expect("net_worth");
    assert_ne!(
        nw.totals[1], month_end.totals[0],
        "…and it genuinely differs from the month-end balance hledger -M -e would show"
    );
}

// ===========================================================================
// 8. Holdings reconcile with the balance sheet
// ===========================================================================

/// `compute_holdings` over `assets:broker`, using the SAME price set the
/// valuing reports use (costs inferred from `@`/`@@`, then explicit `P`).
fn broker_holdings(journal: &Journal, as_of: &str) -> ledgeline_core::holdings::HoldingsReport {
    let scope = HoldingsScope {
        accounts: ["assets:broker".to_string()].into_iter().collect(),
        mode: ScopeMode::Include,
        as_of: as_of.to_string(),
        gain_since: None,
        value_in: None,
    };
    compute_holdings(
        &journal.transactions,
        &all_prices(journal),
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
    )
    .expect("compute_holdings")
}

/// The balance sheet's `assets:broker` subtree at `as_of` (natural signs, raw
/// commodities).
fn broker_row(journal: &Journal, as_of: &str) -> MixedAmount {
    let declared = declared_of(journal);
    balance_sheet(&journal.transactions, as_of, 2, &declared)
        .expect("bs")
        .sections[0]
        .rows
        .iter()
        .find(|row| row.account == "assets:broker")
        .expect("assets:broker row")
        .inclusive
        .clone()
}

/// `Σ holdings[i].market_value + broker cash == the valued balance-sheet
/// `assets:broker` subtree` — and every holding's share count is exactly the
/// balance-sheet quantity for that commodity.
///
/// Cross-checked against hledger at 2025-12-31:
/// ```text
/// $ hledger -f fixtures/sample.journal bal assets:broker -e 2026-01-01 \
///       --infer-market-prices --value=end,'$' --depth 2
///     $16,735.50  assets:broker
/// ```
#[test]
fn holdings_reconcile_with_the_balance_sheet_broker_subtree() {
    let journal = common::fixture_journal();
    let db = price_db(&journal);
    let target = usd();

    // Dates before the 2026-06-22 "sell TSLA that was never bought" record, so
    // every position is genuinely held (see the companion test below).
    for as_of in ["2025-06-30", "2025-12-31"] {
        let holdings = broker_holdings(&journal, as_of);
        let row = broker_row(&journal, as_of);

        // Structural tie: each reported holding's shares are the ledger balance.
        for holding in &holdings.holdings {
            assert_eq!(
                Some(holding.shares),
                row.get(&Commodity(holding.symbol.clone())),
                "{} shares at {as_of} match the balance sheet",
                holding.symbol
            );
        }

        let cash = row.get(&target).unwrap_or(Dec::zero());
        let market = holdings
            .holdings
            .iter()
            .filter_map(|holding| holding.market_value)
            .try_fold(Dec::zero(), |acc, value| acc.add(value))
            .expect("summing market values must not overflow");
        assert_eq!(
            market.add(cash).expect("add"),
            value_at(&row, &target, &db, as_of, None).expect("value_at"),
            "Σ holdings market value + cash == valued assets:broker at {as_of}"
        );
    }
}

/// PINNED DIVERGENCE (a real reconciliation gap, reported under RPT-5).
///
/// At 2026-06-30 the two reports disagree by exactly **$630.00**:
///
/// | side | figure |
/// |---|---|
/// | `Σ holdings.market_value + cash` | `$18,162.375` |
/// | valued balance-sheet `assets:broker` | `$17,532.375` |
///
/// The gap is the 2026-06-22 `assets:broker:taxable:tsla -2 TSLA @ $315.00`
/// record — a sale of a position that was never opened. The balance sheet
/// carries the resulting `-2.0 TSLA` and values it at `-$630.00`; the holdings
/// engine hides any non-positive position (emitting `NegativeShares`) and so
/// contributes nothing. hledger sides with the balance sheet:
///
/// ```text
/// $ hledger -f fixtures/sample.journal bal assets:broker -e 2026-07-01 \
///       --infer-market-prices --value=end,'$' --depth 2
///     $17,532.38  assets:broker
/// ```
///
/// Hiding the row is defensible (it is not a position anyone holds), but the
/// warning does not say how much value is being withheld, so a dashboard shows
/// a portfolio total and a net worth that are $630 apart with nothing tying
/// them together. This test pins the size and the cause, so the gap cannot
/// change silently.
#[test]
fn holdings_omit_short_positions_that_the_balance_sheet_still_values() {
    let journal = common::fixture_journal();
    let db = price_db(&journal);
    let target = usd();
    let as_of = "2026-06-30";

    let holdings = broker_holdings(&journal, as_of);
    let row = broker_row(&journal, as_of);

    // The short position exists on the balance sheet…
    let tsla = Commodity("TSLA".to_string());
    assert_eq!(row.get(&tsla), Some(Dec::new(-2, 0)));
    // …and is absent from holdings, with a warning but no amount.
    assert!(
        !holdings.holdings.iter().any(|h| h.symbol == "TSLA"),
        "holdings hides the short TSLA row"
    );
    assert!(
        holdings.warnings.iter().any(|w| w.symbol == "TSLA"
            && w.kind == ledgeline_core::holdings::WarningKind::NegativeShares),
        "…and warns about it"
    );

    let cash = row.get(&target).unwrap_or(Dec::zero());
    let market = holdings
        .holdings
        .iter()
        .filter_map(|holding| holding.market_value)
        .try_fold(Dec::zero(), |acc, value| acc.add(value))
        .expect("summing market values must not overflow");
    let balance_sheet_value = value_at(&row, &target, &db, as_of, None).expect("value_at");

    // The gap is exactly the value the balance sheet assigns the hidden lot.
    let hidden = MixedAmount::single(tsla, Dec::new(-2, 0));
    let hidden_value = value_at(&hidden, &target, &db, as_of, None).expect("value_at");
    assert_eq!(hidden_value, Dec::new(-630, 0), "-2 TSLA @ $315.00");
    assert_eq!(
        market.add(cash).expect("add"),
        balance_sheet_value.sub(hidden_value).expect("sub"),
        "holdings exceed the valued balance sheet by exactly the hidden short position"
    );

    // The concrete numbers, so a change in either shows up as a diff.
    assert_eq!(market.add(cash).expect("add"), Dec::new(18_162_375, 3));
    assert_eq!(balance_sheet_value, Dec::new(17_532_375, 3));
}

/// GLD — flagged in CLEANUP.md as a $1,000 dashboard discrepancy — reconciles
/// perfectly once holdings is given the SAME price set the valuing reports use.
///
/// The 2025-08-20 gift carries no cost on the GLD leg; its counter-leg is
/// `equity:transfers $-1,000.00 @ 0.005 GLD`, from which `infer_market_prices`
/// derives the reverse edge `GLD = $200`. So the discrepancy is in WHICH prices
/// the caller hands the holdings engine, not in the engine's math: the basis is
/// (correctly) unknown, but the market value is not.
#[test]
fn the_unpriced_gld_gift_is_valued_once_inferred_prices_are_supplied() {
    let journal = common::fixture_journal();
    let holdings = broker_holdings(&journal, "2026-06-30");
    let gld = holdings
        .holdings
        .iter()
        .find(|holding| holding.symbol == "GLD")
        .expect("GLD is reported");

    assert_eq!(gld.shares, Dec::new(5, 0));
    assert_eq!(gld.market_value, Some(Dec::new(1000, 0)), "5 GLD × $200");
    assert_eq!(gld.basis, None, "the gift carries no cost annotation");
    assert!(
        holdings
            .warnings
            .iter()
            .any(|w| w.symbol == "GLD"
                && w.kind == ledgeline_core::holdings::WarningKind::MissingBasis),
        "basis is still (correctly) flagged as unknown"
    );
}

// ===========================================================================
// 9. Buckets TILE their range — no gaps, no double counting
// ===========================================================================

/// Summing enough cash-flow buckets to reach back past the journal's first
/// transaction reproduces the closing cash balance exactly, for EVERY interval.
///
/// This is the strongest statement available about `periods::{bucket_start,
/// bucket_end, last_n_buckets}`: a one-day gap between consecutive buckets, or
/// a one-day overlap, would drop or double-count a posting and show up here.
/// Weekly is the interesting case — ISO weeks straddle month and year
/// boundaries, so weekly buckets are the only ones that do not nest.
#[test]
fn cash_flow_buckets_tile_their_range_for_every_interval() {
    let journal = common::fixture_journal();
    let decls = account_decls(&journal);
    let is_cash = cash_predicate(&decls);

    for end in ["2025-01-01", "2026-02-14", "2026-06-30", "2026-07-03"] {
        // The closing cash balance, straight from the postings — computed
        // without any bucket math at all.
        let closing = direct_totals(&journal.transactions, None, Some(end))
            .into_iter()
            .filter(|(account, _)| is_cash(account))
            .fold(MixedAmount::new(), |acc, (_, ma)| add(&acc, &ma));
        assert!(!closing.is_zero(), "the fixture holds cash at {end}");

        // Counts chosen to reach back before 2024-07-01, the journal's first day.
        for (interval, count) in [
            (Interval::Yearly, 5_usize),
            (Interval::Quarterly, 20),
            (Interval::Monthly, 40),
            (Interval::Weekly, 170),
            (Interval::Daily, 1200),
        ] {
            let report = cash_flow(
                &journal.transactions,
                end,
                interval,
                count,
                1,
                Some(&is_cash),
            )
            .expect("cash_flow");
            let summed = report
                .totals
                .iter()
                .fold(MixedAmount::new(), |acc, ma| add(&acc, ma));
            assert_eq!(
                summed, closing,
                "Σ {count} {interval:?} buckets ending {end} == the closing cash balance"
            );
        }
    }
}

// ===========================================================================
// 10. Insights reconcile with the underlying reports at ANY span
// ===========================================================================

/// Generalises `report_endpoints.rs::insights_reconciles_with_income_statement_and_networth`
/// off its single month-aligned 24-month span.
///
/// The reconciliation is against the sub-periods the insights report ITSELF
/// publishes (`period.prev_start` … `period.curr_end`), so it holds whatever
/// the split rule is — an unaligned or odd-length span still has to add up.
/// That deliberately does NOT paper over CLEANUP.md's split-bias and
/// month-counting findings: it pins that the boxes agree with the reports for
/// whatever window was chosen, which is a separate question from whether the
/// window is fair.
#[test]
fn insights_reconcile_with_income_statement_and_net_worth_at_every_span() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);

    for (start, end) in [
        ("2024-07-01", "2026-06-30"), // month-aligned, even 12/12 split
        ("2025-01-01", "2026-06-30"), // 18 months → odd-length split
        ("2025-02-14", "2026-07-03"), // mid-month at both ends
        ("2026-01-01", "2026-01-31"), // one month; 16/15-day split
    ] {
        let report = insights(
            &journal,
            &InsightsOpts {
                start,
                end,
                cost_exclude: &[],
                change_min: Dec::zero(),
            },
        )
        .expect("insights");
        let period = &report.period;

        // The two sub-periods partition the span exactly once.
        assert_eq!(period.start, start);
        assert_eq!(period.end, end);
        assert_eq!(period.prev_start, period.start);
        assert_eq!(period.curr_end, period.end);
        assert_eq!(period.prev_end, period.mid);
        assert_eq!(add_days(&period.mid, 1), period.curr_start);
        assert_eq!(
            u64::from(period.prev_days) + u64::from(period.curr_days),
            u64::try_from(days_between(start, end) + 1).expect("non-negative span"),
            "prev_days + curr_days covers the span exactly once ({start}..{end})"
        );

        // Revenue / expenses per sub-period == the income statement over it.
        for (label, from, to, revenue, expenses) in [
            (
                "previous",
                &period.prev_start,
                &period.prev_end,
                &report.revenue.previous,
                &report.expenses.previous,
            ),
            (
                "current",
                &period.curr_start,
                &period.curr_end,
                &report.revenue.current,
                &report.expenses.current,
            ),
        ] {
            let is = income_statement(&journal.transactions, from, to, 1, &declared).expect("is");
            assert_eq!(
                *revenue, is.sections[0].total,
                "{label} revenue == income statement Revenues over {from}..{to}"
            );
            assert_eq!(
                *expenses, is.sections[1].total,
                "{label} expenses == income statement Expenses over {from}..{to}"
            );
        }

        // Deltas are exactly current − previous.
        assert_eq!(
            report.revenue.delta,
            sub(&report.revenue.current, &report.revenue.previous),
            "revenue delta ({start}..{end})"
        );
        assert_eq!(
            report.expenses.delta,
            sub(&report.expenses.current, &report.expenses.previous),
            "expenses delta ({start}..{end})"
        );
        assert_eq!(
            report.net_worth.delta,
            sub(&report.net_worth.current, &report.net_worth.previous),
            "net worth delta ({start}..{end})"
        );

        // Net worth at each sub-period end == the net-worth report there.
        for (label, as_of, reported) in [
            ("previous", &period.prev_end, &report.net_worth.previous),
            ("current", &period.curr_end, &report.net_worth.current),
        ] {
            let nw = net_worth(
                &journal.transactions,
                &journal.prices,
                &NetWorthOpts {
                    end: as_of,
                    interval: Interval::Monthly,
                    count: 1,
                    depth: 1,
                    value_in: None,
                    declared: &declared,
                },
            )
            .expect("net_worth");
            assert_eq!(
                *reported, nw.totals[0],
                "{label} net worth == the net-worth report at {as_of}"
            );
        }

        // Cost of living is the expense total minus nothing (empty exclusions),
        // so it must equal the expenses box exactly.
        assert_eq!(
            report.cost_of_living.current_total, report.expenses.current,
            "cost of living (no exclusions) == expenses, current ({start}..{end})"
        );
        assert_eq!(
            report.cost_of_living.previous_total, report.expenses.previous,
            "cost of living (no exclusions) == expenses, previous ({start}..{end})"
        );
    }
}

// ===========================================================================
// 11. The same invariants over the charts of accounts that broke before
// ===========================================================================

/// Run the identity, the partition and the per-section rollup over every
/// awkward chart of accounts in `fixtures/` — the ones RPT-1 and RPT-2 were
/// found on.
///
/// * `nested-types.journal` — every typed account sits BELOW depth 1 under an
///   untyped `Personal:` root (RPT-1: totals summed from depth-1 rows read
///   zero).
/// * `mixed-subtree.journal` — `assets ; type: A` holds
///   `assets:receivable ; type: L` (RPT-2: rolling up before filtering
///   fabricated an `assets = $700` row hledger never prints, and the same
///   account name legitimately appears in BOTH sections).
/// * `account-types/non-english.journal` — no English root name is
///   classifiable, so only the declared `type:` can place an account.
/// * `reports/invariants-basic.journal` — parents with their own postings.
#[test]
fn the_invariants_hold_across_every_awkward_chart_of_accounts() {
    for (fixture, dates) in [
        (
            "reports/nested-types.journal",
            ["2026-01-05", "2026-01-31", "2026-12-31"],
        ),
        (
            "reports/mixed-subtree.journal",
            ["2026-01-05", "2026-01-31", "2026-12-31"],
        ),
        (
            "account-types/non-english.journal",
            ["2026-01-01", "2026-02-14", "2026-12-31"],
        ),
        (
            "reports/invariants-basic.journal",
            ["2026-01-20", "2026-02-14", "2026-12-31"],
        ),
    ] {
        let journal = journal_fixture(fixture);
        let declared = declared_of(&journal);

        for as_of in dates {
            let direct = direct_totals(&journal.transactions, None, Some(as_of));

            // Partition: every posted account lands in exactly one type.
            for account in direct.keys() {
                let hits = TYPES
                    .iter()
                    .filter(|ty| is_account_type(account, &declared, **ty))
                    .count();
                assert_eq!(
                    hits, 1,
                    "{fixture}: {account} at {as_of} matched {hits} types"
                );
            }

            // Identity: none of these fixtures uses a cost annotation, so the
            // five type totals sum to exactly zero.
            let five = TYPES
                .iter()
                .map(|ty| typed_total(&direct, &declared, *ty))
                .fold(MixedAmount::new(), |acc, ma| add(&acc, &ma));
            assert!(
                conversion_residual(&journal.transactions, None, Some(as_of)).is_zero(),
                "{fixture} has no cost conversions"
            );
            assert_eq!(
                five,
                MixedAmount::new(),
                "{fixture}: A+L+E+R+X == 0 at {as_of}"
            );

            // Section totals match the independently typed totals, and rows
            // roll up WITHIN each section at every depth.
            for depth in DEPTHS {
                let bs = balance_sheet(&journal.transactions, as_of, depth, &declared).expect("bs");
                assert_eq!(
                    bs.sections[0].total,
                    typed_total(&direct, &declared, AccountType::Asset),
                    "{fixture}: Assets total at {as_of} depth {depth}"
                );
                assert_eq!(
                    bs.sections[1].total,
                    neg(&typed_total(&direct, &declared, AccountType::Liability)),
                    "{fixture}: Liabilities total at {as_of} depth {depth}"
                );
                assert_sections_roll_up(&format!("{fixture} bs {as_of}"), &bs, depth);

                let is =
                    income_statement(&journal.transactions, "0000-01-01", as_of, depth, &declared)
                        .expect("is");
                assert_eq!(
                    is.sections[0].total,
                    neg(&typed_total(&direct, &declared, AccountType::Revenue)),
                    "{fixture}: Revenues total at {as_of} depth {depth}"
                );
                assert_eq!(
                    is.sections[1].total,
                    typed_total(&direct, &declared, AccountType::Expense),
                    "{fixture}: Expenses total at {as_of} depth {depth}"
                );
                assert_sections_roll_up(&format!("{fixture} is {as_of}"), &is, depth);

                // Net worth agrees with the balance sheet's grand total. None of
                // these fixtures declares a price, so nothing is valued.
                let nw = net_worth(
                    &journal.transactions,
                    &journal.prices,
                    &NetWorthOpts {
                        end: as_of,
                        interval: Interval::Yearly,
                        count: 1,
                        depth,
                        value_in: None,
                        declared: &declared,
                    },
                )
                .expect("net_worth");
                assert_eq!(
                    nw.totals[0], bs.grand_total,
                    "{fixture}: net worth == bs.grand_total at {as_of} depth {depth}"
                );
            }
        }
    }
}

/// The RPT-2 shape hledger prints, pinned as a cross-section invariant: when a
/// parent's declared type disagrees with a child's, the SAME account name
/// appears in both sections carrying that section's own subtotal — and the two
/// never net against each other.
///
/// ```text
/// $ hledger -f fixtures/reports/mixed-subtree.journal bs --depth 1
///     Assets:      assets  $1,000.00
///     Liabilities: assets    $300.00
///     Net:                  $700.00
/// ```
#[test]
fn a_mixed_type_subtree_reports_the_same_parent_in_both_sections() {
    let journal = journal_fixture("reports/mixed-subtree.journal");
    let declared = declared_of(&journal);
    let bs = balance_sheet(&journal.transactions, "2026-12-31", 1, &declared).expect("bs");

    let dollars = |cents: i128| MixedAmount::single(usd(), Dec::new(cents, 2));

    let assets_row = &bs.sections[0].rows;
    assert_eq!(assets_row.len(), 1);
    assert_eq!(assets_row[0].account, "assets");
    assert_eq!(
        assets_row[0].inclusive,
        dollars(100_000),
        "Assets/assets holds only the type-A subtree"
    );

    let liabilities_row = &bs.sections[1].rows;
    assert_eq!(liabilities_row.len(), 1);
    assert_eq!(liabilities_row[0].account, "assets");
    assert_eq!(
        liabilities_row[0].inclusive,
        dollars(30_000),
        "Liabilities/assets holds only the type-L subtree, displayed positive"
    );

    assert_eq!(bs.grand_total, dollars(70_000), "net $700.00");
}

/// The period identity, swept over EVERY month the journal covers rather than
/// the two hand-picked clean windows:
///
/// ```text
/// Δ(assets + liabilities) == net income − Δequity + Δ(conversion residual)
/// ```
///
/// Derived from the as-of identity by differencing it across the month, so it
/// holds for months containing equity transfers and cost conversions alike —
/// which is what makes it a sweep rather than a spot check. A one-day error at
/// a month boundary (`bucket_start`/`bucket_end` off by one, or an exclusive
/// `to` in one report and an inclusive one in another) breaks it immediately.
#[test]
fn the_period_identity_holds_for_every_month_the_journal_covers() {
    let journal = common::fixture_journal();
    let declared = declared_of(&journal);
    let txns = &journal.transactions;

    // 2024-07 (the journal's first month) through 2026-08 (one past its last).
    let mut key = "2024-06".to_string();
    let mut checked = 0_usize;
    for _ in 0..26 {
        key = next_bucket(&key, Interval::Monthly).expect("next_bucket");
        let from = bucket_start(&key).expect("bucket_start");
        let to = bucket_end(&key).expect("bucket_end");
        let before = add_days(&from, -1);

        let window = direct_totals(txns, Some(&from), Some(&to));
        let equity = typed_total(&window, &declared, AccountType::Equity);
        let residual = conversion_residual(txns, Some(&from), Some(&to));

        let opening = balance_sheet(txns, &before, 1, &declared).expect("bs");
        let closing = balance_sheet(txns, &to, 1, &declared).expect("bs");
        let is = income_statement(txns, &from, &to, 1, &declared).expect("is");

        assert_eq!(
            sub(&closing.grand_total, &opening.grand_total),
            add(&sub(&is.grand_total, &equity), &residual),
            "period identity for {key} ({from}..{to})"
        );
        checked += 1;
    }
    assert_eq!(checked, 26, "swept every month");

    // The sweep must actually meet the interesting months, or it proves little.
    let august_2025 = direct_totals(txns, Some("2025-08-01"), Some("2025-08-31"));
    assert!(
        !typed_total(&august_2025, &declared, AccountType::Equity).is_zero(),
        "2025-08 carries the GLD equity transfer"
    );
    assert!(
        !conversion_residual(txns, Some("2026-04-01"), Some("2026-04-30")).is_zero(),
        "2026-04 carries the partial VTI sale"
    );
}
