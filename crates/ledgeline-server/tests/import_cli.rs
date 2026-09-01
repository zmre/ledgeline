//! `ledgeline import` — the scriptable import (WP-16 Phase 3).
//!
//! # What is hermetic and what is not
//!
//! The same three tiers `import_endpoints.rs` uses, and for the same reasons:
//!
//! * **Hermetic** — `--help`, argument validation, and every refusal the runner
//!   makes *before* it would reach hledger (a path outside the journal's tree, a
//!   rules file that is not beside the journal, an unreadable input). These run
//!   in a plain `cargo test` on a machine with nothing installed.
//! * **hledger-backed** — everything that actually imports. Gated behind
//!   `LEDGELINE_HLEDGER_IMPORT_CHECK=1` and run by `just hledger-checks`,
//!   because nothing about an import's *result* can be proved without hledger
//!   and a stub would only test the stub.
//!
//! # This suite spawns the real binary
//!
//! It is the first in the repository to do so: `env!("CARGO_BIN_EXE_ledgeline")`
//! is a path cargo builds and hands us, which is the hermetic version of the
//! `./target/debug/ledgeline` the `justfile` recipes hardcode. That matters here
//! more than anywhere else, because the thing under test IS the process
//! boundary — argument parsing, exit codes, stdout/stderr — and an in-process
//! call to `run_cli_import` would skip exactly the half this file exists for.
//!
//! Every child is given `$LEDGELINE_CONFIG_DIR` pointing at its own scratch
//! directory, for the reason `tests/prefs.rs` established: the preferences store
//! is process-global state, `std::env::set_var` is `unsafe` in edition 2024
//! because libtest is threaded, and a test must not read — or write — the
//! developer's real `prefs.json`.
//!
//! # The two properties worth the most
//!
//! [`the_cli_and_the_screen_write_the_same_journal_byte_for_byte`] is the point
//! of the whole feature: the command line the dry-run panel *displays* is
//! executed against a fresh copy of the same fixture, and the two journals are
//! compared byte for byte. It is `import_endpoints.rs`'s "the preview is the
//! bytes" discipline extended across the process boundary.
//!
//! [`a_refused_balance_writes_nothing_at_all`] holds the CLI to the commit's
//! all-or-nothing property. A script has nobody to look at a red number, so a
//! statement balance that does not reconcile refuses the run — and the proof is
//! that the whole tree is byte-identical afterwards, not merely that the exit
//! code was non-zero.

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use common::fixtures_dir;
use http_body_util::BodyExt;
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use tower::ServiceExt;

/// The upload header carrying the dropped file's name.
const FILENAME: &str = "x-ledgeline-filename";

/// Opts in to the checks that shell out to a real `hledger`. Set by
/// `just hledger-checks`. Same variable `import_endpoints.rs` uses: these are
/// the same pipeline reached through a different door.
const IMPORT_CHECK: &str = "LEDGELINE_HLEDGER_IMPORT_CHECK";

/// Skip, loudly, unless the opt-in variable is set.
macro_rules! require_hledger {
    () => {
        if std::env::var_os(IMPORT_CHECK).is_none() {
            eprintln!("skipping: set {IMPORT_CHECK}=1 (or run `just hledger-checks`)");
            return;
        }
    };
}

/// The statement every import here reads: two rows the checking rules file
/// routes to two different accounts, in `fixtures/import/match/checking.csv.rules`'s
/// column order.
const STATEMENT: &str = "Date,Description,Withdrawal,Deposit\n\
                         03/01/2026,GROCERY STORE,40.50,\n\
                         03/03/2026,ACME PAYROLL,,1650.00\n";

/// A statement whose newest row is dated BEFORE the target journal's last
/// transaction, so importing it leaves the file out of date order — which is
/// what `--sort` is for.
const BACKDATED_STATEMENT: &str = "Date,Description,Withdrawal,Deposit\n\
                                   01/02/2026,GROCERY STORE,12.00,\n";

