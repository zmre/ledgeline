//! Integration test: the native holdings engine over `fixtures/sample.journal`.
//!
//! hledger has no average-cost holdings report, so the TS engine + this fixture
//! are the oracle (not a hledger golden). The expected positions below are
//! hand-derived from the stock transactions documented in the fixture header:
//! - AAPL: 10 @ $220, +5 @ $205.75, +4.5 @ $248.30 → 19.5 sh, basis $4346.10.
//! - VTI: 15 @ $265.40, +10 @ $292.10, −8 @ $301.55 (partial) → 17 sh, $4693.36.
//! - GLD: 5 sh gifted with no cost + no P directive → tainted (basis None), with
//!   both `unpriced` and `missing-basis` warnings.
//! - NVDA: 12 bought then all 12 sold → 0 sh, dropped (must NOT appear).
//! - TSLA: 2 sold, never bought → −2 sh → negative-shares warning, excluded.

mod common;

use common::fixture_journal;
use ledgeline_core::holdings::{
    HoldingsReport, HoldingsScope, PriceSource, ScopeMode, WarningKind, compute_holdings,
    holdings_series,
};
use ledgeline_core::reports::Interval;
use ledgeline_core::{Dec, parse_journal};
use std::collections::BTreeSet;

/// All stock activity is dated ≤ 2026-06-22 and all `P` directives ≤ 2026-06-30,
/// so any `as_of` at/after 2026-06-30 yields the same snapshot.
const AS_OF: &str = "2026-07-16";

fn all_accounts_scope() -> HoldingsScope {
    HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: AS_OF.to_string(),
        gain_since: None,
    }
}

fn report() -> HoldingsReport {
    let journal = fixture_journal();
    compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &all_accounts_scope(),
    )
    .expect("holdings compute succeeds")
}

fn holding<'a>(report: &'a HoldingsReport, symbol: &str) -> &'a ledgeline_core::holdings::Holding {
    report
        .holdings
        .iter()
        .find(|h| h.symbol == symbol)
        .unwrap_or_else(|| panic!("holding {symbol} should exist"))
}

#[test]
fn base_is_dollar_and_only_stocks_appear() {
    let report = report();
    assert_eq!(report.base, "$");
    // Currencies (EUR) never appear; NVDA (fully sold) and TSLA (negative) are
    // excluded. Only AAPL, VTI, GLD remain.
    let symbols: Vec<&str> = report.holdings.iter().map(|h| h.symbol.as_str()).collect();
    // Sorted by market value desc, unpriced (GLD) last.
    assert_eq!(symbols, ["VTI", "AAPL", "GLD"]);
}

#[test]
fn aapl_average_cost_basis_and_gain() {
    let report = report();
    let aapl = holding(&report, "AAPL");
    assert_eq!(aapl.name, "Apple Inc.");
    assert_eq!(
        aapl.accounts,
        vec!["assets:broker:taxable:aapl".to_string()]
    );
    assert_eq!(aapl.shares, Dec::new(195, 1)); // 19.5 shares
    assert_eq!(aapl.basis, Some(Dec::new(434_610, 2))); // $4346.10 average cost
    assert_eq!(aapl.first_basis_date.as_deref(), Some("2024-09-16"));

    let price = aapl.price.as_ref().expect("AAPL priced by directive");
    assert_eq!(price.source, PriceSource::Directive);
    assert_eq!(price.date, "2026-06-30");
    assert_eq!(price.qty, Dec::new(27025, 2)); // $270.25

    assert_eq!(aapl.market_value, Some(Dec::new(5_269_875, 3))); // 19.5 × 270.25 = $5269.875
    assert_eq!(aapl.gain, Some(Dec::new(923_775, 3))); // $923.775 unrealized gain
    let pct = aapl.gain_pct.expect("AAPL has gain %");
    assert!((pct - 21.2552).abs() < 1e-2, "AAPL gain% was {pct}");
}

