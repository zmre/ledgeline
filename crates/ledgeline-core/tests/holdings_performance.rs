//! Plan 47 A1: per-point flows and the time-weighted return on the holdings
//! series, over small inline journals so every expected number is
//! hand-checkable from the text above it.
//!
//! Flow sign: money INTO the scoped holdings is positive (a buy at its cost),
//! money out negative (a sell at its proceeds).

use ledgeline_core::holdings::{
    HoldingsPoint, HoldingsScope, ScopeMode, SeriesWindow, anchor_window, compute_holdings,
    holdings_flows, holdings_series, other_holdings_series, return_anchor_range,
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
        return_income: Default::default(),
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

/// REVISED for plan 48 WP4 (was `a_reinvested_dividend_is_a_small_inflow_and_no_return_on_its_own`,
/// which pinned plan 47's A1f limit: the reinvestment counted as $50 of new
/// money, so the dividend never showed in Performance). Under C6 a buy funded
/// by an income account is return: no flow, no marker, +0.5 %.
#[test]
fn a_reinvested_dividend_is_return_not_new_money() {
    let points = stocks(
        DIVIDEND_REINVEST,
        "2025-02-28",
        &SeriesWindow::counted(Interval::Monthly, 2),
    );
    assert_eq!(points[1].market_value, usd("10050.00"));
    assert_eq!(flows(&points), vec![Dec::zero(), Dec::zero()], "no marker");
    assert!(
        near(points[1].return_pct, 0.005),
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

// ── Plan 48 WP4 (C6): income-funded buys and in-kind fees are not flows ──
//
// Every journal below holds a position at a flat price from January and does
// one thing in February, so the February return is that one thing's value
// over the January value, and the flows say whether it was money in or out.

fn monthly_feb(text: &str) -> Vec<HoldingsPoint> {
    stocks(
        text,
        "2025-02-28",
        &SeriesWindow::counted(Interval::Monthly, 2),
    )
}

fn holding(text: &str, as_of: &str, symbol: &str) -> ledgeline_core::holdings::Holding {
    let journal = journal(text);
    let report = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope(as_of),
    )
    .expect("report computes");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    report
        .holdings
        .into_iter()
        .find(|holding| holding.symbol == symbol)
        .expect("symbol held")
}

fn all_flows(text: &str, as_of: &str) -> Vec<(String, Dec)> {
    let journal = journal(text);
    holdings_flows(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope(as_of),
        None,
    )
    .expect("flows compute")
    .flows
    .into_iter()
    .map(|flow| (flow.date, flow.amount))
    .collect()
}

/// 1 ETH at $3,000, then a 0.01 ETH staking reward booked at fair value.
const STAKING_REWARD: &str = "\
account assets:crypto  ; type: A
account assets:bank  ; type: C
account income:staking  ; type: R

P 2025-01-01 ETH $3000.00

2025-01-05 buy
    assets:crypto    1 ETH @ $3000.00
    assets:bank

2025-02-15 staking reward
    assets:crypto    0.01 ETH @ $3000.00
    income:staking
";

#[test]
fn a_staking_reward_is_return_not_new_money() {
    let points = monthly_feb(STAKING_REWARD);
    assert_eq!(points[1].market_value, usd("3030.00"));
    assert_eq!(flows(&points), vec![Dec::zero(), Dec::zero()], "no marker");
    assert!(
        near(points[1].return_pct, 0.01),
        "{:?}",
        points[1].return_pct
    );
    // The holding's own accounting is unchanged: the reward is basis at its
    // fair value (income now, gain only on what the coin does next).
    let eth = holding(STAKING_REWARD, "2025-02-28", "ETH");
    assert_eq!(eth.shares, usd("1.01"));
    assert_eq!(eth.basis, Some(usd("3030.00")));
    assert_eq!(eth.gain, Some(Dec::zero()));
}

/// The crypto docs' example: a wallet move that pays its network fee in the
/// coin to an expense account.
const COIN_FEE: &str = "\
account assets:crypto:coinbase  ; type: A
account assets:crypto:cold  ; type: A
account assets:bank  ; type: C
account expenses:fees:crypto  ; type: X

P 2025-01-01 BTC $60000.00

2025-01-05 buy
    assets:crypto:coinbase    0.05000000 BTC @ $60000.00
    assets:bank

2025-02-09 move to cold storage
    assets:crypto:cold         0.04990000 BTC
    expenses:fees:crypto       0.00010000 BTC
    assets:crypto:coinbase    -0.05000000 BTC
";

#[test]
fn an_in_kind_coin_fee_is_a_loss_not_a_withdrawal() {
    let points = monthly_feb(COIN_FEE);
    assert_eq!(points[1].market_value, usd("2994.00"));
    assert_eq!(flows(&points), vec![Dec::zero(), Dec::zero()], "no marker");
    // $6 of $3,000 paid away: −0.2 %.
    assert!(
        near(points[1].return_pct, -0.002),
        "{:?}",
        points[1].return_pct
    );
    // The fee leaves with its share of the basis, as a sale at nothing would;
    // the arriving cold-storage leg is the same coins, not a cost-less buy
    // (it used to taint the basis to unknown).
    let btc = holding(COIN_FEE, "2025-02-28", "BTC");
    assert_eq!(btc.shares, usd("0.0499"));
    assert_eq!(btc.basis, Some(usd("2994.00")));
    assert_eq!(btc.gain, Some(Dec::zero()));
}

/// An advisory fee taken in shares of the fund.
const SHARE_FEE: &str = "\
account assets:broker  ; type: A
account assets:cash  ; type: C
account expenses:fees  ; type: X

P 2025-01-01 FUND $100.00

2025-01-05 buy
    assets:broker    100 FUND @ $100.00
    assets:cash

2025-02-15 advisory fee
    assets:broker    -0.5 FUND
    expenses:fees     0.5 FUND
";

#[test]
fn an_in_kind_share_fee_is_a_loss_not_a_withdrawal() {
    let points = monthly_feb(SHARE_FEE);
    assert_eq!(points[1].market_value, usd("9950.00"));
    assert_eq!(flows(&points), vec![Dec::zero(), Dec::zero()], "no marker");
    assert!(
        near(points[1].return_pct, -0.005),
        "{:?}",
        points[1].return_pct
    );
    let fund = holding(SHARE_FEE, "2025-02-28", "FUND");
    assert_eq!(fund.basis, Some(usd("9950.00")));
}

#[test]
fn part_income_part_cash_funding_leaves_only_the_cash_as_a_flow() {
    let text = "\
account assets:broker  ; type: A
account assets:cash  ; type: C
account income:dividends  ; type: R

P 2025-01-01 FUND $100.00

2025-01-05 buy
    assets:broker    100 FUND @ $100.00
    assets:cash

2025-02-15 dividend topped up from cash and reinvested
    assets:broker      1 FUND @ $100.00
    income:dividends   $-40.00
    assets:cash        $-60.00
";
    let points = monthly_feb(text);
    assert_eq!(flows(&points), vec![Dec::zero(), usd("60.00")]);
    // (10100 − 10000 − 60) / (10000 + 30)
    assert!(
        near(points[1].return_pct, 40.0 / 10_030.0),
        "{:?}",
        points[1].return_pct
    );
}

#[test]
fn an_equity_funded_arrival_is_still_money_in() {
    // An ACATS-style transfer from another broker, booked against equity:
    // money arriving from outside the books' holdings, so still a flow.
    let text = "\
account assets:broker  ; type: A
account assets:cash  ; type: C
account equity:transfers  ; type: E

P 2025-01-01 FUND $100.00

2025-01-05 buy
    assets:broker    100 FUND @ $100.00
    assets:cash

2025-02-15 transferred in
    assets:broker      10 FUND @ $100.00
    equity:transfers
";
    let points = monthly_feb(text);
    assert_eq!(flows(&points), vec![Dec::zero(), usd("1000.00")]);
    assert!(near(points[1].return_pct, 0.0));
}

/// Benchmarks replay `holdings_flows`: none of the income-funded or in-kind
/// events reaches them, so the benchmark is not handed money the portfolio
/// never received.
#[test]
fn benchmark_flows_ignore_income_funded_buys_and_in_kind_fees() {
    for text in [DIVIDEND_REINVEST, STAKING_REWARD, COIN_FEE, SHARE_FEE] {
        let flows = all_flows(text, "2025-02-28");
        assert_eq!(flows.len(), 1, "only the January buy: {flows:?}");
        assert_eq!(flows[0].0, "2025-01-05");
    }
}

#[test]
fn a_windowed_gain_counts_the_reward_while_the_all_time_gain_does_not() {
    // The windowed gain subtracts the same flows Performance does, so the
    // reward is gain inside a window; the all-time gain is value − basis, and
    // the reward is basis.
    let journal = journal(STAKING_REWARD);
    let windowed = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &HoldingsScope {
            gain_since: Some("2025-01-31".to_string()),
            ..scope("2025-02-28")
        },
    )
    .expect("report computes");
    assert_eq!(windowed.holdings[0].gain, Some(usd("30.00")));
    let eth = holding(STAKING_REWARD, "2025-02-28", "ETH");
    assert_eq!(eth.gain, Some(Dec::zero()));
}

/// The Other tab reads its flows from balance-sheet counter-legs, so income,
/// expense and equity counter-legs were never flows there. Pinned so WP4's
/// Stocks change cannot leak into it.
#[test]
fn other_tab_flows_still_exclude_income_expense_and_equity_counter_legs() {
    let text = "\
account assets:property:land  ; type: A
account assets:cash  ; type: C
account equity:opening  ; type: E
account income:appreciation  ; type: R
account expenses:depreciation  ; type: X

2025-01-01 opening
    assets:property:land    $100,000.00
    equity:opening

2025-02-15 appraisal booked as income
    assets:property:land    $5,000.00
    income:appreciation

2025-03-15 written down
    expenses:depreciation   $1,000.00
    assets:property:land

2025-04-15 bought the next lot from cash
    assets:property:land    $20,000.00
    assets:cash
";
    let journal = journal(text);
    let series = other_holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &scope("2025-04-30"),
        &SeriesWindow::counted(Interval::Monthly, 4),
    )
    .expect("series computes");
    assert_eq!(
        flows(&series.points),
        vec![Dec::zero(), Dec::zero(), Dec::zero(), usd("20000.00")]
    );
    let returns: Vec<Option<f64>> = series.points.iter().map(|p| p.return_pct).collect();
    assert!(near(returns[1], 0.05), "{returns:?}");
    // Cumulative: $100,000 → $104,000 is +4 %, and the April lot is money in.
    assert!(near(returns[2], 0.04), "{returns:?}");
    assert!(near(returns[3], 0.04), "{returns:?}");
}
