//! `include` of time files (plan 46 T12, facts 5–6): `.timedot`/`.timeclock`
//! by extension, `timedot:`/`timeclock:` prefixes, and globs (`*`, `?`,
//! `[..]`, `**`) sorted for a stable order — every match still held to the
//! include guards (confinement, hidden entries, caps).

use ledgeline_core::parse::parse_journal_in;
use ledgeline_core::time::TimeFormat;
use ledgeline_core::vault::{MemVault, VaultPath};
use ledgeline_core::{Journal, ParseError};
use std::collections::HashMap;

fn vault(files: &[(&str, &str)]) -> MemVault {
    let vault = MemVault::with_root("/books");
    for (path, text) in files {
        vault.insert_file(path, text);
    }
    vault
}

fn parse(vault: &MemVault, main: &str) -> Result<Journal, ParseError> {
    parse_journal_in(vault, &VaultPath::new(main).expect("path"), &HashMap::new())
        .map(|parsed| parsed.journal)
}

fn time_accounts(journal: &Journal) -> Vec<String> {
    journal
        .time_transactions
        .iter()
        .flat_map(|txn| txn.postings.iter().map(|p| p.account.0.clone()))
        .collect()
}

#[test]
fn globs_include_matches_in_sorted_order_and_skip_hidden_and_self() {
    let vault = vault(&[
        (
            "main.journal",
            "include time/**/*.timedot\ninclude *.journal\n2026-01-01 x\n  a  $1\n  b\n",
        ),
        ("other.journal", "account other\n"),
        ("time/b.timedot", "2026-11-02\n  b  1h\n"),
        ("time/pat/a.timedot", "2026-11-01\n  a  1h\n"),
        ("time/pat/deep/c.timedot", "2026-11-03\n  c  30m\n"),
        ("time/.hidden/z.timedot", "2026-11-04\n  z  1h\n"),
        ("time/notes.txt", "2026-11-04\n  z  1h\n"),
    ]);
    let journal = parse(&vault, "main.journal").expect("parses");
    // `time/**/*.timedot` sorted by path: time/b, time/pat/a, time/pat/deep/c.
    assert_eq!(time_accounts(&journal), vec!["b", "a", "c"]);
    assert_eq!(journal.transactions.len(), 1);
    assert!(journal.accounts.iter().any(|a| a.name.0 == "other"));
    let parsed = parse_journal_in(
        &vault,
        &VaultPath::new("main.journal").expect("path"),
        &HashMap::new(),
    )
    .expect("parses");
    let formats: Vec<Option<TimeFormat>> = parsed.sources.iter().map(|s| s.time_format).collect();
    assert_eq!(
        formats,
        vec![
            None,
            Some(TimeFormat::Timedot),
            Some(TimeFormat::Timedot),
            Some(TimeFormat::Timedot),
            None
        ]
    );
}

#[test]
fn time_transactions_are_shaped_like_hledgers() {
    let vault = vault(&[
        (
            "main.journal",
            "include day.timedot\ninclude work.timeclock\n",
        ),
        (
            "day.timedot",
            "2026-11-26 Thu  ; person: pat\n    clients:acme  20m  ; invoice: 108, a note\n    internal  ab\n",
        ),
        (
            "work.timeclock",
            "i 2026-11-26 23:00 clients:acme  late  ; person: pat\no 2026-11-27 01:30 ; invoice: 9\ni 2026-11-27 09:00 open\n",
        ),
    ]);
    let journal = parse(&vault, "main.journal").expect("parses");
    assert!(journal.transactions.is_empty());
    let day = &journal.time_transactions[0];
    assert_eq!(day.date, "2026-11-26");
    assert_eq!(day.description, "Thu");
    assert_eq!(day.status, ledgeline_core::model::Status::Cleared);
    assert_eq!(day.tags, vec![("person".to_string(), "pat".to_string())]);
    assert_eq!(day.postings.len(), 3);
    let first = &day.postings[0];
    assert_eq!(first.ptype, ledgeline_core::model::PostingType::Virtual);
    assert_eq!(first.amounts[0].commodity.0, "");
    // Exact: not hledger 1.52's 0.33.
    assert_eq!(
        ledgeline_core::view::wire::plain_decimal(first.amounts[0].quantity),
        "0.333333333"
    );
    assert!(
        first
            .tags
            .contains(&("invoice".to_string(), "108".to_string()))
    );
    assert!(
        day.postings[1]
            .tags
            .contains(&("t".to_string(), "a".to_string()))
    );
    // The session splits at midnight; the open one is left out (no clock).
    let clocked: Vec<(&str, &str, String)> = journal.time_transactions[1..]
        .iter()
        .map(|txn| {
            (
                txn.date.as_str(),
                txn.postings[0].amounts[0].commodity.0.as_str(),
                ledgeline_core::view::wire::plain_decimal(txn.postings[0].amounts[0].quantity),
            )
        })
        .collect();
    assert_eq!(
        clocked,
        vec![
            ("2026-11-26", "h", "1".to_string()),
            ("2026-11-27", "h", "1.5".to_string())
        ]
    );
    assert!(
        journal.time_transactions[1]
            .tags
            .contains(&("invoice".to_string(), "9".to_string()))
    );
}

