//! Integration tests pinning four arithmetic/attribution fixes in the Insights
//! dashboard. Each one silently produced a plausible-but-wrong number, so every
//! assertion below spells out the truth it is protecting.

mod common;

use common::fixture_journal;
use ledgeline_core::Dec;
use ledgeline_core::holdings::{HoldingsScope, ScopeMode, WarningKind, compute_holdings};
use ledgeline_core::model::Commodity;
use ledgeline_core::parse_journal;
use ledgeline_core::reports::{
    InsightsOpts, InsightsReport, Interval, NetWorthOpts, infer_market_prices, insights, net_worth,
};
use std::collections::{BTreeMap, BTreeSet};

fn usd() -> Commodity {
    Commodity("$".to_string())
}

fn run(text: &str, start: &str, end: &str) -> InsightsReport {
    let journal = parse_journal(text, "insights-arithmetic").expect("journal parses");
    insights(
        &journal,
        &InsightsOpts {
            start,
            end,
            cost_exclude: &[],
            change_min: Dec::zero(),
        },
    )
    .expect("insights succeeds")
}

/// `account → (previous, current, delta)` in dollars, for readable assertions.
fn changes(report: &InsightsReport) -> BTreeMap<String, (f64, f64, f64)> {
    report
        .expense_changes
        .iter()
        .map(|row| {
            (
                row.account.clone(),
                (
                    row.previous.floating_point(),
                    row.current.floating_point(),
                    row.delta.floating_point(),
                ),
            )
        })
        .collect()
}

// ===========================================================================
// Item 1 — a parent's OWN direct postings must not vanish from "biggest changes"
// ===========================================================================

/// `expenses:food` is posted to DIRECTLY ($500 → $900) and also has a posted
/// child `expenses:food:dining` ($100 → $120). Keeping only leaves dropped the
/// parent entirely, so Box 7 reported a +$20 move while Box 2 reported +$420.
const PARENT_WITH_OWN_POSTINGS: &str = "\
2025-03-01 parent direct
    expenses:food                 $500.00
    assets:bank:checking

2025-03-02 child
    expenses:food:dining          $100.00
    assets:bank:checking

2026-03-01 parent direct
    expenses:food                 $900.00
    assets:bank:checking

2026-03-02 child
    expenses:food:dining          $120.00
    assets:bank:checking
";

#[test]
fn parent_own_postings_are_reported_and_reconcile_with_the_expenses_box() {
    let report = run(PARENT_WITH_OWN_POSTINGS, "2025-01-01", "2026-12-31");
    let rows = changes(&report);

    // The parent's own direct spending is a row of its own, marked "(own)" so it
    // is never read as the inclusive `expenses:food` subtree total.
    assert_eq!(
        rows.get("expenses:food (own)"),
        Some(&(500.0, 900.0, 400.0)),
        "the parent's own +$400 must be reported, not silently dropped"
    );
    assert_eq!(
        rows.get("expenses:food:dining"),
        Some(&(100.0, 120.0, 20.0)),
        "the child keeps its plain leaf name"
    );
    assert_eq!(rows.len(), 2, "exactly two comparable accounts");

    // The two rows must add up to the Expenses box — the contradiction that made
    // this a bug rather than a presentation quirk.
    let box2 = &report.expenses;
    assert_eq!(box2.previous.get(&usd()), Some(Dec::new(60_000, 2)));
    assert_eq!(box2.current.get(&usd()), Some(Dec::new(102_000, 2)));
    assert_eq!(box2.delta.get(&usd()), Some(Dec::new(42_000, 2)));
    let listed: f64 = rows.values().map(|(_, _, delta)| delta).sum();
    assert!(
        (listed - 420.0).abs() < 1e-9,
        "Box 7 deltas must sum to Box 2's +$420, got {listed}"
    );
}

#[test]
fn a_parent_without_children_keeps_its_plain_name() {
    // No child account, so no "(own)" marker: the row IS the whole account.
    let report = run(
        "\
2025-03-01 a
    expenses:food                 $500.00
    assets:bank:checking

2026-03-01 b
    expenses:food                 $900.00
    assets:bank:checking
",
        "2025-01-01",
        "2026-12-31",
    );
    let rows = changes(&report);
    assert_eq!(rows.get("expenses:food"), Some(&(500.0, 900.0, 400.0)));
    assert_eq!(rows.len(), 1);
}

