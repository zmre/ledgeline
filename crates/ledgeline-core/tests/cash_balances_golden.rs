//! The engine's cash-balances view reproduces the TypeScript it replaced
//! (plan 26 B3).
//!
//! `fixtures/journal-view/cash-balances.json` was CAPTURED from the SPA's
//! `reports/cashBalances.ts` (`accountBalances`, `guessedLongTerm`,
//! `balanceCommodities`, `summarize`, plus `styleFor` over the whole journal)
//! run over each fixture journal's wire, at several as-of dates, before that
//! module was deleted. A failure here is the port drifting; the fixture is not
//! to be regenerated from Rust.

mod common;

use common::fixtures_dir;
use ledgeline_core::parse_journal;
use ledgeline_core::view::Classes;
use ledgeline_core::view::cash::{cash_balances, cash_balances_view};
use serde_json::{Map, Value, json};

fn golden() -> Value {
    let path = fixtures_dir().join("journal-view/cash-balances.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden readable"))
        .expect("golden is JSON")
}

fn dec(value: ledgeline_core::Dec) -> Value {
    json!({"m": value.mantissa.to_string(), "p": value.places})
}

#[test]
fn cash_balances_match_the_captured_typescript() {
    let golden = golden();
    let journals = golden["cases"].as_array().expect("cases");
    assert!(journals.len() >= 60, "the cash-balances golden lost cases");
    let mut compared = 0;
    for entry in journals {
        let rel = entry["journal"].as_str().expect("journal path");
        let path = fixtures_dir().join(rel);
        let text = std::fs::read_to_string(&path).expect("fixture readable");
        let journal = parse_journal(&text, &path.to_string_lossy()).expect("fixture parses");
        let classes = Classes::of(&journal);
        for case in entry["cases"].as_array().expect("cases") {
            let as_of = case["asOf"].as_str().expect("asOf");
            let set = cash_balances(&journal, &classes, as_of);
            let balances: Vec<Value> = set
                .balances
                .iter()
                .map(|balance| {
                    let amounts: Map<String, Value> = balance
                        .amounts
                        .iter()
                        .map(|(commodity, qty)| (commodity.clone(), dec(*qty)))
                        .collect();
                    json!({
                        "account": balance.account,
                        "kind": balance.kind,
                        "amounts": amounts,
                        "asOf": balance.as_of,
                        "confirmed": balance.confirmed,
                    })
                })
                .collect();
            assert_eq!(
                Value::from(balances),
                case["balances"],
                "{rel} @ {as_of}: balances"
            );
            assert_eq!(
                json!(set.guessed_long_term),
                case["guessedLongTerm"],
                "{rel} @ {as_of}: guessedLongTerm"
            );

            let view = serde_json::to_value(cash_balances_view(&journal, &classes, as_of))
                .expect("serializes");
            assert_eq!(
                view["commodities"], case["commodities"],
                "{rel} @ {as_of}: commodities"
            );
            assert_eq!(
                view["summaries"], case["summaries"],
                "{rel} @ {as_of}: summaries"
            );
            compared += 1;
        }
    }
    assert!(compared > 250, "too few as-of cases compared: {compared}");
}

/// The edge-case journal pins each membership branch by name, so a regression
/// reads as a sentence rather than a JSON diff.
#[test]
fn membership_follows_type_then_term() {
    let path = fixtures_dir().join("journal-view/edge-cases.journal");
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    let journal = parse_journal(&text, &path.to_string_lossy()).expect("parses");
    let set = cash_balances(&journal, &Classes::of(&journal), "9999-12-31");
    let names: Vec<&str> = set.balances.iter().map(|b| b.account.as_str()).collect();
    // Declared Cash and current liabilities are in, including a HELOC whose
    // `bsterm: current` tag beats the name heuristic.
    for account in [
        "assets:bank:checking",
        "assets:bank:savings",
        "liabilities:card",
        "liabilities:heloc",
    ] {
        assert!(
            names.contains(&account),
            "{account} should be in the view: {names:?}"
        );
    }
    // A declared non-current loan is silently out (the owner wrote the tag)...
    assert!(!names.contains(&"liabilities:auto-loan"));
    // ...while name guesses are out but NAMED.
    assert_eq!(
        set.guessed_long_term,
        vec![
            "assets:bank:long-term-cd".to_string(),
            "liabilities:mortgage".to_string()
        ]
    );
    // Non-cash assets never appear.
    assert!(!names.contains(&"assets:brokerage"));
    // The balance assertion on the latest date marks savings as confirmed.
    let savings = set
        .balances
        .iter()
        .find(|b| b.account == "assets:bank:savings")
        .expect("savings present");
    assert!(savings.confirmed);
    assert_eq!(savings.as_of, "2025-01-20");
}
