//! `balance_sheet_columns` — the multi-column grouped balance sheet
//! (`plans/28-projected-statements.md`, Phase 3).
//!
//! ONE claim is defended, from every angle the single-date report has a
//! feature for: **column `i` is exactly `balance_sheet_grouped` at `ends[i]`.**
//! The one-pass walk is an optimization, never a second definition, so the
//! single-date report is the oracle and nothing here restates a number.
//!
//! Covered: multi-commodity books under explicit prices (`sample.journal`,
//! `fx-cross-rates.journal`), `bsgroup:` resolution (`bs-groups.journal`),
//! `bsterm:` halves (`sample.journal`), sub-cent cost dust and its verdict
//! (`bs-cost-dust.journal`, `bs-cent-floor.journal`), a real imbalance
//! (`errors/bs-unbalanced.journal`), retained earnings throughout, columns
//! with no activity, columns before the first transaction, and columns past the
//! last transaction and the last price (prices carried flat).

mod common;

use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    AccountType, BalanceSheetReport, BsOpts, BsSectionKind, Interval, RETAINED_EARNINGS_GROUP,
    Valuation, account_decls, account_groups, balance_sheet_columns, balance_sheet_grouped,
    bs_terms, declared_types, range_buckets,
};
use ledgeline_core::{Journal, parse_journal};
use proptest::prelude::*;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn journal_fixture(relative: &str) -> Journal {
    let path = common::fixtures_dir().join(relative);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"));
    parse_journal(&text, &path.to_string_lossy())
        .unwrap_or_else(|e| panic!("parse {relative}: {e}"))
}

const BASES: [Valuation; 3] = [Valuation::Market, Valuation::Cost, Valuation::None];

fn opts(depth: Option<usize>, value: Valuation) -> BsOpts<'static> {
    BsOpts {
        // Ignored by `balance_sheet_columns`; a deliberately wrong date proves it.
        as_of: "1900-01-01",
        depth,
        value,
        value_in: None,
    }
}

fn columns(
    journal: &Journal,
    depth: Option<usize>,
    value: Valuation,
    ends: &[String],
) -> Vec<BalanceSheetReport> {
    balance_sheet_columns(
        &journal.transactions,
        &journal.prices,
        &opts(depth, value),
        ends,
        &declared_types(&account_decls(journal)),
        &account_groups(journal),
        &bs_terms(journal),
    )
    .expect("balance sheet columns")
}

fn single(
    journal: &Journal,
    as_of: &str,
    depth: Option<usize>,
    value: Valuation,
) -> BalanceSheetReport {
    let declared: BTreeMap<String, AccountType> = declared_types(&account_decls(journal));
    balance_sheet_grouped(
        &journal.transactions,
        &journal.prices,
        &BsOpts {
            as_of,
            ..opts(depth, value)
        },
        &declared,
        &account_groups(journal),
        &bs_terms(journal),
    )
    .expect("grouped balance sheet")
}

/// Month ends `[from, to]`, the shape a monthly projection hands in.
fn month_ends(from: &str, to: &str) -> Vec<String> {
    range_buckets(from, to, Some(Interval::Monthly))
        .expect("monthly buckets")
        .into_iter()
        .map(|span| span.to)
        .collect()
}

/// The property itself: every column, at every basis and depth asked, equals
/// the single-date report at its own end.
fn assert_columns_match(journal: &Journal, ends: &[String], depths: &[Option<usize>]) {
    for value in BASES {
        for &depth in depths {
            let got = columns(journal, depth, value, ends);
            assert_eq!(got.len(), ends.len(), "one report per end");
            for (end, column) in ends.iter().zip(&got) {
                assert_eq!(
                    column,
                    &single(journal, end, depth, value),
                    "column at {end} ({value:?}, depth {depth:?}) differs from \
                     balance_sheet_grouped"
                );
            }
        }
    }
}

fn section_total(
    report: &BalanceSheetReport,
    kind: BsSectionKind,
) -> &ledgeline_core::reports::MixedAmount {
    &report
        .sections
        .iter()
        .find(|section| section.kind == kind)
        .unwrap_or_else(|| panic!("section {kind:?}"))
        .total
}

// ---------------------------------------------------------------------------
// Each column is the single-date report — table over the fixtures
// ---------------------------------------------------------------------------

