//! Posting-row CSV export (plan 39 Phase 4) against hledger 1.52's own
//! `print --auto -O csv`.
//!
//! Byte compatibility with hledger is the contract: the grouped-CSV importer
//! reads this format whether hledger or Ledgeline wrote it.
//!
//! * **Hermetic** — the goldens under `fixtures/export/csv/` were captured from
//!   `hledger 1.52 --no-conf print --auto -O csv` over the named fixtures
//!   (2026-11-12) and are compared byte for byte in a plain `cargo test`.
//! * **hledger-backed** — with `LEDGELINE_HLEDGER_EXPORT_CHECK=1`
//!   (`just hledger-checks`) EVERY `*.journal` under `fixtures/` that hledger
//!   accepts is exported both ways and compared byte for byte, so a new
//!   fixture exercising a new corner is checked without anyone writing a test.

mod common;

use common::fixtures_dir;
use ledgeline_core::export::{csv::export_csv, load_journal};
use std::path::{Path, PathBuf};
use std::process::Command;

const OPT_IN: &str = "LEDGELINE_HLEDGER_EXPORT_CHECK";

/// Fixture journal (relative to `fixtures/`) → golden CSV (relative to
/// `fixtures/export/csv/`).
const GOLDENS: [(&str, &str); 6] = [
    ("export/csv-edge-cases.journal", "csv-edge-cases.csv"),
    ("sample.journal", "sample.csv"),
    ("account-types/non-english.journal", "non-english.csv"),
    ("export/splits-transfers.journal", "splits-transfers.csv"),
    ("export/multi-currency.journal", "multi-currency.csv"),
    ("export/holding.journal", "holding.csv"),
];

fn ours(journal: &Path) -> String {
    export_csv(&load_journal(journal).expect("fixture parses")).text
}

#[test]
fn matches_the_hledger_goldens() {
    let fixtures = fixtures_dir();
    for (journal, golden) in GOLDENS {
        let expected = std::fs::read_to_string(fixtures.join("export/csv").join(golden))
            .expect("golden readable");
        let actual = ours(&fixtures.join(journal));
        assert_eq!(actual, expected, "{journal} differs from {golden}");
    }
}

#[test]
fn counts_transactions_and_rows() {
    let export = export_csv(&load_journal(&fixtures_dir().join("sample.journal")).expect("parses"));
    assert_eq!(export.rows, export.text.lines().count() - 1);
    assert!(export.transactions > 0 && export.rows >= 2 * export.transactions);
}

/// Every `*.journal` under `dir`, sorted.
fn journals(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "journal") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn every_fixture_matches_hledger_print_csv() {
    if std::env::var_os(OPT_IN).is_none() {
        eprintln!("skipping: set {OPT_IN}=1 (or run `just hledger-checks`)");
        return;
    }
    let mut compared = 0;
    let mut failures = Vec::new();
    for journal in journals(&fixtures_dir()) {
        let file = journal.to_string_lossy().to_string();
        let output = Command::new("hledger")
            .args(["-f", &file, "--no-conf", "print", "--auto", "-O", "csv"])
            .output()
            .expect("hledger runs");
        if !output.status.success() {
            // Deliberately broken fixtures (parse-error cases) — hledger
            // refuses them, so there is nothing to compare against.
            continue;
        }
        let Ok(parsed) = load_journal(&journal) else {
            continue;
        };
        let theirs = String::from_utf8_lossy(&output.stdout).to_string();
        let ours = export_csv(&parsed).text;
        compared += 1;
        if ours != theirs {
            let first = ours
                .lines()
                .zip(theirs.lines())
                .find(|(a, b)| a != b)
                .map(|(a, b)| format!("\n  ours:   {a}\n  hledger: {b}"))
                .unwrap_or_else(|| {
                    format!(
                        " (line counts {} vs {})",
                        ours.lines().count(),
                        theirs.lines().count()
                    )
                });
            failures.push(format!("{file}:{first}"));
        }
    }
    assert!(compared > 0, "no fixture compared");
    assert!(
        failures.is_empty(),
        "{} of {compared} fixtures differ from hledger:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
