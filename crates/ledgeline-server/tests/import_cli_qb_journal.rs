//! `ledgeline import` against a QuickBooks Online "Journal" report export
//! (WP-17 Phase D — see `plans/17-quickbooks-journal-import.md`'s Phase D
//! section and its "Contract amendments made during implementation" there).
//!
//! Mirrors `import_cli.rs`'s own discipline (spawn the real binary, assert on
//! exit codes / stdout / stderr / the journal's own bytes) rather than
//! `qb_journal_endpoints.rs`'s HTTP one, because the thing under test here is
//! the SECOND door into the same write pipeline — the CLI has no stage, no
//! HTTP request, and (unlike the CSV path) no `-o`/`-r` at all. Everything
//! hermetic: this pipeline never shells out to hledger (see
//! `qb_journal_api`'s own module docs), so nothing here needs an opt-in gate.

mod common;

use common::fixtures_dir;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// The opening balance every scratch journal starts from — the same shape
/// `qb_journal_endpoints.rs`'s own `Tree::bare` uses, so the accounts and
/// amounts this suite asserts on line up with that file's.
const OPENING: &str = "2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

/// Plain aliases resolving every account `simple.xlsx` needs (see
/// `qb_journal_endpoints.rs`'s `preview_reports_every_account_unmapped_with_no_aliases_declared`
/// for where these four names came from — measured against the fixture, not
/// invented). The alias on the PARENT `6000 Sales and Marketing` covers the
/// sub-account `6000 Sales and Marketing:6001 Sales & Marketing Tools` the
/// posting actually uses, via the plain-alias prefix rule
/// `plans/17-quickbooks-journal-import.md`'s Phase B section documents.
const ALIASES: &str = "alias Riverbank BUSINESS CHECKING (0002) = assets:bank:qb-checking\n\
                       alias 3000 Member Equity = equity:member\n\
                       alias 2005 Northbank Credit Card = liabilities:card\n\
                       alias 6000 Sales and Marketing = expenses:marketing\n";

/// A second, hand-written pair of transactions that is ALREADY out of date
/// order (March before February) before any import runs — so a QuickBooks
/// commit's `InsertPosition::DateOrdered` (which places the two new rows near
/// the very front, both dated January) cannot itself fix it, and `sort::plan`
/// still finds the file disordered afterwards. Exercises `--sort` without
/// needing to know anything about the fixture's own dates.
const OUT_OF_ORDER_TAIL: &str = "\n\
2026-03-01 later transaction\n    assets:bank:checking   $-10.00\n    expenses:misc\n\n\
2026-02-01 earlier than the one above\n    assets:bank:checking   $-5.00\n    expenses:misc\n";

struct Tree {
    dir: TempDir,
    config: TempDir,
}

impl Tree {
    /// No aliases at all: every QuickBooks account in `simple.xlsx` is
    /// unmapped.
    fn bare() -> Self {
        Self::write(OPENING)
    }

    /// `simple.xlsx`'s four accounts already resolve.
    fn with_aliases() -> Self {
        Self::write(&format!("{ALIASES}\n{OPENING}"))
    }

    /// [`Tree::with_aliases`], plus a pre-existing out-of-order pair of
    /// transactions unrelated to anything a QuickBooks import writes.
    fn with_aliases_out_of_order() -> Self {
        Self::write(&format!("{ALIASES}\n{OPENING}{OUT_OF_ORDER_TAIL}"))
    }

    fn write(journal: &str) -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), journal).expect("write journal");
        let config = TempDir::new().expect("config dir");
        Self { dir, config }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn journal_text(&self) -> String {
        std::fs::read_to_string(self.path("main.journal")).expect("journal readable")
    }

    /// Every file in the tree, keyed by its relative path — see
    /// `import_cli.rs`'s own `Tree::snapshot` for why this is the blast-radius
    /// proof rather than checking one file a reader would think to check.
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        walk(self.dir.path(), self.dir.path(), &mut files);
        files
    }

    /// Run `ledgeline import` in this tree. See `import_cli.rs`'s own
    /// `Tree::import` for why `current_dir` on the CHILD, never
    /// `std::env::set_current_dir` on this (threaded) test process.
    fn import(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ledgeline"))
            .arg("import")
            .args(args)
            .current_dir(self.dir.path())
            .env("LEDGELINE_CONFIG_DIR", self.config.path())
            .output()
            .expect("the ledgeline binary runs")
    }
}

fn qb_fixture_path(name: &str) -> PathBuf {
    fixtures_dir().join("import/qb-journal").join(name)
}

fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, into);
        } else if let Ok(bytes) = std::fs::read(&path)
            && let Ok(relative) = path.strip_prefix(root)
        {
            into.insert(relative.to_string_lossy().into_owned(), bytes);
        }
    }
}

