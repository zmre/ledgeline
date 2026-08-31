//! The income statement's two flow graphs (the Sankey diagrams above the Revenue
//! and Expenses boxes) against real journals and hledger 1.52 ground truth.
//!
//! Every expected number below was read off the `hledger` binary in the dev
//! shell, never off our own output, and the command that produced it is quoted
//! beside the assertion. `-e` is EXCLUSIVE in hledger while our `to` is
//! INCLUSIVE, so `to=2026-07-08` is checked with `-e 2026-07-09`.
//!
//! Five claims are being defended:
//!
//! 1. **Each graph decomposes its box exactly.** `total == section_total`, and
//!    that figure is the one the statement prints, so the diagram is the whole
//!    story rather than a suggestive part of it.
//! 2. **Attribution follows the transaction, not a guess.** A four-posting
//!    paycheck draws all three of its counterparties at their posted amounts, and
//!    the withheld tax is funded by GROSS PAY rather than by the cash account.
//!    That distinction is the one a debits-against-credits pairing gets wrong.
//! 3. **Funding is whatever account paid.** Card spending shows the card, a
//!    write-down shows the contra-asset, and a foreign-currency leg shows the
//!    foreign account with its amount valued into the base.
//! 4. **The graphs read the same boxes and the same lines as the table.** Group
//!    names come from the statement's own resolution, and `other` is in neither
//!    graph because it has no single direction.
//! 5. **Nothing is drawn that cannot be.** No node is on both sides of one
//!    graph, no link is non-positive, and `meta.unpriced` names only commodities
//!    the drawn transactions actually hold.

mod common;

use ledgeline_core::Journal;
use ledgeline_core::reports::{
    FlowGraph, FlowOpts, FlowSide, IS_GROUP_TAG, IsOpts, Valuation, account_decls,
    account_sections, declared_groups, declared_types, income_statement_flows,
    income_statement_grouped,
};
use ledgeline_core::{Dec, parse_journal};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn journal_fixture(relative: &str) -> Journal {
    let path = common::fixtures_dir().join(relative);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"));
    parse_journal(&text, &path.to_string_lossy())
        .unwrap_or_else(|e| panic!("parse {relative}: {e}"))
}

fn flows(journal: &Journal, from: &str, to: &str) -> ledgeline_core::reports::FlowReport {
    income_statement_flows(
        &journal.transactions,
        &journal.prices,
        &FlowOpts {
            from,
            to,
            value_in: None,
        },
        &declared_types(&account_decls(journal)),
        &account_sections(journal),
        &declared_groups(journal, IS_GROUP_TAG),
    )
    .expect("flow report")
}

/// `$n.nn` as the wire and the engine hold it.
fn usd(cents: i128) -> Dec {
    Dec::new(cents, 2)
}

/// One link's value, by the LABELS at its ends. Panics rather than returning an
/// option: an assertion about a link that is not there should name it.
fn link(graph: &FlowGraph, source: &str, target: &str) -> Dec {
    let key = |label: &str| {
        graph
            .nodes
            .iter()
            .find(|node| node.label == label)
            .unwrap_or_else(|| panic!("no node labelled {label:?} in {:?}", labels(graph)))
            .key
            .clone()
    };
    let (source_key, target_key) = (key(source), key(target));
    graph
        .links
        .iter()
        .find(|edge| edge.source == source_key && edge.target == target_key)
        .unwrap_or_else(|| panic!("no link {source:?} -> {target:?}"))
        .value
}

fn labels(graph: &FlowGraph) -> Vec<&str> {
    graph.nodes.iter().map(|node| node.label.as_str()).collect()
}

