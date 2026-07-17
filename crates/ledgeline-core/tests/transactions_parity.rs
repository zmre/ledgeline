//! Byte-semantic parity test against the real hledger-web 1.52 snapshot.
//!
//! Parses `fixtures/sample.journal` and asserts the serialized transactions are
//! semantically equal to `fixtures/api/v1.52/transactions.json` across all 185
//! transactions. Two fields are ignored everywhere they occur:
//!
//! - `floatingPoint` — display-only and brittle under float equality; the exact
//!   value lives in `decimalMantissa`/`decimalPlaces`, which we DO compare.
//! - `sourceName` — an absolute filesystem path, which is environment-specific.
//!
//! Everything else is compared exactly, including `decimalMantissa`,
//! `decimalPlaces`, every `astyle` field, costs, `ptags`, `tsourcepos`
//! line/column, `ptransaction_`, and `pbalanceassertion` (incl. `sourceLine`
//! and `sourceColumn`).

use std::path::PathBuf;

use ledgeline_core::{parse_journal, wire};
use serde_json::Value;

/// Keys ignored anywhere in the JSON tree (see module docs).
const IGNORED_KEYS: [&str; 2] = ["floatingPoint", "sourceName"];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .canonicalize()
        .expect("fixtures directory should resolve")
}

fn parsed_transactions() -> Value {
    let dir = fixtures_dir();
    let journal_path = dir.join("sample.journal");
    let text = std::fs::read_to_string(&journal_path).expect("sample.journal readable");
    let source_name = journal_path.to_string_lossy().to_string();

    let journal = parse_journal(&text, &source_name).expect("journal parses");
    wire::journal_to_value(&journal).expect("journal serializes")
}

fn expected_transactions() -> Value {
    let path = fixtures_dir().join("api/v1.52/transactions.json");
    let text = std::fs::read_to_string(&path).expect("transactions.json readable");
    serde_json::from_str(&text).expect("transactions.json parses")
}

/// Recursively compare two JSON values, ignoring [`IGNORED_KEYS`]. On mismatch,
/// returns the JSON path plus the differing values so failures are pinpointed.
fn compare(path: &str, expected: &Value, actual: &Value) -> Result<(), String> {
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            for (key, expected_value) in expected_map {
                if IGNORED_KEYS.contains(&key.as_str()) {
                    continue;
                }
                match actual_map.get(key) {
                    Some(actual_value) => {
                        compare(&format!("{path}.{key}"), expected_value, actual_value)?;
                    }
                    None => {
                        return Err(format!("{path}.{key}: missing in actual output"));
                    }
                }
            }
            for key in actual_map.keys() {
                if IGNORED_KEYS.contains(&key.as_str()) {
                    continue;
                }
                if !expected_map.contains_key(key) {
                    return Err(format!("{path}.{key}: unexpected key in actual output"));
                }
            }
            Ok(())
        }
        (Value::Array(expected_items), Value::Array(actual_items)) => {
            if expected_items.len() != actual_items.len() {
                return Err(format!(
                    "{path}: array length expected {} but was {}",
                    expected_items.len(),
                    actual_items.len()
                ));
            }
            for (index, (expected_value, actual_value)) in
                expected_items.iter().zip(actual_items).enumerate()
            {
                compare(&format!("{path}[{index}]"), expected_value, actual_value)?;
            }
            Ok(())
        }
        (Value::Number(expected_number), Value::Number(actual_number)) => {
            // Compare via canonical decimal text so i128/i64/u64 representation
            // differences never produce false mismatches.
            if expected_number.to_string() == actual_number.to_string() {
                Ok(())
            } else {
                Err(format!(
                    "{path}: number expected {expected_number} but was {actual_number}"
                ))
            }
        }
        _ => {
            if expected == actual {
                Ok(())
            } else {
                Err(format!("{path}: expected {expected} but was {actual}"))
            }
        }
    }
}

