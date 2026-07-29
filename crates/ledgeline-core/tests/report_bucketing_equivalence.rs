//! PERF-5 equivalence suite: the single-pass bucketing must be BIT-identical to
//! the per-bucket rescan it replaced, not merely numerically equal.
//!
//! # Why this suite exists and the golden tests are not enough
//!
//! `net_worth`, `cash_flow` and `budget_report` used to call `account_totals`
//! once per bucket, each time re-scanning every posting in the journal. They now
//! make ONE pass and bucket each posting as they go. That regroups the addends
//! of every per-account sum, so the question this file answers is whether
//! regrouping can move a number.
//!
//! It cannot move a *value* — `Dec` addition is exact. But `Dec` also carries a
//! wire representation (`mantissa` / `places`), and:
//!
//! - `Dec`'s `PartialEq` compares by **numeric value**, so `assert_eq!` on a
//!   `MixedAmount` passes even when the scale drifted (`1.5` == `1.50`);
//! - the golden tests canonicalize (strip trailing zeros) before comparing wire
//!   numbers, so they are blind to the same drift.
//!
//! So neither the existing invariant suite nor the goldens can see a scale
//! regression. This file compares `(mantissa, places)` pairs directly, against a
//! reference implementation that is a literal transcription of the pre-PERF-5
//! per-bucket-rescan code.
//!
//! # The two hazards the fixture is built to trigger
//!
//! `Dec::add` yields `max(self.places, other.places)`, so a sum's scale is the
//! max over its addends and is order-independent — *provided every addend is
//! still present*. Both hazards below are cases where an addend disappears:
//!
//! 1. **Pruning a zero loses the scale it carried.** `1.50 + (-1.50) + 1.0` is
//!    `1.00` (places 2) if evaluated as written, but `1.0` (places 1) if the
//!    intermediate zero is dropped first. The running balances in `net_worth`
//!    therefore must not be pruned between buckets — only each bucket's snapshot
//!    is. `zero_netting_commodity_keeps_its_scale_across_buckets` pins this.
//!
//! 2. **Merging accounts must happen AFTER each account's own total is pruned.**
//!    `budget_report` remaps several accounts onto one name. An account whose
//!    commodity nets to zero contributes nothing to the merged cell under the
//!    original code (it is pruned by `account_totals` first), but would widen the
//!    merged scale if its raw postings were folded in directly. This is why the
//!    single pass sums per FULL account name and remaps afterwards.
//!    `budget_merges_pruned_account_totals_not_raw_postings` pins this.
//!
//! Every fixture below also puts transactions OUT of date order and uses
//! posting-level `date:` overrides, so the single pass genuinely visits postings
//! in a different order than any one bucket's rescan would have.

use ledgeline_core::model::{PeriodicTransaction, Transaction};
use ledgeline_core::reports::{
    AccountType, BudgetOpts, BudgetReport, Interval, MixedAmount, NetWorthOpts, PostingFilter,
    account_totals, at_depth, budget_report, cash_flow, is_account_type, last_n_buckets, net_worth,
    roll_up,
};
use ledgeline_core::reports::{bucket_end, bucket_start, compare_iso};
use ledgeline_core::{Dec, Journal, parse_journal};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Exact (mantissa, places) comparison — what `assert_eq!` cannot see
// ---------------------------------------------------------------------------

/// A `Dec`'s full wire identity, not just its value.
fn exact(d: Dec) -> (i128, u32) {
    (d.mantissa, d.places)
}

/// One `MixedAmount` as `(commodity, mantissa, places)` triples — the exact wire
/// identity, in lexical commodity order.
type ExactMa = Vec<(String, i128, u32)>;

/// A `PeriodReport`'s rows and totals, each flattened to [`ExactMa`] per bucket.
type ExactPeriod = (Vec<ExactMa>, Vec<ExactMa>);

/// One bucket's budget actuals: the OWN totals (which select rows) and the
/// INCLUSIVE totals (which fill cells).
type BudgetBucket = (BTreeMap<String, MixedAmount>, BTreeMap<String, MixedAmount>);