fn side(graph: &FlowGraph, want: FlowSide) -> Vec<&str> {
    graph
        .nodes
        .iter()
        .filter(|node| node.side == want)
        .map(|node| node.label.as_str())
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Each graph decomposes its box exactly
// ---------------------------------------------------------------------------

/// `hledger is -V -b 2026-01-01 -e 2026-07-09` over `fixtures/sample.journal`:
/// Revenues `$34,010.00`, Expenses `$28,626.48`.
#[test]
fn every_link_together_is_the_box_total_and_nothing_is_left_over() {
    let journal = journal_fixture("sample.journal");
    let report = flows(&journal, "2026-01-01", "2026-07-08");

    assert_eq!(report.inflows.section_total, usd(3_401_000));
    assert_eq!(report.outflows.section_total, usd(2_862_648));
    // The claim that makes the diagram worth reading: what it draws IS the box.
    // A gap here would mean a statement posting with no counterparty, or a line
    // refunded past zero, and the UI has to say so rather than quietly shrink.
    assert_eq!(report.inflows.total, report.inflows.section_total);
    assert_eq!(report.outflows.total, report.outflows.section_total);
}

/// The same claim on the tagged book, where it is sharper: the graphs cover five
/// of the seven boxes, so their totals are NOT hledger's type-split Revenues and
/// Expenses.
///
/// `hledger -f fixtures/reports/is-sections.journal is -b 2026-01-01
/// -e 2027-01-01` reports Revenues `$155,000.00` and Expenses `$146,700.00`.
/// Those include the mixed `other` box, which is in neither graph:
/// `$155,000.00 − $5,000.00` of grants is the revenue side, and
/// `$146,700.00 − $8,000.00` of settlement is the cost side.
#[test]
fn the_mixed_other_box_is_in_neither_graph() {
    let journal = journal_fixture("reports/is-sections.journal");
    let report = flows(&journal, "2026-01-01", "2026-12-31");

    assert_eq!(report.inflows.section_total, usd(15_000_000));
    assert_eq!(report.outflows.section_total, usd(13_870_000));
    assert_eq!(report.inflows.total, report.inflows.section_total);
    assert_eq!(report.outflows.total, report.outflows.section_total);

    for graph in [&report.inflows, &report.outflows] {
        assert!(
            !labels(graph).contains(&"Income") && !labels(graph).contains(&"Expenses"),
            "`other`'s two lines leaked into a graph: {:?}",
            labels(graph)
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Attribution follows the transaction
// ---------------------------------------------------------------------------

/// The six 2026 paychecks, each
///
/// ```journal
/// income:salary             $-5,660.00
/// expenses:taxes:federal     $1,150.00
/// expenses:taxes:state         $310.00
/// assets:bank:checking       $4,200.00
/// ```
///
/// so `6 × $4,200.00`, `6 × $1,150.00` and `6 × $310.00`.
#[test]
fn a_four_posting_paycheck_draws_all_three_of_its_counterparties() {
    let journal = journal_fixture("sample.journal");
    let inflows = flows(&journal, "2026-01-01", "2026-07-08").inflows;

    assert_eq!(link(&inflows, "Salary", "Bank: Checking"), usd(2_520_000));
    assert_eq!(link(&inflows, "Salary", "Taxes: Federal"), usd(690_000));
    assert_eq!(link(&inflows, "Salary", "Taxes: State"), usd(186_000));
    // `hledger bal income:salary -V -b 2026-01-01 -e 2026-07-09` → $33,960.00.
    assert_eq!(
        inflows
            .nodes
            .iter()
            .find(|node| node.label == "Salary")
            .expect("the Salary line")
            .total,
        usd(3_396_000)
    );
}

/// The withheld tax on those same paychecks: `6 × ($1,150.00 + $310.00)`.
///
/// This is the assertion that pins the attribution RULE rather than its result.
/// Pairing the transaction's debit total against its credit total would fund
/// `$8,760.00` of tax out of a cash account that received money in that
/// transaction and paid nothing, and would have to invent a negative link to
/// balance the books it broke. The tax was withheld from gross pay, and the graph
/// says gross pay.
#[test]
fn withheld_tax_is_funded_by_gross_pay_and_not_by_the_cash_account() {
    let journal = journal_fixture("sample.journal");
    let outflows = flows(&journal, "2026-01-01", "2026-07-08").outflows;

    assert_eq!(link(&outflows, "Salary", "Taxes"), usd(876_000));
    assert!(
        !outflows
            .links
            .iter()
            .any(|edge| edge.target.ends_with("Taxes") && edge.source.contains("checking")),
        "the checking account did not fund any withheld tax"
    );
}

// ---------------------------------------------------------------------------
// 3. Funding is whatever account paid
// ---------------------------------------------------------------------------

/// `hledger print -b 2026-01-01 -e 2026-07-09 expenses:food` shows ten
/// card-funded legs summing to `$1,590.50`, one `$42.13` restaurant off checking,
/// and `18,75 EUR` off the Wise account, which `P 2026-06-30 EUR $1.16` values at
/// `$21.75`. Together `$1,654.38`, which is what `hledger bal -V` prints for the
/// group.
#[test]
fn one_expense_group_splits_across_the_card_the_bank_and_the_foreign_account() {
    let journal = journal_fixture("sample.journal");
    let outflows = flows(&journal, "2026-01-01", "2026-07-08").outflows;

    assert_eq!(link(&outflows, "Credit cards: Visa", "Food"), usd(159_050));
    assert_eq!(link(&outflows, "Bank: Checking", "Food"), usd(4_213));
    // Valued into the base, on the same link as the account that spent it: the
    // reader sees dollars, and still sees WHICH account.
    assert_eq!(link(&outflows, "Bank: Wise: Eur", "Food"), usd(2_175));
    assert_eq!(
        outflows
            .nodes
            .iter()
            .find(|node| node.label == "Food")
            .expect("the Food line")
            .total,
        usd(165_438)
    );
}

/// `2026-06-30 * vehicle depreciation FY2026` posts `$3,500.00` to
/// `expenses:depreciation` against `assets:vehicles:car:depreciation`. Nothing
/// was spent, and the graph names the contra-asset rather than pretending a bank
/// account paid for it.
#[test]
fn a_non_cash_write_down_names_the_contra_asset_that_funded_it() {
    let journal = journal_fixture("sample.journal");
    let outflows = flows(&journal, "2026-01-01", "2026-07-08").outflows;

    assert_eq!(
        link(&outflows, "Vehicles: Car: Depreciation", "Depreciation"),
        usd(350_000)
    );
}

// ---------------------------------------------------------------------------
// 4. The graphs read the same lines as the table
// ---------------------------------------------------------------------------

/// Every label on a graph's statement side is a line of the box the statement
/// printed for the same window. Checked against the statement itself rather than
/// against a hand-written list, because the two resolutions being the same code
/// is the property, and a list would pass while they drifted.
#[test]
fn statement_side_labels_are_the_statements_own_lines() {
    for fixture in ["sample.journal", "reports/is-sections.journal"] {
        let journal = journal_fixture(fixture);
        let statement = income_statement_grouped(
            &journal.transactions,
            &journal.prices,
            &IsOpts {
                from: "2026-01-01",
                to: "2026-07-08",
                value: Valuation::Market,
                value_in: None,
                compare: true,
            },
            &declared_types(&account_decls(&journal)),
            &account_sections(&journal),
            &declared_groups(&journal, IS_GROUP_TAG),
        )
        .expect("grouped income statement");
        let printed: Vec<&str> = statement
            .sections
            .iter()
            .flat_map(|section| section.groups.iter().map(|group| group.name.as_str()))
            .collect();

        let report = flows(&journal, "2026-01-01", "2026-07-08");
        for (graph, want) in [
            (&report.inflows, FlowSide::Source),
            (&report.outflows, FlowSide::Target),
        ] {
            for label in side(graph, want) {
                assert!(
                    printed.contains(&label),
                    "{fixture}: the graph names a line {label:?} the statement does not print: \
                     {printed:?}"
                );
            }
        }
    }
}

/// An `isgroup:` that merges two unrelated accounts merges their FLOWS too:
/// `expenses:marketing:ads` ($12,000.00) and `expenses:salaries:sales`
/// ($20,000.00) both carry `isgroup: Growth`, so the graph draws one $32,000.00
/// link where the table prints one line.
#[test]
fn accounts_merged_onto_one_line_by_tag_share_one_link() {
    let journal = journal_fixture("reports/is-sections.journal");
    let outflows = flows(&journal, "2026-01-01", "2026-12-31").outflows;

    assert_eq!(link(&outflows, "Bank", "Growth"), usd(3_200_000));
}

// ---------------------------------------------------------------------------
// 5. Nothing is drawn that cannot be
// ---------------------------------------------------------------------------

#[test]
fn no_node_is_on_both_sides_of_one_graph_and_no_link_is_non_positive() {
    for (fixture, from, to) in [
        ("sample.journal", "2026-01-01", "2026-07-08"),
        ("sample.journal", "2024-07-01", "2026-07-08"),
        ("reports/is-sections.journal", "2026-01-01", "2026-12-31"),
    ] {
        let journal = journal_fixture(fixture);
        let report = flows(&journal, from, to);
        for graph in [&report.inflows, &report.outflows] {
            // A d3-sankey layout has no reading for a cycle, and a node in both
            // columns is the only way to get one out of this shape.
            for node in &graph.nodes {
                assert!(
                    graph
                        .nodes
                        .iter()
                        .filter(|other| other.key == node.key)
                        .count()
                        == 1,
                    "{fixture}: duplicate node key {:?}",
                    node.key
                );
            }
            for edge in &graph.links {
                assert!(
                    edge.value.mantissa > 0,
                    "{fixture}: {} -> {} has nothing to draw",
                    edge.source,
                    edge.target
                );
            }
            // Biggest first, which is the order a Sankey is read in.
            assert!(
                graph
                    .nodes
                    .windows(2)
                    .all(|pair| pair[0].total >= pair[1].total),
                "{fixture}: nodes are not ordered by size"
            );
            assert!(
                graph
                    .links
                    .windows(2)
                    .all(|pair| pair[0].value >= pair[1].value),
                "{fixture}: links are not ordered by size"
            );
        }
    }
}

/// `fixtures/sample.journal` holds GLD, NVDA and TSLA without a usable price,
/// and the grouped statement over this window reports none of them: no income or
/// expense account touches one. Neither may this report, or the P&L tab would
/// raise "some holdings are not valued" over a statement that shows no holdings.
#[test]
fn unpriced_names_only_commodities_the_drawn_transactions_hold() {
    let journal = journal_fixture("sample.journal");
    let report = flows(&journal, "2026-01-01", "2026-07-08");

    assert_eq!(report.meta.unpriced, Vec::new());
    assert_eq!(report.base.as_ref().map(|base| base.0.as_str()), Some("$"));
}

/// One valuation target and one funding account, so the shape is unmistakable:
/// every 2026 cost in the tagged book was paid from `assets:bank`, and every
/// dollar of revenue landed there.
#[test]
fn a_single_account_book_draws_a_single_hub() {
    let journal = journal_fixture("reports/is-sections.journal");
    let report = flows(&journal, "2026-01-01", "2026-12-31");

    assert_eq!(side(&report.inflows, FlowSide::Target), vec!["Bank"]);
    assert_eq!(side(&report.outflows, FlowSide::Source), vec!["Bank"]);
    assert_eq!(
        side(&report.inflows, FlowSide::Source),
        vec!["Subscriptions", "Services"]
    );
}