#[test]
fn vti_partial_sell_reduces_basis_at_average_cost() {
    let report = report();
    let vti = holding(&report, "VTI");
    assert_eq!(vti.name, "Vanguard Total Market");
    assert_eq!(vti.accounts, vec!["assets:broker:taxable:vti".to_string()]);
    assert_eq!(vti.shares, Dec::new(17, 0));
    // (15 @ 265.40 + 10 @ 292.10 = $6902.00) × 17/25 = $4693.36, exact.
    assert_eq!(vti.basis, Some(Dec::new(469_336, 2)));
    assert_eq!(vti.first_basis_date.as_deref(), Some("2025-02-20"));

    let price = vti.price.as_ref().expect("VTI priced by directive");
    assert_eq!(price.source, PriceSource::Directive);
    assert_eq!(price.date, "2026-06-30");
    assert_eq!(price.qty, Dec::new(31075, 2)); // $310.75

    assert_eq!(vti.market_value, Some(Dec::new(528_275, 2))); // 17 × 310.75 = $5282.75
    assert_eq!(vti.gain, Some(Dec::new(58_939, 2))); // $589.39 unrealized gain
    let pct = vti.gain_pct.expect("VTI has gain %");
    assert!((pct - 12.5580).abs() < 1e-2, "VTI gain% was {pct}");
}

#[test]
fn gld_is_tainted_and_unpriced() {
    let report = report();
    let gld = holding(&report, "GLD");
    assert_eq!(gld.accounts, vec!["assets:broker:taxable:gld".to_string()]);
    assert_eq!(gld.shares, Dec::new(5, 0));
    assert_eq!(gld.basis, None, "GLD acquired with no cost → tainted");
    assert_eq!(
        gld.price, None,
        "GLD has no P directive and no cost annotation"
    );
    assert_eq!(gld.market_value, None);
    assert_eq!(gld.gain, None);
    assert_eq!(gld.gain_pct, None);
    assert_eq!(gld.first_basis_date.as_deref(), Some("2025-08-20"));
}

#[test]
fn nvda_fully_sold_does_not_appear() {
    let report = report();
    assert!(
        report.holdings.iter().all(|h| h.symbol != "NVDA"),
        "NVDA was fully sold → 0 shares → dropped"
    );
    assert!(
        report.warnings.iter().all(|w| w.symbol != "NVDA"),
        "a clean full sell-out warns about nothing"
    );
}

#[test]
fn tsla_sold_never_bought_warns_and_is_excluded() {
    let report = report();
    assert!(
        report.holdings.iter().all(|h| h.symbol != "TSLA"),
        "TSLA is net-negative → excluded from holdings"
    );
    let tsla = report
        .warnings
        .iter()
        .find(|w| w.symbol == "TSLA")
        .expect("TSLA warning");
    assert_eq!(tsla.kind, WarningKind::NegativeShares);
    // The message now states the size of the deficit (fixture: 2 sold, 0 bought).
    assert!(
        tsla.message.contains("-2.00 shares"),
        "message was: {}",
        tsla.message
    );
}

#[test]
fn warnings_are_exactly_gld_and_tsla() {
    let report = report();
    let observed: Vec<(&str, WarningKind)> = report
        .warnings
        .iter()
        .map(|w| (w.symbol.as_str(), w.kind))
        .collect();
    // Symbol-sorted iteration: GLD (unpriced then missing-basis), then TSLA.
    assert_eq!(
        observed,
        [
            ("GLD", WarningKind::Unpriced),
            ("GLD", WarningKind::MissingBasis),
            ("TSLA", WarningKind::NegativeShares),
        ]
    );
}

#[test]
fn totals_refuse_basis_and_gain_when_gld_is_tainted() {
    let report = report();
    // Priced market value = VTI $5282.75 + AAPL $5269.875 = $10552.625.
    assert_eq!(report.totals.market_value, Dec::new(10_552_625, 3));
    assert_eq!(
        report.totals.basis, None,
        "GLD taint/unpriced refuses the basis total"
    );
    assert_eq!(report.totals.gain, None);
    assert_eq!(report.totals.gain_pct, None);
}

#[test]
fn gainers_are_aapl_then_vti_and_there_are_no_losers() {
    let report = report();
    let gainers: Vec<&str> = report
        .top_gainers
        .iter()
        .map(|h| h.symbol.as_str())
        .collect();
    // AAPL (+21.3%) ranks above VTI (+12.6%); GLD has no gain %.
    assert_eq!(gainers, ["AAPL", "VTI"]);
    assert!(report.top_losers.is_empty());
}

#[test]
fn scoping_to_a_single_stock_account_isolates_it() {
    let journal = fixture_journal();
    let scope = HoldingsScope {
        accounts: BTreeSet::from(["assets:broker:taxable:vti".to_string()]),
        mode: ScopeMode::Include,
        as_of: AS_OF.to_string(),
        gain_since: None,
    };
    let report = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &scope,
    )
    .expect("compute");
    let symbols: Vec<&str> = report.holdings.iter().map(|h| h.symbol.as_str()).collect();
    assert_eq!(symbols, ["VTI"]);
    assert_eq!(
        report.warnings,
        vec![],
        "scoping out GLD/TSLA drops their warnings"
    );
    assert_eq!(report.totals.market_value, Dec::new(528_275, 2));
    assert_eq!(report.totals.basis, Some(Dec::new(469_336, 2)));
}

