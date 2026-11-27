//! The statement of cash flows (`plans/27-cash-flow-statement.md`) against
//! `fixtures/reports/cf-statement.journal` and hledger 1.52 ground truth.
//!
//! Every expected figure was read off the `hledger` binary, never off our own
//! output. The closing entry is excluded from hledger's side with
//! `not:desc:Close`, which is what the statement's structural detection must
//! reproduce:
//!
//! ```text
//! hledger -f fixtures/reports/cf-statement.journal bal -b 2026-01-01 -e 2026-04-01 not:desc:Close --flat
//!   $50.00 assets:petty-cash            $1200.00 assets:prepaid
//!   $3000.00 assets:equipment           $-100.00 assets:equipment:accum-depreciation
//!   $1450.00 assets:bank:checking       $8000.00 assets:bank:savings
//!   $1500.00 expenses:rent              $500.00 expenses:software
//!   $100.00 expenses:depreciation       $-300.00 liabilities:payable
//!   $-2200.00 liabilities:deferred-revenue
//!   $-8000.00 liabilities:note          $-5200.00 revenue:sales
//! hledger -f … bal -H -M -b 2026-01-01 -e 2026-04-01 type:C --flat
//!   total $9700.00 / $11700.00 / $19500.00 at Jan/Feb/Mar end
//! ```
//!
//! Each line shows −Δ: an asset that grew used cash (negative), a liability
//! that grew supplied it (positive).

mod common;

use ledgeline_core::Dec;
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::outline::OutlineKind;
use ledgeline_core::reports::{
    AccountTypes, CashFlowStatement, CfLine, CfLineSource, CfOpts, CfRole, CfSectionKind, CfTags,
    Interval, MixedAmount, cash_flow_statement, cf_role_resolver, declared_types, is_closing_entry,
};
use ledgeline_core::{Journal, parse_journal};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture() -> Journal {
    let path = common::fixtures_dir().join("reports/cf-statement.journal");
    let text = std::fs::read_to_string(&path).expect("fixture reads");
    parse_journal(&text, &path.to_string_lossy()).expect("fixture parses")
}

fn statement(journal: &Journal, interval: Option<Interval>, depth: usize) -> CashFlowStatement {
    cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: "2026-01-01",
            to: "2026-03-31",
            interval,
            depth,
        },
        &CfTags::from_declarations(&journal.accounts),
    )
    .expect("statement")
}

/// Dollars, in cents, with zero commodities dropped so an empty figure is 0.
fn cents(ma: &MixedAmount) -> i128 {
    let usd = Commodity("$".to_string());
    let value = ma.get(&usd).unwrap_or_else(Dec::zero);
    assert!(
        ma.iter()
            .all(|(commodity, dec)| commodity == &usd || dec.is_zero()),
        "only dollars expected: {ma:?}"
    );
    let exact = value.rounded(2).expect("cents");
    assert_eq!(exact, value, "a whole number of cents: {value:?}");
    exact.mantissa * 10i128.pow(2 - exact.places.min(2))
}

fn section(report: &CashFlowStatement, kind: CfSectionKind) -> &ledgeline_core::reports::CfSection {
    report
        .sections
        .iter()
        .find(|section| section.kind == kind)
        .unwrap_or_else(|| panic!("section {kind:?}"))
}

/// `(line name, Total)` for each line of a section.
fn lines(report: &CashFlowStatement, kind: CfSectionKind) -> Vec<(String, i128)> {
    section(report, kind)
        .lines
        .iter()
        .map(|line: &CfLine| (line.name.clone(), cents(&line.total.total)))
        .collect()
}

fn named(pairs: &[(&str, i128)]) -> Vec<(String, i128)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), *value))
        .collect()
}

// ---------------------------------------------------------------------------
// The fixture, Total column
// ---------------------------------------------------------------------------

