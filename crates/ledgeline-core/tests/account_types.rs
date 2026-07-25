//! Reports must classify accounts by their DECLARED type, never by what the
//! account happens to be called.
//!
//! `fixtures/golden/` cannot cover this: `sample.journal` uses standard English
//! roots, so name inference and type resolution agree on every account there and
//! a name-based filter passes the goldens anyway. This fixture removes that
//! coincidence — every account is declared with a `type:` and named so no
//! English heuristic can classify it (`cogs:`, `gastos:`, `ingresos:`,
//! `activo:`, `pasivo:`). A report that reads names instead of types reports
//! zeroes here.

mod common;

use common::fixtures_dir;
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    AccountType, Interval, MixedAmount, NetWorthOpts, account_decls, balance_sheet, declared_types,
    income_statement, net_worth,
};
use ledgeline_core::{Dec, Journal, parse_journal};
use std::collections::BTreeMap;

fn fixture() -> Journal {
    let path = fixtures_dir()
        .join("account-types")
        .join("non-english.journal");
    let text = std::fs::read_to_string(&path).expect("non-english.journal readable");
    parse_journal(&text, &path.to_string_lossy()).expect("non-english.journal parses")
}

fn types(journal: &Journal) -> BTreeMap<String, AccountType> {
    declared_types(&account_decls(journal))
}

/// The `$` total of a mixed amount.
fn usd(ma: &MixedAmount) -> Dec {
    ma.get(&Commodity("$".into())).unwrap_or_else(Dec::zero)
}

fn section<'a>(
    report: &'a ledgeline_core::reports::SectionedReport,
    title: &str,
) -> &'a ledgeline_core::reports::Section {
    report
        .sections
        .iter()
        .find(|s| s.title == title)
        .unwrap_or_else(|| panic!("section {title}"))
}

#[test]
fn income_statement_classifies_by_declared_type() {
    let journal = fixture();
    let report = income_statement(
        &journal.transactions,
        "2026-01-01",
        "2026-12-31",
        2,
        &types(&journal),
    )
    .unwrap();

    // ingresos:consultoria is `type: R` — revenue, shown sign-flipped positive.
    assert_eq!(
        usd(&section(&report, "Revenues").total),
        Dec::new(400_000, 2)
    );
    // cogs:infraestructura ($600) + gastos:oficina ($150) are both `type: X`.
    assert_eq!(
        usd(&section(&report, "Expenses").total),
        Dec::new(75_000, 2)
    );
    assert_eq!(usd(&report.grand_total), Dec::new(325_000, 2));
}

#[test]
fn balance_sheet_classifies_by_declared_type_and_folds_cash_into_assets() {
    let journal = fixture();
    let report = balance_sheet(&journal.transactions, "2026-12-31", 2, &types(&journal)).unwrap();

    // activo:banco $14,000 + cuenta:efectivo $350 — the latter is `type: C`, an
    // Asset subtype that must not be dropped from the Assets section.
    assert_eq!(
        usd(&section(&report, "Assets").total),
        Dec::new(1_435_000, 2)
    );
    let asset_rows: Vec<&str> = section(&report, "Assets")
        .rows
        .iter()
        .map(|row| row.account.as_str())
        .collect();
    assert!(asset_rows.contains(&"cuenta:efectivo"), "{asset_rows:?}");

    // pasivo:tarjeta is `type: L`, shown sign-flipped positive.
    assert_eq!(
        usd(&section(&report, "Liabilities").total),
        Dec::new(60_000, 2)
    );
    assert_eq!(usd(&report.grand_total), Dec::new(1_375_000, 2));
}

#[test]
fn net_worth_counts_typed_assets_and_liabilities() {
    let journal = fixture();
    let report = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-12-31",
            interval: Interval::Yearly,
            count: 1,
            depth: 1,
            value_in: None,
            declared: &types(&journal),
        },
    )
    .unwrap();

    // $14,350 of assets less $600 owed — equity and income are excluded.
    assert_eq!(usd(&report.totals[0]), Dec::new(1_375_000, 2));
}