/// Every commodity of a `MixedAmount` with its exact wire representation.
fn exact_ma(ma: &MixedAmount) -> ExactMa {
    ma.iter()
        .map(|(commodity, qty)| (commodity.0.clone(), qty.mantissa, qty.places))
        .collect()
}

// ---------------------------------------------------------------------------
// Reference implementations — literal transcriptions of the pre-PERF-5 loops
// ---------------------------------------------------------------------------

/// The per-bucket-rescan `net_worth` body, verbatim, from before PERF-5. Only
/// the rows/totals shape is reproduced (valuation is untouched by PERF-5, so the
/// fixtures below declare no prices and the report is reported unvalued).
fn reference_net_worth(
    txns: &[Transaction],
    end: &str,
    interval: Interval,
    count: usize,
    depth: usize,
    declared: &BTreeMap<String, AccountType>,
) -> ExactPeriod {
    let buckets = last_n_buckets(end, interval, count).unwrap();
    let mut rows_per_bucket: Vec<BTreeMap<String, MixedAmount>> = Vec::new();
    let mut totals: Vec<MixedAmount> = Vec::new();
    for key in &buckets {
        let end_of_bucket = bucket_end(key).unwrap();
        let as_of = if compare_iso(end, &end_of_bucket) == Ordering::Less {
            end.to_string()
        } else {
            end_of_bucket
        };
        let direct = account_totals(
            txns,
            &PostingFilter {
                to: Some(&as_of),
                ..PostingFilter::default()
            },
        )
        .unwrap();
        let members: BTreeMap<String, MixedAmount> = direct
            .into_iter()
            .filter(|(account, _)| {
                is_account_type(account, declared, AccountType::Asset)
                    || is_account_type(account, declared, AccountType::Liability)
            })
            .collect();
        let total = members
            .values()
            .try_fold(MixedAmount::new(), |acc, ma| acc.ma_add(ma))
            .unwrap();
        totals.push(total);
        rows_per_bucket.push(at_depth(&roll_up(&members).unwrap(), depth));
    }
    let accounts: BTreeSet<String> = rows_per_bucket
        .iter()
        .flat_map(|bucket| bucket.keys().cloned())
        .collect();
    let rows = accounts
        .iter()
        .map(|account| {
            rows_per_bucket
                .iter()
                .flat_map(|bucket| exact_ma(&bucket.get(account).cloned().unwrap_or_default()))
                .collect()
        })
        .collect();
    (rows, totals.iter().map(exact_ma).collect())
}

/// The per-bucket-rescan `cash_flow` body, verbatim, from before PERF-5.
fn reference_cash_flow(
    txns: &[Transaction],
    end: &str,
    interval: Interval,
    count: usize,
    depth: usize,
    is_cash: &dyn Fn(&str) -> bool,
) -> ExactPeriod {
    let buckets = last_n_buckets(end, interval, count).unwrap();
    let mut per_bucket: Vec<BTreeMap<String, MixedAmount>> = Vec::new();
    let mut totals: Vec<MixedAmount> = Vec::new();
    for key in &buckets {
        let start = bucket_start(key).unwrap();
        let end_of_bucket = bucket_end(key).unwrap();
        let to = if compare_iso(end, &end_of_bucket) == Ordering::Less {
            end
        } else {
            end_of_bucket.as_str()
        };
        let mut direct = account_totals(
            txns,
            &PostingFilter {
                from: Some(&start),
                to: Some(to),
                ..PostingFilter::default()
            },
        )
        .unwrap();
        direct.retain(|account, _| is_cash(account));
        let mut total = MixedAmount::new();
        for ma in direct.values() {
            total = total.ma_add(ma).unwrap();
        }
        totals.push(total);
        per_bucket.push(at_depth(&roll_up(&direct).unwrap(), depth));
    }
    let accounts: BTreeSet<String> = per_bucket
        .iter()
        .flat_map(|clamped| clamped.keys().cloned())
        .collect();
    let rows = accounts
        .iter()
        .map(|account| {
            per_bucket
                .iter()
                .flat_map(|clamped| exact_ma(&clamped.get(account).cloned().unwrap_or_default()))
                .collect()
        })
        .collect();
    (rows, totals.iter().map(exact_ma).collect())
}

