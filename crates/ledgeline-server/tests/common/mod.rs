//! Test helpers for the server integration suite: fixture loading, snapshot
//! reading, the account contract projection, and the same key-ignoring JSON
//! semantic comparator used by the core parity tests.
//!
//! The comparator is duplicated here (rather than shared from `ledgeline-core`)
//! because a crate's `tests/common` module cannot be imported across crate
//! boundaries, and we deliberately keep this test-only utility out of the
//! library's public API.

#![allow(dead_code)]

use ledgeline_core::{Journal, parse_journal};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub const DEFAULT_IGNORED_KEYS: [&str; 2] = ["floatingPoint", "sourceName"];

/// `{(aname, aditags-or-null)}`: `None` = undeclared, `Some(tags)` = declared.
pub type AccountContract = BTreeSet<(String, Option<Vec<(String, String)>>)>;

#[must_use]
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .canonicalize()
        .expect("fixtures directory should resolve")
}

#[must_use]
pub fn fixture_journal_path() -> PathBuf {
    fixtures_dir().join("sample.journal")
}

#[must_use]
pub fn fixture_journal() -> Journal {
    let path = fixture_journal_path();
    let text = std::fs::read_to_string(&path).expect("sample.journal readable");
    let source_name = path.to_string_lossy().to_string();
    parse_journal(&text, &source_name).expect("journal parses")
}

#[must_use]
pub fn read_snapshot(name: &str) -> Value {
    let path = fixtures_dir().join("api/v1.52").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} readable: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} parses: {e}"))
}

/// The `/accounts` SPA contract: `{(aname, aditags-or-null)}`. `None` marks an
/// undeclared account; `Some(vec)` a declared one (empty vec = declared, no
/// tags). `adata` is never read here.
#[must_use]
pub fn account_contract(value: &Value) -> AccountContract {
    value
        .as_array()
        .expect("accounts is an array")
        .iter()
        .map(|node| {
            let name = node["aname"].as_str().expect("aname string").to_string();
            let tags = match node.get("adeclarationinfo") {
                Some(Value::Object(info)) => Some(
                    info["aditags"]
                        .as_array()
                        .expect("aditags array")
                        .iter()
                        .map(|pair| {
                            let entries = pair.as_array().expect("tag pair array");
                            (
                                entries[0].as_str().expect("tag key").to_string(),
                                entries[1].as_str().expect("tag value").to_string(),
                            )
                        })
                        .collect(),
                ),
                _ => None,
            };
            (name, tags)
        })
        .collect()
}

/// Compare two JSON values semantically, ignoring [`DEFAULT_IGNORED_KEYS`].
///
/// # Errors
/// Returns the JSON path plus the differing values on the first mismatch.
pub fn compare(path: &str, expected: &Value, actual: &Value) -> Result<(), String> {
    compare_with(path, expected, actual, &DEFAULT_IGNORED_KEYS)
}

/// Compare two JSON values semantically, ignoring the given keys anywhere.
///
/// # Errors
/// Returns the JSON path plus the differing values on the first mismatch.
pub fn compare_with(
    path: &str,
    expected: &Value,
    actual: &Value,
    ignored: &[&str],
) -> Result<(), String> {
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            for (key, expected_value) in expected_map {
                if ignored.contains(&key.as_str()) {
                    continue;
                }
                match actual_map.get(key) {
                    Some(actual_value) => {
                        compare_with(
                            &format!("{path}.{key}"),
                            expected_value,
                            actual_value,
                            ignored,
                        )?;
                    }
                    None => return Err(format!("{path}.{key}: missing in actual output")),
                }
            }
            for key in actual_map.keys() {
                if ignored.contains(&key.as_str()) {
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
                compare_with(
                    &format!("{path}[{index}]"),
                    expected_value,
                    actual_value,
                    ignored,
                )?;
            }
            Ok(())
        }
        (Value::Number(expected_number), Value::Number(actual_number)) => {
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