/// Monthly columns from before the first transaction to well past the last one
/// and past the last `P` directive, on every fixture the single-date suite
/// defends, at every basis and at both a clamped and an unclamped depth.
#[test]
fn every_column_equals_the_single_date_report_on_every_fixture() {
    let cases = [
        // Multi-commodity (EUR, AAPL, VTI, unpriced GLD/TSLA), prices,
        // `bsterm:` halves, retained earnings. Spans 2024-07 → 2026-07.
        ("sample.journal", "2024-05-01", "2027-03-31"),
        // `bsgroup:` resolution with no English names.
        ("reports/bs-groups.journal", "2025-11-01", "2026-12-31"),
        // Priced lots that leave sub-cent residue: `check` non-zero, `balanced`.
        ("reports/bs-cost-dust.journal", "2025-12-01", "2026-06-30"),
        ("reports/bs-cent-floor.journal", "2025-12-01", "2026-06-30"),
        // A real imbalance: `check` non-zero and NOT balanced.
        (
            "reports/errors/bs-unbalanced.journal",
            "2025-12-01",
            "2026-06-30",
        ),
        // Cross-rates through a price graph, prices dated after the trades.
        ("reports/fx-cross-rates.journal", "2025-12-01", "2026-12-31"),
        (
            "reports/fx-cross-rates-declared.journal",
            "2025-12-01",
            "2026-12-31",
        ),
        ("reports/nested-types.journal", "2025-12-01", "2026-12-31"),
        ("reports/mixed-subtree.journal", "2025-12-01", "2026-12-31"),
        ("reports/other-holdings.journal", "2025-12-01", "2026-12-31"),
    ];
    for (fixture, from, to) in cases {
        let journal = journal_fixture(fixture);
        assert_columns_match(&journal, &month_ends(from, to), &[None, Some(2)]);
    }
}

/// The fixtures above really do reach the branches this suite claims to cover;
/// otherwise "equal to the oracle" could be equal on empty sheets.
#[test]
fn the_fixtures_exercise_what_the_table_claims() {
    let sample = journal_fixture("sample.journal");
    let ends = month_ends("2024-05-01", "2027-03-31");
    let market = columns(&sample, None, Valuation::Market, &ends);

    // Before the first transaction: an empty sheet with a zero retained line.
    let empty = &market[0];
    assert!(section_total(empty, BsSectionKind::Assets).is_zero());
    assert!(empty.check.is_zero() && empty.balanced);

    // Multi-commodity, priced, split into halves, with retained earnings.
    let mid = market
        .iter()
        .find(|report| report.as_of == "2026-06-30")
        .expect("June 2026 column");
    assert_eq!(mid.base, Some(Commodity("$".into())));
    assert!(!mid.meta.unpriced.is_empty(), "GLD/TSLA stay unpriced");
    assert!(!mid.sections[0].subsections.is_empty(), "bsterm: split on");
    let equity = &mid.sections[2];
    assert!(
        equity
            .groups
            .iter()
            .any(|group| group.name == RETAINED_EARNINGS_GROUP && !group.total.is_zero()),
        "retained earnings carried"
    );

    // The unbalanced fixture's later columns are genuinely not balanced.
    let unbalanced = journal_fixture("reports/errors/bs-unbalanced.journal");
    let last = columns(
        &unbalanced,
        None,
        Valuation::Cost,
        &["2026-06-30".to_string()],
    );
    assert!(!last[0].balanced);
}

/// The verdict's tolerance comes from the precision the journal WRITES through
/// each column's end, which the one pass accumulates rather than rescans. A
/// fifty-cent imbalance is above the one-cent tolerance a cents journal earns,
/// but under the whole-unit one a commodity with no written precision would get
/// — so a column that lost track of the written precision would call it
/// balanced.
#[test]
fn the_verdict_reads_the_precision_written_through_each_column() {
    let text = "\
account assets:bank        ; type: C
account equity:opening     ; type: E

2026-01-01 opening
    assets:bank        $100.00
    equity:opening    $-100.00

2026-03-01 fifty cents out
    assets:bank        $0.50
    equity:opening     $0
";
    let journal = parse_journal(text, "half-dollar.journal").expect("parse");
    let ends = month_ends("2026-01-01", "2026-04-30");
    assert_columns_match(&journal, &ends, &[None]);
    let got = columns(&journal, None, Valuation::Cost, &ends);
    assert!(got[0].balanced && got[1].balanced);
    assert!(!got[2].balanced && !got[3].balanced, "{:?}", got[2].check);
}

// ---------------------------------------------------------------------------
// Column order, duplicates, gaps
// ---------------------------------------------------------------------------

/// `ends` need not be sorted or distinct: each report answers its own entry.
#[test]
fn unsorted_and_duplicate_ends_come_back_in_the_order_given() {
    let journal = journal_fixture("sample.journal");
    let ends: Vec<String> = [
        "2026-06-30",
        "2024-12-31",
        "2026-06-30",
        "2030-01-01",
        "2025-03-15",
        "2023-01-01",
    ]
    .map(String::from)
    .to_vec();
    assert_columns_match(&journal, &ends, &[None]);
    let got = columns(&journal, None, Valuation::Market, &ends);
    assert_eq!(
        got.iter()
            .map(|report| report.as_of.as_str())
            .collect::<Vec<_>>(),
        ends.iter().map(String::as_str).collect::<Vec<_>>()
    );
}