#[test]
fn the_quarter_reads_as_a_statement_that_reconciles_to_cash() {
    let report = statement(&fixture(), None, 3);

    // Revenue $5200 − rent $1500 − software $500 − depreciation $100. The
    // closing entry is skipped; with it, net income would read 0.
    assert_eq!(cents(&report.net_income.total), 310_000);

    assert_eq!(
        lines(&report, CfSectionKind::Operating),
        named(&[
            // Accumulated depreciation sits under a non-current asset, but its
            // `valuation: depreciation` makes it an operating add-back.
            ("assets:equipment:accum-depreciation", 10_000),
            ("assets:prepaid", -120_000),
            ("liabilities:deferred-revenue", 220_000),
            ("liabilities:payable", 30_000),
            // Receivable and card both net to zero over the quarter.
        ])
    );
    assert_eq!(
        cents(&section(&report, CfSectionKind::Operating).total.total),
        450_000
    );

    assert_eq!(
        lines(&report, CfSectionKind::Investing),
        named(&[("assets:equipment", -300_000)])
    );
    assert_eq!(
        lines(&report, CfSectionKind::Financing),
        named(&[("liabilities:note", 800_000)])
    );

    assert_eq!(cents(&report.net_change.total), 950_000);
    assert_eq!(cents(&report.beginning_cash.total), 1_000_000);
    assert_eq!(cents(&report.ending_cash.total), 1_950_000);
    assert!(report.reconciles);
}

#[test]
fn cash_accounts_roll_up_by_cfgroup_and_otherwise_by_account() {
    let report = statement(&fixture(), None, 3);
    let cash: Vec<(&str, CfLineSource, i128, i128, usize)> = report
        .cash
        .iter()
        .map(|line| {
            (
                line.name.as_str(),
                line.source,
                cents(&line.beginning.total),
                cents(&line.ending.total),
                line.rows.len(),
            )
        })
        .collect();
    assert_eq!(
        cash,
        [
            (
                "Operating cash",
                CfLineSource::Group,
                1_000_000,
                1_945_000,
                2
            ),
            ("assets:petty-cash", CfLineSource::Account, 0, 5_000, 1),
        ]
    );
}

#[test]
fn equity_is_untouched_because_the_closing_entry_is_skipped() {
    let report = statement(&fixture(), None, 3);
    let financing = section(&report, CfSectionKind::Financing);
    assert!(
        financing
            .lines
            .iter()
            .all(|line| !line.name.starts_with("equity")),
        "retained earnings must not repeat net income: {:?}",
        financing.lines
    );
}

// ---------------------------------------------------------------------------
// The fixture, monthly columns
// ---------------------------------------------------------------------------

#[test]
fn monthly_columns_each_reconcile_and_the_total_is_their_sum() {
    let report = statement(&fixture(), Some(Interval::Monthly), 3);
    assert_eq!(report.buckets.len(), 3);
    let per = |columns: &ledgeline_core::reports::Columns| -> Vec<i128> {
        columns.values.iter().map(cents).collect()
    };

    // Jan: revenue 5200 − rent 1500 − software 200; Feb: − software 300 −
    // depreciation 100; Mar: nothing but the (skipped) close.
    assert_eq!(per(&report.net_income), [350_000, -40_000, 0]);
    // `hledger bal -M … type:C`: checking −300 / +2000 / −250, savings and
    // petty cash +8050 in March.
    assert_eq!(per(&report.net_change), [-30_000, 200_000, 780_000]);
    assert_eq!(cents(&report.net_change.total), 950_000);

    // Balances: the Total takes the first column's beginning and the last
    // column's ending, never a sum of month-end balances.
    assert_eq!(per(&report.beginning_cash), [1_000_000, 970_000, 1_170_000]);
    assert_eq!(per(&report.ending_cash), [970_000, 1_170_000, 1_950_000]);
    assert_eq!(cents(&report.beginning_cash.total), 1_000_000);
    assert_eq!(cents(&report.ending_cash.total), 1_950_000);
    assert!(report.reconciles);

    // A line that nets to zero over the range still shows its months.
    let receivable = section(&report, CfSectionKind::Operating)
        .lines
        .iter()
        .find(|line| line.name == "assets:receivable")
        .expect("receivable has monthly movement");
    assert_eq!(per(&receivable.total), [-500_000, 500_000, 0]);
    assert_eq!(cents(&receivable.total.total), 0);
}

// ---------------------------------------------------------------------------
// Lines: depth and cfgroup
// ---------------------------------------------------------------------------

