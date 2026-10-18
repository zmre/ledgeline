//! `/api/journal`'s whole-journal metadata reproduces the TypeScript it
//! replaced (plan 26 B1).
//!
//! `fixtures/journal-meta/cases.json` was CAPTURED from the SPA's own
//! implementations — `reportStyles`, `dominantCommodity`, `resolveAccountType`
//! plus the `bsterm:` resolution and name guess — run over each fixture
//! journal's `/transactions`, `/accounts` and `/accountnames` wire, before that
//! TypeScript was deleted. Each case here re-parses the same journal and checks
//! the engine's `journal_to_info` gives the same answers, so the move cannot
//! have changed what any screen shows. A failure is the port drifting; the
//! fixture is not to be regenerated from Rust.

mod common;

use common::fixtures_dir;
use ledgeline_core::{parse_journal, wire};
use serde_json::{Value, json};

fn golden() -> Value {
    let path = fixtures_dir().join("journal-meta").join("cases.json");
    let text = std::fs::read_to_string(&path).expect("journal-meta/cases.json readable");
    serde_json::from_str(&text).expect("journal-meta/cases.json is JSON")
}

/// An `astyle` as the SPA decodes it (`normalize.ts` `toStyle`): the shape the
/// golden recorded. A missing decimal mark reads as `.`, and group sizes that
/// are not positive are dropped (a group with none left is no grouping).
fn decoded_style(astyle: &Value) -> Value {
    let groups = astyle["asdigitgroups"].as_array().and_then(|pair| {
        let sizes: Vec<Value> = pair[1]
            .as_array()?
            .iter()
            .filter(|size| size.as_u64().is_some_and(|n| n > 0))
            .cloned()
            .collect();
        (!sizes.is_empty()).then(|| json!([pair[0], sizes]))
    });
    json!({
        "side": if astyle["ascommodityside"] == "R" { "R" } else { "L" },
        "spaced": astyle["ascommodityspaced"] == true,
        "precision": astyle["asprecision"],
        "decimalPoint": astyle["asdecimalmark"].as_str().unwrap_or("."),
        "digitGroups": groups.unwrap_or(Value::Null),
    })
}

#[test]
fn journal_info_matches_the_captured_typescript_metadata() {
    let golden = golden();
    let cases = golden["cases"].as_array().expect("cases is an array");
    assert!(cases.len() >= 60, "the journal-meta golden lost its cases");
    for case in cases {
        let rel = case["journal"].as_str().expect("a journal path");
        let path = fixtures_dir().join(rel);
        let text = std::fs::read_to_string(&path).expect("fixture journal readable");
        let journal =
            parse_journal(&text, &path.to_string_lossy()).expect("fixture journal parses");
        let info = serde_json::to_value(wire::journal_to_info(&journal)).expect("serializes");

        assert_eq!(info["txnCount"], case["txnCount"], "{rel}: txnCount");
        assert_eq!(
            info["dominantCommodity"], case["dominantCommodity"],
            "{rel}: dominantCommodity"
        );

        let styles: serde_json::Map<String, Value> = info["reportStyles"]
            .as_object()
            .expect("reportStyles is an object")
            .iter()
            .map(|(commodity, astyle)| (commodity.clone(), decoded_style(astyle)))
            .collect();
        assert_eq!(
            Value::Object(styles),
            case["reportStyles"],
            "{rel}: reportStyles"
        );

        assert_eq!(info["accounts"], case["accounts"], "{rel}: accounts");
    }
}

#[test]
fn date_span_brackets_every_transaction() {
    let path = fixtures_dir().join("sample.journal");
    let text = std::fs::read_to_string(&path).expect("sample.journal readable");
    let journal = parse_journal(&text, &path.to_string_lossy()).expect("parses");
    let info = serde_json::to_value(wire::journal_to_info(&journal)).expect("serializes");
    let dates: Vec<&str> = journal
        .transactions
        .iter()
        .map(|t| t.date.as_str())
        .collect();
    assert_eq!(info["firstDate"], json!(dates.iter().min()));
    assert_eq!(info["lastDate"], json!(dates.iter().max()));

    let empty = parse_journal("", "empty.journal").expect("parses");
    let info = serde_json::to_value(wire::journal_to_info(&empty)).expect("serializes");
    assert_eq!(info["firstDate"], Value::Null);
    assert_eq!(info["lastDate"], Value::Null);
    assert_eq!(info["txnCount"], 0);
    assert_eq!(info["dominantCommodity"], "$");
}