// ---------------------------------------------------------------------------
// The scratch tree
// ---------------------------------------------------------------------------

/// A throwaway copy of a fixture layout, plus the private config directory its
/// child processes are given.
///
/// Every test gets its own, so nothing here is order-dependent and nothing
/// outlives the test that made it.
struct Tree {
    dir: TempDir,
    config: TempDir,
}

impl Tree {
    /// `fixtures/import/layouts/split-year/`, copied so it can be written to,
    /// with the checking rules file and the statement beside the journal.
    ///
    /// The split layout on purpose: `main.journal` is include-only and the
    /// import target is `2026/2026.journal`, so every test here exercises the
    /// two-journals distinction (`--journal` vs `--root-journal`) rather than
    /// the single-file case where the two collapse and a confusion between them
    /// would not show.
    fn split_year() -> Self {
        let dir = TempDir::new().expect("temp dir");
        copy_tree(
            &fixtures_dir().join("import/layouts/split-year"),
            dir.path(),
        );
        std::fs::create_dir(dir.path().join("import")).expect("import dir");
        std::fs::copy(
            fixtures_dir().join("import/match/checking.csv.rules"),
            dir.path().join("import/bank.csv.rules"),
        )
        .expect("copy the rules fixture");
        Self::with_statement(dir, STATEMENT)
    }

    /// [`Tree::split_year`] with a different statement dropped in.
    fn with_statement(dir: TempDir, statement: &str) -> Self {
        std::fs::write(dir.path().join("statement.csv"), statement).expect("write the statement");
        // A bystander. Anything that touches this has swept up something it was
        // not asked to.
        std::fs::write(dir.path().join("notes.txt"), "do not touch me\n").expect("bystander");
        let config = TempDir::new().expect("config dir");
        Self { dir, config }
    }

    /// The same tree, carrying a statement whose dates land before the journal's
    /// last transaction.
    fn backdated() -> Self {
        let tree = Self::split_year();
        std::fs::write(tree.path("statement.csv"), BACKDATED_STATEMENT).expect("rewrite");
        tree
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    /// Every file in the tree, keyed by its relative path — the before/after
    /// comparison that proves a run's blast radius.
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        walk(self.dir.path(), self.dir.path(), &mut files);
        files
    }

    /// Run `ledgeline import` in this tree, from the journal's own directory.
    ///
    /// `current_dir` rather than `std::env::set_current_dir`: the working
    /// directory is process-global and libtest is threaded, so setting it here
    /// would corrupt every other test in this binary. Setting it on the CHILD is
    /// both safe and exactly right — the rendered command line is documented as
    /// being run from the journal's directory, so this is the documented usage.
    fn import(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ledgeline"))
            .arg("import")
            .args(args)
            .current_dir(self.dir.path())
            .env("LEDGELINE_CONFIG_DIR", self.config.path())
            .output()
            .expect("the ledgeline binary runs")
    }

    /// The arguments every ordinary run here shares.
    fn standard_args(&self) -> Vec<&'static str> {
        vec![
            "-i",
            "statement.csv",
            "-o",
            "import/bank.csv",
            "-r",
            "import/bank.csv.rules",
            "-j",
            "2026/2026.journal",
            "--root-journal",
            "main.journal",
        ]
    }
}

/// Copy a committed fixture tree into a scratch directory, subdirectories and
/// all. The fixtures are a read-only corpus; a test that writes needs its own.
fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from)
        .expect("fixture tree readable")
        .flatten()
    {
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            std::fs::create_dir_all(&destination).expect("create scratch subdir");
            copy_tree(&source, &destination);
        } else {
            std::fs::copy(&source, &destination).expect("copy fixture file");
        }
    }
}

/// Collect every regular file under `dir`, keyed relative to `root`.
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

/// The relative paths whose content differs between two tree snapshots.
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

