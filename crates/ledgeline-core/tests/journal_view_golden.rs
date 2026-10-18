//! The windowed journal view reproduces the TypeScript it replaced (plan 26 B5).
//!
//! Two goldens, both CAPTURED from the SPA over each fixture journal's wire
//! before the code was deleted:
//!
//! - `fixtures/journal-view/view.json` — for 15 filters per journal (dates,
//!   account subtrees, free-text queries including case/whitespace and amount
//!   text): `journal/rowModel.ts` `filterTxns` + `sortTxnsDesc` (the exact
//!   tindex SEQUENCE, so search parity and display order are both pinned), and
//!   `insights/series.ts` `commoditiesInUse`, `maxAccountDepth`, `visibleNet`,
//!   `bigNumbers`, `categoriesInUse`, `styleFor`, plus the whole-journal
//!   `signConventions`.
//! - `fixtures/journal-view/charts.json` — `rankedAccounts`, `pieData` and
//!   `lineData` (monthly/weekly/daily) across depths, category scopes and
//!   group caps.
//!
//! A failure is the port drifting; neither fixture is to be regenerated from
//! Rust.

mod common;

use common::fixtures_dir;
use ledgeline_core::model::Journal;
use ledgeline_core::parse_journal;
use ledgeline_core::reports::Interval;
use ledgeline_core::view::Classes;
use ledgeline_core::view::filter::{JournalFilter, display_order, filter_positions};
use ledgeline_core::view::rows::haystack;
use ledgeline_core::view::series::{
    ChartSpec, JournalView, SignTable, line_data, parse_category, pie_data, ranked_accounts,
    sign_table, summary,
};
use ledgeline_core::view::wire::DeclaredStyles;
use serde_json::{Value, json};
use std::collections::HashMap;

/// `value` with every number as a double: the TypeScript serialized `-127550`
/// where serde writes `-127550.0`, which `Value` equality tells apart.
fn numeric(value: Value) -> Value {
    match value {
        Value::Number(number) => json!(number.as_f64().expect("finite")),
        Value::Array(items) => Value::Array(items.into_iter().map(numeric).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, numeric(v))).collect())
        }
        other => other,
    }
}

fn golden(name: &str) -> Value {
    let path = fixtures_dir().join("journal-view").join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden readable"))
        .expect("golden is JSON")
}

/// One parsed fixture journal with every per-snapshot derivation built.
struct Fixture {
    journal: Journal,
    classes: Classes,
    signs: SignTable,
    declared: DeclaredStyles,
    order: Vec<u32>,
    haystacks: Vec<Box<str>>,
}

impl Fixture {
    fn load(rel: &str) -> Self {
        let path = fixtures_dir().join(rel);
        let text = std::fs::read_to_string(&path).expect("fixture readable");
        let journal = parse_journal(&text, &path.to_string_lossy()).expect("fixture parses");
        let classes = Classes::of(&journal);
        let signs = sign_table(&journal, &classes);
        let declared = DeclaredStyles::of(&journal);
        let order = display_order(&journal);
        let haystacks = journal
            .transactions
            .iter()
            .map(|txn| haystack(txn, &declared).into_boxed_str())
            .collect();
        Self {
            journal,
            classes,
            signs,
            declared,
            order,
            haystacks,
        }
    }

    fn view(&self) -> JournalView<'_> {
        JournalView {
            journal: &self.journal,
            classes: &self.classes,
            signs: &self.signs,
            declared: &self.declared,
        }
    }

    fn filter(spec: &Value) -> JournalFilter {
        let date = |key: &str| spec[key].as_str().map(str::to_string);
        let accounts = spec["accounts"]
            .as_array()
            .expect("accounts")
            .iter()
            .map(|a| a.as_str().expect("account").to_string());
        JournalFilter::new(
            date("from"),
            date("to"),
            accounts,
            spec["query"].as_str().expect("query"),
        )
    }

    fn positions(&self, filter: &JournalFilter) -> Vec<u32> {
        filter_positions(&self.journal, &self.order, Some(&self.haystacks), filter)
    }

    fn tindexes(&self, positions: &[u32]) -> Vec<u32> {
        positions
            .iter()
            .map(|p| self.journal.transactions[*p as usize].index.0)
            .collect()
    }
}