#[test]
fn prefixes_name_the_format() {
    let vault = vault(&[
        (
            "main.journal",
            "include timedot:hours.txt\ninclude timeclock:clock.log\ninclude journal:more.dat\n",
        ),
        ("hours.txt", "2026-11-01\n  a  1h\n"),
        ("clock.log", "i 2026-11-01 09:00 b\no 2026-11-01 10:00\n"),
        ("more.dat", "account c\n"),
    ]);
    let journal = parse(&vault, "main.journal").expect("parses");
    assert_eq!(time_accounts(&journal), vec!["a", "b"]);
    assert!(journal.accounts.iter().any(|a| a.name.0 == "c"));
}

#[test]
fn a_time_file_can_be_the_main_file() {
    let vault = vault(&[("hours.timedot", "2026-11-01\n  a  1h\n")]);
    let journal = parse(&vault, "hours.timedot").expect("parses");
    assert_eq!(time_accounts(&journal), vec!["a"]);
}

#[test]
fn failures_are_located_and_guards_still_hold() {
    let error = |files: &[(&str, &str)]| {
        parse(&vault(files), "main.journal")
            .expect_err("fails")
            .to_string()
    };
    let message = error(&[("main.journal", "include none/*.timedot\n")]);
    assert!(
        message.contains("No files were matched by: none/*.timedot"),
        "{message}"
    );
    let message = error(&[
        ("main.journal", "include day.timedot\n"),
        ("day.timedot", "2026-11-01\n  a  1.5 h\n"),
    ]);
    assert!(
        message.contains(":2:") && message.contains("1.5 h"),
        "{message}"
    );
    let message = error(&[
        ("main.journal", "include work.timeclock\n"),
        (
            "work.timeclock",
            "i 2026-11-01 09:00 a\ni 2026-11-01 09:30 a\n",
        ),
    ]);
    assert!(message.contains("already clocked in"), "{message}");
    let message = error(&[("main.journal", "include ../*.timedot\n")]);
    assert!(
        message.contains("outside the journal directory"),
        "{message}"
    );
    let message = error(&[
        ("main.journal", "include .hidden/*.timedot\n"),
        (".hidden/a.timedot", "2026-11-01\n  a  1h\n"),
    ]);
    assert!(message.contains("hidden entry"), "{message}");
}

#[test]
fn glob_classes_and_question_marks() {
    let vault = vault(&[
        ("main.journal", "include t/[ab]?.timedot\n"),
        ("t/a1.timedot", "2026-11-01\n  a  1h\n"),
        ("t/b2.timedot", "2026-11-01\n  b  1h\n"),
        ("t/c3.timedot", "2026-11-01\n  c  1h\n"),
        ("t/a12.timedot", "2026-11-01\n  d  1h\n"),
    ]);
    let journal = parse(&vault, "main.journal").expect("parses");
    assert_eq!(time_accounts(&journal), vec!["a", "b"]);
}
