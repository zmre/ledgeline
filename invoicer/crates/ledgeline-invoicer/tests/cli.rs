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

    /// `value` as committed: pretty JSON, keys sorted, with this copy's
    /// folder written `<BOOKS>`. Sorted here because the build's
    /// serde_json may keep insertion order (`preserve_order`, which MBR's
    /// Tera turns on in the app build) or sort, and a JSON object's key
    /// order is no part of the output's meaning.
    fn normalized(&self, value: &Value) -> String {
        let root = fs::canonicalize(self.root()).expect("canonical");
        let text = serde_json::to_string_pretty(&sorted(value)).expect("json");
        format!("{}\n", text.replace(&root.display().to_string(), "<BOOKS>"))
    }
}

/// `value` with every object's keys in sorted order, whatever the map type.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), sorted(&map[k])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
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

/// `--today` alone (no filter) still decides each row's `overdue`, in both
/// argument orders (P4).
#[test]
fn list_json_honours_today_without_other_flags() {
    let overdue = |books: &Books, args: &[&str]| -> Vec<String> {
        let out = books
            .bare()
            .arg("-f")
            .arg(books.journal())
            .args(args)
            .output()
            .expect("runs");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: Value = serde_json::from_slice(&out.stdout).expect("json");
        value
            .as_array()
            .expect("array")
            .iter()
            .filter(|d| d["overdue"] == Value::Bool(true))
            .map(|d| d["number"].as_str().unwrap_or("").to_string())
            .collect()
    };
    let accrual = Books::of("accrual");
    assert_eq!(
        overdue(&accrual, &["--today", TODAY, "list", "--json"]),
        ["107", "106"]
    );
    for args in [
        ["--today", "2030-01-01", "list", "--json"],
        ["list", "--json", "--today", "2030-01-01"],
        ["--json", "list", "--today", "2030-01-01"],
    ] {
        assert_eq!(overdue(&accrual, &args), ["105", "107", "106"], "{args:?}");
    }
    let cash = Books::of("cash");
    assert_eq!(
        overdue(&cash, &["--today", TODAY, "list", "--json"]),
        Vec::<String>::new()
    );
    assert_eq!(
        overdue(&cash, &["list", "--json", "--today", "2030-01-01"]),
        ["207", "203", "202"]
    );
}

#[test]
fn human_output_is_aligned() {
    let books = Books::of("accrual");
    snapshot("accrual-list.txt", &books.text(&["list"]));
    snapshot("accrual-show-105.txt", &books.text(&["show", "105"]));
    snapshot("accrual-customers.txt", &books.text(&["customers", "list"]));
    assert_eq!(books.text(&["next-number"]), "2005\n");
    // Plan 45's invoices 2001–2004 are booked without files: warnings.
    let check = books.text(&["check"]);
    assert_eq!(
        check.lines().filter(|l| l.contains("has no file")).count(),
        4
    );
    assert!(
        check.ends_with("4 warnings in 8 documents and 6 customers.\n"),
        "{check}"
    );
    assert_eq!(
        Books::of("cash").text(&["check"]),
        "No problems in 7 documents and 6 customers.\n"
    );
}

#[test]
fn next_number_counts_the_books() {
    // Plan 45's invoices 2001–2004 are booked without files; a vendor's
    // bill numbered PA-9 is in its own namespace.
    let accrual = Books::of("accrual").json(&["next-number"], 0);
    assert_eq!(accrual["next"], "2005");
    // What it follows: 2004, booked in the journal with no file (H19).
    assert_eq!(accrual["after"], "2004");
    assert_eq!(accrual["source"], "books");
    assert_eq!(Books::of("cash").json(&["next-number"], 0)["next"], "208");
}