#[test]
fn account_lines_roll_up_to_the_report_depth() {
    let report = statement(&fixture(), None, 1);
    assert_eq!(
        lines(&report, CfSectionKind::Operating),
        named(&[("assets", -110_000), ("liabilities", 250_000)])
    );
    let assets = &section(&report, CfSectionKind::Operating).lines[0];
    assert_eq!(
        assets.rows.len(),
        2,
        "prepaid and accumulated depreciation, still separable"
    );
}

#[test]
fn a_cfgroup_collapses_accounts_from_different_parts_of_the_tree_into_one_line() {
    let text = std::fs::read_to_string(common::fixtures_dir().join("reports/cf-statement.journal"))
        .expect("reads")
        .replace(
            "account liabilities:payable                  ; type: L",
            "account liabilities:payable                  ; type: L, cfgroup: Payables",
        )
        .replace(
            "account liabilities:card                     ; type: L",
            "account liabilities:card                     ; type: L, cfgroup: Payables",
        );
    assert_eq!(
        text.matches("cfgroup: Payables").count(),
        2,
        "both declarations were tagged"
    );
    let journal = parse_journal(&text, "t.journal").expect("parses");
    let report = statement(&journal, Some(Interval::Monthly), 3);
    let payables = section(&report, CfSectionKind::Operating)
        .lines
        .iter()
        .find(|line| line.name == "Payables")
        .expect("the group line");
    assert_eq!(payables.source, CfLineSource::Group);
    assert_eq!(payables.rows.len(), 2);
    // Card +200 in Jan, payable +300 in Feb, card −200 in Mar.
    assert_eq!(
        payables.total.values.iter().map(cents).collect::<Vec<_>>(),
        [20_000, 30_000, -20_000]
    );
}

// ---------------------------------------------------------------------------
// Lines as outlines (plan 50 WP3)
// ---------------------------------------------------------------------------

/// Two banks with checking and savings each (one bank named by `name:`), a
/// receivable split by client, a prepaid policy and a named card. The cash
/// is opened before the range, so the cash block has beginning balances.
const OUTLINE_JOURNAL: &str = "\
account assets:bank                 ; type: C
account assets:bank:fcb             ; name: First Community Bank
account assets:receivable           ; type: A
account assets:prepaid              ; type: A
account liabilities:card            ; type: L, name: Amex Platinum
account equity:capital              ; type: E
account revenue:sales               ; type: R
account expenses:software           ; type: X

2026-01-01 Open
    assets:bank:ally:checking   $1000.00
    assets:bank:ally:savings    $2000.00
    assets:bank:fcb:checking    $3000.00
    assets:bank:fcb:savings     $4000.00
    equity:capital

2026-02-01 Invoice Acme
    assets:receivable:clients:acme    $500.00
    revenue:sales

2026-02-02 Invoice Globex
    assets:receivable:clients:globex  $300.00
    revenue:sales

2026-02-03 Prepay insurance
    assets:prepaid:insurance    $120.00
    assets:bank:ally:checking

2026-02-04 Software on the card
    expenses:software           $50.00
    liabilities:card
";

fn outline_statement(text: &str, depth: usize) -> CashFlowStatement {
    let journal = parse_journal(text, "t.journal").expect("parses");
    cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: "2026-02-01",
            to: "2026-03-31",
            interval: None,
            depth,
        },
        &CfTags::from_declarations(&journal.accounts),
    )
    .expect("statement")
}

fn marker(kind: OutlineKind) -> &'static str {
    match kind {
        OutlineKind::Heading => "#",
        OutlineKind::Account => "-",
        OutlineKind::Other => "~",
        OutlineKind::Total => "=",
    }
}

/// Each operating line as `label` then its outline rows, indented by level.
fn operating_outline(report: &CashFlowStatement) -> Vec<String> {
    section(report, CfSectionKind::Operating)
        .lines
        .iter()
        .flat_map(|line| {
            std::iter::once(line.label.clone()).chain(line.rows.iter().map(|row| {
                format!(
                    "{}{} {}",
                    "  ".repeat(row.level + 1),
                    marker(row.kind),
                    row.label
                )
            }))
        })
        .collect()
}

fn texts(rows: &[&str]) -> Vec<String> {
    rows.iter().map(|row| (*row).to_string()).collect()
}

