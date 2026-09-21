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
    AccountType, BudgetOpts, GapRow, InsightsOpts, Interval, MixedAmount, NetWorthOpts,
    accepted_type_tags, account_decls, balance_sheet, budget_gaps, declared_types,
    income_statement, insights, is_account_type, net_worth, parse_account_type_tag,
    resolve_account_type,
};
use ledgeline_core::{Dec, Journal, parse_journal};
use std::collections::{BTreeMap, BTreeSet};

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

/// The account names of a gaps section, in the order the engine ordered them.
fn gap_accounts(rows: &[GapRow]) -> Vec<&str> {
    rows.iter().map(|row| row.account.as_str()).collect()
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

/// Insights' "top transactions" box must drop opening-balance entries by their
/// DECLARED type. `patrimonio:inicio` is `type: E` and named so that no English
/// heuristic can reach it, so a name-based `equity:` filter would leave the
/// fixture's `Apertura` entry — $10,500, the largest row in the journal by far —
/// sitting at the top of a list that is supposed to show spending.
///
/// The span is two years so the midpoint split puts all four of the fixture's
/// transactions in the CURRENT half, which is the only half this box ranks.
#[test]
fn top_transactions_drop_opening_balances_declared_by_type() {
    let journal = fixture();
    let report = insights(
        &journal,
        &InsightsOpts {
            start: "2025-01-01",
            end: "2026-12-31",
            cost_exclude: &[],
            change_min: Dec::zero(),
        },
    )
    .unwrap();
    assert_eq!(report.period.curr_start, "2026-01-01");

    // Only the three economic events survive: the consulting fee ($4,000), the
    // servers ($600) and the office supplies ($150). `Apertura` is absent.
    let ranked: Vec<(&str, Dec)> = report
        .top_txns
        .iter()
        .map(|row| (row.description.as_str(), row.amount))
        .collect();
    assert_eq!(
        ranked,
        [
            ("Cliente Uno | consulting fee", Dec::new(400_000, 2)),
            ("Proveedor | servers", Dec::new(60_000, 2)),
            ("Papeleria | office supplies", Dec::new(15_000, 2)),
        ]
    );
}

/// The Budget tab's "what my budget does not cover" section, over a chart of
/// accounts no English heuristic can read.
///
/// This is the one the golden fixtures structurally cannot catch: the gaps list
/// is filtered by resolved account type, and `sample.journal` uses standard
/// English roots, so a name-based filter passes every golden and then reports an
/// empty section for this journal. The symptom of getting it wrong is zero, not
/// wrong — which is exactly the failure that goes unnoticed.
#[test]
fn budget_gaps_classify_by_declared_type() {
    let journal = fixture();
    let gaps = budget_gaps(
        &journal.transactions,
        &journal.periodic_transactions,
        &types(&journal),
        &BudgetOpts {
            end: "2026-12-31",
            interval: Interval::Monthly,
            count: 12,
            depth: 2,
            budget_desc: None,
        },
    )
    .unwrap();

    // The fixture declares no `~` rules, so everything with activity is a gap —
    // and the only things listed are the revenue and the two expenses. The bank,
    // the cash box, the card and the opening equity are all absent: they fund the
    // spending, they are not spending.
    assert_eq!(gap_accounts(&gaps.revenue), ["ingresos:consultoria"]);
    assert_eq!(
        gap_accounts(&gaps.expense),
        ["cogs:infraestructura", "gastos:oficina"]
    );

    assert_eq!(usd(&gaps.revenue[0].total), Dec::new(-400_000, 2));
    // Ordered by magnitude: the $600 of servers before the $150 of stationery.
    assert_eq!(usd(&gaps.expense[0].total), Dec::new(60_000, 2));
    assert_eq!(usd(&gaps.expense[1].total), Dec::new(15_000, 2));
    assert_eq!(
        (gaps.from.as_str(), gaps.to.as_str()),
        ("2026-01-01", "2026-12-31")
    );
}

// ===========================================================================
// The shared truth table (fixtures/account-types/classification-cases.json)
// ===========================================================================
//
// The engine and the browser are two implementations of ONE specification, and
// both are checked against the same file — see its `_comment`, and
// `web/src/lib/domain/accountTypes.test.ts` for the other half.
//
// The 2026-09 Projections sign bug is what this exists to stop. The browser's
// `type:` vocabulary was a subset of the engine's, so a declaration it could not
// parse was dropped and the account fell through to ENGLISH NAME INFERENCE.
// `income:contractors ; type: expenses` then resolved as revenue in the browser
// and as expense in the engine — and the browser's one sign flip negated a cost
// on its way to an engine that booked it as one. A $100,000 payroll paid money
// IN, forever.

/// The shared table, parsed.
fn classification_cases() -> serde_json::Value {
    let path = fixtures_dir()
        .join("account-types")
        .join("classification-cases.json");
    let text = std::fs::read_to_string(&path).expect("classification-cases.json readable");
    serde_json::from_str(&text).expect("classification-cases.json is JSON")
}

fn type_name(ty: AccountType) -> &'static str {
    match ty {
        AccountType::Asset => "asset",
        AccountType::Liability => "liability",
        AccountType::Equity => "equity",
        AccountType::Revenue => "revenue",
        AccountType::Expense => "expense",
        AccountType::Cash => "cash",
        AccountType::Conversion => "conversion",
        AccountType::Gain => "gain",
    }
}