#[test]
fn no_ends_is_no_columns() {
    let journal = journal_fixture("sample.journal");
    assert!(columns(&journal, None, Valuation::Market, &[]).is_empty());
}

/// A column with nothing posted since the previous one repeats it exactly
/// (prices permitting), rather than resetting to zero — the totals are
/// cumulative, not per-period.
#[test]
fn a_column_with_no_activity_carries_the_previous_balance() {
    // The fixture's activity is all in the first half of 2026; take a run of
    // quiet days after the last transaction AND the last price.
    let journal = journal_fixture("reports/bs-groups.journal");
    let ends: Vec<String> = ["2026-08-31", "2026-09-30", "2026-10-31"]
        .map(String::from)
        .to_vec();
    for value in BASES {
        let got = columns(&journal, None, value, &ends);
        for pair in got.windows(2) {
            assert_eq!(pair[0].sections, pair[1].sections, "{value:?}");
            assert_eq!(pair[0].net_worth, pair[1].net_worth);
            assert!(!section_total(&pair[1], BsSectionKind::Assets).is_zero());
        }
    }
}

// ---------------------------------------------------------------------------
// Projection shape: future-dated transactions and future prices
// ---------------------------------------------------------------------------

/// What a projection hands in: real history, synthetic future-dated entries and
/// future `P` directives. Each column must value at its own date — the future
/// price where one is in effect, the latest one carried flat past the end of
/// the price series — and still be the single-date report.
#[test]
fn future_transactions_and_prices_value_each_column_at_its_own_end() {
    let text = "\
account assets:bank        ; type: C
account assets:broker      ; type: A
account liabilities:card   ; type: L
account equity:opening     ; type: E
account income:salary      ; type: R
account expenses:food      ; type: X

P 2026-01-01 VTI $100.00
P 2026-01-01 EUR $1.10

2026-01-01 opening
    assets:bank        $10,000.00
    equity:opening

2026-01-15 buy
    assets:broker      10 VTI @ $100.00
    assets:bank

2026-02-01 salary
    assets:bank        $3,000.00
    income:salary

2026-02-10 groceries
    expenses:food      $200.00
    liabilities:card

; --- synthetic, future-dated (a projection's forecast) ---
P 2027-03-01 VTI $130.00
P 2027-06-01 EUR $1.20

2027-03-01 forecast salary
    assets:bank        $3,000.00
    income:salary

2027-04-01 forecast EUR deposit
    assets:bank        100 EUR @ $1.15
    assets:bank        $-115.00
";
    let journal = parse_journal(text, "projection.journal").expect("parse");
    let ends = month_ends("2026-01-01", "2028-12-31");
    assert_columns_match(&journal, &ends, &[None]);

    let market = columns(&journal, None, Valuation::Market, &ends);
    let at = |date: &str| {
        market
            .iter()
            .find(|report| report.as_of == date)
            .unwrap_or_else(|| panic!("column {date}"))
    };
    let assets = |date: &str| section_total(at(date), BsSectionKind::Assets).clone();
    // Before the future VTI price: $100 a share; after: $130.
    let feb_2027 = assets("2027-02-28");
    let mar_2027 = assets("2027-03-31");
    assert_ne!(feb_2027, mar_2027);
    // Past the last price and the last transaction, columns are flat.
    assert_eq!(assets("2027-12-31"), assets("2028-12-31"));
    assert!(at("2028-12-31").balanced);
}

// ---------------------------------------------------------------------------
// Property: any set of ends, anywhere on the calendar
// ---------------------------------------------------------------------------

/// A day-offset into a span wider than `sample.journal` on both sides.
fn date_from_offset(offset: u32) -> String {
    ledgeline_core::reports::periods::add_days("2024-01-01", i64::from(offset))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// For ANY end dates — unsorted, repeated, before, inside and after the
    /// journal — and any basis and depth, each column equals the single-date
    /// report at its end.
    #[test]
    fn any_ends_match_the_single_date_report(
        offsets in proptest::collection::vec(0u32..1500, 0..12),
        basis in 0usize..3,
        depth in proptest::option::of(0usize..5),
    ) {
        let journal = journal_fixture("sample.journal");
        let ends: Vec<String> = offsets.into_iter().map(date_from_offset).collect();
        let value = BASES[basis];
        let got = columns(&journal, depth, value, &ends);
        prop_assert_eq!(got.len(), ends.len());
        for (end, column) in ends.iter().zip(&got) {
            prop_assert_eq!(column, &single(&journal, end, depth, value));
        }
    }
}