#[test]
fn each_depth_reads_differently() {
    let at = |depth| operating_outline(&outline_statement(OUTLINE_JOURNAL, depth));
    assert_eq!(
        at(1),
        texts(&[
            "Assets",
            "  - Insurance",
            "  # Clients",
            "    - Acme",
            "    - Globex",
            "Liabilities",
            "  - Amex Platinum",
        ])
    );
    assert_eq!(
        at(2),
        texts(&[
            "Prepaid",
            "  - Insurance",
            "Receivable",
            "  - Acme",
            "  - Globex",
            "Amex Platinum",
            "  - Amex Platinum",
        ])
    );
    assert_eq!(
        at(3),
        texts(&[
            "Insurance",
            "  - Insurance",
            "Clients",
            "  - Acme",
            "  - Globex",
            "Amex Platinum",
            "  - Amex Platinum",
        ])
    );
    assert_eq!(
        at(4),
        texts(&[
            "Insurance",
            "  - Insurance",
            "Acme",
            "  - Acme",
            "Globex",
            "  - Globex",
            "Amex Platinum",
            "  - Amex Platinum",
        ])
    );
}

#[test]
fn max_depth_is_the_deepest_account_behind_any_line_whatever_the_depth() {
    for depth in [1, 2, 4] {
        assert_eq!(outline_statement(OUTLINE_JOURNAL, depth).max_depth, 4);
    }
}

#[test]
fn a_single_account_line_is_labelled_by_its_account_not_its_path() {
    let report = outline_statement(OUTLINE_JOURNAL, 2);
    let lines = &section(&report, CfSectionKind::Operating).lines;
    let prepaid = &lines[0];
    assert_eq!(prepaid.name, "assets:prepaid");
    assert_eq!(prepaid.label, "Prepaid");
    assert_eq!(prepaid.account.as_deref(), Some("assets:prepaid"));
    assert_eq!(prepaid.rows.len(), 1);
    assert_eq!(prepaid.rows[0].account, "assets:prepaid:insurance");
    // Its own `name:` tag wins over the segment.
    let card = &lines[2];
    assert_eq!(card.label, "Amex Platinum");
    assert_eq!(card.account.as_deref(), Some("liabilities:card"));
    // Row figures are the members they show; the line total is unchanged.
    assert_eq!(cents(&lines[1].rows[0].amounts.total), -50_000);
    assert_eq!(cents(&lines[1].rows[1].amounts.total), -30_000);
    assert_eq!(cents(&lines[1].total.total), -80_000);
}

#[test]
fn a_cfgroup_line_spanning_subtrees_outlines_its_accounts() {
    let text = OUTLINE_JOURNAL
        .replace(
            "account assets:receivable           ; type: A",
            "account assets:receivable           ; type: A, cfgroup: Working capital",
        )
        .replace(
            "; type: L, name: Amex Platinum",
            "; type: L, name: Amex Platinum, cfgroup: Working capital",
        );
    let report = outline_statement(&text, 2);
    let line = section(&report, CfSectionKind::Operating)
        .lines
        .iter()
        .find(|line| line.name == "Working capital")
        .expect("the group line");
    assert_eq!(line.label, "Working capital");
    assert_eq!(line.account, None);
    let rows: Vec<String> = line
        .rows
        .iter()
        .map(|row| {
            format!(
                "{}{} {}",
                "  ".repeat(row.level),
                marker(row.kind),
                row.label
            )
        })
        .collect();
    assert_eq!(
        rows,
        texts(&["# Clients", "  - Acme", "  - Globex", "- Amex Platinum"])
    );
    // A heading's figures are its subtree's (not printed, but exact).
    assert_eq!(cents(&line.rows[0].amounts.total), -80_000);
    assert_eq!(cents(&line.total.total), -75_000);
}

