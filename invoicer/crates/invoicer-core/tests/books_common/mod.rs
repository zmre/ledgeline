//! Helpers for the books bridge's integration tests: fixture copies under
//! `target/`, opened the way the CLI opens them.

#![allow(dead_code)]

use invoicer_core::books::InvoiceBooks;
use invoicer_core::doc::Money;
use invoicer_core::index::DocumentIndex;
use invoicer_core::settings::Settings;
use std::path::{Path, PathBuf};

/// `invoicer/fixtures/<name>`.
#[must_use]
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .canonicalize()
        .expect("fixture exists")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("dir");
    for entry in std::fs::read_dir(from).expect("reads") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copies");
        }
    }
}

/// A fresh copy of fixture `name` under `target/`, as `<test>`; the fixture
/// itself is never touched.
#[must_use]
pub fn scratch(name: &str, test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("invoicer-books")
        .join(format!("{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&fixture(name), &dir);
    dir
}

/// The books and a fresh index over `dir`.
#[must_use]
pub fn open(dir: &Path) -> (InvoiceBooks, DocumentIndex) {
    let books = InvoiceBooks::open(&dir.join("books.journal")).expect("opens");
    reopen(books)
}

/// A fresh index for `books` (after the files changed underneath).
#[must_use]
pub fn reopen(books: InvoiceBooks) -> (InvoiceBooks, DocumentIndex) {
    let loaded = books.load().expect("loads");
    let settings = Settings::read(books.vault()).expect("settings");
    let money = Money::of_journal(loaded.journal());
    let index = DocumentIndex::open(books.vault(), settings, money);
    (books, index)
}

/// A file's text.
#[must_use]
pub fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).expect("reads")
}

/// Write a file.
pub fn write(dir: &Path, path: &str, text: &str) {
    std::fs::write(dir.join(path), text).expect("writes");
}

/// `text` with `from` replaced by `to`, which must be there exactly once.
#[must_use]
pub fn replace_once(text: &str, from: &str, to: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "{from:?} once in\n{text}");
    text.replacen(from, to, 1)
}

/// `text` with the transaction starting `header` (its first line) removed,
/// with the blank line before it; returns the transaction too.
#[must_use]
pub fn cut_transaction(text: &str, header: &str) -> (String, String) {
    let start = text
        .find(header)
        .unwrap_or_else(|| panic!("{header:?} in\n{text}"));
    let end = text[start..]
        .find("\n\n")
        .map_or(text.len(), |at| start + at + 1);
    let txn = text[start..end].to_string();
    let cut_from = text[..start]
        .strip_suffix('\n')
        .map_or(start, |_| start - 1);
    (format!("{}{}", &text[..cut_from], &text[end..]), txn)
}

/// Run hledger on files in `dir` (named with `-f`, never `LEDGER_FILE`), when
/// `LEDGELINE_HLEDGER_INVOICER_CHECK` is set: `None` otherwise.
#[must_use]
pub fn hledger(dir: &Path, files: &[&str], args: &[&str]) -> Option<String> {
    std::env::var("LEDGELINE_HLEDGER_INVOICER_CHECK").ok()?;
    let mut command = std::process::Command::new("hledger");
    command.current_dir(dir).env_remove("LEDGER_FILE");
    for file in files {
        command.args(["-f", file]);
    }
    let out = command.args(args).output().expect("hledger runs");
    assert!(
        out.status.success(),
        "hledger {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8(out.stdout).expect("utf-8"))
}

/// The one contiguous run of text `after` adds to `before` (and what it
/// replaces): `(removed, added)`, trimmed to whole lines.
#[must_use]
pub fn diff(before: &str, after: &str) -> (String, String) {
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

/// A Wingtip invoice: `rows` under the standard header, then the total.
#[must_use]
pub fn wingtip(number: &str, date: &str, status: &str, rows: &str, total: &str) -> String {
    format!(
        "---\ntitle: {date} Wingtip Invoice {number}\ntype: invoice\ntemplate: /invoices/tpl/myco-default.html\n\
         status: {status}\ninvoice_num: {number}\ndate: {date}\nterms: 30\ncustomer: \"[[Wingtip]]\"\n\
         invoicee_name: Wingtip\ninvoicee_email: ap@wingtip.example\ninvoicee_address: |\n  1 Wingtip Way\n\
         invoicer: \"[[My Company]]\"\ninvoicer_name: My Company\ninvoicer_email: finance@mycompany.com\n\
         invoicer_address: |\n  1234 Background Blvd.\n---\n\n\
         | Description             | Qty | Rate   | Amount    | *Account*                     |\n\
         |:------------------------|----:|-------:|----------:|:------------------------------|\n\
         {rows}\
         |                         |     | **Total due** | **{total}** |                  |\n"
    )
}