/// An `Output`'s two streams, for an assertion message that says what happened.
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
// The GUI's own door
// ---------------------------------------------------------------------------
//
// A small oneshot harness, because the parity test's whole point is that the
// string it replays is the one a BROWSER receives. Reading it from anywhere
// closer to the renderer would prove less. It is deliberately the minimum —
// `import_endpoints.rs` owns the exhaustive HTTP coverage, and a test module
// cannot be shared across integration-test binaries.

/// Send a request through a router built over `state`, returning the JSON body.
async fn send(state: &AppState, request: Request<Body>) -> Value {
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("the router responds");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    let text = String::from_utf8_lossy(&body).into_owned();
    assert_eq!(status, StatusCode::OK, "{text}");
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

/// `POST` a JSON body.
async fn post_json(state: &AppState, uri: &str, body: Value) -> Value {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request builds");
    send(state, request).await
}

/// `POST` raw bytes to the one upload route, as the browser's drop does.
async fn post_bytes(state: &AppState, uri: &str, bytes: Vec<u8>) -> Value {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(HeaderName::from_static(FILENAME), "statement.csv")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("the request builds");
    send(state, request).await
}

// ---------------------------------------------------------------------------
// Hermetic: the command line itself
// ---------------------------------------------------------------------------

/// The subcommand exists, is documented, and names every flag a caller needs.
///
/// A `--help` assertion earns its place here because this is a *contract with
/// scripts*: a flag that silently disappears breaks somebody's cron job, and
/// nothing else in this suite would notice a rename.
#[test]
fn the_import_subcommand_documents_every_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_ledgeline"))
        .args(["import", "--help"])
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{}", transcript(&output));
    let help = stdout(&output);
    for flag in [
        "--input",
        "--output",
        "--rules",
        "--journal",
        "--root-journal",
        "--balance",
        "--balance-account",
        "--write-assertion",
        "--sort",
        "--dry-run",
        "--no-git",
    ] {
        assert!(help.contains(flag), "`{flag}` must be documented:\n{help}");
    }
    // The short forms the rendered command line uses.
    for short in ["-i,", "-o,", "-r,", "-j,"] {
        assert!(help.contains(short), "`{short}` must exist:\n{help}");
    }
}

/// Adding a subcommand must not change what the binary does WITHOUT one. The
/// old invocations are still the old invocations.
#[test]
fn the_existing_command_line_is_untouched() {
    let output = Command::new(env!("CARGO_BIN_EXE_ledgeline"))
        .arg("--help")
        .output()
        .expect("the binary runs");
    let help = stdout(&output);
    assert!(
        help.contains("[JOURNAL]"),
        "the positional journal survives:\n{help}"
    );
    for flag in ["--server", "--host", "--port", "--allow-origin"] {
        assert!(help.contains(flag), "`{flag}` must survive:\n{help}");
    }
}

/// `--balance` and `--balance-account` are useless apart, so clap refuses one
/// without the other rather than the runner discovering it later. Validation at
/// the boundary, which is where a scripted caller can act on it.
#[test]
fn a_balance_without_its_account_is_refused_by_the_parser() {
    let tree = Tree::split_year();
    let mut args = tree.standard_args();
    args.extend(["--balance", "100.00"]);
    let output = tree.import(&args);
    assert!(!output.status.success(), "{}", transcript(&output));
    assert!(
        stderr(&output).contains("--balance-account"),
        "the refusal must name what is missing:\n{}",
        stderr(&output)
    );
}