#[test]
fn the_cash_block_outlines_two_banks_each_with_checking_and_savings() {
    let report = outline_statement(OUTLINE_JOURNAL, 2);
    assert_eq!(report.cash.len(), 1);
    let bank = &report.cash[0];
    assert_eq!(bank.label, "Bank");
    assert_eq!(bank.account.as_deref(), Some("assets:bank"));
    let rows: Vec<(String, String, i128, i128)> = bank
        .rows
        .iter()
        .map(|row| {
            (
                format!(
                    "{}{} {}",
                    "  ".repeat(row.level),
                    marker(row.kind),
                    row.label
                ),
                row.account.clone(),
                cents(&row.beginning.total),
                cents(&row.ending.total),
            )
        })
        .collect();
    let expect = |label: &str, account: &str, beginning: i128, ending: i128| {
        (label.to_string(), account.to_string(), beginning, ending)
    };
    assert_eq!(
        rows,
        [
            expect("# Ally", "assets:bank:ally", 300_000, 288_000),
            expect("  - Checking", "assets:bank:ally:checking", 100_000, 88_000),
            expect("  - Savings", "assets:bank:ally:savings", 200_000, 200_000),
            expect(
                "# First Community Bank",
                "assets:bank:fcb",
                700_000,
                700_000
            ),
            expect("  - Checking", "assets:bank:fcb:checking", 300_000, 300_000),
            expect("  - Savings", "assets:bank:fcb:savings", 400_000, 400_000),
        ]
    );
    assert_eq!(cents(&bank.beginning.total), 1_000_000);
    assert_eq!(cents(&bank.ending.total), 988_000);
    assert!(report.reconciles);
}

// ---------------------------------------------------------------------------
// Resolution: the defaults table, one rule each
// ---------------------------------------------------------------------------

fn role(declarations: &str, account: &str) -> CfRole {
    let journal = parse_journal(declarations, "t.journal").expect("parses");
    let tags = CfTags::from_declarations(&journal.accounts);
    let types = AccountTypes::from_declared(tags.types.clone());
    cf_role_resolver(&tags, &types, &std::collections::BTreeSet::new())(account)
}

#[test]
fn an_asset_that_holds_shares_is_investing_like_the_balance_sheets_investments() {
    let journal = parse_journal(
        "P 2026-01-01 AAPL $150.00\n\n\
         2026-01-01 Fund\n    assets:checking  $5000.00\n    equity:capital\n\n\
         2026-02-01 Buy\n    assets:broker  10 AAPL @ $150.00\n    assets:checking  $-1500.00\n",
        "t.journal",
    )
    .expect("parses");
    let report = cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: "2026-01-01",
            to: "2026-12-31",
            interval: None,
            depth: 2,
        },
        &CfTags::from_journal(&journal),
    )
    .expect("statement");
    assert!(report.reconciles);
    assert_eq!(
        lines(&report, CfSectionKind::Investing),
        named(&[("assets:broker", -150_000)])
    );
    assert!(section(&report, CfSectionKind::Operating).lines.is_empty());
}

#[test]
fn revenue_and_expense_are_net_income_even_when_tagged() {
    assert_eq!(
        role(
            "account revenue:x  ; type: R, cfsection: financing\n",
            "revenue:x"
        ),
        CfRole::NetIncome
    );
    assert_eq!(
        role("account expenses:x  ; type: X\n", "expenses:x"),
        CfRole::NetIncome
    );
}

#[test]
fn cfsection_overrides_every_default_and_inherits() {
    let journal =
        "account liabilities:long-term  ; type: L, bsterm: noncurrent, cfsection: operating\n";
    assert_eq!(
        role(journal, "liabilities:long-term:note"),
        CfRole::Line(CfSectionKind::Operating)
    );
    assert_eq!(
        role(
            "account assets:tbills  ; type: A, cfsection: cash\n",
            "assets:tbills"
        ),
        CfRole::Cash
    );
}

#[test]
fn cash_type_is_cash() {
    assert_eq!(
        role("account assets:bank  ; type: C\n", "assets:bank:checking"),
        CfRole::Cash
    );
}

#[test]
fn a_valuation_adjustment_is_operating_even_under_a_noncurrent_asset() {
    let journal = "account assets:fixed  ; type: A, bsterm: noncurrent\n\
                   account assets:fixed:accum  ; valuation: depreciation\n";
    assert_eq!(
        role(journal, "assets:fixed:accum"),
        CfRole::Line(CfSectionKind::Operating)
    );
    assert_eq!(
        role(journal, "assets:fixed:laptop"),
        CfRole::Line(CfSectionKind::Investing)
    );
}

