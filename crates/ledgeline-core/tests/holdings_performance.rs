//! Plan 47 A1: per-point flows and the time-weighted return on the holdings
//! series, over small inline journals so every expected number is
//! hand-checkable from the text above it.
//!
//! Flow sign: money INTO the scoped holdings is positive (a buy at its cost),
//! money out negative (a sell at its proceeds).

use ledgeline_core::holdings::{
    HoldingsPoint, HoldingsScope, ScopeMode, SeriesWindow, anchor_window, holdings_series,
    other_holdings_series, return_anchor_range,
};
use ledgeline_core::reports::Interval;
use ledgeline_core::{Dec, Journal, parse_journal};
use std::collections::BTreeSet;

fn journal(text: &str) -> Journal {
    parse_journal(text, "plan47-a1.journal").expect("journal parses")
}

fn scope(as_of: &str) -> HoldingsScope {
    HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: as_of.to_string(),
        gain_since: None,
        value_in: None,
        selection: BTreeSet::new(),
    }
}

fn usd(text: &str) -> Dec {
    Dec::parse(text, '.').expect("expectation parses")
}

fn stocks(text: &str, as_of: &str, window: &SeriesWindow) -> Vec<HoldingsPoint> {
    let journal = journal(text);
    holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope(as_of),
        window,
    )
    .expect("series computes")
    .points
}

fn flows(points: &[HoldingsPoint]) -> Vec<Dec> {
    points.iter().map(|point| point.flow).collect()
}

fn near(actual: Option<f64>, expected: f64) -> bool {
    actual.is_some_and(|actual| (actual - expected).abs() < 1e-9)
}

/// The scenario behind A1: 1,000 shares at $100; on 2025-02-15 999 of them are
/// sold at $105; the price ends February at $110. The value collapses from
/// $100,000 to $110, and the return is positive.
const SELL_ALMOST_ALL: &str = "\
account assets:broker  ; type: A
account assets:cash  ; type: C

P 2025-01-01 FUND $100.00
P 2025-02-15 FUND $105.00
P 2025-02-28 FUND $110.00

2025-01-05 buy
    assets:broker    1000 FUND @ $100.00
    assets:cash

2025-02-15 sell nearly everything
    assets:broker    -999 FUND @ $105.00
    assets:cash      $104,895.00
";

#[test]
fn selling_999_of_1000_while_the_price_rises_reads_as_a_gain() {
    let points = stocks(
        SELL_ALMOST_ALL,
        "2025-02-28",
        &SeriesWindow::counted(Interval::Monthly, 2),
    );
    assert_eq!(points[0].market_value, usd("100000.00"));
    assert_eq!(points[1].market_value, usd("110.00"));
    // A sell is money OUT: negative, at its proceeds.
    assert_eq!(flows(&points), vec![Dec::zero(), usd("-104895.00")]);
    assert!(near(points[0].return_pct, 0.0));
    // (110 − 100000 + 104895) / (100000 − 104895 / 2)
    let expected = 5005.0 / 47_552.5;
    assert!(
        near(points[1].return_pct, expected),
        "{:?}",
        points[1].return_pct
    );
}

/// Nothing held in January; 10 bought at $100 on 2025-02-10; $110 by the end
/// of February; $121 by the end of March.
const MID_WINDOW_BUY: &str = "\
account assets:broker  ; type: A
account assets:cash  ; type: C

P 2025-01-01 FUND $100.00
P 2025-02-20 FUND $110.00
P 2025-03-31 FUND $121.00

2025-02-10 buy
    assets:broker    10 FUND @ $100.00
    assets:cash
";

#[test]
fn a_first_purchase_inside_the_window_starts_the_chain_at_its_cost() {
    let points = stocks(
        MID_WINDOW_BUY,
        "2025-03-31",
        &SeriesWindow::counted(Interval::Monthly, 3),
    );
    assert_eq!(
        flows(&points),
        vec![Dec::zero(), usd("1000.00"), Dec::zero()]
    );
    assert_eq!(points[0].return_pct, None, "nothing held yet");
    // Bought for $1,000, worth $1,100: +10 % (the flow is the whole capital).
    assert!(
        near(points[1].return_pct, 0.1),
        "{:?}",
        points[1].return_pct
    );
    assert!(
        near(points[2].return_pct, 0.21),
        "{:?}",
        points[2].return_pct
    );
}

/// 100 shares at $100, flat price; a $50 dividend reinvested as 0.5 shares.
const DIVIDEND_REINVEST: &str = "\
account assets:broker  ; type: A
account assets:cash  ; type: C
account income:dividends  ; type: R

P 2025-01-01 FUND $100.00

