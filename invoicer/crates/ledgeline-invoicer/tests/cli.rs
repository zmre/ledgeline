//! CLI integration tests (invoicer PLAN 1c): the built binary, run on a copy
//! of `invoicer/fixtures/{accrual,cash}` under `target/`, never on the
//! fixtures themselves. JSON output is snapshotted in `tests/snapshots/`.
//!
//! Every run starts from an empty environment (`env_clear`): no
//! `$LEDGER_FILE`, no `$LEDGELINE_FIXTURE` unless a test sets it, and `HOME`
//! and `$LEDGELINE_CONFIG_DIR` inside the test's own folder, so the binary
//! can never reach a real journal or the developer's recents.
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p ledgeline-invoicer --test cli   # re-record
//! ```

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_ledgeline-invoicer");
/// The fixtures' clock (fixtures/README.md).
const TODAY: &str = "2026-11-17";

/// One fixture copy and an isolated home for the binary.
struct Books {
    dir: tempfile::TempDir,
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("mkdir");
    for entry in fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

impl Books {
    fn of(fixture: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("invoicer-cli-{fixture}-"))
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .expect("tempdir");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(fixture);
        copy_dir(&source, &dir.path().join("books"));
        fs::create_dir_all(dir.path().join("home")).expect("home");
        fs::create_dir_all(dir.path().join("config")).expect("config");
        Self { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("books")
    }

    fn journal(&self) -> PathBuf {
        self.root().join("books.journal")
    }

    fn file(&self, relative: &str) -> PathBuf {
        self.root().join(relative)
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.file(relative)).expect("reads")
    }

    /// The binary with an empty environment and this test's home.
    fn bare(&self) -> Command {
        let mut command = Command::new(BIN);
        command
            .env_clear()
            .env("HOME", self.dir.path().join("home"))
            .env("LEDGELINE_CONFIG_DIR", self.dir.path().join("config"))
            .current_dir(self.dir.path());
        command
    }

    /// `ledgeline-invoicer -f <copy> --today 2026-11-17 <args>`.
    fn run(&self, args: &[&str]) -> Output {
        self.bare()
            .arg("-f")
            .arg(self.journal())
            .args(["--today", TODAY])
            .args(args)
            .output()
            .expect("runs")
    }

    /// `run` with `--json`, parsed, asserting the exit code.
    fn json(&self, args: &[&str], code: i32) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.run(&all);
        assert_eq!(
            out.status.code(),
            Some(code),
            "{args:?}: stdout {} stderr {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "{args:?}: not JSON ({e}): {}",
                String::from_utf8_lossy(&out.stdout)
            )
        })
    }

    /// `run`, asserting success, as text.
    fn text(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8")
    }

    /// `value` as committed: pretty JSON with this copy's folder written
    /// `<BOOKS>`.
    fn normalized(&self, value: &Value) -> String {
        let root = fs::canonicalize(self.root()).expect("canonical");
        let text = serde_json::to_string_pretty(value).expect("json");
        format!("{}\n", text.replace(&root.display().to_string(), "<BOOKS>"))
    }
}

/// Compare `actual` with `tests/snapshots/<name>`, or write it there when
/// `UPDATE_SNAPSHOTS` is set.
fn snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(name);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        fs::create_dir_all(path.parent().expect("dir")).expect("mkdir");
        fs::write(&path, actual).expect("writes");
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no snapshot {}; run with UPDATE_SNAPSHOTS=1 to record it",
            path.display()
        )
    });
    assert!(
        expected == actual,
        "{name} differs from its snapshot (UPDATE_SNAPSHOTS=1 re-records):\n--- expected\n{expected}\n--- actual\n{actual}"
    );
}

fn snapshot_json(books: &Books, name: &str, args: &[&str]) -> Value {
    let value = books.json(args, 0);
    snapshot(name, &books.normalized(&value));
    value
}

