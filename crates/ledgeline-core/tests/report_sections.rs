//! RPT-1 / RPT-2 — section membership, section totals and roll-up ordering.
//!
//! Every expected number in this file was produced by `hledger 1.52` against the
//! very fixture the test loads; the command is quoted next to the assertion.
//!
//! The two headline defects these lock down:
//!
//! * **RPT-1** — a section total summed from `depth == 1` rows reads ZERO for any
//!   chart of accounts whose typed accounts sit below depth 1
//!   (`fixtures/reports/nested-types.journal`).
//! * **RPT-2** — rolling up BEFORE the type filter lets a parent row net in
//!   children of a different effective type, fabricating rows hledger never
//!   shows (`fixtures/reports/mixed-subtree.journal`).

mod common;

use common::fixtures_dir;
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    AccountType, InsightsOpts, Interval, MixedAmount, NetWorthOpts, Section, SectionedReport,
    account_decls, balance_sheet, declared_types, income_statement, insights, net_worth,
};
use ledgeline_core::{Dec, Journal, parse_journal};
use std::collections::BTreeMap;

fn fixture(name: &str) -> Journal {
    let path = fixtures_dir().join("reports").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} readable: {e}"));
    parse_journal(&text, &path.to_string_lossy()).unwrap_or_else(|e| panic!("{name} parses: {e}"))
}

fn types(journal: &Journal) -> BTreeMap<String, AccountType> {
    declared_types(&account_decls(journal))
}

/// The `$` component of a mixed amount.
fn usd(ma: &MixedAmount) -> Dec {
    ma.get(&Commodity("$".into())).unwrap_or_else(Dec::zero)
}

fn dollars(cents: i128) -> Dec {
    Dec::new(cents, 2)
}

fn section<'a>(report: &'a SectionedReport, title: &str) -> &'a Section {
    report
        .sections
        .iter()
        .find(|s| s.title == title)
        .unwrap_or_else(|| panic!("section {title}"))
}

/// `(account, inclusive $)` for every row of a section.
fn rows(report: &SectionedReport, title: &str) -> Vec<(String, Dec)> {
    section(report, title)
        .rows
        .iter()
        .map(|row| (row.account.clone(), usd(&row.inclusive)))
        .collect()
}

// ---------------------------------------------------------------------------
// RPT-1 — typed accounts below depth 1 must not read zero
// ---------------------------------------------------------------------------

#[test]
fn nested_types_balance_sheet_matches_hledger() {
    // hledger -f fixtures/reports/nested-types.journal bs
    //   Assets $1000.00 / Liabilities $200.00 / Net $800.00
    let journal = fixture("nested-types.journal");
    let report = balance_sheet(&journal.transactions, "2026-12-31", 3, &types(&journal)).unwrap();

    assert_eq!(usd(&section(&report, "Assets").total), dollars(100_000));
    assert_eq!(usd(&section(&report, "Liabilities").total), dollars(20_000));
    assert_eq!(usd(&report.grand_total), dollars(80_000));
}

#[test]
fn nested_types_income_statement_matches_hledger() {
    // hledger -f fixtures/reports/nested-types.journal is
    //   Revenues $1000.00 / Expenses $200.00 / Net $800.00
    let journal = fixture("nested-types.journal");
    let report = income_statement(
        &journal.transactions,
        "2026-01-01",
        "2026-12-31",
        3,
        &types(&journal),
    )
    .unwrap();

    assert_eq!(usd(&section(&report, "Revenues").total), dollars(100_000));
    assert_eq!(usd(&section(&report, "Expenses").total), dollars(20_000));
    assert_eq!(usd(&report.grand_total), dollars(80_000));
}

#[test]
fn nested_types_net_worth_matches_hledger() {
    // hledger -f fixtures/reports/nested-types.journal bal --value=end,'$' type:AL
    //   -> $800.00
    let journal = fixture("nested-types.journal");
    let report = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-12-31",
            interval: Interval::Yearly,
            count: 1,
            depth: 3,
            value_in: Some(Commodity("$".into())),
            declared: &types(&journal),
        },
    )
    .unwrap();
    assert_eq!(usd(&report.totals[0]), dollars(80_000));
}

/// hledger's section totals do not move with `--depth`; neither may ours. This
/// is the invariant the `depth == 1` summation broke.
#[test]
fn section_totals_are_depth_independent() {
    for name in ["nested-types.journal", "mixed-subtree.journal"] {
        let journal = fixture(name);
        let declared = types(&journal);
        let at = |depth| {
            let bs = balance_sheet(&journal.transactions, "2026-12-31", depth, &declared).unwrap();
            let is = income_statement(
                &journal.transactions,
                "2026-01-01",
                "2026-12-31",
                depth,
                &declared,
            )
            .unwrap();
            let nw = net_worth(
                &journal.transactions,
                &journal.prices,
                &NetWorthOpts {
                    end: "2026-12-31",
                    interval: Interval::Yearly,
                    count: 1,
                    depth,
                    value_in: Some(Commodity("$".into())),
                    declared: &declared,
                },
            )
            .unwrap();
            (
                usd(&bs.grand_total),
                usd(&is.grand_total),
                usd(&nw.totals[0]),
            )
        };
        let baseline = at(1);
        for depth in [2, 3, 4, 9] {
            assert_eq!(at(depth), baseline, "{name} totals moved at depth {depth}");
        }
    }
}