fn changed(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|name| before.get(*name) != after.get(*name))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn transcript(output: &Output) -> String {
    format!(
        "exit {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ---------------------------------------------------------------------------
// Detection and the happy path
// ---------------------------------------------------------------------------

/// No flag says "this is a QuickBooks Journal export" — content sniffing
/// alone (`ledgeline_core::qb_journal::detect`) picks the branch, the same
/// check the GUI's upload runs. This is the point of the whole feature: `-i
/// simple.xlsx -j main.journal` with none of `-o`/`-r`/`--balance` succeeds
/// and writes real transactions.
#[test]
fn a_quickbooks_journal_export_is_detected_and_committed_from_the_cli() {
    let tree = Tree::with_aliases();
    let before = tree.snapshot();
    let input = qb_fixture_path("simple.xlsx");

    let output = tree.import(&[
        "-i",
        input.to_str().expect("utf-8"),
        "-j",
        "main.journal",
        "--no-git",
    ]);
    assert!(output.status.success(), "{}", transcript(&output));

    let text = tree.journal_text();
    assert!(text.contains("id: 441"), "{text}");
    assert!(text.contains("id: 33"), "{text}");
    // The deposit's own sign check, carried all the way to the written file —
    // same assertion `qb_journal_endpoints.rs`'s HTTP test makes.
    assert!(text.contains("74999.71"), "{text}");
    // The sub-account, alias-rewritten by the PARENT alias's prefix rule.
    assert!(
        text.contains("expenses:marketing:6001 Sales & Marketing Tools"),
        "{text}"
    );

    assert!(
        stdout(&output).contains("2 transactions"),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("appended 2 transactions to main.journal"),
        "{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("ledgeline import -i simple.xlsx -j main.journal --no-git"),
        "the run echoes its own re-runnable command line:\n{}",
        stderr(&output)
    );

    // Blast radius: only the journal. This pipeline never produces a CSV or
    // touches a rules file.
    assert_eq!(
        changed(&before, &tree.snapshot()),
        vec!["main.journal".to_string()],
    );
}

// ---------------------------------------------------------------------------
// The flags this path refuses BY NAME
// ---------------------------------------------------------------------------

/// `-o`/`-r` name a CSV destination and a rules file that this path never
/// produces or reads — refused, not silently ignored.
#[test]
fn output_and_rules_flags_are_refused_for_a_quickbooks_journal_export() {
    let tree = Tree::with_aliases();
    let before = tree.snapshot();
    let input = qb_fixture_path("simple.xlsx");

    let output = tree.import(&[
        "-i",
        input.to_str().expect("utf-8"),
        "-j",
        "main.journal",
        "-o",
        "import/bank.csv",
        "-r",
        "import/bank.csv.rules",
        "--no-git",
    ]);
    assert!(!output.status.success(), "{}", transcript(&output));
    let said = stderr(&output);
    assert!(said.contains("--output"), "{said}");
    assert!(said.contains("--rules"), "{said}");
    assert!(said.contains("QuickBooks Journal"), "{said}");
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "a refused run writes nothing"
    );
}

/// `--balance`/`--balance-account`/`--write-assertion` name a single
/// statement-closing balance this format has none of — an export can write
/// into several accounts across several files in one run.
#[test]
fn balance_flags_are_refused_for_a_quickbooks_journal_export() {
    let tree = Tree::with_aliases();
    let before = tree.snapshot();
    let input = qb_fixture_path("simple.xlsx");

    let output = tree.import(&[
        "-i",
        input.to_str().expect("utf-8"),
        "-j",
        "main.journal",
        "--balance",
        "100.00",
        "--balance-account",
        "assets:bank:checking",
        "--write-assertion",
        "--no-git",
    ]);
    assert!(!output.status.success(), "{}", transcript(&output));
    let said = stderr(&output);
    assert!(said.contains("--balance"), "{said}");
    assert!(said.contains("--balance-account"), "{said}");
    assert!(said.contains("--write-assertion"), "{said}");
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "a refused run writes nothing"
    );
}

// ---------------------------------------------------------------------------
// Unmapped accounts: refuse and list them (nobody to prompt)
// ---------------------------------------------------------------------------

/// The GUI would ask for an alias per unmapped account; a script has nobody
/// to ask, so the run refuses and names every one — the same "ask, don't
/// guess" refusal `qb_journal_api::commit`'s HTTP handler already produces
/// (`commit_refuses_and_names_every_unmapped_account` in
/// `qb_journal_endpoints.rs` is the HTTP half of this same property).
#[test]
fn unmapped_accounts_refuse_and_list_them_from_the_cli() {
    let tree = Tree::bare();
    let before = tree.snapshot();
    let input = qb_fixture_path("simple.xlsx");

    let output = tree.import(&[
        "-i",
        input.to_str().expect("utf-8"),
        "-j",
        "main.journal",
        "--no-git",
    ]);
    assert!(!output.status.success(), "{}", transcript(&output));
    let said = stderr(&output);
    assert!(
        said.contains("Riverbank BUSINESS CHECKING (0002)"),
        "{said}"
    );
    assert!(said.contains("3000 Member Equity"), "{said}");
    assert!(said.contains("2005 Northbank Credit Card"), "{said}");
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "a refused run writes nothing"
    );
}

