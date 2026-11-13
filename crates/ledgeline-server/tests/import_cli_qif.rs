//! `ledgeline import` against a QIF file (plan 39 Phase 2): the same CLI
//! branch the QuickBooks Journal import takes (`cli_import_qb_journal`),
//! chosen by the file's name and content. Spawns the real binary, as
//! `import_cli_qb_journal.rs` does; nothing here shells out to hledger.

mod common;

use common::fixtures_dir;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

const OPENING: &str = "2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

/// Plain aliases for every source name `transfers.qif` uses.
const ALIASES: &str = "alias Checking = assets:bank:checking\n\
                       alias Savings = assets:bank:savings\n\
                       alias Visa = liabilities:visa\n\
                       alias Equity:Opening Balances = equity:opening\n\
                       alias Bank Charges = expenses:bank\n";

struct Tree {
    dir: TempDir,
    config: TempDir,
}

impl Tree {
    fn write(journal: &str) -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), journal).expect("write journal");
        Self {
            dir,
            config: TempDir::new().expect("config dir"),
        }
    }

    fn journal_text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("main.journal")).expect("journal readable")
    }

    fn import(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ledgeline"))
            .arg("import")
            .args(args)
            .current_dir(self.dir.path())
            .env("LEDGELINE_CONFIG_DIR", self.config.path())
            .env("LEDGELINE_IGNORE_USER_HLEDGER_CONF", "1")
            .output()
            .expect("the ledgeline binary runs")
    }
}

fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join("import/qif").join(name)
}

fn transcript(output: &Output) -> String {
    format!(
        "exit {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn input() -> String {
    fixture("transfers.qif")
        .to_str()
        .expect("utf-8 path")
        .to_string()
}

#[test]
fn a_qif_file_is_imported_and_a_second_run_writes_nothing() {
    let tree = Tree::write(&format!("{ALIASES}\n{OPENING}"));
    let output = tree.import(&["-i", &input(), "-j", "main.journal"]);
    assert!(output.status.success(), "{}", transcript(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("4 transactions"), "{stdout}");
    assert!(stdout.contains("appended 4 transactions"), "{stdout}");

    let text = tree.journal_text();
    assert!(text.contains("id: qif-"), "{text}");
    assert!(text.contains("assets:bank:savings"), "{text}");
    assert!(text.contains("* Monthly savings"), "{text}");

    let again = tree.import(&["-i", &input(), "-j", "main.journal"]);
    assert!(again.status.success(), "{}", transcript(&again));
    let stdout = String::from_utf8_lossy(&again.stdout);
    assert!(stdout.contains("0 new, 4 already imported"), "{stdout}");
    assert_eq!(tree.journal_text(), text, "the re-run wrote nothing");
}

#[test]
fn unmapped_qif_accounts_are_named_and_nothing_is_written() {
    let tree = Tree::write(OPENING);
    let output = tree.import(&["-i", &input(), "-j", "main.journal"]);
    assert!(!output.status.success(), "{}", transcript(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("these QIF accounts have no matching alias"),
        "{stderr}"
    );
    assert!(stderr.contains("Checking"), "{stderr}");
    assert_eq!(tree.journal_text(), OPENING);
}

#[test]
fn csv_flags_are_refused_for_a_qif_export() {
    let tree = Tree::write(&format!("{ALIASES}\n{OPENING}"));
    let output = tree.import(&[
        "-i",
        &input(),
        "-j",
        "main.journal",
        "-o",
        "bank.csv",
        "-r",
        "bank.csv.rules",
    ]);
    assert!(!output.status.success(), "{}", transcript(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not used for a QIF export"), "{stderr}");
}

#[test]
fn a_dry_run_reports_and_writes_nothing() {
    let tree = Tree::write(&format!("{ALIASES}\n{OPENING}"));
    let before = tree.journal_text();
    let output = tree.import(&["-i", &input(), "-j", "main.journal", "--dry-run"]);
    assert!(output.status.success(), "{}", transcript(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("4 new"), "{stdout}");
    assert!(stdout.contains("Nothing was written"), "{stdout}");
    assert_eq!(tree.journal_text(), before);
}