#[test]
fn filters_order_and_summaries_match_the_captured_typescript() {
    let golden = golden("view.json");
    let journals = golden["cases"].as_array().expect("cases");
    assert!(journals.len() >= 60, "the view golden lost cases");
    let mut views = 0;
    for entry in journals {
        let rel = entry["journal"].as_str().expect("journal");
        let fixture = Fixture::load(rel);
        for (commodity, signs) in entry["signs"].as_object().expect("signs") {
            let ours = fixture.signs.get(commodity);
            assert_eq!(
                json!({"revenue": ours.revenue, "expense": ours.expense}),
                *signs,
                "{rel}: sign conventions for {commodity}"
            );
        }
        for case in entry["views"].as_array().expect("views") {
            let label = format!("{rel} {}", case["filter"]);
            let filter = Fixture::filter(&case["filter"]);
            let positions = fixture.positions(&filter);
            assert_eq!(
                json!(fixture.tindexes(&positions)),
                case["tindexes"],
                "{label}: filtered rows in display order"
            );
            let ours = serde_json::to_value(summary(fixture.view(), &positions, &filter))
                .expect("serializes");
            assert_eq!(ours["total"], json!(positions.len()), "{label}: total");
            assert_eq!(
                ours["commodities"], case["commodities"],
                "{label}: commodities"
            );
            assert_eq!(ours["maxDepth"], case["maxDepth"], "{label}: maxDepth");
            assert_eq!(
                ours["visibleNet"], case["visibleNet"],
                "{label}: visibleNet"
            );
            assert_eq!(
                ours["perCommodity"], case["perCommodity"],
                "{label}: perCommodity"
            );
            views += 1;
        }
    }
    assert!(views > 900, "too few filter cases compared: {views}");
}

#[test]
fn charts_match_the_captured_typescript() {
    let golden = golden("charts.json");
    let cases = golden["cases"].as_array().expect("cases");
    assert!(cases.len() > 300, "the charts golden lost cases");
    let mut fixtures: HashMap<String, Fixture> = HashMap::new();
    for case in cases {
        let rel = case["journal"].as_str().expect("journal");
        let fixture = fixtures
            .entry(rel.to_string())
            .or_insert_with(|| Fixture::load(rel));
        let filter = Fixture::filter(&case["filter"]);
        let positions = fixture.positions(&filter);
        let spec = ChartSpec {
            depth: usize::try_from(case["depth"].as_u64().expect("depth")).expect("fits"),
            commodity: case["commodity"].as_str().expect("commodity").to_string(),
            category: case["category"]
                .as_str()
                .map(|code| parse_category(code).expect("known category")),
            max: usize::try_from(case["max"].as_u64().expect("max")).expect("fits"),
        };
        let label = format!(
            "{rel} {} {} depth={} category={} max={}",
            case["filter"], case["mode"], spec.depth, case["category"], spec.max
        );
        let ranked = ranked_accounts(fixture.view(), &positions, &filter, &spec);
        match case["mode"].as_str() {
            Some("pie") => {
                assert_eq!(json!(ranked), case["ranked"], "{label}: ranked");
                let pie = pie_data(fixture.view(), &positions, &filter, &spec, &ranked);
                assert_eq!(
                    numeric(serde_json::to_value(pie).expect("pie")),
                    numeric(case["pie"].clone()),
                    "{label}: pie"
                );
            }
            Some("line") => {
                let interval = match case["interval"].as_str() {
                    Some("daily") => Interval::Daily,
                    Some("weekly") => Interval::Weekly,
                    _ => Interval::Monthly,
                };
                let line = line_data(
                    fixture.view(),
                    &positions,
                    &filter,
                    &spec,
                    interval,
                    &ranked,
                );
                assert_eq!(
                    numeric(serde_json::to_value(line).expect("line")),
                    numeric(case["line"].clone()),
                    "{label} {}: line",
                    case["interval"]
                );
            }
            other => panic!("unknown mode {other:?}"),
        }
    }
}

/// Every transaction, in the domain shape the row components render, and its
/// search text — against `fixtures/journal-view/rows.json`, captured from the
/// SPA's `normalize.ts` (`normalizeTransactions` over the hledger wire) before
/// the journal stopped being downloaded whole.
#[test]
fn rows_and_haystacks_match_the_captured_typescript() {
    use ledgeline_core::view::rows::RowContext;
    let golden = golden("rows.json");
    let journals = golden["cases"].as_array().expect("cases");
    assert!(journals.len() >= 60, "the rows golden lost cases");
    let mut rows = 0;
    for entry in journals {
        let rel = entry["journal"].as_str().expect("journal");
        let fixture = Fixture::load(rel);
        let context = RowContext::new(&fixture.journal, &fixture.declared);
        let expected = entry["rows"].as_array().expect("rows");
        assert_eq!(expected.len(), fixture.journal.transactions.len(), "{rel}");
        for ((txn, want), text) in fixture
            .journal
            .transactions
            .iter()
            .zip(expected)
            .zip(&fixture.haystacks)
        {
            let label = format!("{rel} #{}", txn.index.0);
            let mut ours = serde_json::to_value(context.row(txn, Vec::new())).expect("serializes");
            assert_eq!(ours["flags"], json!([]), "{label}");
            ours.as_object_mut().expect("object").remove("flags");
            let mut want = want.clone();
            let haystack = want
                .as_object_mut()
                .expect("object")
                .remove("haystack")
                .expect("a haystack");
            assert_eq!(ours, want, "{label}: row");
            assert_eq!(json!(text.as_ref()), haystack, "{label}: haystack");
            rows += 1;
        }
    }
    assert!(rows > 400, "too few rows compared: {rows}");
}