#[test]
fn list_show_and_customers_json_on_both_fixtures() {
    let accrual = Books::of("accrual");
    let list = snapshot_json(&accrual, "accrual-list.json", &["list"]);
    let numbers: Vec<&str> = list
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|d| d["number"].as_str())
        .collect();
    assert_eq!(
        numbers,
        ["108", "105", "107", "104", "103", "102", "101", "106"]
    );
    snapshot_json(&accrual, "accrual-show-105.json", &["show", "105"]);
    snapshot_json(&accrual, "accrual-show-108.json", &["show", "108"]);
    snapshot_json(&accrual, "accrual-customers.json", &["customers", "list"]);
    snapshot_json(
        &accrual,
        "accrual-customer-x.json",
        &["customers", "show", "Customer X"],
    );
    snapshot_json(&accrual, "accrual-config.json", &["config"]);

    let cash = Books::of("cash");
    snapshot_json(&cash, "cash-list.json", &["list"]);
    snapshot_json(&cash, "cash-show-207.json", &["show", "207"]);
    snapshot_json(&cash, "cash-config.json", &["config"]);
}

#[test]
fn list_filters() {
    let books = Books::of("accrual");
    let numbers = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["list"];
        all.extend_from_slice(args);
        books
            .json(&all, 0)
            .as_array()
            .expect("array")
            .iter()
            .map(|d| d["number"].as_str().unwrap_or("").to_string())
            .collect()
    };
    assert_eq!(numbers(&["--status", "draft"]), ["108"]);
    assert_eq!(numbers(&["--open"]), ["105", "107", "106"]);
    assert_eq!(numbers(&["--overdue"]), ["107", "106"]);
    assert_eq!(numbers(&["--customer", "contoso"]), ["107", "101"]);
    assert_eq!(numbers(&["--type", "estimate"]), Vec::<String>::new());
    let bad = books.json(&["list", "--status", "paid"], 2);
    assert!(bad["error"].as_str().expect("message").contains("--status"));
}

#[test]
fn human_output_is_aligned() {
    let books = Books::of("accrual");
    snapshot("accrual-list.txt", &books.text(&["list"]));
    snapshot("accrual-show-105.txt", &books.text(&["show", "105"]));
    snapshot("accrual-customers.txt", &books.text(&["customers", "list"]));
    assert_eq!(books.text(&["next-number"]), "2005\n");
    assert_eq!(
        books.text(&["check"]),
        "No problems in 8 documents and 6 customers.\n"
    );
}

#[test]
fn next_number_counts_the_books() {
    // Plan 45's invoices 2001–2004 are booked without files; a vendor's
    // bill numbered PA-9 is in its own namespace.
    assert_eq!(
        Books::of("accrual").json(&["next-number"], 0)["next"],
        "2005"
    );
    assert_eq!(Books::of("cash").json(&["next-number"], 0)["next"], "208");
}

