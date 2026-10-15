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
use std::process::Command;

pub const DEFAULT_IGNORED_KEYS: [&str; 2] = ["floatingPoint", "sourceName"];

/// `{(aname, aditags-or-null)}`: `None` = undeclared, `Some(tags)` = declared.
pub type AccountContract = BTreeSet<(String, Option<Vec<(String, String)>>)>;

/// `git`, with every `GIT_*` variable removed, for tests that build throwaway
/// repositories.
///
/// git exports `GIT_DIR` and `GIT_INDEX_FILE` to its hooks, so `cargo test`
/// run from the pre-push hook used to point every scaffolding `git init` /
/// `git config` / `git commit` at the REAL repository instead of the temp
/// directory: it rewrote the shared config and committed over the branch. A
/// test repo must depend on its `current_dir` and nothing else.
#[must_use]
pub fn git_command() -> Command {
    let mut command = Command::new("git");
    std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_string_lossy().starts_with("GIT_"))
        .for_each(|key| {
            command.env_remove(key);
        });
    command
}

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

/// A minimal xlsx built in memory: one sheet, `Statement`, whose
/// `<sheetData>` is `sheet_data`, plus `xl/sharedStrings.xml` when given.
///
/// For the allocation-bomb tests, whose whole point is a workbook no exporter
/// would write — a committed fixture holding one would look like a mistake.
/// Cells are written by hand: `<c r="A1" t="inlineStr"><is><t>Date</t></is></c>`
/// or `<c r="B2"><v>-54.2</v></c>`.
pub fn xlsx(sheet_data: &str, shared_strings: Option<&str>) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const RELATIONSHIPS: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let workbook = format!(
        r#"<workbook xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}"><sheets><sheet name="Statement" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let package = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELATIONSHIPS}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
    );
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELATIONSHIPS}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
    );
    let sheet =
        format!(r#"<worksheet xmlns="{MAIN}"><sheetData>{sheet_data}</sheetData></worksheet>"#);
    let parts = [
        Some(("_rels/.rels", package.as_str())),
        Some(("xl/workbook.xml", workbook.as_str())),
        Some(("xl/_rels/workbook.xml.rels", rels.as_str())),
        Some(("xl/worksheets/sheet1.xml", sheet.as_str())),
        shared_strings.map(|strings| ("xl/sharedStrings.xml", strings)),
    ];

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in parts.into_iter().flatten() {
        zip.start_file(name, SimpleFileOptions::default())
            .expect("a zip entry starts");
        zip.write_all(body.as_bytes()).expect("a zip entry writes");
    }
    zip.finish().expect("the zip finishes").into_inner()
}

/// The two-cell workbook from the original report: a real two-row table in
/// `A1:B2`, and one number at `XFD1048576` — the last cell a sheet can have.
/// Made dense, the bounding box is 16,384 columns by 1,048,576 rows: ~17 billion
/// cells, ~550 GB.
pub fn sparse_bomb_xlsx() -> Vec<u8> {
    xlsx(
        concat!(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Date</t></is></c>"#,
            r#"<c r="B1" t="inlineStr"><is><t>Amount</t></is></c></row>"#,
            r#"<row r="2"><c r="A2" t="inlineStr"><is><t>2026-01-05</t></is></c>"#,
            r#"<c r="B2"><v>-54.2</v></c></row>"#,
            r#"<row r="1048576"><c r="XFD1048576"><v>1</v></c></row>"#,
        ),
        None,
    )
}

/// A workbook that asks for ~240 GB while merely being *opened*: its
/// shared-string table declares ten billion entries, and `calamine` reserves
/// room for all of them before reading one. No size cap in the spreadsheet lane
/// can see this coming, so only the sandboxed worker's memory budget stops it —
/// which is what makes it the test of that budget. Never parse it in-process.
pub fn shared_strings_bomb_xlsx() -> Vec<u8> {
    xlsx(
        r#"<row r="1"><c r="A1" t="s"><v>0</v></c></row>"#,
        Some(
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="10000000000"><si><t>Date</t></si></sst>"#,
        ),
    )
}

/// Point the import routes' workbook worker at the `ledgeline` binary cargo
/// built for this suite.
///
/// The routes parse spreadsheets by re-executing the current binary, and in an
/// in-process suite that is the test harness, which does not know how to be a
/// worker. Idempotent, so every `Tree` constructor can call it.
pub fn use_built_workbook_worker() {
    ledgeline::set_worker_executable(env!("CARGO_BIN_EXE_ledgeline"));
}