#[test]
fn noncurrent_liabilities_are_financing_and_current_ones_operating() {
    let journal = "account liabilities:note  ; type: L, bsterm: noncurrent\n\
                   account liabilities:card  ; type: L, bsterm: current\n";
    assert_eq!(
        role(journal, "liabilities:note"),
        CfRole::Line(CfSectionKind::Financing)
    );
    assert_eq!(
        role(journal, "liabilities:card"),
        CfRole::Line(CfSectionKind::Operating)
    );
    // No bsterm at all is current.
    assert_eq!(
        role("", "liabilities:payable"),
        CfRole::Line(CfSectionKind::Operating)
    );
    assert_eq!(
        role("", "assets:receivable"),
        CfRole::Line(CfSectionKind::Operating)
    );
}

#[test]
fn equity_is_financing() {
    assert_eq!(
        role("", "equity:capital"),
        CfRole::Line(CfSectionKind::Financing)
    );
}

#[test]
fn an_account_with_no_type_at_all_is_operating() {
    assert_eq!(role("", "suspense"), CfRole::Line(CfSectionKind::Operating));
}

// ---------------------------------------------------------------------------
// Closing entries
// ---------------------------------------------------------------------------

#[test]
fn a_closing_entry_is_pnl_and_equity_only_and_an_opening_entry_is_not_one() {
    let journal = fixture();
    let types = AccountTypes::from_declared(declared_types(
        &ledgeline_core::reports::account_decls(&journal),
    ));
    let closing: Vec<&str> = journal
        .transactions
        .iter()
        .filter(|txn| is_closing_entry(txn, &types))
        .map(|txn| txn.description.as_str())
        .collect();
    assert_eq!(closing, ["Close the quarter"]);
}

// ---------------------------------------------------------------------------
// The identity: at cost, virtual postings, posting dates
// ---------------------------------------------------------------------------

#[test]
fn a_purchase_at_cost_reconciles_in_the_cost_commodity() {
    let journal = parse_journal(
        "2026-01-01 Fund\n    assets:checking  $5000.00\n    equity:capital\n\n\
         2026-02-01 Buy\n    assets:broker  10 AAPL @ $150.00\n    assets:checking  $-1500.00\n",
        "t.journal",
    )
    .expect("parses");
    let report = cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: "2026-01-01",
            to: "2026-12-31",
            interval: None,
            depth: 2,
        },
        &CfTags::from_declarations(&journal.accounts),
    )
    .expect("statement");
    assert!(report.reconciles);
    assert_eq!(
        lines(&report, CfSectionKind::Operating),
        named(&[("assets:broker", -150_000)])
    );
}

#[test]
fn changing_currency_is_the_effect_of_exchange_rates_and_cash_stays_as_held() {
    let journal = parse_journal(
        "account assets:bank  ; type: C\n\n\
         2026-01-01 Fund\n    assets:bank:usd  $1000.00\n    equity:capital\n\n\
         2026-02-01 Change money\n    assets:bank:eur  500 EUR @ $1.142\n    assets:bank:usd  $-571.00\n",
        "t.journal",
    )
    .expect("parses");
    let report = cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: "2026-02-01",
            to: "2026-02-28",
            interval: None,
            depth: 3,
        },
        &CfTags::from_declarations(&journal.accounts),
    )
    .expect("statement");
    assert!(report.reconciles);
    let eur = Commodity("EUR".to_string());
    let usd = Commodity("$".to_string());
    assert_eq!(report.exchange.total.get(&eur), Some(Dec::new(500, 0)));
    assert_eq!(
        report.exchange.total.get(&usd).map(|d| d.normalized()),
        Some(Dec::new(-571, 0))
    );
    let euros = report
        .cash
        .iter()
        .find(|line| line.name == "assets:bank:eur")
        .expect("the euro account");
    assert_eq!(
        euros.ending.total.get(&eur),
        Some(Dec::new(500, 0)),
        "held in euros"
    );
    assert_eq!(euros.ending.total.get(&usd), None);
}