/// A rules file that is not beside this journal cannot be named, whichever door
/// the request arrives at. The refusal lists what IS available, because "no"
/// with no alternatives is a bad error message for a command line.
#[test]
fn a_rules_file_outside_the_journals_tree_is_refused() {
    let tree = Tree::split_year();
    let elsewhere = TempDir::new().expect("temp dir");
    let stray = elsewhere.path().join("stray.csv.rules");
    std::fs::write(&stray, "skip 1\nfields date, description, amount\n").expect("write");

    let before = tree.snapshot();
    let output = tree.import(&[
        "-i",
        "statement.csv",
        "-o",
        "import/bank.csv",
        "-r",
        stray.to_str().expect("utf-8"),
        "-j",
        "2026/2026.journal",
        "--root-journal",
        "main.journal",
    ]);
    assert!(!output.status.success(), "{}", transcript(&output));
    assert!(
        stderr(&output).contains("not a rules file beside this journal"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "a refused run writes nothing"
    );
}

/// A CSV destination outside the journal's own directory is refused for the same
/// reason `resolve_destination` refuses one: an import may not write there.
#[test]
fn a_destination_outside_the_journals_directory_is_refused() {
    let tree = Tree::split_year();
    let elsewhere = TempDir::new().expect("temp dir");
    let escape = elsewhere.path().join("bank.csv");

    let before = tree.snapshot();
    let output = tree.import(&[
        "-i",
        "statement.csv",
        "-o",
        escape.to_str().expect("utf-8"),
        "-r",
        "import/bank.csv.rules",
        "-j",
        "2026/2026.journal",
        "--root-journal",
        "main.journal",
    ]);
    assert!(!output.status.success(), "{}", transcript(&output));
    assert!(
        stderr(&output).contains("not inside this journal's own directory"),
        "{}",
        stderr(&output)
    );
    assert!(!escape.exists(), "nothing was written outside the tree");
    assert_eq!(changed(&before, &tree.snapshot()), Vec::<String>::new());
}

/// A journal file the parse never read is not an import target, even when it
/// sits in the right directory. Same rule as `resolve_journal`'s membership
/// test, reached from the command line.
#[test]
fn a_journal_this_tree_does_not_include_is_refused() {
    let tree = Tree::split_year();
    std::fs::write(tree.path("orphan.journal"), "; not included by anything\n").expect("write");

    let output = tree.import(&[
        "-i",
        "statement.csv",
        "-o",
        "import/bank.csv",
        "-r",
        "import/bank.csv.rules",
        "-j",
        "orphan.journal",
        "--root-journal",
        "main.journal",
    ]);
    assert!(!output.status.success(), "{}", transcript(&output));
    let said = stderr(&output);
    assert!(said.contains("is not part of this journal"), "{said}");
    assert!(
        said.contains("2026/2026.journal"),
        "the refusal lists the targets that ARE available:\n{said}"
    );
}

/// An input file that is not there is a plain, early refusal — before any
/// journal is parsed and before hledger is looked for.
#[test]
fn a_missing_input_file_is_refused() {
    let tree = Tree::split_year();
    let mut args = tree.standard_args();
    args[1] = "no-such-statement.csv";
    let output = tree.import(&args);
    assert!(!output.status.success(), "{}", transcript(&output));
    assert!(
        stderr(&output).contains("could not be read"),
        "{}",
        stderr(&output)
    );
}

// ---------------------------------------------------------------------------
// hledger-backed: what a run actually does
// ---------------------------------------------------------------------------

/// The ordinary case: a statement goes in, the CSV is kept, the transactions are
/// appended, and the process exits 0.
#[test]
fn a_committing_run_writes_the_csv_and_appends_the_journal() {
    require_hledger!();
    let tree = Tree::split_year();
    let before = std::fs::read_to_string(tree.path("2026/2026.journal")).expect("target");
    let snapshot = tree.snapshot();

    let output = tree.import(&[&tree.standard_args()[..], &["--no-git"]].concat());
    assert!(output.status.success(), "{}", transcript(&output));

    // The CSV landed where `--output` said.
    let csv = std::fs::read_to_string(tree.path("import/bank.csv")).expect("the CSV was kept");
    assert!(csv.contains("GROCERY STORE"), "{csv}");

    // The journal GREW; it was not rewritten.
    let after = std::fs::read_to_string(tree.path("2026/2026.journal")).expect("target");
    assert!(
        after.starts_with(&before),
        "the import must append, never rewrite"
    );
    assert!(after.contains("ACME PAYROLL"), "{after}");
    assert!(
        after.contains("expenses:groceries"),
        "the rules file's routing was applied:\n{after}"
    );

    // Blast radius: the CSV, the journal, and hledger's own dedup marker. The
    // bystander and every other fixture file are untouched.
    let touched = changed(&snapshot, &tree.snapshot());
    assert_eq!(
        touched,
        vec![
            "2026/2026.journal".to_string(),
            "import/.latest.bank.csv".to_string(),
            "import/bank.csv".to_string(),
        ],
        "a commit writes exactly three files"
    );

    // stdout says what happened; stderr carries the re-runnable command line.
    assert!(
        stdout(&output).contains("appended 2 transactions"),
        "{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("ledgeline import -i statement.csv"),
        "the run echoes its own command line:\n{}",
        stderr(&output)
    );
}

/// `--dry-run` reports and writes NOTHING — not the CSV, not the journal, not
/// even hledger's dedup marker. Proved over the whole tree rather than over the
/// two files a reader would think to check.
#[test]
fn a_dry_run_writes_nothing_at_all() {
    require_hledger!();
    let tree = Tree::split_year();
    let before = tree.snapshot();

    let output = tree.import(&[&tree.standard_args()[..], &["--dry-run", "--no-git"]].concat());
    assert!(output.status.success(), "{}", transcript(&output));
    assert!(
        stdout(&output).contains("Nothing was written"),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("2 transaction"),
        "a preview still reports the count:\n{}",
        stdout(&output)
    );

    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "--dry-run must leave the tree byte-identical"
    );
    // Said again the direct way, because this is the property the flag exists
    // for and a snapshot comparison is easy to misread.
    assert!(!tree.path("import/bank.csv").exists());
    assert!(!tree.path("import/.latest.bank.csv").exists());
}