2025-01-05 buy
    assets:broker    100 FUND @ $100.00
    assets:cash

2025-02-15 dividend reinvested
    assets:broker    0.5 FUND @ $100.00
    income:dividends
";

#[test]
fn a_reinvested_dividend_is_a_small_inflow_and_no_return_on_its_own() {
    let points = stocks(
        DIVIDEND_REINVEST,
        "2025-02-28",
        &SeriesWindow::counted(Interval::Monthly, 2),
    );
    assert_eq!(points[1].market_value, usd("10050.00"));
    assert_eq!(flows(&points), vec![Dec::zero(), usd("50.00")]);
    assert!(
        near(points[1].return_pct, 0.0),
        "{:?}",
        points[1].return_pct
    );
}

#[test]
fn the_return_anchor_rebases_the_horizon_to_the_window_start() {
    // A year of history: $100 → $150 by June, back to $120 by December. The
    // shown window is the last month only.
    let text = "\
account assets:broker  ; type: A
account assets:cash  ; type: C

P 2025-01-01 FUND $100.00
P 2025-06-30 FUND $150.00
P 2025-11-30 FUND $125.00
P 2025-12-31 FUND $120.00

2025-01-05 buy
    assets:broker    10 FUND @ $100.00
    assets:cash
";
    let journal = journal(text);
    let as_of = "2025-12-31";
    let series_over = |window: &SeriesWindow| {
        holdings_series(
            &journal.transactions,
            &journal.prices,
            &journal.accounts,
            &journal.commodity_tags,
            &scope(as_of),
            window,
        )
        .expect("series computes")
    };
    let shown = series_over(&SeriesWindow::counted(Interval::Monthly, 2));
    // Nov 30 → Dec 31: $125 → $120 = −4 %.
    assert!(near(shown.points[1].return_pct, -0.04));
    let horizon = series_over(&anchor_window("2024-12-31", as_of));
    let range = return_anchor_range(&horizon.points, &shown.points).expect("a range");
    // Rebased on November's $125: the June peak of $150 is +20 %, and the
    // January cost of $100 is −20 % — both outside the window's own −4 %.
    assert!((range.max - 0.2).abs() < 1e-9, "{range:?}");
    assert!(
        (range.min - (100.0 / 125.0 - 1.0)).abs() < 1e-9,
        "{range:?}"
    );
}

/// A house bought from cash and a mortgage, revalued against equity, then sold
/// for more than its last mark. The mortgage payment touches no row.
const OTHER: &str = "\
account assets:property:house  ; type: A
account assets:cash  ; type: C
account liabilities:mortgage  ; type: L
account equity:unrealized  ; type: E
account income:gains  ; type: R

2025-01-01 opening cash
    assets:cash    $500,000.00
    equity:unrealized

2025-02-10 buy the house
    assets:property:house    $400,000.00
    assets:cash             $-100,000.00
    liabilities:mortgage    $-300,000.00

2025-03-31 revalue
    assets:property:house    $40,000.00
    equity:unrealized

2025-04-15 mortgage payment
    liabilities:mortgage    $2,000.00
    assets:cash

2025-05-20 sell the house
    assets:property:house    $-440,000.00
    assets:cash              $460,000.00
    income:gains             $-20,000.00
";

#[test]
fn other_flows_are_read_from_the_balance_sheet_counter_legs() {
    let journal = journal(OTHER);
    let series = other_holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &scope("2025-05-31"),
        &SeriesWindow::counted(Interval::Monthly, 5),
    )
    .expect("series computes");
    let values: Vec<Dec> = series.points.iter().map(|p| p.market_value).collect();
    assert_eq!(
        values,
        vec![
            Dec::zero(),
            usd("400000.00"),
            usd("440000.00"),
            usd("440000.00"),
            Dec::zero()
        ]
    );
    assert_eq!(
        flows(&series.points),
        vec![
            Dec::zero(),
            usd("400000.00"),  // cash + mortgage in
            Dec::zero(),       // a revaluation is the return, not a flow
            Dec::zero(),       // the mortgage payment touches no row
            usd("-460000.00"), // the proceeds out
        ]
    );
    let returns: Vec<Option<f64>> = series.points.iter().map(|p| p.return_pct).collect();
    assert_eq!(returns[0], None);
    assert!(near(returns[1], 0.0));
    assert!(near(returns[2], 0.1));
    assert!(near(returns[3], 0.1));
    // Sold for $460,000 against $440,000: ×(1 + 20/440) on top of +10 %.
    assert!(
        near(returns[4], 1.1 * (1.0 + 20.0 / 440.0) - 1.0),
        "{returns:?}"
    );
}
