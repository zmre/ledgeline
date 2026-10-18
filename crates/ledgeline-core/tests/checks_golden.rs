//! The engine's problem list reproduces what the SPA's checks pipeline
//! produced before the five local rules moved here (plan 26 B4).
//!
//! `fixtures/journal-view/checks.json` was CAPTURED from
//! `web/src/lib/checks/engine.ts` `runChecks(ALL_RULES)` — the engine
//! diagnostics as `normalizeDiagnostics` decoded them, then pending,
//! uncategorized, negative-cash, missing-description and future-date — over
//! each fixture journal at two pinned "today"s, before `checks/rules.ts` was
//! deleted. Order matters (the drawer groups by first appearance) and is
//! compared exactly. A failure here is the port drifting; the fixture is not
//! to be regenerated from Rust.

mod common;

use common::fixtures_dir;
use ledgeline_core::parse_journal;
use ledgeline_core::view::Classes;
use ledgeline_core::view::checks::{Diagnostics, Severity, engine_findings, local_findings};
use serde_json::{Value, json};

fn golden() -> Value {
    let path = fixtures_dir().join("journal-view/checks.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden readable"))
        .expect("golden is JSON")
}

#[test]
fn problems_match_the_captured_typescript() {
    let golden = golden();
    let journals = golden["cases"].as_array().expect("cases");
    assert!(journals.len() >= 60, "the checks golden lost cases");
    let mut findings_compared = 0;
    for entry in journals {
        let rel = entry["journal"].as_str().expect("journal path");
        let path = fixtures_dir().join(rel);
        let text = std::fs::read_to_string(&path).expect("fixture readable");
        let journal = parse_journal(&text, &path.to_string_lossy()).expect("fixture parses");
        let classes = Classes::of(&journal);
        let engine = engine_findings(&journal);
        for case in entry["cases"].as_array().expect("cases") {
            let today = case["today"].as_str().expect("today");
            let diagnostics = Diagnostics::new(&engine, local_findings(&journal, &classes, today));
            let ours: Vec<Value> = diagnostics
                .findings
                .iter()
                .map(|finding| {
                    let tindex = finding
                        .position
                        .map(|position| journal.transactions[position].index.0);
                    let mut value = json!({
                        "txnIndex": tindex,
                        "rule": finding.rule,
                        "severity": finding.severity,
                        "message": finding.message,
                    });
                    if let Some(account) = &finding.account {
                        value["account"] = json!(account);
                    }
                    value
                })
                .collect();
            assert_eq!(
                Value::from(ours.clone()),
                case["problems"],
                "{rel} @ {today}"
            );
            findings_compared += ours.len();
        }
    }
    assert!(
        findings_compared > 200,
        "too few findings compared: {findings_compared}"
    );
}

/// Every rule fires on the edge-case journal, and the per-transaction index
/// the row flags read agrees with the list.
#[test]
fn every_local_rule_fires_and_rows_see_their_own_flags() {
    let path = fixtures_dir().join("journal-view/edge-cases.journal");
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    let journal = parse_journal(&text, &path.to_string_lossy()).expect("parses");
    let classes = Classes::of(&journal);
    let diagnostics = Diagnostics::new(
        &engine_findings(&journal),
        local_findings(&journal, &classes, "2026-10-17"),
    );
    let rules: std::collections::BTreeSet<&str> =
        diagnostics.findings.iter().map(|f| f.rule).collect();
    for rule in [
        "pending",
        "uncategorized",
        "negative-cash",
        "missing-description",
        "future-date",
    ] {
        assert!(rules.contains(rule), "{rule} should fire: {rules:?}");
    }
    assert_eq!(diagnostics.max_severity(), Some(Severity::Warning));
    // The 2030 rent is the last transaction; its only flag is the future date.
    let last = journal.transactions.len() - 1;
    let flags: Vec<&str> = diagnostics.for_position(last).map(|f| f.rule).collect();
    assert_eq!(flags, vec!["future-date"]);
    // Account-anchored findings belong to no row.
    assert!(
        (0..journal.transactions.len())
            .flat_map(|at| diagnostics.for_position(at))
            .all(|f| f.account.is_none())
    );
}

/// `today` is the CLIENT's date: moving it moves exactly the two calendar rules.
#[test]
fn today_moves_only_the_calendar_rules() {
    let path = fixtures_dir().join("journal-view/edge-cases.journal");
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    let journal = parse_journal(&text, &path.to_string_lossy()).expect("parses");
    let classes = Classes::of(&journal);
    let count = |today: &str, rule: &str| {
        local_findings(&journal, &classes, today)
            .iter()
            .filter(|f| f.rule == rule)
            .count()
    };
    assert_eq!(count("2025-01-15", "future-date"), 5);
    assert_eq!(count("2031-01-01", "future-date"), 0);
    assert_eq!(
        count("2025-01-15", "pending"),
        count("2031-01-01", "pending")
    );
}