/// A back-dated import leaves the journal out of date order, and `--sort` puts
/// it back — in the same run, without a second command.
#[test]
fn sort_restores_date_order_after_a_backdated_import() {
    require_hledger!();
    let tree = Tree::backdated();

    // Without --sort the file is left out of order, and the run SAYS so rather
    // than leaving the user to find out.
    let unsorted = tree.import(&[&tree.standard_args()[..], &["--no-git"]].concat());
    assert!(unsorted.status.success(), "{}", transcript(&unsorted));
    assert!(
        stdout(&unsorted).contains("no longer in date order"),
        "{}",
        stdout(&unsorted)
    );

    // A second, sorting run over a fresh copy of the same tree.
    let sorted_tree = Tree::backdated();
    let sorted =
        sorted_tree.import(&[&sorted_tree.standard_args()[..], &["--sort", "--no-git"]].concat());
    assert!(sorted.status.success(), "{}", transcript(&sorted));
    assert!(stdout(&sorted).contains("re-sorted"), "{}", stdout(&sorted));

    let text = std::fs::read_to_string(sorted_tree.path("2026/2026.journal")).expect("target");
    let dates: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("2026-"))
        .map(|line| &line[..10])
        .collect();
    let mut expected = dates.clone();
    expected.sort_unstable();
    assert_eq!(dates, expected, "the journal is in date order:\n{text}");

    // The same transactions are still there — a sort moves, it does not drop.
    assert!(text.contains("GROCERY STORE"), "{text}");
}