/// The pre-PERF-5 `clip` (allocating, `join(":")`-based).
fn reference_clip(account: &str, depth: usize) -> String {
    account
        .split(':')
        .take(depth.max(1))
        .collect::<Vec<_>>()
        .join(":")
}

/// The pre-PERF-5 `parent_accounts` (eagerly joins every ancestor).
fn reference_parent_accounts(account: &str) -> Vec<String> {
    let segments: Vec<&str> = account.split(':').collect();
    (1..segments.len())
        .rev()
        .map(|n| segments[..n].join(":"))
        .collect()
}

/// The pre-PERF-5 `remap_account`.
fn reference_remap(account: &str, budgeted: &BTreeSet<String>) -> String {
    if budgeted.contains(account) {
        return account.to_string();
    }
    reference_parent_accounts(account)
        .into_iter()
        .find(|ancestor| budgeted.contains(ancestor))
        .unwrap_or_else(|| "<unbudgeted>".to_string())
}

/// The pre-PERF-5 budget ACTUALS loop (per-bucket rescan + per-bucket remap),
/// returning each bucket's `(own, inclusive)` pair — the two maps the report's
/// row selection and cell values are respectively derived from.
fn reference_budget_actuals(
    txns: &[Transaction],
    rules: &[PeriodicTransaction],
    opts: &BudgetOpts,
) -> Vec<BudgetBucket> {
    let buckets = last_n_buckets(opts.end, opts.interval, opts.count).unwrap();
    let budgeted: BTreeSet<String> = rules
        .iter()
        .flat_map(|rule| &rule.postings)
        .flat_map(|posting| posting.account.self_and_ancestors())
        .collect();
    let mut out = Vec::new();
    for key in &buckets {
        let start = bucket_start(key).unwrap();
        let bucket_end_date = bucket_end(key).unwrap();
        let to = if compare_iso(opts.end, &bucket_end_date) == Ordering::Less {
            opts.end
        } else {
            bucket_end_date.as_str()
        };
        let direct = account_totals(
            txns,
            &PostingFilter {
                from: Some(&start),
                to: Some(to),
                ..PostingFilter::default()
            },
        )
        .unwrap();
        let mut own: BTreeMap<String, MixedAmount> = BTreeMap::new();
        for (account, ma) in &direct {
            let remapped = reference_clip(&reference_remap(account, &budgeted), opts.depth);
            let entry: &mut MixedAmount = own.entry(remapped).or_default();
            for (commodity, qty) in ma.iter() {
                entry.accumulate(commodity, *qty).unwrap();
            }
        }
        for ma in own.values_mut() {
            ma.drop_zeros();
        }
        let incl = roll_up(&own).unwrap();
        out.push((own, incl));
    }
    out
}