#[test]
fn unbalanced_virtual_postings_are_left_out() {
    let journal = parse_journal(
        "2026-01-01 Fund\n    assets:checking  $100.00\n    equity:capital\n    (budget:food)  $-40.00\n",
        "t.journal",
    )
    .expect("parses");
    let report = cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: "2026-01-01",
            to: "2026-12-31",
            interval: None,
            depth: 2,
        },
        &CfTags::from_declarations(&journal.accounts),
    )
    .expect("statement");
    assert!(report.reconciles);
    assert_eq!(cents(&report.net_change.total), 10_000);
}

#[test]
fn a_posting_date_that_splits_a_transaction_across_columns_is_reported() {
    let journal = parse_journal(
        "2026-01-30 Invoice paid later\n    assets:receivable  $-100.00\n    assets:checking  $100.00  ; date:2026-02-02\n",
        "t.journal",
    )
    .expect("parses");
    let report = cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: "2026-01-01",
            to: "2026-02-28",
            interval: Some(Interval::Monthly),
            depth: 2,
        },
        &CfTags::from_declarations(&journal.accounts),
    )
    .expect("statement");
    assert!(!report.reconciles, "each month is off by the split leg");
    assert_eq!(
        cents(&report.net_change.total),
        cents(&report.ending_cash.total)
    );
}

// ---------------------------------------------------------------------------
// Property: every column reconciles, whatever the journal
// ---------------------------------------------------------------------------

const ACCOUNTS: [&str; 10] = [
    "assets:checking",
    "assets:savings",
    "assets:receivable",
    "assets:equipment",
    "liabilities:card",
    "liabilities:note",
    "equity:capital",
    "revenue:sales",
    "expenses:rent",
    "expenses:food",
];

const DECLARATIONS: &str = "account assets:checking  ; type: C\n\
                            account assets:savings  ; type: C, cfgroup: Banks\n\
                            account assets:equipment  ; type: A, bsterm: noncurrent\n\
                            account liabilities:note  ; type: L, bsterm: noncurrent\n";

proptest! {
    #[test]
    fn every_column_reconciles_to_the_change_in_cash(
        txns in prop::collection::vec(
            (0u32..120, 0usize..ACCOUNTS.len(), 0usize..ACCOUNTS.len(), 1i64..100_000),
            1..40,
        ),
        monthly in any::<bool>(),
    ) {
        let mut text = String::from(DECLARATIONS);
        for (day, debit, credit, cents) in &txns {
            let date = chrono_like(*day);
            // Every seventh one lands in euros at a cost, so currency exchange
            // into cash and into non-cash accounts is exercised too.
            let debit_amount = if cents % 7 == 0 {
                format!("{} EUR @ $1.10", cents / 100 + 1)
            } else {
                format!("${}.{:02}", cents / 100, cents % 100)
            };
            text.push_str(&format!(
                "\n{date} t\n    {}  {debit_amount}\n    {}\n",
                ACCOUNTS[*debit], ACCOUNTS[*credit]
            ));
        }
        let journal = parse_journal(&text, "p.journal").expect("generated journal parses");
        let report = cash_flow_statement(
            &journal.transactions,
            &CfOpts {
                from: "2026-02-01",
                to: "2026-03-31",
                interval: monthly.then_some(Interval::Monthly),
                depth: 2,
            },
            &CfTags::from_declarations(&journal.accounts),
        ).expect("statement");
        prop_assert!(report.reconciles, "{text}");
        let moved = report.ending_cash.total.ma_add(&report.beginning_cash.total.ma_neg().expect("neg")).expect("add");
        let mut gap = moved.ma_add(&report.net_change.total.ma_neg().expect("neg")).expect("add");
        gap.drop_zeros();
        prop_assert!(gap.is_zero(), "Total column: {gap:?}\n{text}");
    }
}

/// `day` days after 2026-01-01, as an ISO date (January through April).
fn chrono_like(day: u32) -> String {
    let months = [(1, 31), (2, 28), (3, 31), (4, 30)];
    let mut left = day;
    for (month, length) in months {
        if left < length {
            return format!("2026-{month:02}-{:02}", left + 1);
        }
        left -= length;
    }
    "2026-04-30".to_string()
}

#[test]
fn chrono_like_spans_the_first_four_months() {
    assert_eq!(chrono_like(0), "2026-01-01");
    assert_eq!(chrono_like(31), "2026-02-01");
    assert_eq!(chrono_like(119), "2026-04-30");
}