/// A statement balance that does not reconcile refuses the WHOLE run, before
/// anything is written.
///
/// The commit route's all-or-nothing property (`docs/imports.md` § "A failing
/// assertion refuses the commit *before* the import is applied"), held to on the
/// command line — where it matters more, because there is no person watching a
/// number turn red and deciding not to press the button.
#[test]
fn a_refused_balance_writes_nothing_at_all() {
    require_hledger!();
    let tree = Tree::split_year();
    let before = tree.snapshot();

    let output = tree.import(
        &[
            &tree.standard_args()[..],
            &[
                "--balance",
                "999999.00",
                "--balance-account",
                "assets:bank:checking",
                "--no-git",
            ],
        ]
        .concat(),
    );
    assert!(
        !output.status.success(),
        "a balance that does not reconcile must fail:\n{}",
        transcript(&output)
    );
    assert!(
        stderr(&output).contains("does not match"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "a refused balance leaves the tree byte-identical"
    );

    // …and the SAME run with the balance the journal actually reconciles to
    // succeeds, so the refusal above is about the number and not about the flag
    // being unusable.
    let computed = balance_after_import(&tree);
    let good = tree.import(
        &[
            &tree.standard_args()[..],
            &[
                "--balance",
                computed.as_str(),
                "--balance-account",
                "assets:bank:checking",
                "--no-git",
            ],
        ]
        .concat(),
    );
    assert!(good.status.success(), "{}", transcript(&good));
    assert!(
        tree.path("import/bank.csv").exists(),
        "the reconciling run did write"
    );
}

/// What `assets:bank:checking` comes to once this statement is imported,
/// measured by doing a `--dry-run` first and reading the number back out of the
/// refusal. Keeps the fixture's arithmetic out of this file, where it would go
/// stale silently.
fn balance_after_import(tree: &Tree) -> String {
    let probe = tree.import(
        &[
            &tree.standard_args()[..],
            &[
                "--balance",
                "0.00",
                "--balance-account",
                "assets:bank:checking",
                "--dry-run",
                "--no-git",
            ],
        ]
        .concat(),
    );
    let said = stderr(&probe);
    let after = said
        .split_once("journal's ")
        .expect("the refusal names the computed balance")
        .1;
    after
        .split_once(',')
        .expect("the computed balance is followed by the difference")
        .0
        .trim()
        .trim_start_matches('$')
        .to_string()
}

/// **The point of the feature.** The command line the dry-run panel *displays*
/// is executed, as a real subprocess, against a fresh copy of the same fixture —
/// and the two journals are compared byte for byte.
///
/// The GUI half goes through the HTTP routes, because `cliCommand` is a field on
/// the dry-run RESPONSE and the panel's copy button copies exactly that string;
/// taking it from anywhere else would be testing a different string from the one
/// a user gets. The CLI half is given nothing but that string, split the way a
/// shell would split it.
///
/// This is `import_endpoints.rs`'s `the_preview_is_the_bytes_that_are_appended`
/// carried across the process boundary: there, the preview and the write cannot
/// disagree because they are the same string; here, the screen and the terminal
/// cannot disagree because the string the screen shows is the one the terminal
/// is given. A weaker test — comparing our own idea of the flags against our own
/// renderer — would pass on the day the two drifted together.
#[tokio::test]
async fn the_cli_and_the_screen_write_the_same_journal_byte_for_byte() {
    require_hledger!();

    // ---- the GUI half: stage, dry-run, commit, exactly as the screen does ----
    let gui = Tree::split_year();
    let state = AppState::from_journal_path(gui.path("main.journal")).expect("the journal opens");

    let staged = post_bytes(
        &state,
        "/api/import/stage",
        std::fs::read(gui.path("statement.csv")).expect("statement"),
    )
    .await;
    let stage_id = staged["stageId"].as_str().expect("a stageId").to_string();
    let body = json!({
        "stageId": stage_id,
        "rulesId": "import/bank.csv.rules",
        "csvPath": "import/bank.csv",
        "journalId": "2026/2026.journal",
    });

    let preview = post_json(&state, "/api/import/dry-run", body.clone()).await;
    assert_eq!(preview["ok"], json!(true), "{preview}");
    // The literal string the panel puts in its copy affordance.
    let command = preview["cliCommand"]
        .as_str()
        .expect("a successful preview carries the command line")
        .to_string();

    let mut commit_body = body;
    commit_body["writeAssertion"] = json!(false);
    let committed = post_json(&state, "/api/import/commit", commit_body).await;
    assert_eq!(
        committed["imported"],
        json!(2),
        "the GUI half imported: {committed}"
    );

    // ---- the CLI half: a FRESH copy, driven by the advertised string ----
    let cli = Tree::split_year();
    assert!(
        !command.contains('\''),
        "this fixture's handles need no quoting, so a plain split is faithful: {command}"
    );
    let args: Vec<&str> = command.split_whitespace().collect();
    assert_eq!(
        &args[..2],
        ["ledgeline", "import"],
        "the panel advertises a `ledgeline import` line: {command}"
    );
    let replayed = cli.import(&args[2..]);
    assert!(replayed.status.success(), "{}", transcript(&replayed));

    assert_eq!(
        std::fs::read(gui.path("2026/2026.journal")).expect("gui journal"),
        std::fs::read(cli.path("2026/2026.journal")).expect("cli journal"),
        "the screen's import and the command line's import must produce the same bytes"
    );
    assert_eq!(
        std::fs::read(gui.path("import/bank.csv")).expect("gui csv"),
        std::fs::read(cli.path("import/bank.csv")).expect("cli csv"),
        "…and the same CSV"
    );
}

/// The advertised command names the SECOND journal whenever there is one.
///
/// A split layout is the case where `--journal` and `--root-journal` differ, and
/// a command line that omitted the root would reconcile balances against a
/// fragment — the exact confusion `Plan`'s own docs exist to prevent. The
/// single-file layout is asserted alongside it, because "always print it" would
/// pass the first assertion and be wrong.
#[tokio::test]
async fn the_advertised_command_names_the_root_journal_only_when_it_differs() {
    require_hledger!();

    let split = Tree::split_year();
    let state = AppState::from_journal_path(split.path("main.journal")).expect("opens");
    let command = advertised_command(&state, &split, "2026/2026.journal").await;
    assert!(
        command.contains("-j 2026/2026.journal --root-journal main.journal"),
        "a split layout must name both journals: {command}"
    );

    // The same tree, importing into the root itself: the two journals coincide,
    // so there is nothing to say.
    let single = Tree::split_year();
    let state = AppState::from_journal_path(single.path("main.journal")).expect("opens");
    let command = advertised_command(&state, &single, "main.journal").await;
    assert!(
        !command.contains("--root-journal"),
        "importing into the root itself needs no second journal: {command}"
    );
}

/// The `cliCommand` a dry-run advertises for `journal_id`.
async fn advertised_command(state: &AppState, tree: &Tree, journal_id: &str) -> String {
    let staged = post_bytes(
        state,
        "/api/import/stage",
        std::fs::read(tree.path("statement.csv")).expect("statement"),
    )
    .await;
    let preview = post_json(
        state,
        "/api/import/dry-run",
        json!({
            "stageId": staged["stageId"].as_str().expect("a stageId"),
            "rulesId": "import/bank.csv.rules",
            "csvPath": "import/bank.csv",
            "journalId": journal_id,
        }),
    )
    .await;
    assert_eq!(preview["ok"], json!(true), "{preview}");
    preview["cliCommand"]
        .as_str()
        .expect("a command line")
        .to_string()
}

/// Re-running the same statement imports nothing the second time, because the
/// CLI writes the CSV to its real destination and lets hledger keep its
/// `.latest` marker beside it — the same dedup the screen gets.
///
/// Worth its own test because it is the property a *scripted* import needs most:
/// a cron job that re-downloads an overlapping statement every night must not
/// duplicate transactions.
#[test]
fn re_importing_the_same_statement_adds_nothing() {
    require_hledger!();
    let tree = Tree::split_year();

    let first = tree.import(&[&tree.standard_args()[..], &["--no-git"]].concat());
    assert!(first.status.success(), "{}", transcript(&first));
    let after_first = std::fs::read(tree.path("2026/2026.journal")).expect("target");

    let second = tree.import(&[&tree.standard_args()[..], &["--no-git"]].concat());
    assert!(second.status.success(), "{}", transcript(&second));
    assert_eq!(
        after_first,
        std::fs::read(tree.path("2026/2026.journal")).expect("target"),
        "a second import of the same statement must add nothing"
    );
    assert!(
        stdout(&second).contains("appended 0 transactions"),
        "{}",
        stdout(&second)
    );
}
