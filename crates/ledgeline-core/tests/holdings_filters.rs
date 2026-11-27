//! Plan 47: the holdings filter (`HoldingsScope::selection`), the y-axis
//! anchor range, and live same-day quotes — over small inline journals, so
//! every expected number is hand-checkable from the text above it.

use ledgeline_core::holdings::{
    AnchorHorizon, HoldingsScope, LiveQuote, ScopeMode, SeriesWindow, anchor_range, anchor_window,
    compute_holdings, first_holding_date, first_other_holding_date, holdings_flows,
    holdings_series, live_price_directives, other_holdings, other_holdings_series,
};
use ledgeline_core::reports::Interval;
use ledgeline_core::{Dec, Journal, parse_journal};
use std::collections::BTreeSet;

/// VTI: 10 @ $200 on 2026-01-05, priced $210 then $230. AAPL: 4 @ $150 on
/// 2026-03-02, priced $160 then $170. Both in one brokerage account.
const STOCKS: &str = "\
account assets:broker  ; type: A
account assets:cash  ; type: C

P 2026-01-31 VTI $210.00
P 2026-01-31 AAPL $140.00
P 2026-03-31 VTI $230.00
P 2026-03-31 AAPL $160.00
P 2026-04-30 AAPL $170.00

2026-01-05 buy vti
    assets:broker    10 VTI @ $200.00
    assets:cash

2026-03-02 buy apple
    assets:broker    4 AAPL @ $150.00
    assets:cash
";

/// A house and a van, each its own Other-tab row.
const OTHER: &str = "\
account assets:property:house  ; type: A
account assets:vehicles:van  ; type: A
account equity:opening  ; type: E

2026-01-01 opening
    assets:property:house    $400,000.00
    assets:vehicles:van      $30,000.00
    equity:opening
";

fn journal(text: &str) -> Journal {
    parse_journal(text, "plan47.journal").expect("journal parses")
}

fn scope(as_of: &str, selection: &[&str]) -> HoldingsScope {
    HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: as_of.to_string(),
        gain_since: None,
        value_in: None,
        selection: selection.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn usd(text: &str) -> Dec {
    Dec::parse(text, '.').expect("expectation parses")
}

// ---------------------------------------------------------------------------
// The holdings filter
// ---------------------------------------------------------------------------

#[test]
fn a_symbol_filter_counts_only_the_selected_holdings() {
    let journal = journal(STOCKS);
    let report = |selection: &[&str]| {
        compute_holdings(
            &journal.transactions,
            &journal.prices,
            &journal.accounts,
            &journal.commodity_tags,
            &scope("2026-04-30", selection),
        )
        .expect("report")
    };
    let all = report(&[]);
    assert_eq!(all.totals.market_value, usd("2980")); // 10×230 + 4×170
    let vti = report(&["VTI"]);
    let symbols: Vec<&str> = vti.holdings.iter().map(|h| h.symbol.as_str()).collect();
    assert_eq!(symbols, ["VTI"]);
    assert_eq!(vti.totals.market_value, usd("2300"));
    assert_eq!(vti.totals.basis, Some(usd("2000")));
    assert_eq!(
        vti.base, all.base,
        "the filter never moves the valuation base"
    );
    // The chooser's options ignore the filter, like the account scope.
    assert_eq!(vti.accounts, all.accounts);
    // A symbol nobody holds selects nothing: an honest empty report.
    assert!(report(&["NOPE"]).holdings.is_empty());
}

#[test]
fn the_series_and_its_inception_follow_the_filter() {
    let journal = journal(STOCKS);
    let window = SeriesWindow::counted(Interval::Monthly, 4);
    let series = holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope("2026-04-30", &["AAPL"]),
        &window,
    )
    .expect("series");
    let values: Vec<Dec> = series.points.iter().map(|p| p.market_value).collect();
    assert_eq!(values, [usd("0"), usd("0"), usd("640"), usd("680")]);

    let first = |selection: &[&str]| {
        first_holding_date(
            &journal.transactions,
            &journal.accounts,
            &scope("2026-04-30", selection),
        )
    };
    assert_eq!(first(&[]).as_deref(), Some("2026-01-05"));
    assert_eq!(first(&["AAPL"]).as_deref(), Some("2026-03-02"));
}

#[test]
fn benchmark_flows_seed_from_only_the_selected_holdings() {
    let journal = journal(STOCKS);
    let flows = |selection: &[&str]| {
        holdings_flows(
            &journal.transactions,
            &journal.prices,
            &journal.accounts,
            &journal.commodity_tags,
            &scope("2026-04-30", selection),
            None,
        )
        .expect("flows")
        .flows
        .into_iter()
        .map(|flow| (flow.date, flow.amount))
        .collect::<Vec<_>>()
    };
    assert_eq!(
        flows(&[]),
        [
            ("2026-01-05".to_string(), usd("2000")),
            ("2026-03-02".to_string(), usd("600"))
        ]
    );
    assert_eq!(flows(&["VTI"]), [("2026-01-05".to_string(), usd("2000"))]);
}