#[test]
fn a_pure_container_parent_is_not_invented() {
    // `expenses:food` is never posted to directly, so it must NOT appear at all —
    // an "(own)" row of zero would be noise, and a rollup row would double-count.
    let report = run(
        "\
2025-03-01 a
    expenses:food:dining          $100.00
    assets:bank:checking

2026-03-01 b
    expenses:food:dining          $120.00
    assets:bank:checking
",
        "2025-01-01",
        "2026-12-31",
    );
    let rows = changes(&report);
    assert_eq!(rows.keys().collect::<Vec<_>>(), ["expenses:food:dining"]);
}

// ===========================================================================
// Items 2 & 3 — month counting and the period split (they interact)
// ===========================================================================

#[test]
fn months_measure_elapsed_time_not_touched_calendar_months() {
    // 2025-01-15 … 2026-01-14 is exactly 12 months. Counting TOUCHED calendar
    // months gave each half 7 (Jan…Jul and Jul…Jan), so the SPA divided by 7
    // instead of 6 and understated average monthly cost of living by ~14%.
    let report = run("", "2025-01-15", "2026-01-14");
    assert_eq!(report.period.mid, "2025-07-16");
    assert_eq!(report.period.prev_start, "2025-01-15");
    assert_eq!(report.period.curr_start, "2025-07-17");

    // previous = [2025-01-15, 2025-07-16] = 6 months + 1 day   -> 6
    // current  = [2025-07-17, 2026-01-14] = 5 months + 29 days -> 6
    assert_eq!(report.cost_of_living.months_previous, 6);
    assert_eq!(report.cost_of_living.months_current, 6);

    // The two fixes interact: correcting the month count does NOT remove the
    // day-level imbalance the split leaves behind. 365 days is odd, so the
    // previous period keeps 183 against the current period's 182 — now visible
    // instead of hidden behind two equal "7 month" labels.
    assert_eq!(report.period.prev_days, 183);
    assert_eq!(report.period.curr_days, 182);
    assert_eq!(report.period.prev_days + report.period.curr_days, 365);
}

#[test]
fn month_aligned_spans_keep_their_exact_month_counts() {
    // The "Year-over-year" preset must be untouched by the elapsed-time change:
    // for a month-aligned span the new definition is arithmetically identical to
    // the old one, so the calendar split and both counts are unchanged.
    let report = run("", "2024-07-01", "2026-06-30");
    assert_eq!(report.period.mid, "2025-06-30");
    assert_eq!(report.period.curr_start, "2025-07-01");
    assert_eq!(report.cost_of_living.months_previous, 12);
    assert_eq!(report.cost_of_living.months_current, 12);
    assert_eq!(report.period.prev_days, 365);
    assert_eq!(report.period.curr_days, 365);
}

#[test]
fn an_odd_day_span_publishes_its_unequal_halves() {
    // 31 days cannot be halved. The split gives the previous period 16 days and
    // the current 15 — a ~6.7% head start that used to be invisible because both
    // halves were labelled "1 month" and nothing else was reported.
    let report = run("", "2026-01-01", "2026-01-31");
    assert_eq!(report.period.mid, "2026-01-16");
    assert_eq!(report.period.prev_days, 16);
    assert_eq!(report.period.curr_days, 15);
    assert_eq!(
        report.period.prev_days + report.period.curr_days,
        31,
        "the halves must still tile the whole span"
    );
    // Whole months are too coarse to resolve one day; the day counts are the only
    // place that difference is visible.
    assert_eq!(report.cost_of_living.months_previous, 1);
    assert_eq!(report.cost_of_living.months_current, 1);
}

#[test]
fn an_even_day_span_splits_exactly_in_half() {
    // 30 days -> 15/15, no imbalance to report.
    let report = run("", "2026-01-01", "2026-01-30");
    assert_eq!(report.period.prev_days, 15);
    assert_eq!(report.period.curr_days, 15);
}