fn type_or_null(ty: Option<AccountType>) -> serde_json::Value {
    ty.map_or(serde_json::Value::Null, |ty| {
        serde_json::Value::String(type_name(ty).to_string())
    })
}

fn why_of(row: &serde_json::Value) -> &str {
    row.get("_why")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

/// Every `type:` spelling in the shared table parses to the type it states.
///
/// The plurals are the load-bearing rows. hledger REJECTS `type: expenses` with
/// a parse error while we degrade silently to name inference, so accepting them
/// is what stops a `cogs:` account declaring nothing at all — and pinning them
/// HERE is what stops the two implementations disagreeing about it again.
#[test]
fn every_declared_type_spelling_parses_as_the_shared_table_says() {
    let cases = classification_cases();
    let tags = cases["tags"].as_array().expect("tags is an array");
    assert!(tags.len() >= 37, "the shared table lost its tag rows");
    for tag in tags {
        let value = tag["value"].as_str().expect("a tag value");
        assert_eq!(
            type_or_null(parse_account_type_tag(value)),
            tag["type"],
            "`; type: {value}` must parse as the shared table says. {}",
            why_of(tag)
        );
    }
}

/// The shared table names EVERY spelling this side accepts, and no others.
///
/// The mirror of the browser's assertion over `ACCEPTED_TYPE_TAGS`, and the one
/// that turns the fixture from example-pinning into exhaustive pinning. The
/// test above proves each spelling the fixture names parses correctly; it
/// cannot notice a spelling added HERE and never written down — which is the
/// direction the original drift ran, and the direction that silently sends an
/// account to name inference in the other implementation.
#[test]
fn the_shared_table_names_every_accepted_type_spelling() {
    let cases = classification_cases();
    let named: BTreeSet<String> = cases["tags"]
        .as_array()
        .expect("tags is an array")
        .iter()
        .filter(|tag| !tag["type"].is_null())
        .map(|tag| {
            tag["value"]
                .as_str()
                .expect("a tag value")
                .trim()
                .to_lowercase()
        })
        .collect();
    let accepted: BTreeSet<String> = accepted_type_tags().map(str::to_string).collect();
    assert_eq!(
        accepted, named,
        "`parse_account_type_tag` and the shared fixture must accept exactly the \
         same spellings — anything only on the left is unpinned and can drift \
         away from the browser's table, anything only on the right is a spelling \
         this side has stopped accepting"
    );
}

/// Every (declarations, account) row resolves — and answers the three
/// membership questions — exactly as the shared table says.
#[test]
fn every_account_resolves_as_the_shared_table_says() {
    let cases = classification_cases();
    let rows = cases["cases"].as_array().expect("cases is an array");
    assert!(
        rows.len() >= 15,
        "the shared table lost its resolution rows"
    );
    for row in rows {
        let account = row["account"].as_str().expect("an account");
        let why = why_of(row);
        let declared: BTreeMap<String, AccountType> = row["declared"]
            .as_array()
            .expect("declared is an array")
            .iter()
            .filter_map(|pair| {
                let name = pair[0].as_str().expect("a declared name").to_string();
                parse_account_type_tag(pair[1].as_str().expect("a declared type"))
                    .map(|ty| (name, ty))
            })
            .collect();

        assert_eq!(
            type_or_null(resolve_account_type(account, &declared)),
            row["type"],
            "resolving `{account}` against {}. {why}",
            row["declared"]
        );
        for (category, key) in [
            (AccountType::Revenue, "isRevenue"),
            (AccountType::Expense, "isExpense"),
            (AccountType::Asset, "isAsset"),
            (AccountType::Equity, "isEquity"),
        ] {
            assert_eq!(
                serde_json::Value::Bool(is_account_type(account, &declared, category)),
                row[key],
                "`{account}` {key} against {}. {why}",
                row["declared"]
            );
        }
    }
}