#[test]
fn a_rows_filter_narrows_the_other_tab_to_those_accounts() {
    let journal = journal(OTHER);
    let sc = scope("2026-06-30", &["assets:vehicles:van"]);
    let report = other_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &sc,
    )
    .expect("report");
    let rows: Vec<&str> = report.holdings.iter().map(|h| h.account.as_str()).collect();
    assert_eq!(rows, ["assets:vehicles:van"]);
    assert_eq!(report.totals.value, usd("30000"));
    assert_eq!(
        report.accounts,
        ["assets:property:house", "assets:vehicles:van"]
    );

    let series = other_holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &sc,
        &SeriesWindow::counted(Interval::Monthly, 2),
    )
    .expect("series");
    assert!(series.points.iter().all(|p| p.market_value == usd("30000")));
    assert_eq!(
        first_other_holding_date(
            &journal.transactions,
            &journal.prices,
            &journal.accounts,
            &sc
        )
        .expect("first")
        .as_deref(),
        Some("2026-01-01")
    );
}

// ---------------------------------------------------------------------------
// The anchor range
// ---------------------------------------------------------------------------

/// The case D2 exists for: an opening balance lands $50,000 on a portfolio
/// that read $0 the month before. The axis must start near $50k, not $0.
#[test]
fn leading_zeros_before_an_opening_balance_never_pin_the_axis_at_zero() {
    let text = "\
account assets:broker  ; type: A
account equity:opening  ; type: E

P 2026-03-01 VTI $250.00
P 2026-05-01 VTI $255.00

2026-03-15 opening balance
    assets:broker    200 VTI @ $250.00
    equity:opening
";
    let journal = journal(text);
    let sc = scope("2026-06-30", &[]);
    let start = AnchorHorizon::OneYear
        .start(&sc.as_of, || unreachable!())
        .expect("a year back");
    let anchor = holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &sc,
        &anchor_window(&start, &sc.as_of),
    )
    .expect("anchor series");
    assert!(
        anchor
            .points
            .first()
            .is_some_and(|p| p.market_value.is_zero()),
        "the horizon starts before the opening balance"
    );
    let range = anchor_range([anchor.points.as_slice()]).expect("something > 0");
    assert_eq!(range.min, usd("50000"));
    assert_eq!(range.max, usd("51000"));
}

#[test]
fn the_shown_window_joins_the_range_and_a_later_zero_still_counts() {
    let journal = journal(STOCKS);
    let points = |as_of: &str, count| {
        holdings_series(
            &journal.transactions,
            &journal.prices,
            &journal.accounts,
            &journal.commodity_tags,
            &scope(as_of, &[]),
            &SeriesWindow::counted(Interval::Monthly, count),
        )
        .expect("series")
        .points
    };
    // Anchor Jan–Mar: 2100, 2100, 2940. Shown Apr: 2980 — above the anchor.
    let anchor = points("2026-03-31", 3);
    let shown = points("2026-04-30", 1);
    let range = anchor_range([anchor.as_slice(), shown.as_slice()]).expect("range");
    assert_eq!(range.min, usd("2100"));
    assert_eq!(range.max, usd("2980"));

    // A zero AFTER the first positive point is a real value, not a lead-in.
    let mut dipped = anchor.clone();
    dipped[1].market_value = Dec::zero();
    let range = anchor_range([dipped.as_slice()]).expect("range");
    assert_eq!(range.min, Dec::zero());
}

#[test]
fn nothing_above_zero_is_no_anchor() {
    let journal = journal(STOCKS);
    let series = holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope("2025-12-31", &[]),
        &SeriesWindow::counted(Interval::Monthly, 3),
    )
    .expect("series");
    assert_eq!(anchor_range([series.points.as_slice()]), None);
    assert_eq!(anchor_range(std::iter::empty()), None);
}

// ---------------------------------------------------------------------------
// Live quotes
// ---------------------------------------------------------------------------

#[test]
fn a_live_quote_values_today_and_overrides_todays_close() {
    let text = format!("{STOCKS}\nP 2026-05-04 VTI $231.00\n");
    let journal = journal(&text);
    let today = "2026-05-04";
    let quotes = [LiveQuote {
        symbol: "VTI".to_string(),
        price: usd("235.50"),
        date: today.to_string(),
        currency: Some("USD".to_string()),
    }];
    let mut prices = journal.prices.clone();
    prices.extend(live_price_directives(&journal.prices, &quotes, today));
    let report = |prices, as_of: &str| {
        compute_holdings(
            &journal.transactions,
            prices,
            &journal.accounts,
            &journal.commodity_tags,
            &scope(as_of, &["VTI"]),
        )
        .expect("report")
    };
    assert_eq!(report(&prices, today).totals.market_value, usd("2355"));
    assert_eq!(
        report(&journal.prices, today).totals.market_value,
        usd("2310")
    );
    // Yesterday cannot see today's quote.
    assert_eq!(
        report(&prices, "2026-05-03").totals.market_value,
        usd("2300")
    );
}