/// hledger `--depth 0` collapses every account into a single `...` row and still
/// prints the section totals. We keep the totals (they are computed from the
/// members, not from the rows) and emit no rows — "totals only".
#[test]
fn depth_zero_is_totals_only_not_zeros() {
    // hledger -f fixtures/reports/nested-types.journal bs --depth 0
    //   Assets: ... $1000.00 (total $1000.00); Liabilities: ... $200.00; Net $800.00
    let journal = fixture("nested-types.journal");
    let report = balance_sheet(&journal.transactions, "2026-12-31", 0, &types(&journal)).unwrap();

    assert!(section(&report, "Assets").rows.is_empty());
    assert!(section(&report, "Liabilities").rows.is_empty());
    assert_eq!(usd(&section(&report, "Assets").total), dollars(100_000));
    assert_eq!(usd(&section(&report, "Liabilities").total), dollars(20_000));
    assert_eq!(usd(&report.grand_total), dollars(80_000));
}

/// The dashboard used to contradict itself: `revenue.current` came from a
/// depth-1 income statement (zero) while `cash_balance.current` called
/// `resolve_account_type` directly (correct).
#[test]
fn insights_revenue_no_longer_contradicts_cash_balance() {
    let journal = fixture("nested-types.journal");
    let report = insights(
        &journal,
        &InsightsOpts {
            start: "2025-11-01",
            end: "2026-01-31",
            cost_exclude: &[],
            change_min: Dec::zero(),
        },
    )
    .unwrap();

    // Both transactions land in the CURRENT half of the span.
    assert_eq!(usd(&report.cash_balance.current), dollars(100_000));
    assert_eq!(
        usd(&report.revenue.current),
        dollars(100_000),
        "revenue must agree with the cash it produced"
    );
    assert_eq!(usd(&report.expenses.current), dollars(20_000));
    assert_eq!(usd(&report.cost_of_living.current_total), dollars(20_000));
    assert_eq!(usd(&report.net_worth.current), dollars(80_000));
}

// ---------------------------------------------------------------------------
// RPT-2 — filter by type BEFORE rolling up
// ---------------------------------------------------------------------------

#[test]
fn mixed_subtree_rows_and_totals_match_hledger() {
    // hledger -f fixtures/reports/mixed-subtree.journal bs --depth 3
    //   Assets: assets:bank $1000.00 (total $1000.00)
    //   Liabilities: assets:receivable $300.00 (total $300.00)
    //   Net $700.00
    let journal = fixture("mixed-subtree.journal");
    let report = balance_sheet(&journal.transactions, "2026-12-31", 3, &types(&journal)).unwrap();

    // The `assets` parent is real (hledger elides it as a boring parent), but it
    // must carry ONLY this section's members: $1000, never the $700 net.
    assert_eq!(
        rows(&report, "Assets"),
        vec![
            ("assets".to_string(), dollars(100_000)),
            ("assets:bank".to_string(), dollars(100_000)),
        ]
    );
    assert_eq!(
        rows(&report, "Liabilities"),
        vec![
            ("assets".to_string(), dollars(30_000)),
            ("assets:receivable".to_string(), dollars(30_000)),
        ]
    );
    assert_eq!(usd(&section(&report, "Assets").total), dollars(100_000));
    assert_eq!(usd(&section(&report, "Liabilities").total), dollars(30_000));
    assert_eq!(usd(&report.grand_total), dollars(70_000));
}

/// hledger clamps WITHIN each section, so at `--depth 1` the name `assets`
/// appears in both sections carrying that section's own subtotal.
#[test]
fn mixed_subtree_depth_one_clamps_within_each_section() {
    // hledger -f fixtures/reports/mixed-subtree.journal bs --depth 1
    //   Assets: assets $1000.00 | Liabilities: assets $300.00 | Net $700.00
    let journal = fixture("mixed-subtree.journal");
    let report = balance_sheet(&journal.transactions, "2026-12-31", 1, &types(&journal)).unwrap();

    assert_eq!(
        rows(&report, "Assets"),
        vec![("assets".to_string(), dollars(100_000))]
    );
    assert_eq!(
        rows(&report, "Liabilities"),
        vec![("assets".to_string(), dollars(30_000))]
    );
    assert_eq!(usd(&report.grand_total), dollars(70_000));
}

/// A parent's `own` must be attributed to the section the parent belongs to, and
/// nowhere else: `assets` is type A, so the Liabilities section sees `own = 0`.
#[test]
fn mixed_subtree_parent_own_belongs_to_one_section_only() {
    let journal = fixture("mixed-subtree.journal");
    let report = balance_sheet(&journal.transactions, "2026-12-31", 3, &types(&journal)).unwrap();
    let liab_parent = section(&report, "Liabilities")
        .rows
        .iter()
        .find(|row| row.account == "assets")
        .expect("synthesized `assets` parent in Liabilities");
    assert_eq!(usd(&liab_parent.own), Dec::zero());
}

#[test]
fn mixed_subtree_net_worth_matches_hledger() {
    // hledger -f fixtures/reports/mixed-subtree.journal bal type:AL -> $700.00
    let journal = fixture("mixed-subtree.journal");
    let report = net_worth(
        &journal.transactions,
        &journal.prices,
        &NetWorthOpts {
            end: "2026-12-31",
            interval: Interval::Yearly,
            count: 1,
            depth: 3,
            value_in: Some(Commodity("$".into())),
            declared: &types(&journal),
        },
    )
    .unwrap();

    assert_eq!(usd(&report.totals[0]), dollars(70_000));
    // `assets` is an asset root, so net worth still shows it — carrying the
    // asset-and-liability members below it, which here is the $700 net.
    let by = |name: &str| {
        report
            .rows
            .iter()
            .find(|r| r.account == name)
            .map(|r| usd(&r.values[0]))
    };
    assert_eq!(by("assets:bank"), Some(dollars(100_000)));
    assert_eq!(by("assets:receivable"), Some(dollars(-30_000)));
}