#[test]
fn check_exits_1_on_errors_and_reports_them() {
    let books = Books::of("accrual");
    let clean = books.json(&["check"], 0);
    assert_eq!(clean["errors"], 0);
    assert_eq!(clean["documents"], 8);

    let wingtip = "invoices/2026-08-01 Wingtip Invoice 106.md";
    let text = books.read(wingtip).replace("| $1,200", "| $1,100");
    fs::write(books.file(wingtip), text).expect("writes");
    let found = books.json(&["check"], 1);
    snapshot("accrual-check-arithmetic.json", &books.normalized(&found));
    assert_eq!(found["problems"][0]["code"], "arithmetic");
    assert_eq!(found["problems"][0]["fix"]["edit"]["kind"], "recalc");

    let out = books.run(&["check"]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(text.contains(&format!("{wingtip}:")), "{text}");
    assert!(
        text.ends_with("1 error in 8 documents and 6 customers.\n"),
        "{text}"
    );
}

#[test]
fn check_strict_fails_on_warnings() {
    let books = Books::of("cash");
    let path = "invoices/2026-11-05 Wingtip Invoice 203.md";
    let text = books.read(path);
    let without: String = text
        .lines()
        .filter(|l| !l.starts_with("invoicee_email:"))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(books.file(path), without).expect("writes");
    let report = books.json(&["check"], 0);
    assert_eq!(report["warnings"], 1);
    books.json(&["check", "--strict"], 1);
}

#[test]
fn new_writes_a_draft_and_never_the_journal() {
    let books = Books::of("accrual");
    let journal = books.read("books.journal");
    let args = [
        "new",
        "--customer",
        "Customer X",
        "--date",
        "2026-11-20",
        "--reference",
        "PO #PY75334",
        "--from-catalog",
        "ARCH:10",
        "--row",
        "Discount||-10%",
        "--row",
        "Subtotal",
        "--row",
        "Sales tax||8.25%|liabilities:sales-tax",
        "--field",
        "support_tier=gold",
    ];
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let preview = snapshot_json(&books, "accrual-new-dry-run.json", &dry);
    assert_eq!(preview["written"], false);
    let path = preview["path"].as_str().expect("path").to_string();
    assert!(!books.file(&path).exists());

    let made = books.json(&args, 0);
    assert_eq!(made["number"], "2005");
    assert_eq!(made["total"], "$7,794.00");
    assert_eq!(made["written"], true);
    assert_eq!(made["problems"], serde_json::json!([]));
    assert!(made.get("text").is_none());
    assert_eq!(books.read(&path), preview["text"].as_str().expect("text"));
    assert_eq!(
        books.read("books.journal"),
        journal,
        "drafts never touch the books"
    );

    let shown = books.json(&["show", "2005"], 0);
    assert_eq!(shown["status"], "draft");
    assert_eq!(shown["frontmatter"]["support_tier"], "gold");
    let kinds: Vec<&str> = shown["table"]["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|r| r["kind"].as_str())
        .collect();
    assert_eq!(
        kinds,
        ["item", "adjustment", "total", "adjustment", "total"]
    );

    // The next one takes the next number; a taken number is refused.
    assert_eq!(books.json(&["next-number"], 0)["next"], "2006");
    let taken = books.json(
        &[
            "new",
            "--customer",
            "Contoso",
            "--number",
            "2005",
            "--row",
            "Work|1|$1",
        ],
        2,
    );
    assert!(taken["error"].as_str().expect("message").contains("2005"));
    // Customer X's portal needs a reference: the draft is made, with the
    // error reported.
    let unreferenced = books.json(
        &[
            "new",
            "--customer",
            "Customer X",
            "--date",
            "2026-11-21",
            "--row",
            "Work|1|$100",
        ],
        0,
    );
    assert_eq!(unreferenced["problems"][0]["code"], "reference-required");
    let unknown = books.json(&["new", "--customer", "Nobody", "--row", "Work|1|$1"], 2);
    assert!(
        unknown["error"]
            .as_str()
            .expect("message")
            .contains("Nobody")
    );
    assert_eq!(books.read("books.journal"), journal);
}

#[test]
fn set_edits_one_key_and_guards_the_lifecycle() {
    let books = Books::of("accrual");
    let path = "invoices/2026-11-17 Customer X Invoice 108.md";
    let before = books.read(path);
    let done = books.json(&["set", "108", "reference=PO #PY99999", "terms=45"], 0);
    assert_eq!(done["written"], true);
    assert_eq!(done["changed"], serde_json::json!(["reference", "terms"]));
    let after = books.read(path);
    let changed: Vec<(&str, &str)> = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(
        changed,
        [
            ("terms: 30", "terms: 45"),
            ("reference: \"PO #PY75334\"", "reference: \"PO #PY99999\""),
        ]
    );
    assert_eq!(before.lines().count(), after.lines().count());

    // By file path too, relative to the working directory; --unset removes.
    let local = books.file(path);
    let out = books.text(&[
        "set",
        local.to_str().expect("utf-8"),
        "--unset",
        "support_tier",
    ]);
    assert_eq!(out, format!("Updated {path}: support_tier\n"));
    assert!(!books.read(path).contains("support_tier"));
    assert_eq!(
        books.text(&["set", "108", "terms=45"]),
        format!("No change: {path}\n")
    );

    let unchanged = books.read(path);
    for (args, needle) in [
        (vec!["set", "108", "status=sent"], "status"),
        (vec!["set", "108", "paid=no"], "paid"),
        (vec!["set", "105", "invoice_num=109"], "invoice_num"),
        (vec!["set", "108", "invoice_num=105"], "105"),
        (vec!["set", "108"], "nothing to set"),
        (vec!["set", "999", "a=b"], "999"),
    ] {
        let refused = books.json(&args, 2);
        assert!(
            refused["error"].as_str().expect("message").contains(needle),
            "{args:?}: {refused}"
        );
    }
    assert_eq!(books.read(path), unchanged);
}

#[test]
fn recalc_fixes_drafts_and_names_the_rest() {
    let books = Books::of("accrual");
    let journal = books.read("books.journal");
    let draft = "invoices/2026-11-17 Customer X Invoice 108.md";
    let sent = "invoices/2026-08-01 Wingtip Invoice 106.md";
    let broken = books
        .read(draft)
        .replace("| $1,600.00  |", "| $1,500     |");
    fs::write(books.file(draft), &broken).expect("writes");
    let wrong_sent = books.read(sent).replace("| $1,200", "| $1,100");
    fs::write(books.file(sent), &wrong_sent).expect("writes");

    let dry = books.json(&["recalc", "--dry-run"], 0);
    snapshot("accrual-recalc-dry-run.json", &books.normalized(&dry));
    assert_eq!(books.read(draft), broken);

    let done = books.json(&["recalc"], 0);
    assert_eq!(done["changed"][0]["path"], draft);
    assert_eq!(done["changed"][0]["written"], true);
    assert_eq!(done["skipped"][0]["path"], sent);
    // The amount is rewritten; the engine re-aligns the table as it writes.
    let fixed = books.read(draft);
    assert!(
        fixed.contains("| $1,600.00 ") && !fixed.contains("$1,500"),
        "{fixed}"
    );
    assert_eq!(books.read(sent), wrong_sent);

    let named = books.json(&["recalc", "106"], 0);
    assert_eq!(named["changed"][0]["path"], sent);
    assert_eq!(books.json(&["check"], 0)["errors"], 0);
    assert_eq!(
        books.text(&["recalc"]),
        "Every amount already agrees with its arithmetic.\n"
    );
    assert_eq!(books.read("books.journal"), journal);
}

#[test]
fn the_journal_comes_from_the_argument_then_the_env_then_recents() {
    let books = Books::of("cash");
    let journal = books.journal();

    // The positional journal, before the command.
    let out = books
        .bare()
        .arg(&journal)
        .args(["--json", "next-number"])
        .output()
        .expect("runs");
    assert!(out.status.success());

    // Nothing to go on: a clear error, exit 2, and no fallback to
    // $LEDGER_FILE.
    let out = books
        .bare()
        .env("LEDGER_FILE", &journal)
        .args(["--json", "next-number"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(
        error["error"]
            .as_str()
            .expect("message")
            .contains("no journal")
    );

    // $LEDGELINE_FIXTURE, with a note on stderr.
    let out = books
        .bare()
        .env("LEDGELINE_FIXTURE", &journal)
        .args(["--json", "next-number"])
        .output()
        .expect("runs");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("LEDGELINE_FIXTURE"));

    // The journal opened last (the recents Ledgeline shares).
    let dirs = ledgeline_shell::AppDirs::under(books.dir.path());
    ledgeline_shell::recents::record(&dirs, &journal);
    let out = books
        .bare()
        .env("LEDGELINE_CONFIG_DIR", books.dir.path().join("config"))
        .args(["--json", "next-number"])
        .output()
        .expect("runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let next: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(next["next"], "208");
    assert!(String::from_utf8_lossy(&out.stderr).contains("opened last"));
}

#[test]
fn bad_arguments_exit_2() {
    let books = Books::of("cash");
    assert_eq!(books.run(&["show"]).status.code(), Some(2));
    let bad_today = books
        .bare()
        .arg("-f")
        .arg(books.journal())
        .args(["--today", "tomorrow", "list"])
        .output()
        .expect("runs");
    assert_eq!(bad_today.status.code(), Some(2));
    let missing = books.json(&["show", "999"], 2);
    assert!(missing["error"].as_str().expect("message").contains("999"));
    let out = books.bare().arg("--json").output().expect("runs");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn version_and_help_name_the_app() {
    let run = |arg: &str| -> String {
        let output = Command::new(BIN)
            .env_clear()
            .arg(arg)
            .output()
            .expect("the ledgeline-invoicer binary runs");
        assert!(output.status.success(), "`{arg}` exits 0: {output:?}");
        String::from_utf8(output.stdout).expect("stdout is UTF-8")
    };
    assert_eq!(
        run("--version"),
        format!("ledgeline-invoicer {}\n", env!("CARGO_PKG_VERSION"))
    );
    let help = run("--help");
    assert!(help.starts_with("Ledgeline Invoicer"), "{help}");
    assert!(help.contains("Usage: ledgeline-invoicer"), "{help}");
    for command in [
        "new",
        "list",
        "show",
        "set",
        "recalc",
        "check",
        "next-number",
        "config",
        "customers",
    ] {
        assert!(help.contains(&format!("  {command}")), "{command}: {help}");
    }
}