#[test]
fn check_exits_1_on_errors_and_reports_them() {
    let books = Books::of("accrual");
    let clean = books.json(&["check"], 0);
    assert_eq!(clean["errors"], 0);
    assert_eq!(clean["warnings"], 4, "plan 45's orphans: {clean}");
    assert_eq!(clean["documents"], 8);

    let wingtip = "invoices/2026-08-01 Wingtip Invoice 106.md";
    let text = books.read(wingtip).replace("| $1,200", "| $1,100");
    fs::write(books.file(wingtip), text).expect("writes");
    let found = books.json(&["check"], 1);
    snapshot("accrual-check-arithmetic.json", &books.normalized(&found));
    let arithmetic = found["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .find(|p| p["code"] == "arithmetic")
        .expect("the arithmetic error");
    assert_eq!(arithmetic["file"], wingtip);
    assert_eq!(arithmetic["fix"]["edit"]["kind"], "recalc");

    let out = books.run(&["check"]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(text.contains(&format!("{wingtip}:")), "{text}");
    assert!(
        text.ends_with("1 error and 4 warnings in 8 documents and 6 customers.\n"),
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

/// The shipped starter's text (`templates/` beside this crate).
fn starter(file: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("templates")
            .join(file),
    )
    .expect("a starter")
}

#[test]
fn new_copies_a_starter_template_on_first_use_and_never_overwrites() {
    let books = Books::of("accrual");
    // Books with no template at all (nor the base the starters extend), and
    // a customer who names none.
    fs::remove_file(books.file("invoices/tpl/myco-default.html")).expect("removes");
    fs::remove_file(books.file("invoices/tpl/_invoice_base.v1.html")).expect("removes");
    let wingtip = books.read("customers/Wingtip.md");
    let without: String = wingtip
        .lines()
        .filter(|l| !l.starts_with("invoice_template:"))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(books.file("customers/Wingtip.md"), without).expect("writes");
    let row = ["--row", "Work|1|$100"];

    // A dry run copies nothing.
    let mut dry = vec!["new", "--customer", "Wingtip", "--dry-run"];
    dry.extend_from_slice(&row);
    let preview = books.json(&dry, 0);
    assert!(preview.get("template_copied").is_none(), "{preview}");
    assert!(!books.file("invoices/tpl/classic.html").exists());

    // The first invoice gets the default starter, copied in byte for byte.
    let mut args = vec!["new", "--customer", "Wingtip"];
    args.extend_from_slice(&row);
    let made = books.json(&args, 0);
    assert_eq!(made["template_copied"], "invoices/tpl/classic.html");
    assert_eq!(
        made["problems"],
        serde_json::json!([]),
        "the template is known at once"
    );
    assert_eq!(
        books.read("invoices/tpl/classic.html"),
        starter("classic.html")
    );
    // With the base it extends (PLAN Q1).
    assert_eq!(
        books.read("invoices/tpl/_invoice_base.v1.html"),
        starter("_invoice_base.v1.html")
    );
    let path = made["path"].as_str().expect("path");
    assert!(
        books
            .read(path)
            .contains("template: /invoices/tpl/classic.html")
    );

    // From then on it is the user's: the next invoice uses it as it is.
    fs::write(
        books.file("invoices/tpl/classic.html"),
        "{# invoicer: name=\"Mine\" #}\n{{ frontmatter.invoice_num }}",
    )
    .expect("edits");
    let next = books.json(&args, 0);
    assert!(next.get("template_copied").is_none(), "{next}");
    assert!(books.read("invoices/tpl/classic.html").contains("Mine"));

    // Naming the other starter copies that one.
    let mut modern = vec![
        "new",
        "--customer",
        "Wingtip",
        "--template",
        "/invoices/tpl/modern.html",
    ];
    modern.extend_from_slice(&row);
    let made = books.json(&modern, 0);
    assert_eq!(made["template_copied"], "invoices/tpl/modern.html");
    assert_eq!(
        books.read("invoices/tpl/modern.html"),
        starter("modern.html")
    );

    // A template that is no starter is never invented: the draft names it
    // and `check` warns.
    let mut contoso = vec!["new", "--customer", "Contoso"];
    contoso.extend_from_slice(&row);
    let made = books.json(&contoso, 0);
    assert!(made.get("template_copied").is_none(), "{made}");
    assert_eq!(made["problems"][0]["code"], "unknown-template", "{made}");
    assert!(!books.file("invoices/tpl/myco-default.html").exists());

    // The text output says what was copied.
    fs::remove_file(books.file("invoices/tpl/modern.html")).expect("removes");
    let mut text = vec![
        "new",
        "--customer",
        "Wingtip",
        "--template",
        "/invoices/tpl/modern.html",
    ];
    text.extend_from_slice(&row);
    let out = books.text(&text);
    assert!(
        out.contains("copied the starter template to invoices/tpl/modern.html"),
        "{out}"
    );
}

/// `set template=` naming a starter copies it in on first use, as `new`
/// does (PLAN F9): the CLI side of the app's "Use this template".
#[test]
fn set_template_copies_a_starter_in() {
    let books = Books::of("accrual");
    // The base the fixture's template extends is the user's: kept.
    let base = "invoices/tpl/_invoice_base.v1.html";
    fs::write(
        books.file(base),
        books.read(base).replace("version 1", "version 1, mine"),
    )
    .expect("edits");
    let set = books.json(&["set", "108", "template=/invoices/tpl/modern.html"], 0);
    assert_eq!(set["changed"], serde_json::json!(["template"]), "{set}");
    assert_eq!(set["template_copied"], "invoices/tpl/modern.html");
    assert_eq!(
        books.read("invoices/tpl/modern.html"),
        starter("modern.html")
    );
    assert!(books.read(base).contains("version 1, mine"));
    // Named again: the file is the user's now, never replaced.
    fs::write(
        books.file("invoices/tpl/modern.html"),
        "{{ frontmatter.invoice_num }} mine",
    )
    .expect("edits");
    let again = books.json(&["set", "105", "template=/invoices/tpl/modern.html"], 0);
    assert!(again.get("template_copied").is_none(), "{again}");
    assert_eq!(
        books.read("invoices/tpl/modern.html"),
        "{{ frontmatter.invoice_num }} mine"
    );
    // A template that is no starter is written as given; `check` warns.
    let other = books.json(&["set", "108", "template=/invoices/tpl/nope.html"], 0);
    assert!(other.get("template_copied").is_none(), "{other}");
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
    // New terms move the due date (P2): 2026-11-17 + 45.
    assert_eq!(
        done["changed"],
        serde_json::json!(["reference", "terms", "due_date"])
    );
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
            ("due_date: 2026-12-17", "due_date: 2027-01-01"),
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
    // No command: `--json` never opens the window (a script is asking).
    let out = books.bare().arg("--json").output().expect("runs");
    assert_eq!(out.status.code(), Some(2));
    // `--server` with no journal to serve refuses before binding anything.
    let out = books.bare().arg("--server").output().expect("runs");
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
        "send",
        "void",
        "rebook",
        "apply-payment",
        "reconcile",
        "receivables",
        "recognize",
    ] {
        assert!(help.contains(&format!("  {command}")), "{command}: {help}");
    }
}

/// The one contiguous run of lines `after` changes in `before`:
/// `(removed, added)`.
fn changed(before: &str, after: &str) -> (String, String) {
    let b: Vec<&str> = before.split_inclusive('\n').collect();
    let a: Vec<&str> = after.split_inclusive('\n').collect();
    let head = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
    let tail = b[head..]
        .iter()
        .rev()
        .zip(a[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    (
        b[head..b.len() - tail].concat(),
        a[head..a.len() - tail].concat(),
    )
}

/// `text` with every `recognition:` id written `<id>` (ids are random).
fn ids_blanked(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| match line.find("recognition: ") {
            Some(at) => {
                let start = at + "recognition: ".len();
                let end = line[start..]
                    .find([',', '\n'])
                    .map_or(line.len(), |e| start + e);
                format!("{}<id>{}", &line[..start], &line[end..])
            }
            None => line.to_string(),
        })
        .collect()
}

const A_105: &str = "invoices/2026-11-10 Customer X Invoice 105.md";
const A_106: &str = "invoices/2026-08-01 Wingtip Invoice 106.md";
const A_107: &str = "invoices/2026-10-15 Contoso Invoice 107.md";
const A_108: &str = "invoices/2026-11-17 Customer X Invoice 108.md";

#[test]
fn accrual_books_end_to_end() {
    let books = Books::of("accrual");
    let journal = books.read("books.journal");
    let draft = books.read(A_108);

    // 1. Preview the send: nothing written.
    let preview = books.json(&["send", "108", "--dry-run"], 0);
    assert_eq!(preview["status"], "draft");
    assert_eq!(preview["basis"], "accrual");
    assert!(preview["booked"].is_null());
    // The text shows money as the books do: `commodity $1,000.00` (P3).
    let text = books.text(&["send", "108", "--dry-run"]);
    for amount in [
        "receivable    $13,600.00  ;",
        "deferred-revenue  $-12,000.00  ;",
        "consulting            $-1,600.00  ;",
    ] {
        assert!(text.contains(amount), "{amount}: {text}");
    }
    assert_eq!(books.read("books.journal"), journal);

    // 2. Send: the journal gains 108's transaction; the file is marked sent.
    let sent = books.json(&["send", "108", "--via", "email"], 0);
    assert_eq!(
        (&sent["journal_written"], &sent["markdown_written"]),
        (&Value::Bool(true), &Value::Bool(true))
    );
    let after_send = books.read("books.journal");
    let (removed, added) = changed(&journal, &after_send);
    assert_eq!(removed, "");
    assert_eq!(
        ids_blanked(&added),
        "\n2026-11-17 Customer X | Invoice 108  ; invoice: 108, document: invoices/2026-11-17 \
         Customer X Invoice 108.md, due: 2026-12-17, reference: PO #PY75334\n    assets:accounts \
         receivable    $13600.00  ; customer: Customer X\n    liabilities:deferred-revenue  \
         $-12000.00  ; customer: Customer X, recognition: <id>, service-start: 2026-12-01, \
         term-months: 12, item: Platform subscription 1 year\n    revenue:consulting            \
         $-1600.00  ; customer: Customer X, item: Senior architect hours\n"
    );
    let sent_file = books.read(A_108);
    assert!(sent_file.contains("status: sent\n"));
    assert!(sent_file.ends_with(&draft[draft.find("\n---\n").expect("body")..]));
    for line in ["sent_date: 2026-11-17\n", "sent_via: email\n", "paid: no\n"] {
        assert!(sent_file.contains(line), "{line}: {sent_file}");
    }
    // Sending again is refused (exit 2, with the error's code).
    let again = books.json(&["send", "108"], 2);
    assert_eq!(again["code"], "status");

    // 3. Ledgeline links the rest of 107's payment: reconcile mirrors it.
    let linked = format!(
        "{after_send}\n2026-11-16 CONTOSO ACH 4  ; invoice: 107\n    assets:bank:checking        \
         $2000.00\n    assets:accounts receivable  $-2000.00  ; customer: Contoso, match:in0107\n"
    );
    fs::write(books.file("books.journal"), &linked).expect("writes");
    let before_107 = books.read(A_107);
    assert_eq!(
        books
            .json(&["list", "--open"], 0)
            .as_array()
            .expect("list")
            .iter()
            .filter_map(|d| d["number"].as_str())
            .collect::<Vec<_>>(),
        ["108", "105", "106"],
        "open reads the books, before the mirror is refreshed"
    );
    let dry = books.json(&["reconcile", "--dry-run"], 0);
    snapshot("accrual-reconcile-dry-run.json", &books.normalized(&dry));
    assert_eq!(books.read(A_107), before_107);
    let done = books.json(&["reconcile"], 0);
    assert_eq!(done["changed"][0]["written"], true);
    assert_eq!(
        changed(&before_107, &books.read(A_107)),
        (
            "paid: partial 2026-11-05 $1,000\n".to_string(),
            "paid: 2026-11-16\n".to_string()
        )
    );
    assert_eq!(
        books.read("books.journal"),
        linked,
        "reconcile never writes the books"
    );
    assert_eq!(
        books.json(&["reconcile"], 0)["changed"],
        serde_json::json!([])
    );

    // 4. Edit 105 after sending: check says the books are out of date;
    // rebook replaces its transaction in place.
    let edited = books.read(A_105).replace(
        "| Junior engineer              | 50  |",
        "| Junior engineer              | 51  |",
    );
    fs::write(books.file(A_105), &edited).expect("writes");
    books.json(&["recalc", "105"], 0);
    let check = books.json(&["check"], 0);
    let stale: Vec<&Value> = check["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .filter(|p| p["code"] == "books-out-of-date")
        .collect();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0]["fix"]["edit"]["command"], "rebook 105");
    let recalculated = books.read(A_105);
    let rebooked = books.json(&["rebook", "105"], 0);
    assert_eq!(rebooked["changed"], true);
    assert_eq!(
        books.read(A_105),
        recalculated,
        "rebook writes the books only"
    );
    let (removed, added) = changed(&linked, &books.read("books.journal"));
    assert_eq!(
        (removed.as_str(), added.as_str()),
        (
            "    assets:accounts receivable     $8350.00  ; customer: Customer X\n    \
             revenue:consulting             $-7200.00  ; customer: Customer X, item: Senior \
             architect hours\n    revenue:consulting:delivery    $-9500.00  ; customer: Customer \
             X, item: Junior engineer\n",
            "    assets:accounts receivable     $8540.00  ; customer: Customer X\n    \
             revenue:consulting             $-7200.00  ; customer: Customer X, item: Senior \
             architect hours\n    revenue:consulting:delivery    $-9690.00  ; customer: Customer \
             X, item: Junior engineer\n"
        )
    );
    assert_eq!(books.json(&["check"], 0)["warnings"], 4);

    // 5. Void 106: a reversal linked to its receivable; the file is void.
    let before_void = books.read("books.journal");
    let voided = books.json(&["void", "106"], 0);
    assert_eq!(voided["journal_written"], true);
    let link = voided["link"].as_str().expect("link").to_string();
    let after_void = books.read("books.journal");
    // The receivable joins the reversal's link; the reversal is placed by
    // date. Nothing else changes.
    let unlinked = after_void
        .replace(&format!(", match: {link}"), "")
        .replace(&format!(", match:{link}"), "");
    let reversal = "\n2026-11-17 Wingtip | Invoice 106 (void)  ; invoice: 106, void: 2026-11-17\n    \
                    assets:accounts receivable  $-1200.00  ; customer: Wingtip\n    \
                    revenue:consulting          $1200.00  ; customer: Wingtip, item: Retainer\n";
    assert_eq!(unlinked.replacen(reversal, "", 1), before_void);
    assert_eq!(
        after_void.matches(&link.to_string()).count(),
        2,
        "{after_void}"
    );
    let file = books.read(A_106);
    assert!(file.contains("status: void\n") && file.contains("void_date: 2026-11-17\n"));

    // 6. Receivables: 105 and 108 open; 106 gone with its void.
    let receivables = books.json(&["receivables"], 0);
    snapshot("accrual-receivables.json", &books.normalized(&receivables));
    let open: Vec<&str> = receivables["open"]
        .as_array()
        .expect("open")
        .iter()
        .filter_map(|o| o["number"].as_str())
        .collect();
    assert_eq!(open, ["2001", "2002", "2003", "2004", "105", "108"]);
    snapshot(
        "accrual-receivables-aging.txt",
        &books.text(&["receivables", "--aging"]),
    );

    // 7. Recognition: listed, refused after today, posted once.
    let before_recognize = books.read("books.journal");
    let due = books.json(&["recognize", "--dry-run"], 0);
    assert_eq!(due["entries"].as_array().map(Vec::len), Some(10));
    assert_eq!(books.read("books.journal"), before_recognize);
    let late = books.json(&["recognize", "--through", "2026-11-18"], 2);
    assert_eq!(late["code"], "refused");
    let posted = books.json(&["recognize"], 0);
    assert_eq!(posted["written"], true);
    let after = books.read("books.journal");
    assert_eq!(
        after.matches("Recognized  ; recognition:").count(),
        before_recognize
            .matches("Recognized  ; recognition:")
            .count()
            + 10
    );
    assert_eq!(books.json(&["recognize"], 0)["written"], false);

    // 8. Everything agrees.
    let last = books.json(&["reconcile"], 0);
    assert_eq!(
        (last["changed"].as_array().map(Vec::len), &last["errors"]),
        (Some(0), &Value::from(0))
    );
}

const C_203: &str = "invoices/2026-11-05 Wingtip Invoice 203.md";
const C_207: &str = "invoices/2026-11-16 Customer X Invoice 207.md";

/// Deposits as Ledgeline's importer writes them.
const IMPORTS: &str = "
2026-11-16 * ACH WINGTIP PAYMENTS  ; id: ach-41
    assets:bank:checking  $1,200.00
    income:unknown

2026-11-17 * WIRE CUSTOMER X  ; id: wx-7
    assets:bank:checking   $4,000.00
    income:unknown        $-4,000.00
";

#[test]
fn cash_books_end_to_end() {
    let books = Books::of("cash");
    let imported = format!("{}{IMPORTS}", books.read("books.journal"));
    fs::write(books.file("books.journal"), &imported).expect("writes");
    let (file_203, file_207) = (books.read(C_203), books.read(C_207));

    // 1. Candidates for 207: the wire, a partial payment. Nothing written.
    let options = books.json(&["apply-payment", "207"], 0);
    snapshot("cash-apply-payment-207.json", &books.normalized(&options));
    assert_eq!(options["remaining"], "7000.00");
    let wire = options["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .find(|c| c["description"] == "WIRE CUSTOMER X")
        .and_then(|c| c["reference"].as_str())
        .expect("the wire")
        .to_string();
    assert_eq!(books.read("books.journal"), imported);

    // 2. Apply it by a reference prefix: the deposit is re-split (its bank
    // leg kept), 207's mirror follows, and an undo record is kept.
    let applied = books.json(&["apply-payment", "207", "--txn", &wire[..8]], 0);
    let (removed, added) = changed(&imported, &books.read("books.journal"));
    assert_eq!(
        (removed.as_str(), added.as_str()),
        (
            "2026-11-17 * WIRE CUSTOMER X  ; id: wx-7\n    assets:bank:checking   $4,000.00\n    \
             income:unknown        $-4,000.00\n",
            "2026-11-17 * WIRE CUSTOMER X  ; id: wx-7, invoice:207\n    assets:bank:checking   \
             $4,000.00\n    revenue:consulting           $-1714.29  ; customer: Customer X\n    \
             revenue:consulting:delivery  $-2285.71  ; customer: Customer X\n"
        )
    );
    assert_eq!(applied["mirror"]["after"], "partial 2026-11-17 $4,000");
    assert_eq!(
        changed(&file_207, &books.read(C_207)),
        (
            "paid: no\n".to_string(),
            "paid: partial 2026-11-17 $4,000\n".to_string()
        )
    );
    let undo_file = applied["undo_file"]
        .as_str()
        .expect("undo file")
        .to_string();
    assert!(undo_file.starts_with(".ledgeline-invoicer/undo/apply-payment-207-"));
    let record: Value = serde_json::from_str(&books.read(&undo_file)).expect("json");
    assert_eq!(
        record["original_text"],
        "2026-11-17 * WIRE CUSTOMER X  ; id: wx-7\n    assets:bank:checking   $4,000.00\n    income:unknown        $-4,000.00\n"
    );

    // 3. 203 paid exactly by its ACH: paid on the deposit's date.
    let ach = books.json(&["apply-payment", "203"], 0)["candidates"][0]["reference"]
        .as_str()
        .expect("reference")
        .to_string();
    books.json(&["apply-payment", "203", "--txn", &ach], 0);
    assert_eq!(
        changed(&file_203, &books.read(C_203)),
        ("paid: no\n".to_string(), "paid: 2026-11-16\n".to_string())
    );
    // A paid invoice has nothing left to apply.
    assert_eq!(
        books.json(&["apply-payment", "203"], 0)["candidates"],
        serde_json::json!([])
    );
    // Cash books refuse to void an invoice with payments applied.
    assert_eq!(books.json(&["void", "203"], 2)["code"], "refused");

    // 4. Receivables from the files, less what is applied.
    let receivables = books.json(&["receivables"], 0);
    snapshot("cash-receivables.json", &books.normalized(&receivables));

    // 5. Undo 207's payment: the deposit is back byte for byte, the mirror
    // follows, the record is gone.
    let before_undo = books.read("books.journal");
    let undone = books.json(&["apply-payment", "207", "--undo"], 0);
    assert_eq!(undone["mirror"]["after"], "no");
    let (removed, added) = changed(&before_undo, &books.read("books.journal"));
    assert!(removed.contains("invoice:207"), "{removed}");
    assert_eq!(
        added,
        "2026-11-17 * WIRE CUSTOMER X  ; id: wx-7\n    assets:bank:checking   $4,000.00\n    \
         income:unknown        $-4,000.00\n"
    );
    assert_eq!(books.read(C_207), file_207);
    assert!(!books.file(&undo_file).exists());
    let none = books.json(&["apply-payment", "207", "--undo"], 2);
    assert!(
        none["error"]
            .as_str()
            .expect("message")
            .contains("no undo record")
    );

    // 6. Accrual-only and cash-only commands say so.
    assert_eq!(
        Books::of("accrual").json(&["apply-payment", "105"], 2)["code"],
        "refused"
    );

    // 7. Everything agrees.
    let last = books.json(&["reconcile"], 0);
    assert_eq!(last["changed"], serde_json::json!([]));
    assert_eq!(last["errors"], 0);
}

#[test]
fn reconcile_and_check_exit_1_on_books_errors() {
    let books = Books::of("accrual");
    // 107's transaction is gone: a sent invoice with nothing booked.
    let journal = books.read("books.journal");
    let start = journal
        .find("2026-10-15 Contoso | Invoice 107")
        .expect("107");
    let end = start + journal[start..].find("\n\n").expect("end") + 2;
    fs::write(
        books.file("books.journal"),
        format!("{}{}", &journal[..start], &journal[end..]),
    )
    .expect("writes");
    let report = books.json(&["reconcile"], 1);
    let missing: Vec<&Value> = report["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .filter(|p| p["code"] == "missing-transaction")
        .collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0]["file"], A_107);
    assert_eq!(missing[0]["fix"]["edit"]["command"], "send 107");
    assert_eq!(books.json(&["check"], 1)["errors"], 1);
    let text = books.run(&["reconcile"]);
    assert_eq!(text.status.code(), Some(1));
    // `send` books a sent invoice that was never booked.
    let sent = books.json(&["send", "107"], 0);
    assert_eq!(sent["journal_written"], true);
    assert_eq!(books.json(&["check"], 0)["errors"], 0);
}

// ---------------------------------------------------------------------------
// `customers new` and `customers import-csv` (ARCHITECTURE §6.4). That they
// write the app's bytes is `app::customers_tests`'
// `the_cli_writes_the_file_the_app_writes`.
// ---------------------------------------------------------------------------

/// The customer notes in the books' `customers/` folder, by file name.
fn customer_files(books: &Books) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(books.file("customers"))
        .expect("customers/")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md"))
        .collect();
    names.sort();
    names
}

impl Books {
    /// `text` written as a CSV file beside the books (not in them).
    fn csv(&self, name: &str, text: &str) -> String {
        let path = self.dir.path().join(name);
        fs::write(&path, text).expect("writes");
        path.display().to_string()
    }
}

#[test]
fn customers_new_writes_a_note_and_never_overwrites() {
    let books = Books::of("accrual");
    let before = customer_files(&books);
    let args = [
        "customers",
        "new",
        "Northwind Traders",
        "--email",
        "ap@northwind.example",
        "--address",
        "1 Main St\nSpringfield, CO 80000",
        "--terms",
        "15",
        "--revenue-account",
        "revenue:consulting",
        "--portal-name",
        "Coupa",
        "--portal-url",
        "https://coupa.example/",
        "--portal-requires-reference",
    ];
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let preview = snapshot_json(&books, "accrual-customers-new-dry-run.json", &dry);
    assert_eq!(preview["written"], false);
    assert_eq!(customer_files(&books), before, "a dry run writes nothing");

    let made = books.json(&args, 0);
    for key in [
        "path",
        "title",
        "link",
        "ledger_customer",
        "kind",
        "hash",
        "written",
        "changed",
    ] {
        assert!(made.get(key).is_some(), "{key}: {made}");
    }
    assert!(made.get("text").is_none(), "{made}");
    assert_eq!(made["path"], "customers/Northwind Traders.md");
    assert_eq!(made["link"], "[[Northwind Traders]]");
    assert_eq!(made["kind"], "organization");
    assert_eq!(made["written"], true);
    assert_eq!(made["hash"], preview["hash"]);
    let text = books.read("customers/Northwind Traders.md");
    assert_eq!(text, preview["text"].as_str().expect("text"));

    // It is a customer at once.
    let shown = books.json(&["customers", "show", "Northwind Traders"], 0);
    assert_eq!(shown["terms"], 15);
    assert_eq!(shown["portal"], "Coupa");

    // Taken: refused, any case, and the note is left as it was.
    for name in ["Northwind Traders", "northwind traders", "Contoso"] {
        let refused = books.json(&["customers", "new", name, "--kind", "person"], 2);
        let error = refused["error"].as_str().expect("error");
        assert!(error.contains("already exists"), "{name}: {error}");
    }
    assert_eq!(books.read("customers/Northwind Traders.md"), text);
    let refused = books.run(&["customers", "new", "Contoso"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("already exists"));

    // Fields that don't read are refused.
    for bad in [
        &["--kind", "company"][..],
        &["--terms", "net 30"],
        &["--portal-url", "ftp://portal.example"],
    ] {
        let mut args = vec!["customers", "new", "Initech"];
        args.extend_from_slice(bad);
        let refused = books.json(&args, 2);
        assert!(refused["error"].is_string(), "{bad:?}: {refused}");
    }
    assert!(!books.file("customers/Initech.md").exists());

    let human = books.text(&["customers", "new", "Zoë Müller", "--kind", "person"]);
    assert_eq!(
        human,
        "Created customers/Zoë Müller.md (person)\n  invoices link it as [[Zoë Müller]]; the \
         books know it as Zoë Müller\n"
    );
}

/// A customer list as a spreadsheet exports it: headers in any case, a
/// quoted name holding a comma, a quoted address holding line breaks, a
/// quote written twice, UTF-8, a blank row.
const CUSTOMERS_CSV: &str = "Name,Kind,E Mail,Address,Terms,Revenue Account\n\
    \"Smith, Jones & Co\",organization,ap@sj.example,\"1 Main St\nSpringfield, CO\",30,revenue:consulting\n\
    \n\
    \"The \"\"Best\"\" Bakery\",,orders@bakery.example,,,\n\
    Zoë Müller,Person,zoe@example.de,,14,\n";

#[test]
fn customers_import_csv_writes_every_row_or_none() {
    let books = Books::of("accrual");
    let before = customer_files(&books);

    // An unknown column is refused, listing the ones there are.
    let file = books.csv("customers.csv", CUSTOMERS_CSV);
    let refused = books.json(&["customers", "import-csv", &file], 2);
    assert_eq!(refused["code"], "header", "{refused}");
    let error = refused["error"].as_str().expect("error");
    assert!(error.contains("unknown column \"E Mail\""), "{error}");
    assert!(
        error.contains("name, kind, email, phone, address"),
        "{error}"
    );
    assert_eq!(customer_files(&books), before);

    let file = books.csv("customers.csv", &CUSTOMERS_CSV.replace("E Mail", "EMAIL"));
    let preview = snapshot_json(
        &books,
        "accrual-customers-import-dry-run.json",
        &["customers", "import-csv", &file, "--dry-run"],
    );
    assert_eq!(preview["dry_run"], true);
    assert_eq!(customer_files(&books), before, "a dry run writes nothing");

    let report = books.json(&["customers", "import-csv", &file], 0);
    assert_eq!(report["dry_run"], false);
    let created: Vec<(&str, u64)> = report["created"]
        .as_array()
        .expect("created")
        .iter()
        .map(|r| {
            (
                r["path"].as_str().expect("path"),
                r["line"].as_u64().expect("line"),
            )
        })
        .collect();
    assert_eq!(
        created,
        [
            ("customers/Smith, Jones & Co.md", 2),
            // `"` is no file-name character: `file_stem` makes it `-`.
            ("customers/The -Best- Bakery.md", 5),
            ("customers/Zoë Müller.md", 6),
        ]
    );
    for (row, (path, _)) in created.iter().enumerate() {
        assert_eq!(
            books.read(path),
            preview["created"][row]["text"].as_str().expect("text"),
            "{path}: written as previewed"
        );
    }
    let smith = books.read("customers/Smith, Jones & Co.md");
    assert!(
        smith.contains("  - billing: |\n      1 Main St\n      Springfield, CO\n"),
        "{smith}"
    );
    let zoe = books.json(&["customers", "show", "Zoë Müller"], 0);
    assert_eq!(zoe["kind"], "person");
    assert_eq!(zoe["terms"], 14);

    // The same file again: every row exists, so nothing is written ...
    let again = books.json(&["customers", "import-csv", &file], 2);
    assert_eq!(again["code"], "invalid");
    assert_eq!(again["rows"].as_array().expect("rows").len(), 3);
    assert!(
        again["rows"][0]["error"]
            .as_str()
            .expect("error")
            .contains("--skip-existing"),
        "{again}"
    );
    // ... unless existing rows are skipped.
    let more = books.csv(
        "more.csv",
        "name,email\n\"Smith, Jones & Co\",\nInitech,ap@initech.example\n",
    );
    let skipped = books.json(&["customers", "import-csv", &more, "--skip-existing"], 0);
    assert_eq!(
        skipped["skipped"][0]["path"],
        "customers/Smith, Jones & Co.md"
    );
    assert_eq!(skipped["skipped"][0]["line"], 2);
    assert_eq!(skipped["created"][0]["path"], "customers/Initech.md");
    assert_eq!(
        books.read("customers/Smith, Jones & Co.md"),
        smith,
        "untouched"
    );
}

#[test]
fn customers_import_csv_one_bad_row_writes_nothing() {
    let books = Books::of("accrual");
    let before = customer_files(&books);
    let file = books.csv(
        "bad.csv",
        "name,terms,portal_url\n\
         Good One,30,\n\
         Bad Terms,soon,\n\
         Short Row\n\
         \"Comma, Co\",,https://portal.example/\n\
         GOOD ONE,,\n",
    );
    let refused = books.json(&["customers", "import-csv", &file], 2);
    assert_eq!(refused["code"], "invalid");
    let rows: Vec<(u64, &str)> = refused["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| {
            (
                r["line"].as_u64().expect("line"),
                r["name"].as_str().expect("name"),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [(3, "Bad Terms"), (4, "Short Row"), (6, "GOOD ONE")],
        "{refused}"
    );
    assert_eq!(customer_files(&books), before, "all or nothing");

    // As text: every row on stderr, exit 2.
    let out = books.run(&["customers", "import-csv", &file]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("3 rows don't read, so nothing was written:"),
        "{stderr}"
    );
    assert!(
        stderr.contains("  line 3 (Bad Terms): terms \"soon\""),
        "{stderr}"
    );

    // A missing file, and a file past the size cap.
    let missing = books.json(&["customers", "import-csv", "nowhere.csv"], 2);
    assert!(
        missing["error"]
            .as_str()
            .expect("error")
            .contains("nowhere.csv")
    );
    let big = books.csv("big.csv", &"x".repeat(4 * 1024 * 1024 + 1));
    let refused = books.json(&["customers", "import-csv", &big], 2);
    assert!(
        refused["error"]
            .as_str()
            .expect("error")
            .contains("larger than"),
        "{refused}"
    );
}

#[test]
fn customers_import_csv_help_lists_the_columns() {
    let books = Books::of("cash");
    let help = books.text(&["customers", "import-csv", "--help"]);
    for column in [
        "name",
        "kind",
        "email",
        "address",
        "terms",
        "revenue_account",
        "portal_requires_reference",
    ] {
        assert!(help.contains(&format!("\n  {column} ")), "{column}: {help}");
    }
    assert!(help.contains("--skip-existing"), "{help}");
}

// ---------------------------------------------------------------------------
// `pdf` (PLAN 3c). Printing needs a real webview and the main thread, so the
// export itself is `#[ignore]`d: the lead runs it on a desktop session with
//   nix develop -c cargo test -p ledgeline-invoicer --test cli -- --ignored pdf_
// ---------------------------------------------------------------------------

#[test]
fn pdf_refuses_an_unknown_invoice_before_starting_anything() {
    let books = Books::of("accrual");
    let error = books.json(&["pdf", "999"], 2);
    assert!(error["error"].as_str().unwrap().contains("999"), "{error}");
}

#[cfg(not(feature = "gui"))]
#[test]
fn pdf_needs_the_desktop_build() {
    let books = Books::of("accrual");
    let error = books.json(&["pdf", "105"], 2);
    assert_eq!(error["code"], "needs-gui", "{error}");
    assert!(
        error["error"].as_str().unwrap().contains("`gui` feature"),
        "{error}"
    );
}

/// `pdftotext FILE -` when it is on PATH (the dev shell has it).
#[cfg(feature = "gui")]
fn pdf_text(file: &Path) -> Option<String> {
    let out = Command::new("pdftotext").arg(file).arg("-").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(feature = "gui")]
#[test]
#[ignore = "prints through the system webview: needs a desktop session (see above)"]
fn pdf_keeps_a_sent_invoice_and_its_earlier_export() {
    let books = Books::of("accrual");
    let first = books.json(&["pdf", "105"], 0);
    assert_eq!(first["destination"], "kept", "{first}");
    assert_eq!(first["sent_hash"], "written", "{first}");
    let kept = books.file("invoices/pdf/2026-11-10 Customer X Invoice 105.pdf");
    let bytes = fs::read(&kept).expect("kept");
    assert!(bytes.starts_with(b"%PDF-"));
    if let Some(text) = pdf_text(&kept) {
        assert!(text.contains("Customer X"), "{text}");
        // The fixture's template prints amounts as the bill table writes them.
        assert!(text.contains("$8,350"), "{text}");
        assert!(text.contains("Less deposit (Invoice 104)"), "{text}");
        // The books column is hidden by the template.
        assert!(!text.contains("liabilities:customer-deposits"), "{text}");
    }
    assert!(
        books
            .read("invoices/2026-11-10 Customer X Invoice 105.md")
            .contains("sent_hash: ")
    );

    let second = books.json(&["pdf", "105"], 0);
    let earlier = second["earlier"].as_str().expect("the first is kept");
    assert!(books.file(earlier).is_file(), "{second}");

    // A draft: a temporary file, the books untouched.
    let draft = books.json(&["pdf", "108"], 0);
    assert_eq!(draft["destination"], "temp", "{draft}");
    let path = PathBuf::from(draft["path"].as_str().unwrap());
    assert!(fs::read(&path).unwrap().starts_with(b"%PDF-"));
    assert!(
        !books
            .read("invoices/2026-11-17 Customer X Invoice 108.md")
            .contains("sent_hash")
    );

    // -o: a copy, nothing kept.
    let copy = books.dir.path().join("copy.pdf");
    let out = books.json(&["pdf", "105", "-o", copy.to_str().unwrap()], 0);
    assert_eq!(out["destination"], "copy", "{out}");
    assert!(fs::read(&copy).unwrap().starts_with(b"%PDF-"));
}

/// `config` shows the settings plan's new keys, and the payment matching
/// shared with Ledgeline (`ui-prefs.toml`, else these books' `[ui]`).
#[test]
fn config_shows_the_new_settings_and_the_shared_matching() {
    let books = Books::of("cash");
    let config = books.json(&["config"], 0);
    assert_eq!(config["invoicer"]["default-terms"], Value::Null);
    assert_eq!(config["invoicer"]["default-template"], Value::Null);
    assert_eq!(
        config["matching"],
        serde_json::json!({
            "settlementLookbackDays": 180,
            "settlementLeadDays": 4,
        })
    );
    let settings = books.read("ledgeline.toml");
    fs::write(
        books.file("ledgeline.toml"),
        settings.replace(
            "number-start = 201\n",
            "number-start = 201\ndefault-terms = 15\ndefault-template = \"/invoices/tpl/myco-default.html\"\n\n[ui]\nsettlementLeadDays = 9\n",
        ),
    )
    .expect("writes");
    fs::write(
        books.dir.path().join("config/ui-prefs.toml"),
        "settlementLookbackDays = 365\nsettlementToleranceBps = 100\nsettlementLeadDays = 2\n",
    )
    .expect("writes");
    let config = books.json(&["config"], 0);
    // Ledgeline's tolerance does nothing in the invoicer: not shown.
    assert_eq!(config["matching"].get("settlementToleranceBps"), None);
    assert_eq!(config["invoicer"]["default-terms"], 15);
    assert_eq!(
        config["invoicer"]["default-template"],
        "/invoices/tpl/myco-default.html"
    );
    // The books' pin wins over the shared file, which wins over the default.
    assert_eq!(config["matching"]["settlementLookbackDays"], 365);
    assert_eq!(config["matching"]["settlementLeadDays"], 9);
    let text = books.text(&["config"]);
    assert!(text.contains("default-terms = 15\n"), "{text}");
    assert!(text.contains("# settlementLeadDays = 9\n"), "{text}");
    assert!(!text.contains("settlementToleranceBps"), "{text}");
}

/// RC-10: `apply-payment`'s matching flags, defaulting to the shared
/// settings the app uses. 203 ($1,200, Wingtip) is paid by an ACH of
/// 2026-11-16; on 2027-06-04 that is 200 days back.
#[test]
fn apply_payment_takes_the_matching_flags() {
    let books = Books::of("cash");
    let journal = books.read("books.journal");
    fs::write(
        books.journal(),
        format!(
            "{journal}\n2026-11-16 * ACH WINGTIP PAYMENTS  ; id: ach-41\n    \
             assets:bank:checking  $1,200.00\n    income:unknown\n"
        ),
    )
    .expect("writes");
    let list = |extra: &[&str]| -> Output {
        books
            .bare()
            .arg("-f")
            .arg(books.journal())
            .args(["--today", "2027-06-04", "--json", "apply-payment", "203"])
            .args(extra)
            .output()
            .expect("runs")
    };
    let found = |out: &Output| -> bool {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let options: Value = serde_json::from_slice(&out.stdout).expect("json");
        options["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .any(|c| c["description"] == "ACH WINGTIP PAYMENTS")
    };
    assert!(!found(&list(&[])), "the default 180 days");
    assert!(found(&list(&["--lookback-days", "365"])));
    assert!(
        found(&list(&["--lookback-days", "0"])),
        "0 looks back forever"
    );
    assert!(found(&list(&[
        "--lookback-days",
        "365",
        "--lead-days",
        "4"
    ])));
    // The shared setting is the default, as in the app.
    fs::write(
        books.dir.path().join("config/ui-prefs.toml"),
        "settlementLookbackDays = 0\n",
    )
    .expect("writes");
    assert!(found(&list(&[])));
    assert!(!found(&list(&["--lookback-days", "30"])), "a flag wins");
    for bad in [
        ["--lookback-days", "4000"],
        ["--lead-days", "61"],
        // No tolerance flag: the matcher offers a short deposit as a part
        // payment whatever the tolerance.
        ["--tolerance-percent", "2.5"],
    ] {
        assert_eq!(list(&bad).status.code(), Some(2), "{bad:?}");
    }
}

/// A starter keeps the base's `ready` block (MBR's export-ready script),
/// unlike the fixture's own template: the export page must run that script
/// under the invoicer's CSP (scripts from the server only, its export pass
/// lifting the sandbox), or the export waits out MBR's whole readiness
/// timeout (30 s) before printing.
#[cfg(feature = "gui")]
#[test]
#[ignore = "prints through the system webview: needs a desktop session (see above)"]
fn pdf_prints_a_starter_on_its_ready_signal() {
    let books = Books::of("accrual");
    let invoice = "invoices/2026-11-10 Customer X Invoice 105.md";
    let source = books.read(invoice).replace(
        "template: /invoices/tpl/myco-default.html",
        "template: /invoices/tpl/classic.html",
    );
    assert!(source.contains("tpl/classic.html"));
    fs::write(books.file(invoice), source).unwrap();
    let copy = books.dir.path().join("classic.pdf");
    let started = std::time::Instant::now();
    let out = books.json(&["pdf", "105", "-o", copy.to_str().unwrap()], 0);
    let took = started.elapsed();
    assert_eq!(out["destination"], "copy", "{out}");
    assert!(fs::read(&copy).unwrap().starts_with(b"%PDF-"));
    assert!(
        took < std::time::Duration::from_secs(20),
        "{took:?}: the page never said it was ready"
    );
    if let Some(text) = pdf_text(&copy) {
        assert!(text.contains("Customer X"), "{text}");
        assert!(text.contains("No. 105"), "{text}");
    }
}