#[test]
fn gain_since_windows_the_gain_without_touching_basis() {
    // At 2026-01-01 the AAPL position was 15 sh (the 4.5-sh buy is 2026-03-10),
    // priced $255.00 (P 2025-12-31) → value_at_start = $3825.00. So the windowed
    // gain is $5269.875 − $3825 = $1444.875, distinct from the all-time $923.775,
    // while `basis` stays the all-time average cost $4346.10.
    let journal = fixture_journal();
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: AS_OF.to_string(),
        gain_since: Some("2026-01-01".to_string()),
    };
    let report = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &scope,
    )
    .expect("compute");
    let aapl = holding(&report, "AAPL");
    assert_eq!(
        aapl.basis,
        Some(Dec::new(434_610, 2)),
        "basis stays all-time"
    );
    assert_eq!(aapl.market_value, Some(Dec::new(5_269_875, 3)));
    assert_eq!(aapl.gain, Some(Dec::new(1_444_875, 3)), "windowed gain");
}

#[test]
fn series_tracks_the_portfolio_over_time() {
    let journal = fixture_journal();
    let series = holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &all_accounts_scope(),
        Interval::Monthly,
        6,
    )
    .expect("series computes");

    assert_eq!(series.base, "$");
    assert_eq!(series.points.len(), 6);
    let buckets: Vec<&str> = series.points.iter().map(|p| p.bucket.as_str()).collect();
    assert_eq!(
        buckets,
        [
            "2026-02", "2026-03", "2026-04", "2026-05", "2026-06", "2026-07"
        ]
    );
    // The final bucket clamps to as_of.
    assert_eq!(series.points.last().unwrap().date, AS_OF);
    // GLD taints the basis at every point where it is held, so the basis total
    // refuses (None) while market value is still tracked.
    assert!(series.points.iter().all(|p| p.basis.is_none()));
    // Market value is monotonically present and positive by the final month.
    assert!(!series.points.last().unwrap().market_value.is_zero());
}

// ---- account-directive name inheritance (regression) ----

/// Parse `text` and resolve the `name` of the single expected holding `symbol`,
/// exercising the full parse → holdings path (so `account`-directive `name:`
/// tags must be inherited exactly as the wire `ptags` do).
fn holding_name(text: &str, symbol: &str) -> String {
    let journal = parse_journal(text, "regression.journal").expect("journal parses");
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: "2024-12-31".to_string(),
        gain_since: None,
    };
    let report = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &scope,
    )
    .expect("holdings compute succeeds");
    report
        .holdings
        .iter()
        .find(|h| h.symbol == symbol)
        .unwrap_or_else(|| panic!("holding {symbol} should exist"))
        .name
        .clone()
}

#[test]
fn account_directive_name_flows_into_holdings() {
    // The reported repro: the display name lives ONLY on the `account`
    // directive; the posting carries no `name:` comment tag.
    let journal = "\
account assets:broker:aapl    ; name: Apple Inc.
account assets:cash
commodity 1.0000 AAPL
2024-01-01 buy AAPL
    assets:broker:aapl   10 AAPL @ $220.00
    assets:cash
";
    assert_eq!(holding_name(journal, "AAPL"), "Apple Inc.");
}

#[test]
fn posting_comment_name_still_wins_over_account_directive() {
    let journal = "\
account assets:broker:aapl    ; name: Apple Inc.
account assets:cash
2024-01-01 buy AAPL
    assets:broker:aapl   10 AAPL @ $220.00  ; name: Posting Wins
    assets:cash
";
    assert_eq!(holding_name(journal, "AAPL"), "Posting Wins");
}

#[test]
fn ancestor_account_directive_name_is_inherited() {
    // Only the ANCESTOR `assets:broker` declares the name; the posted leaf
    // `assets:broker:aapl` has no declaration of its own.
    let journal = "\
account assets:broker    ; name: Broker Holdings
account assets:cash
2024-01-01 buy AAPL
    assets:broker:aapl   10 AAPL @ $220.00
    assets:cash
";
    assert_eq!(holding_name(journal, "AAPL"), "Broker Holdings");
}
