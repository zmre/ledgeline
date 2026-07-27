//! Integration test: which commodity a holdings report ends up denominated in
//! (CLEANUP.md HOLD-3), over `fixtures/reports/fx-cross-rates*.journal`.
//!
//! The engine reduces a whole portfolio to ONE base commodity, which hledger
//! never has to do — its `-V` values each commodity in that commodity's own
//! latest price target, so an unrelated exchange rate cannot reach a stock. Ours
//! picked "the commodity most things are priced IN", which a handful of travel
//! cross-rates could outvote; the portfolio then had no route to the winner and
//! reported zero. The reference numbers below are `hledger 1.52`'s, since a
//! per-commodity valuation and a single-base one agree exactly when the base is
//! the one that prices the holdings.

mod common;

use common::fixtures_dir;
use ledgeline_core::holdings::{
    HoldingsReport, HoldingsScope, ScopeMode, WarningKind, compute_holdings, valuation_base,
};
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::PriceDb;
use ledgeline_core::{Dec, Journal, parse_journal};
use std::collections::BTreeSet;

/// Every price and posting is dated ≤ 2026-07-03, so any later `as_of` is stable.
const AS_OF: &str = "2026-07-16";

fn journal(name: &str) -> Journal {
    let path = fixtures_dir().join("reports").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} readable: {e}"));
    parse_journal(&text, &path.to_string_lossy()).expect("journal parses")
}

fn scope(value_in: Option<&str>) -> HoldingsScope {
    HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: AS_OF.to_string(),
        gain_since: None,
        value_in: value_in.map(|symbol| Commodity(symbol.to_string())),
    }
}

fn report(journal: &Journal, value_in: Option<&str>) -> HoldingsReport {
    compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope(value_in),
    )
    .expect("compute_holdings succeeds")
}

/// HOLD-3, exactly as reported: three `P … 1.15 EUR` travel rates outvoted the
/// one `P VTI $120.00`, EUR became the base, no chain of prices reached VTI from
/// it, and a $1,200 portfolio read $0 with a null basis and two warnings.
///
/// hledger 1.52 reference:
/// `hledger -f fixtures/reports/fx-cross-rates.journal bal --value=end,'$' -e 2026-07-16`
/// → `$1,200.00  assets:broker:vti`.
#[test]
fn travel_cross_rates_no_longer_zero_a_dollar_portfolio() {
    let journal = journal("fx-cross-rates.journal");
    // The frequency ranking on its own still answers EUR — the bug's cause is
    // intact and it is the CHOICE built on top of it that changed.
    assert_eq!(
        PriceDb::build(&journal.prices).base_commodity(),
        Some(&Commodity("EUR".to_string()))
    );

    let report = report(&journal, None);
    assert_eq!(report.base, "$");
    assert_eq!(report.totals.market_value, Dec::new(120_000, 2));
    assert_eq!(report.totals.basis, Some(Dec::new(120_000, 2)));
    assert_eq!(report.totals.gain, Some(Dec::zero()));

    let vti = &report.holdings[0];
    assert_eq!(vti.symbol, "VTI");
    assert_eq!(vti.shares, Dec::new(10, 0));
    assert_eq!(vti.market_value, Some(Dec::new(120_000, 2)));
    assert_eq!(vti.basis, Some(Dec::new(120_000, 2)));

    // Both of the finding's warnings are gone — including the misleading
    // "acquired without a cost annotation", which named the wrong cause for a
    // lot that WAS annotated (`10 VTI @ $120.00`).
    assert!(
        report.warnings.is_empty(),
        "expected no warnings, got {:?}",
        report.warnings
    );
}

/// The same journal, valued in EUR on purpose: the caller's choice is used
/// verbatim, and the report says so rather than quietly substituting `$`.
/// (`/api/holdings` refuses this request up front — see the endpoint tests.)
#[test]
fn an_explicit_value_in_is_used_even_when_it_prices_nothing() {
    let journal = journal("fx-cross-rates.journal");
    let report = report(&journal, Some("EUR"));
    assert_eq!(report.base, "EUR");
    assert!(report.totals.market_value.is_zero());
    assert_eq!(report.holdings[0].market_value, None);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.kind == WarningKind::Unpriced),
        "an unpriceable base has to SAY so: {:?}",
        report.warnings
    );
}

/// When both candidates price the portfolio, coverage cannot break the tie and
/// the frequency ranking answers EUR (two `… EUR` directives against one `… $`).
/// This is what the journal's `D` directive exists to override.
///
/// hledger 1.52 reference:
/// `… bal --value=end,EUR -e 2026-07-16` → `1100.00 EUR  assets:broker:vti`.
#[test]
fn a_coverage_tie_still_falls_back_to_the_frequency_ranking() {
    let journal = journal("fx-cross-rates-declared.journal");
    let report = report(&journal, None);
    assert_eq!(report.base, "EUR");
    assert_eq!(report.totals.market_value, Dec::new(110_000, 2));
    // No rate converts the `$120.00` cost annotation into EUR, so the basis is
    // honestly unknown rather than silently wrong.
    assert_eq!(report.totals.basis, None);
}

/// …and with the journal's own `D $1,000.00` honoured (which is what
/// `/api/holdings` does when no `valueIn` is given), the same journal reports in
/// dollars — matching `hledger … bal --value=end,'$'` → `$1,200.00`.
#[test]
fn the_declared_default_commodity_settles_the_tie() {
    let journal = journal("fx-cross-rates-declared.journal");
    assert_eq!(
        journal.default_commodity,
        Some(Commodity("$".to_string())),
        "the `D $1,000.00` directive is what the HTTP layer reads"
    );
    let report = report(&journal, journal.default_commodity.as_ref().map(|c| &*c.0));
    assert_eq!(report.base, "$");
    assert_eq!(report.totals.market_value, Dec::new(120_000, 2));
    assert_eq!(report.totals.basis, Some(Dec::new(120_000, 2)));
}

/// A journal with no `D` directive reports `None`, so the fallback stays off.
#[test]
fn a_journal_without_a_d_directive_declares_no_default_commodity() {
    assert_eq!(journal("fx-cross-rates.journal").default_commodity, None);
}

/// The base a request will be denominated in, resolvable without computing the
/// report — what the HTTP layer validates `valueIn` against.
#[test]
fn valuation_base_agrees_with_the_report_it_predicts() {
    for (name, expected) in [
        ("fx-cross-rates.journal", "$"),
        ("fx-cross-rates-declared.journal", "EUR"),
    ] {
        let journal = journal(name);
        let base = valuation_base(
            &journal.transactions,
            &journal.prices,
            &journal.accounts,
            &scope(None),
        )
        .expect("base resolution succeeds");
        assert_eq!(base.0, expected, "{name}");
        assert_eq!(report(&journal, None).base, expected, "{name}");
    }
}