/// Assert a `BudgetReport`'s actuals against the pre-PERF-5 reference: every
/// actual-bearing account still has a row, and every cell matches exactly on
/// `(mantissa, places)`.
///
/// Row selection is deliberately checked one-directionally. A report row can
/// also be created by a GOAL with no actual behind it (a `~ monthly` rule clips
/// onto an account nothing was posted to), and goals are untouched by PERF-5 —
/// pinning them here would just restate the budget goldens. What PERF-5 could
/// break is an ACTUAL going missing, so that direction is asserted.
fn assert_budget_actuals_match(report: &BudgetReport, reference: &[BudgetBucket], context: &str) {
    let got_rows: BTreeSet<&String> = report.rows.iter().map(|row| &row.account).collect();
    for (own, _) in reference {
        for (account, ma) in own {
            assert!(
                ma.is_zero() || got_rows.contains(account),
                "budget row {account} carrying a non-zero actual vanished {context}"
            );
        }
    }

    for row in &report.rows {
        for (bucket, (_, incl)) in reference.iter().enumerate() {
            let want = incl.get(&row.account).cloned().unwrap_or_default();
            assert_eq!(
                exact_ma(&row.cells[bucket].actual),
                exact_ma(&want),
                "budget actual drifted for {} at bucket {bucket} {context}",
                row.account
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn parse(text: &str) -> Journal {
    parse_journal(text, "equivalence.journal").expect("fixture parses")
}

/// Transactions deliberately OUT of date order, with mixed decimal scales, a
/// commodity that nets to exactly zero mid-span, posting-level `date:`
/// overrides, an amountless posting and two commodities.
///
/// The scale mix is the point: `$` amounts are written at 1, 2 and 3 decimal
/// places on the same account, so the per-account sum's `places` is the max over
/// whichever postings a given column includes — and any regrouping that dropped
/// one would show up as a different `places`.
fn scale_hazard_journal() -> Journal {
    parse(
        "\
account assets:bank ; type: A
account assets:wallet ; type: A
account liabilities:card ; type: L
account expenses:food ; type: X
account equity:opening ; type: E

2026-03-15 later txn first in file
    assets:bank         $1.500
    equity:opening     $-1.500

2026-01-10 opening
    assets:bank          $10.5
    equity:opening      $-10.5

2026-02-20 nets to zero within february
    assets:wallet       $12.25
    assets:wallet      $-12.25
    expenses:food         $0.0
    equity:opening        $0.0

2026-01-25 posting date override pushes this leg into february
    assets:bank         $-2.00  ; date: 2026-02-03
    expenses:food         $2.00

2026-02-14 card and a two-commodity leg
    liabilities:card    $-33.0
    assets:wallet     3.50 EUR
    expenses:food        $33.0
    equity:opening   -3.50 EUR

2026-03-02 amountless posting on a typed account
    assets:wallet
    equity:opening        $0.0
",
    )
}

/// Two accounts that remap onto the SAME budgeted parent, where one of them
/// nets to exactly zero at a wider scale than the other's surviving total.
///
/// Under the original code `expenses:food:zeroed`'s `$0.000` is pruned by
/// `account_totals` before the remap, so the merged `expenses:food` cell has the
/// scale of `expenses:food:real` alone. Folding raw postings into the merged
/// name instead would widen it, which is the drift this fixture catches.
fn budget_merge_journal() -> Journal {
    parse(
        "\
account expenses:food
account expenses:food:real
account expenses:food:zeroed
account assets:checking

~ monthly  household
    (expenses:food)      $100

2026-01-06 zero-netting child at a wide scale
    expenses:food:zeroed    $5.000
    expenses:food:zeroed   $-5.000
    assets:checking           $0.0

2026-01-08 surviving child at a narrow scale
    expenses:food:real         $7.5
    assets:checking           $-7.5
",
    )
}

/// Account names chosen to break a naive ancestry walk. The load-bearing ones:
///
/// - `expenses:food-truck` sorts BETWEEN `expenses:food` and
///   `expenses:food:dining` (`-` is `0x2D`, `:` is `0x3A`), so any rule that
///   reasons about a sorted account list by looking only at the immediate
///   successor gets it wrong;
/// - `expensesx` and `assets:banking` share a prefix with a budgeted account
///   without being under it, so a `starts_with` that forgets the `:` separator
///   re-homes them to the wrong parent;
/// - `a:b:c:d:e:f` is deep enough that the old `parent_accounts` allocated six
///   joined strings per call.
const PREFIX_COLLISION_ACCOUNTS: [&str; 11] = [
    "expenses",
    "expenses:food",
    "expenses:food:dining",
    "expenses:food:dining:tips",
    "expenses:food-truck",
    "expenses:foodstuffs:x",
    "expensesx",
    "assets:bank:checking",
    "assets:banking",
    "income:salary",
    "a:b:c:d:e:f",
];

/// Every name in [`PREFIX_COLLISION_ACCOUNTS`] posted against a budget tree that
/// covers only some of them, so the remap must pick the right nearest budgeted
/// ancestor (or `<unbudgeted>`) for each.
fn prefix_collision_journal() -> Journal {
    let rules = "~ monthly  household\n    (expenses)            $10\n    (expenses:food)       $20\n    (expenses:food:dining) $30\n    (assets:bank)         $40\n";
    let postings: String = PREFIX_COLLISION_ACCOUNTS
        .iter()
        .enumerate()
        .map(|(i, account)| {
            // Distinct scales AND distinct magnitudes so no two can cancel and
            // hide a mis-homed posting.
            let day = 5 + i;
            format!(
                "\n2026-01-{day:02} probe {i}\n    {account}      $1.{i:02}\n    equity:src     $-1.{i:02}\n"
            )
        })
        .collect();
    parse(&format!("{rules}{postings}"))
}

fn declared_for(journal: &Journal) -> BTreeMap<String, AccountType> {
    ledgeline_core::reports::declared_types(&ledgeline_core::reports::account_decls(journal))
}

// ---------------------------------------------------------------------------
// net_worth
// ---------------------------------------------------------------------------

/// The single-pass cumulative net worth must reproduce the per-bucket rescan
/// exactly — same values AND same `mantissa`/`places` — at every depth and
/// bucket count.
#[test]
fn net_worth_matches_the_per_bucket_rescan_exactly() {
    let journal = scale_hazard_journal();
    let declared = declared_for(&journal);
    for count in [1usize, 2, 3, 5, 12] {
        for depth in [1usize, 2, 3, 9] {
            let got = net_worth(
                &journal.transactions,
                &journal.prices,
                &NetWorthOpts {
                    end: "2026-03-20",
                    interval: Interval::Monthly,
                    count,
                    depth,
                    value_in: None,
                    declared: &declared,
                },
            )
            .unwrap();
            let (want_rows, want_totals) = reference_net_worth(
                &journal.transactions,
                "2026-03-20",
                Interval::Monthly,
                count,
                depth,
                &declared,
            );
            let got_rows: Vec<Vec<(String, i128, u32)>> = got
                .rows
                .iter()
                .map(|row| row.values.iter().flat_map(exact_ma).collect())
                .collect();
            assert_eq!(
                got_rows, want_rows,
                "net_worth rows drifted at count={count} depth={depth}"
            );
            assert_eq!(
                got.totals.iter().map(exact_ma).collect::<Vec<_>>(),
                want_totals,
                "net_worth totals drifted at count={count} depth={depth}"
            );
        }
    }
}

/// Hazard 1, stated directly: a commodity that nets to zero in an early bucket
/// still contributes its SCALE to every later cumulative column.
///
/// `assets:bank` receives `$10.5` (1 dp) in January and `$-2.00` (2 dp, via a
/// posting `date:` override) in February. The February and March columns must
/// therefore report `places = 2`, not 1 — a running balance that pruned or
/// renormalized between buckets would report `$8.5` instead of `$8.50`.
#[test]
fn zero_netting_commodity_keeps_its_scale_across_buckets() {
    let journal = scale_hazard_journal();
    let declared = declared_for(&journal);
    let report = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-03-20",
            interval: Interval::Monthly,
            count: 3,
            depth: 9,
            value_in: None,
            declared: &declared,
        },
    )
    .unwrap();
    let bank = report
        .rows
        .iter()
        .find(|row| row.account == "assets:bank")
        .expect("assets:bank is a row");
    let usd: Vec<(i128, u32)> = bank
        .values
        .iter()
        .map(|ma| {
            ma.iter()
                .find(|(commodity, _)| commodity.0 == "$")
                .map_or((0, 0), |(_, qty)| exact(*qty))
        })
        .collect();
    // Jan: 10.5 (1 dp). Feb: 10.5 − 2.00 = 8.50 (2 dp). Mar: + 1.500 = 10.000 (3 dp).
    assert_eq!(usd, [(105, 1), (850, 2), (10_000, 3)]);
}

// ---------------------------------------------------------------------------
// cash_flow
// ---------------------------------------------------------------------------

/// The single-pass cash flow must reproduce the per-bucket rescan exactly.
#[test]
fn cash_flow_matches_the_per_bucket_rescan_exactly() {
    let journal = scale_hazard_journal();
    let is_cash = |account: &str| account.starts_with("assets:");
    for count in [1usize, 2, 3, 5] {
        for depth in [1usize, 2, 9] {
            let got = cash_flow(
                &journal.transactions,
                "2026-03-20",
                Interval::Monthly,
                count,
                depth,
                Some(&is_cash),
            )
            .unwrap();
            let (want_rows, want_totals) = reference_cash_flow(
                &journal.transactions,
                "2026-03-20",
                Interval::Monthly,
                count,
                depth,
                &is_cash,
            );
            let got_rows: Vec<Vec<(String, i128, u32)>> = got
                .rows
                .iter()
                .map(|row| row.values.iter().flat_map(exact_ma).collect())
                .collect();
            assert_eq!(
                got_rows, want_rows,
                "cash_flow rows drifted at count={count} depth={depth}"
            );
            assert_eq!(
                got.totals.iter().map(exact_ma).collect::<Vec<_>>(),
                want_totals,
                "cash_flow totals drifted at count={count} depth={depth}"
            );
        }
    }
}

/// A posting whose `date:` override moves it out of its transaction's bucket
/// must be counted in the bucket its OWN date selects, in both engines.
#[test]
fn cash_flow_honors_posting_date_overrides_in_the_single_pass() {
    let journal = scale_hazard_journal();
    let is_cash = |account: &str| account == "assets:bank";
    let report = cash_flow(
        &journal.transactions,
        "2026-03-20",
        Interval::Monthly,
        3,
        9,
        Some(&is_cash),
    )
    .unwrap();
    let bank = report
        .rows
        .iter()
        .find(|row| row.account == "assets:bank")
        .expect("assets:bank is a row");
    let usd: Vec<(i128, u32)> = bank
        .values
        .iter()
        .map(|ma| {
            ma.iter()
                .find(|(commodity, _)| commodity.0 == "$")
                .map_or((0, 0), |(_, qty)| exact(*qty))
        })
        .collect();
    // The 2026-01-25 transaction's bank leg carries `date: 2026-02-03`, so its
    // −$2.00 lands in February, not January.
    assert_eq!(usd, [(105, 1), (-200, 2), (1500, 3)]);
}

// ---------------------------------------------------------------------------
// budget
// ---------------------------------------------------------------------------

/// The single-pass budget actuals must reproduce the per-bucket rescan exactly,
/// including the remap + clip at every depth.
#[test]
fn budget_matches_the_per_bucket_rescan_exactly() {
    let fixtures = [
        ("scale_hazard", scale_hazard_journal()),
        ("budget_merge", budget_merge_journal()),
        ("prefix_collision", prefix_collision_journal()),
    ];
    for (name, journal) in &fixtures {
        for count in [1usize, 2, 3] {
            for depth in [1usize, 2, 3, 4, 99] {
                let opts = BudgetOpts {
                    end: "2026-03-20",
                    interval: Interval::Monthly,
                    count,
                    depth,
                    budget_desc: None,
                };
                let got =
                    budget_report(&journal.transactions, &journal.periodic_transactions, &opts)
                        .unwrap();
                let reference = reference_budget_actuals(
                    &journal.transactions,
                    &journal.periodic_transactions,
                    &opts,
                );
                assert_budget_actuals_match(
                    &got,
                    &reference,
                    &format!("in {name} at count={count} depth={depth}"),
                );
            }
        }
    }
}

/// Hazard 2, stated directly: the remap merges PRUNED per-account totals, so an
/// account that nets to exactly zero cannot widen the merged cell's scale.
///
/// `expenses:food:zeroed` nets to `$0.000` (3 dp) and `expenses:food:real` to
/// `$7.5` (1 dp); both remap onto the budgeted `expenses:food`. The merged
/// actual must be `$7.5` at `places = 1`. Folding raw postings straight into the
/// merged name would give `$7.500` at `places = 3` — the same value, a different
/// number on the wire.
#[test]
fn budget_merges_pruned_account_totals_not_raw_postings() {
    let journal = budget_merge_journal();
    let report = budget_report(
        &journal.transactions,
        &journal.periodic_transactions,
        &BudgetOpts {
            end: "2026-01-31",
            interval: Interval::Monthly,
            count: 1,
            depth: 99,
            budget_desc: None,
        },
    )
    .unwrap();
    let food = report
        .rows
        .iter()
        .find(|row| row.account == "expenses:food")
        .expect("expenses:food is a row");
    assert_eq!(exact_ma(&food.cells[0].actual), [("$".to_string(), 75, 1)]);
}
