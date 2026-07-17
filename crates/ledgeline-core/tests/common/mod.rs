//! Shared test helpers for the parity suites: fixture loading and a recursive,
//! key-ignoring JSON semantic comparator.
//!
//! The comparator mirrors the pattern proven in `transactions_parity.rs`: two
//! JSON trees are equal when every non-ignored key matches, arrays match
//! elementwise, and numbers match by canonical decimal text (so i128/i64/u64
//! representation never causes a false mismatch).

#![allow(dead_code)]

use ledgeline_core::{Journal, parse_journal};
use serde_json::Value;
use std::path::PathBuf;

/// Keys ignored everywhere by default: `floatingPoint` (display-only, brittle
/// under float equality) and `sourceName` (an environment-specific absolute
/// path). The exact decimal lives in `decimalMantissa`/`decimalPlaces`, which we
/// still compare.
pub const DEFAULT_IGNORED_KEYS: [&str; 2] = ["floatingPoint", "sourceName"];

/// The repository `fixtures/` directory.
#[must_use]
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .canonicalize()
        .expect("fixtures directory should resolve")
}

/// Parse `fixtures/sample.journal`, recording its absolute path as the source
/// name (so `sourceName` fields match the committed snapshots).
#[must_use]
pub fn fixture_journal() -> Journal {
    let journal_path = fixtures_dir().join("sample.journal");
    let text = std::fs::read_to_string(&journal_path).expect("sample.journal readable");
    let source_name = journal_path.to_string_lossy().to_string();
    parse_journal(&text, &source_name).expect("journal parses")
}

/// Read and parse a snapshot under `fixtures/api/v1.52/`.
#[must_use]
pub fn read_snapshot(name: &str) -> Value {
    let path = fixtures_dir().join("api/v1.52").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} readable: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} parses: {e}"))
}

/// Compare two JSON values semantically, ignoring [`DEFAULT_IGNORED_KEYS`].
///
/// # Errors
/// Returns the JSON path plus the differing values on the first mismatch.
pub fn compare(path: &str, expected: &Value, actual: &Value) -> Result<(), String> {
    compare_with(path, expected, actual, &DEFAULT_IGNORED_KEYS)
}

/// Compare two JSON values semantically, ignoring the given keys anywhere they
/// occur in the tree.
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