#[test]
fn a_sub_month_period_never_reports_zero_months() {
    // The caller divides by this, so zero would blank the cost-of-living box.
    let report = run("", "2025-01-01", "2025-01-11");
    assert_eq!(report.cost_of_living.months_previous, 1);
    assert_eq!(report.cost_of_living.months_current, 1);
}

// ===========================================================================
// Item 4 — one price set for every box (the $1,000 GLD discrepancy)
// ===========================================================================

/// `fixtures/sample.journal` @ 2026-06-30: net worth counts the gifted GLD lot at
/// 5 × $200 = $1,000 (hledger agrees: `bal --value=end,'$' --infer-market-prices`
/// reports `$61,795.50`), because the only thing pricing GLD is the `@ 0.005 GLD`
/// cost annotation on its balancing equity leg. The holdings boxes were handed
/// explicit `P` directives ONLY, so they reported GLD unpriced and dropped it.
#[test]
fn holdings_and_net_worth_agree_on_the_cost_priced_gld_lot() {
    let journal = fixture_journal();
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: "2026-06-30".to_string(),
        gain_since: None,
    };

    // The set every Insights box now uses: inferred first so an explicit price
    // wins a same-date tie.
    let mut all = infer_market_prices(&journal.transactions).expect("inference succeeds");
    all.extend_from_slice(&journal.prices);

    let with_inferred = compute_holdings(
        &journal.transactions,
        &all,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
    )
    .expect("holdings compute succeeds");
    let gld = with_inferred
        .holdings
        .iter()
        .find(|h| h.symbol == "GLD")
        .expect("GLD is held");
    assert_eq!(
        gld.market_value.map(|v| v.floating_point()),
        Some(1_000.0),
        "GLD is worth 5 x $200 = $1,000, exactly what net worth counts"
    );
    assert!(
        !with_inferred
            .warnings
            .iter()
            .any(|w| w.symbol == "GLD" && w.kind == WarningKind::Unpriced),
        "GLD is no longer unpriced"
    );

    // The explicit-only set is what produced the contradiction: same journal,
    // same date, $1,000 less.
    let explicit_only = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
    )
    .expect("holdings compute succeeds");
    let missing = with_inferred
        .totals
        .market_value
        .sub(explicit_only.totals.market_value)
        .expect("exact subtraction");
    assert_eq!(
        missing.floating_point(),
        1_000.0,
        "the two price sets differ by exactly the GLD lot"
    );
}

#[test]
fn insights_movers_price_the_gld_lot() {
    // Box 8 is built from the same holdings snapshot, so GLD must now be
    // priceable there too rather than silently absent.
    let journal = fixture_journal();
    let report = insights(
        &journal,
        &InsightsOpts {
            start: "2024-07-01",
            end: "2026-06-30",
            cost_exclude: &[],
            change_min: Dec::zero(),
        },
    )
    .expect("insights succeeds");
    assert!(
        report.movers.iter().any(|m| m.symbol == "GLD"),
        "GLD appears among the movers now that it has a price; got {:?}",
        report.movers.iter().map(|m| &m.symbol).collect::<Vec<_>>()
    );
}

#[test]
fn insights_net_worth_matches_the_net_worth_report_at_the_period_end() {
    // Box 3 is unchanged by the price-set fix (net_worth already inferred), and
    // this pins the $61,795.505 figure the holdings boxes now reconcile against.
    let journal = fixture_journal();
    let report = insights(
        &journal,
        &InsightsOpts {
            start: "2024-07-01",
            end: "2026-06-30",
            cost_exclude: &[],
            change_min: Dec::zero(),
        },
    )
    .expect("insights succeeds");

    let declared = BTreeMap::new();
    let nw = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-06-30",
            interval: Interval::Daily,
            count: 1,
            depth: 1,
            value_in: Some(usd()),
            declared: &declared,
        },
    )
    .expect("net worth succeeds");
    assert_eq!(
        report.net_worth.current.get(&usd()),
        nw.totals[0].get(&usd())
    );
    assert_eq!(
        report
            .net_worth
            .current
            .get(&usd())
            .expect("base present")
            .floating_point(),
        61_795.505,
        "hledger reports $61,795.50 for the same query"
    );
}