#[test]
fn transactions_match_hledger_snapshot() {
    let expected = expected_transactions();
    let actual = parsed_transactions();

    // Sanity: the snapshot is the full 185-transaction array.
    assert_eq!(
        expected.as_array().map(Vec::len),
        Some(185),
        "snapshot should contain 185 transactions"
    );

    if let Err(message) = compare("$", &expected, &actual) {
        panic!("transactions parity mismatch at {message}");
    }
}

/// A focused check on the opening-balances transaction: the inferred
/// `equity:opening` leg, the balance assertion column/line, and tag
/// inheritance (`assets:bank:checking` -> `[C, A]`).
#[test]
fn opening_balances_inference_and_assertion() {
    let actual = parsed_transactions();
    let first = &actual.as_array().expect("array")[0];

    assert_eq!(first["tindex"], serde_json::json!(1));
    assert_eq!(first["tstatus"], serde_json::json!("Cleared"));
    assert_eq!(first["tsourcepos"][0]["sourceLine"], serde_json::json!(88));
    assert_eq!(first["tsourcepos"][1]["sourceLine"], serde_json::json!(93));

    let postings = first["tpostings"].as_array().expect("postings");

    // Inferred equity leg: -14550.00 with the canonical $ style, precision 2.
    let equity = postings
        .iter()
        .find(|p| p["paccount"] == serde_json::json!("equity:opening"))
        .expect("equity leg present");
    let quantity = &equity["pamount"][0]["aquantity"];
    assert_eq!(quantity["decimalMantissa"], serde_json::json!(-1455000));
    assert_eq!(quantity["decimalPlaces"], serde_json::json!(2));
    assert_eq!(
        equity["pamount"][0]["astyle"]["asprecision"],
        serde_json::json!(2)
    );

    // Balance assertion on the checking leg: line 89, column 42 (the `=`).
    let checking = &postings[0];
    assert_eq!(
        checking["paccount"],
        serde_json::json!("assets:bank:checking")
    );
    assert_eq!(
        checking["pbalanceassertion"]["baposition"]["sourceLine"],
        serde_json::json!(89)
    );
    assert_eq!(
        checking["pbalanceassertion"]["baposition"]["sourceColumn"],
        serde_json::json!(42)
    );
    assert_eq!(
        checking["ptags"],
        serde_json::json!([["type", "C"], ["type", "A"]])
    );
}

/// A focused check on a priced posting with a normalized inferred counter-leg:
/// `10 AAPL @ $220.00` -> cash `$-2200` at scale 0 but style precision 2, plus
/// the `name:` comment tag ordered before the inherited `type:` tag.
#[test]
fn aapl_buy_cost_and_normalized_inference() {
    let actual = parsed_transactions();
    let txn = actual
        .as_array()
        .expect("array")
        .iter()
        .find(|t| t["tdate"] == serde_json::json!("2024-09-16"))
        .expect("AAPL buy present");
    let postings = txn["tpostings"].as_array().expect("postings");

    let aapl = &postings[0];
    let amount = &aapl["pamount"][0];
    assert_eq!(amount["acommodity"], serde_json::json!("AAPL"));
    assert_eq!(amount["acost"]["tag"], serde_json::json!("UnitCost"));
    assert_eq!(
        amount["acost"]["contents"]["aquantity"]["decimalMantissa"],
        serde_json::json!(22000)
    );
    assert_eq!(
        aapl["ptags"],
        serde_json::json!([["name", "Apple Inc."], ["type", "A"]])
    );

    let cash = &postings[1];
    let cash_qty = &cash["pamount"][0]["aquantity"];
    assert_eq!(cash_qty["decimalMantissa"], serde_json::json!(-2200));
    assert_eq!(cash_qty["decimalPlaces"], serde_json::json!(0));
    assert_eq!(
        cash["pamount"][0]["astyle"]["asprecision"],
        serde_json::json!(2)
    );
}