// ---------------------------------------------------------------------------
// --dry-run
// ---------------------------------------------------------------------------

/// `--dry-run` reports counts and writes nothing at all — proved over the
/// whole tree, the same discipline `import_cli.rs`'s own
/// `a_dry_run_writes_nothing_at_all` uses for the CSV path.
#[test]
fn dry_run_reports_and_writes_nothing_for_a_quickbooks_journal_export() {
    let tree = Tree::with_aliases();
    let before = tree.snapshot();
    let input = qb_fixture_path("simple.xlsx");

    let output = tree.import(&[
        "-i",
        input.to_str().expect("utf-8"),
        "-j",
        "main.journal",
        "--dry-run",
        "--no-git",
    ]);
    assert!(output.status.success(), "{}", transcript(&output));
    assert!(
        stdout(&output).contains("2 transactions"),
        "{}",
        stdout(&output)
    );
    assert!(stdout(&output).contains("2 new"), "{}", stdout(&output));
    assert!(
        stdout(&output).contains("Nothing was written"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "--dry-run must leave the tree byte-identical"
    );
}

// ---------------------------------------------------------------------------
// Re-import / id-based dedup, from the command line
// ---------------------------------------------------------------------------

/// Re-running the same export a second time writes nothing new — the
/// "re-downloading is safe" property the plan documents, reached from the
/// command line rather than the HTTP surface
/// (`a_second_commit_of_the_same_export_imports_nothing_new` in
/// `qb_journal_endpoints.rs` is the HTTP half).
#[test]
fn a_second_cli_commit_of_the_same_export_imports_nothing_new() {
    let tree = Tree::with_aliases();
    let input = qb_fixture_path("simple.xlsx");
    let args = [
        "-i",
        input.to_str().expect("utf-8"),
        "-j",
        "main.journal",
        "--no-git",
    ];

    let first = tree.import(&args);
    assert!(first.status.success(), "{}", transcript(&first));
    let after_first = tree.journal_text();

    let second = tree.import(&args);
    assert!(second.status.success(), "{}", transcript(&second));
    assert_eq!(
        tree.journal_text(),
        after_first,
        "a re-commit of an unchanged export must not touch the file"
    );
    assert!(
        stdout(&second).contains("appended 0 transactions"),
        "{}",
        stdout(&second)
    );
    assert!(
        stdout(&second).contains("2 already imported"),
        "{}",
        stdout(&second)
    );
}

// ---------------------------------------------------------------------------
// --sort
// ---------------------------------------------------------------------------

/// Without `--sort`, a QuickBooks commit that leaves the file out of order
/// (because it ALREADY was, before this import) says so rather than leaving
/// the user to find out — the same note the CSV path prints.
#[test]
fn without_sort_the_cli_notes_the_file_is_out_of_order() {
    let tree = Tree::with_aliases_out_of_order();
    let input = qb_fixture_path("simple.xlsx");

    let output = tree.import(&[
        "-i",
        input.to_str().expect("utf-8"),
        "-j",
        "main.journal",
        "--no-git",
    ]);
    assert!(output.status.success(), "{}", transcript(&output));
    assert!(
        stdout(&output).contains("no longer in date order"),
        "{}",
        stdout(&output)
    );
}

/// `--sort` puts the file back in date order in the same run, without a
/// second command — mirroring `import_cli.rs`'s own
/// `sort_restores_date_order_after_a_backdated_import` for the CSV path.
#[test]
fn sort_restores_date_order_after_a_quickbooks_commit() {
    let tree = Tree::with_aliases_out_of_order();
    let input = qb_fixture_path("simple.xlsx");

    let output = tree.import(&[
        "-i",
        input.to_str().expect("utf-8"),
        "-j",
        "main.journal",
        "--sort",
        "--no-git",
    ]);
    assert!(output.status.success(), "{}", transcript(&output));
    assert!(stdout(&output).contains("re-sorted"), "{}", stdout(&output));

    let text = tree.journal_text();
    let dates: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("2026-"))
        .map(|line| &line[..10])
        .collect();
    let mut expected = dates.clone();
    expected.sort_unstable();
    assert_eq!(dates, expected, "the journal is in date order:\n{text}");

    // A sort moves, it does not drop — the QuickBooks-written rows and the
    // hand-written ones are all still there.
    assert!(text.contains("id: 441"), "{text}");
    assert!(text.contains("later transaction"), "{text}");
    assert!(text.contains("earlier than the one above"), "{text}");
}
