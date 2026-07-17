//! Byte-semantic parity tests for the Phase 2 read endpoints against the real
//! hledger-web 1.52 snapshots in `fixtures/api/v1.52/`.
//!
//! `/version`, `/accountnames`, `/commodities`, and `/prices` are compared in
//! full (ignoring only `floatingPoint`). `/accounts` is compared on two levels:
//!   1. REQUIRED — the SPA contract: the set of `(aname, aditags)` pairs must
//!      equal the snapshot's exactly (declared-vs-undeclared distinguished by
//!      `null` tags), with `adata` EXPLICITLY EXCLUDED because its real balances
//!      are Phase-3 work we do not yet compute.
//!   2. BONUS — every node field EXCEPT `adata` (ignoring `sourceName`), keyed
//!      by account name, to prove the tree links and full declaration info also
//!      reproduce the snapshot.

mod common;

use common::{compare, fixture_journal, read_snapshot};
use ledgeline_core::wire;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// `{(aname, aditags-or-null)}`: `None` = undeclared, `Some(tags)` = declared.
type AccountContract = BTreeSet<(String, Option<Vec<(String, String)>>)>;

#[test]
fn version_matches_snapshot() {
    assert_eq!(wire::version_value(), read_snapshot("version.json"));
}

#[test]
fn accountnames_match_snapshot() {
    let expected = read_snapshot("accountnames.json");
    let actual =
        wire::journal_to_accountnames_value(&fixture_journal()).expect("accountnames serialize");

    assert_eq!(
        expected.as_array().map(Vec::len),
        Some(42),
        "snapshot should contain 42 account names"
    );
    if let Err(message) = compare("$", &expected, &actual) {
        panic!("accountnames parity mismatch at {message}");
    }
}

#[test]
fn commodities_match_snapshot() {
    let expected = read_snapshot("commodities.json");
    let actual =
        wire::journal_to_commodities_value(&fixture_journal()).expect("commodities serialize");

    // `"$"` (0x24) must sort before the alphabetic symbols.
    assert_eq!(
        actual,
        serde_json::json!(["$", "AAPL", "EUR", "GLD", "NVDA", "TSLA", "VTI"])
    );
    if let Err(message) = compare("$", &expected, &actual) {
        panic!("commodities parity mismatch at {message}");
    }
}

#[test]
fn prices_match_snapshot() {
    let expected = read_snapshot("prices.json");
    let actual = wire::journal_to_prices_value(&fixture_journal()).expect("prices serialize");

    assert_eq!(
        expected.as_array().map(Vec::len),
        Some(11),
        "snapshot should contain 11 market prices"
    );
    if let Err(message) = compare("$", &expected, &actual) {
        panic!("prices parity mismatch at {message}");
    }
}

/// REQUIRED /accounts contract: `{(aname, aditags)}` set equality. `adata` is
/// EXCLUDED — we emit a structurally-valid empty balance tree, not real
/// balances (Phase 3), so comparing it would be meaningless here.
#[test]
fn accounts_name_to_tags_contract_matches_snapshot() {
    let expected = read_snapshot("accounts.json");
    let actual = wire::journal_to_accounts_value(&fixture_journal()).expect("accounts serialize");

    let expected_contract = account_contract(&expected);
    let actual_contract = account_contract(&actual);

    // 41 posting-referenced accounts + the synthetic root (declared-but-unused
    // `expenses:shopping` is intentionally absent — it is only in /accountnames).
    assert_eq!(
        expected_contract.len(),
        42,
        "snapshot should expose 42 account nodes"
    );
    assert_eq!(
        actual_contract, expected_contract,
        "the (aname -> aditags) contract must match the snapshot exactly"
    );
}

/// BONUS: full node parity EXCEPT `adata`, keyed by name (ignoring
/// `sourceName`). Proves `aparent_`/`asubs_`/`aboring`/`asubs` and the complete
/// `adeclarationinfo` (adicomment/adideclarationorder/adisourcepos) reproduce
/// the snapshot too.
#[test]
fn accounts_full_except_adata_matches_snapshot() {
    let expected = read_snapshot("accounts.json");
    let actual = wire::journal_to_accounts_value(&fixture_journal()).expect("accounts serialize");

    let expected_nodes = accounts_by_name_without_adata(&expected);
    let actual_nodes = accounts_by_name_without_adata(&actual);

    assert_eq!(
        actual_nodes.keys().collect::<Vec<_>>(),
        expected_nodes.keys().collect::<Vec<_>>(),
        "account node names must match the snapshot"
    );
    for (name, expected_node) in &expected_nodes {
        let actual_node = &actual_nodes[name];
        if let Err(message) = compare(&format!("account[{name}]"), expected_node, actual_node) {
            panic!("accounts (excl. adata) parity mismatch at {message}");
        }
    }
}

/// The `{(aname, aditags-or-null)}` set. `None` = undeclared (adeclarationinfo
/// null); `Some([])` = declared with no tags — a distinction the contract keeps.
fn account_contract(value: &Value) -> AccountContract {
    value
        .as_array()
        .expect("accounts is an array")
        .iter()
        .map(|node| {
            let name = node["aname"].as_str().expect("aname string").to_string();
            let tags = match node.get("adeclarationinfo") {
                Some(Value::Object(info)) => Some(read_tags(&info["aditags"])),
                _ => None,
            };
            (name, tags)
        })
        .collect()
}

/// Each account node with its `adata` removed, keyed by `aname`.
fn accounts_by_name_without_adata(value: &Value) -> BTreeMap<String, Value> {
    value
        .as_array()
        .expect("accounts is an array")
        .iter()
        .map(|node| {
            let name = node["aname"].as_str().expect("aname string").to_string();
            let mut object = node.as_object().expect("node is an object").clone();
            object.remove("adata");
            (name, Value::Object(object))
        })
        .collect()
}

fn read_tags(value: &Value) -> Vec<(String, String)> {
    value
        .as_array()
        .expect("aditags is an array")
        .iter()
        .map(|pair| {
            let entries = pair.as_array().expect("tag pair is an array");
            (
                entries[0].as_str().expect("tag key string").to_string(),
                entries[1].as_str().expect("tag value string").to_string(),
            )
        })
        .collect()
}
