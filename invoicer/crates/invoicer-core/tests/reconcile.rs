//! The reconciler and the books checks (PLAN 2c, ARCHITECTURE §4.3, §4.5,
//! §5.4) on fixture copies under `target/`, plus the concurrent-write rule:
//! a write racing Ledgeline's editor is refused whole and retried once.

mod books_common;

use books_common::*;
use invoicer_core::books::{self, BooksError, InvoiceBooks, Recognize, Send, Void, retry_once};
use invoicer_core::doc::Paid;
use invoicer_core::index::DocumentIndex;
use invoicer_core::problem::{FixEdit, Problem, ProblemCode, Severity};
use invoicer_core::reconcile::{self, BookStates, BooksPaid, PaidAmount, ReconcileOptions};
use ledgeline_core::Dec;
use ledgeline_core::vault::{Entry, LocalVault, Revision, Vault, VaultError, VaultPath};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const TODAY: &str = "2026-11-17";
const INV_106: &str = "invoices/2026-08-01 Wingtip Invoice 106.md";
const INV_107: &str = "invoices/2026-10-15 Contoso Invoice 107.md";
const INV_108: &str = "invoices/2026-11-17 Customer X Invoice 108.md";

fn states(books: &InvoiceBooks) -> BookStates {
    let loaded = books.load().expect("loads");
    BookStates::build(loaded.journal(), books::Basis::of(&loaded))
}

fn paid_of(books: &InvoiceBooks, index: &DocumentIndex, number: &str) -> Option<BooksPaid> {
    let doc = index.by_number(number)[0];
    states(books).paid(doc)
}

fn full(date: &str) -> Option<BooksPaid> {
    Some(BooksPaid::Full { date: date.into() })
}

fn partial(parts: &[(&str, i128)]) -> Option<BooksPaid> {
    Some(BooksPaid::Partial {
        parts: parts
            .iter()
            .map(|(date, cents)| PaidAmount {
                date: (*date).into(),
                amount: Dec::new(*cents, 2),
            })
            .collect(),
    })
}

fn run(books: &InvoiceBooks, index: &mut DocumentIndex, dry_run: bool) -> reconcile::Reconciled {
    let loaded = books.load().expect("loads");
    reconcile::reconcile(
        books,
        index,
        &loaded,
        &ReconcileOptions {
            only: None,
            dry_run,
        },
    )
}

/// The books checks, less plan 45's orphans (2001–2004, booked without
/// files in the accrual fixture).
fn findings(books: &InvoiceBooks, index: &DocumentIndex) -> Vec<Problem> {
    let loaded = books.load().expect("loads");
    let states = BookStates::build(loaded.journal(), books::Basis::of(&loaded));
    reconcile::audit(books, index, &loaded, &states, None)
        .into_iter()
        .filter(|p| !(p.code == ProblemCode::OrphanTransaction && p.message.contains(" 200")))
        .collect()
}

fn command(problem: &Problem) -> Option<&str> {
    match problem.fix.as_ref()?.edit.as_ref()? {
        FixEdit::Command { command } => Some(command),
        _ => None,
    }
}

#[test]
fn the_fixtures_mirrors_are_what_their_books_say() {
    let dir = scratch("accrual", "mirrors-accrual");
    let (books, index) = open(&dir);
    for (number, expected) in [
        ("101", full("2026-10-05")),
        ("102", full("2026-10-20")),
        ("104", full("2026-10-08")),
        ("105", Some(BooksPaid::No)),
        ("106", Some(BooksPaid::No)),
        ("107", partial(&[("2026-11-05", 100_000)])),
        // 103 is void in the books: they can't say it is paid.
        ("103", None),
    ] {
        assert_eq!(paid_of(&books, &index, number), expected, "{number}");
    }
    let dir = scratch("cash", "mirrors-cash");
    let (books, index) = open(&dir);
    for (number, expected) in [
        ("201", full("2026-11-03")),
        ("202", partial(&[("2026-11-12", 50_000)])),
        ("203", Some(BooksPaid::No)),
        ("206", full("2026-11-08")),
        ("207", Some(BooksPaid::No)),
    ] {
        assert_eq!(paid_of(&books, &index, number), expected, "{number}");
    }
    // Each mirror as written already says so: nothing to reconcile.
    for name in ["accrual", "cash"] {
        let dir = scratch(name, &format!("mirrors-clean-{name}"));
        let (books, mut index) = open(&dir);
        let report = run(&books, &mut index, false);
        assert!(report.changed.is_empty(), "{name}: {:#?}", report.changed);
        assert_eq!(report.counts.errors, 0, "{name}: {:#?}", report.problems);
        assert!(findings(&books, &index).is_empty(), "{name}");
    }
}

#[test]
fn reconcile_mirrors_new_payments_and_changes_only_paid() {
    let dir = scratch("accrual", "reconcile-payments");
    let (books, _) = open(&dir);
    let before = states(&books);
    // Ledgeline links two payments: the rest of 107 (N:1), part of 106.
    let journal = replace_once(
        &read(&dir, "books.journal"),
        "    assets:accounts receivable  $1200.00  ; customer: Wingtip\n",
        "    assets:accounts receivable  $1200.00  ; customer: Wingtip, match:in0106\n",
    );
    write(
        &dir,
        "books.journal",
        &format!(
            "{journal}\n2026-11-16 CONTOSO ACH 4  ; invoice: 107\n    assets:bank:checking        \
             $2000.00\n    assets:accounts receivable  $-2000.00  ; customer: Contoso, \
             match:in0107\n\n2026-11-16 WINGTIP ACH\n    assets:bank:checking        $200.00\n    \
             assets:accounts receivable  $-200.00  ; customer: Wingtip, match:in0106\n\n\
             2026-11-16 WINGTIP ACH 2\n    assets:bank:checking        $300.00\n    \
             assets:accounts receivable  $-300.00  ; customer: Wingtip, match:in0106\n"
        ),
    );
    let (books, mut index) = reopen(books);
    assert_eq!(states(&books).changed_since(&before), ["106", "107"]);
    let (old_106, old_107) = (read(&dir, INV_106), read(&dir, INV_107));

    // Dry: reported, nothing written.
    let dry = run(&books, &mut index, true);
    let changes: Vec<(&str, Option<&str>, &str, bool)> = dry
        .changed
        .iter()
        .map(|c| {
            (
                c.number.as_str(),
                c.before.as_deref(),
                c.after.as_str(),
                c.written,
            )
        })
        .collect();
    assert_eq!(
        changes,
        [
            ("106", Some("no"), "partial 2026-11-16 $500", false),
            (
                "107",
                Some("partial 2026-11-05 $1,000"),
                "2026-11-16",
                false
            ),
        ]
    );
    assert_eq!((dry.checked, dry.unchanged), (6, 4));
    assert_eq!(read(&dir, INV_106), old_106);

    // For real: one line each, the rest byte for byte.
    let done = run(&books, &mut index, false);
    assert!(done.changed.iter().all(|c| c.written), "{done:#?}");
    assert!(done.failed.is_empty());
    assert_eq!(
        diff(&old_106, &read(&dir, INV_106)),
        (
            "paid: no\n".into(),
            "paid: partial 2026-11-16 $500\n".into()
        )
    );
    assert_eq!(
        diff(&old_107, &read(&dir, INV_107)),
        (
            "paid: partial 2026-11-05 $1,000\n".into(),
            "paid: 2026-11-16\n".into()
        )
    );
    // The index sees it; reconciling again writes nothing.
    assert!(matches!(
        index.by_number("107")[0].header.paid,
        Some(Paid::Full { .. })
    ));
    let again = run(&books, &mut index, false);
    assert!(again.changed.is_empty(), "{:#?}", again.changed);
    assert!(findings(&books, &index).is_empty());

    // Only: one invoice at a time.
    let loaded = books.load().expect("loads");
    let one = reconcile::reconcile(
        &books,
        &mut index,
        &loaded,
        &ReconcileOptions {
            only: Some(vec!["105".into()]),
            dry_run: true,
        },
    );
    assert_eq!(one.checked, 1);
}

#[test]
fn a_zero_total_is_paid_on_its_date() {
    let dir = scratch("cash", "zero-total");
    let rows = "| Discovery               | 1   | $500   | $500.00   | revenue:consulting            |\n\
                | Less deposit (Invoice 206) |   |        | -$500.00  | liabilities:customer-deposits |\n";
    write(
        &dir,
        "invoices/2026-11-15 Wingtip Invoice 209.md",
        &wingtip("209", "2026-11-15", "sent", rows, "$0.00")
            .replace("---\n\n|", "paid: no\n---\n\n|"),
    );
    let (books, mut index) = open(&dir);
    assert_eq!(paid_of(&books, &index, "209"), full("2026-11-15"));
    let report = run(&books, &mut index, false);
    let changed: Vec<(&str, &str)> = report
        .changed
        .iter()
        .map(|c| (c.number.as_str(), c.after.as_str()))
        .collect();
    assert_eq!(changed, [("209", "2026-11-15")]);
    // Cash books mark a sent zero total with a `$0` booking: none here.
    let missing: Vec<_> = report
        .problems
        .iter()
        .filter(|p| p.code == ProblemCode::MissingTransaction)
        .map(command)
        .collect();
    assert_eq!(missing, [Some("send 209")]);
}

/// The one finding on `code`, asserting its severity.
fn one(problems: &[Problem], code: ProblemCode, severity: Severity) -> Problem {
    let found: Vec<&Problem> = problems.iter().filter(|p| p.code == code).collect();
    assert_eq!(found.len(), 1, "{code:?} in {problems:#?}");
    assert_eq!(found[0].severity, severity, "{:?}", found[0]);
    found[0].clone()
}

#[test]
fn the_books_checks_name_each_mismatch_and_its_fix() {
    // An invoice booked twice.
    let dir = scratch("accrual", "check-duplicate");
    let (_, txn) = cut_transaction(
        &read(&dir, "books.journal"),
        "2026-08-01 Wingtip | Invoice 106",
    );
    write(
        &dir,
        "books.journal",
        &format!("{}\n{txn}", read(&dir, "books.journal")),
    );
    let (books, index) = open(&dir);
    let found = findings(&books, &index);
    let dup = one(&found, ProblemCode::DuplicateNumber, Severity::Error);
    assert_eq!(
        dup.file.as_ref().map(VaultPath::as_str),
        Some("books.journal")
    );
    assert!(dup.message.contains("booked 2 times"), "{dup}");

    // A sent invoice with no transaction; its payment, now an orphan link,
    // is Ledgeline's business.
    let dir = scratch("accrual", "check-missing");
    let (journal, _) = cut_transaction(
        &read(&dir, "books.journal"),
        "2026-10-15 Contoso | Invoice 107",
    );
    write(&dir, "books.journal", &journal);
    let (books, index) = open(&dir);
    let missing = one(
        &findings(&books, &index),
        ProblemCode::MissingTransaction,
        Severity::Error,
    );
    assert_eq!(missing.file.as_ref().map(VaultPath::as_str), Some(INV_107));
    assert_eq!(command(&missing), Some("send 107"));

    // A transaction whose file is gone.
    let dir = scratch("accrual", "check-orphan");
    std::fs::remove_file(dir.join(INV_106)).expect("removes");
    let (books, index) = open(&dir);
    let orphan = one(
        &findings(&books, &index),
        ProblemCode::OrphanTransaction,
        Severity::Warning,
    );
    assert!(
        orphan.message.contains("invoice 106")
            && orphan
                .message
                .contains("its `document:` says invoices/2026-08-01"),
        "{orphan}"
    );

    // A file renamed after it was booked: re-booking repairs `document:`.
    const INV_105: &str = "invoices/2026-11-10 Customer X Invoice 105.md";
    let dir = scratch("accrual", "check-moved");
    std::fs::rename(dir.join(INV_105), dir.join("invoices/Customer X 105.md")).expect("renames");
    let (books, index) = open(&dir);
    let moved = one(
        &findings(&books, &index),
        ProblemCode::DocumentPath,
        Severity::Warning,
    );
    assert_eq!(command(&moved), Some("rebook 105"));
    let rebooked = books
        .rebook(
            &index,
            &books::Rebook {
                number: "105".into(),
                today: TODAY.into(),
                expected: None,
            },
        )
        .expect("rebooks");
    assert!(rebooked.changed);
    assert!(read(&dir, "books.journal").contains("document: invoices/Customer X 105.md, due:"));
    assert!(findings(&books, &index).is_empty());
    // In the closed period (106 is dated before closed-through), re-booking
    // is refused, so it isn't offered.
    std::fs::rename(dir.join(INV_106), dir.join("invoices/Wingtip 106.md")).expect("renames");
    let (books, index) = reopen(books);
    let closed = one(
        &findings(&books, &index),
        ProblemCode::DocumentPath,
        Severity::Warning,
    );
    assert_eq!(command(&closed), None);
    assert!(
        closed
            .fix
            .as_ref()
            .is_some_and(|f| f.message.contains("closed period")),
        "{closed}"
    );

    // A file name with a comma can't match its tag (2b's sanitizing).
    let dir = scratch("accrual", "check-comma");
    let comma = "invoices/2026-08-01 Wingtip, Inc Invoice 106.md";
    std::fs::rename(dir.join(INV_106), dir.join(comma)).expect("renames");
    write(
        &dir,
        "books.journal",
        &replace_once(
            &read(&dir, "books.journal"),
            "document: invoices/2026-08-01 Wingtip Invoice 106.md",
            "document: invoices/2026-08-01 Wingtip Inc Invoice 106.md",
        ),
    );
    let (books, index) = open(&dir);
    let comma_found = one(
        &findings(&books, &index),
        ProblemCode::DocumentPath,
        Severity::Warning,
    );
    assert!(comma_found.message.contains("a comma"), "{comma_found}");
    assert_eq!(command(&comma_found), None);

    // Edited after sending: books out of date, fixed by re-booking (none
    // offered in the closed period).
    let dir = scratch("accrual", "check-out-of-date");
    for (path, from, to) in [
        (INV_107, "Follow-up workshop", "Follow-up workshops"),
        (INV_106, "Retainer", "Monthly retainer"),
    ] {
        write(&dir, path, &replace_once(&read(&dir, path), from, to));
    }
    let (books, index) = open(&dir);
    let stale: Vec<(Option<&str>, Option<&str>)> = findings(&books, &index)
        .iter()
        .filter(|p| p.code == ProblemCode::BooksOutOfDate)
        .map(|p| (p.file.as_ref().map(VaultPath::as_str), command(p)))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(f, c)| {
            (
                f.map(|f| if f == INV_106 { "106" } else { "107" }),
                c.map(|_| "rebook"),
            )
        })
        .collect();
    assert_eq!(stale, [(Some("106"), None), (Some("107"), Some("rebook"))]);

    // A stale mirror, with the edit that fixes it.
    let dir = scratch("accrual", "check-mirror");
    write(
        &dir,
        INV_107,
        &replace_once(
            &read(&dir, INV_107),
            "paid: partial 2026-11-05 $1,000",
            "paid: no",
        ),
    );
    let (books, index) = open(&dir);
    let mirror = one(
        &findings(&books, &index),
        ProblemCode::MissingMirror,
        Severity::Warning,
    );
    assert_eq!(
        mirror.fix.and_then(|f| f.edit),
        Some(FixEdit::SetField {
            key: "paid".into(),
            value: serde_json::json!("partial 2026-11-05 $1,000"),
        })
    );

    // Void in the file, open in the books.
    let dir = scratch("accrual", "check-void-file");
    write(
        &dir,
        INV_106,
        &replace_once(&read(&dir, INV_106), "status: sent", "status: void"),
    );
    let (books, index) = open(&dir);
    one(
        &findings(&books, &index),
        ProblemCode::VoidMismatch,
        Severity::Error,
    );
}

/// A local vault with hooks before each write: a failing markdown write (a
/// full disk), or another program saving the file first (Ledgeline's
/// editor, a text editor).
#[derive(Debug)]
struct Hooked {
    inner: LocalVault,
    /// Fail every markdown write while set.
    fail_markdown: AtomicBool,
    /// Before this many journal writes, save an edit of the journal first.
    race_journal: AtomicUsize,
    /// Before this many markdown writes, save an edit of that file first.
    race_markdown: AtomicUsize,
}

impl Hooked {
    fn new(inner: LocalVault) -> Self {
        Self {
            inner,
            fail_markdown: AtomicBool::new(false),
            race_journal: AtomicUsize::new(0),
            race_markdown: AtomicUsize::new(0),
        }
    }

    fn take(counter: &AtomicUsize) -> bool {
        counter
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }

    fn before_write(&self, path: &VaultPath) -> Result<(), VaultError> {
        let markdown = path.as_str().ends_with(".md");
        if markdown && self.fail_markdown.load(Ordering::SeqCst) {
            return Err(VaultError::from_io(
                path,
                std::io::Error::other("injected failure"),
            ));
        }
        let race = if markdown {
            Self::take(&self.race_markdown)
        } else {
            path.as_str() == "books.journal" && Self::take(&self.race_journal)
        };
        if race {
            let text = self.inner.read_to_string(path)?;
            let edit = if markdown {
                format!("{text}\nEdited elsewhere.\n")
            } else {
                format!("{text}\n; Ledgeline saved this while the invoicer was writing.\n")
            };
            self.inner.write_atomic(path, edit.as_bytes())?;
        }
        Ok(())
    }
}

impl Vault for Hooked {
    fn root(&self) -> &Path {
        self.inner.root()
    }
    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        self.inner.stat(path)
    }
    fn list_dir(&self, dir: &VaultPath, cap: Option<usize>) -> Result<Vec<Entry>, VaultError> {
        self.inner.list_dir(dir, cap)
    }
    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        self.inner.canonicalize(path)
    }
    fn read_prefix(&self, path: &VaultPath, max_len: u64) -> Result<Vec<u8>, VaultError> {
        self.inner.read_prefix(path, max_len)
    }
    fn read_pinned(
        &self,
        path: &VaultPath,
        cap: Option<u64>,
    ) -> Result<(Vec<u8>, Revision), VaultError> {
        self.inner.read_pinned(path, cap)
    }
    fn replace(
        &self,
        path: &VaultPath,
        bytes: &[u8],
        expected: &Revision,
    ) -> Result<Revision, VaultError> {
        self.before_write(path)?;
        self.inner.replace(path, bytes, expected)
    }
    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.before_write(path)?;
        self.inner.write_atomic(path, bytes)
    }
    fn create_new(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.before_write(path)?;
        self.inner.create_new(path, bytes)
    }
    fn create_new_shared(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.before_write(path)?;
        self.inner.create_new_shared(path, bytes)
    }
    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        self.inner.create_dir_all(path)
    }
    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        self.inner.remove_file(path)
    }
    fn local_path(&self, path: &VaultPath) -> Option<std::path::PathBuf> {
        self.inner.local_path(path)
    }
}

fn hooked(dir: &Path) -> (Arc<Hooked>, InvoiceBooks, DocumentIndex) {
    let (vault, main) = LocalVault::for_journal(&dir.join("books.journal")).expect("vault");
    let vault = Arc::new(Hooked::new(vault));
    let (books, index) = reopen(InvoiceBooks::open_in(vault.clone(), main));
    (vault, books, index)
}

fn send_108() -> Send {
    Send {
        number: "108".into(),
        today: TODAY.into(),
        sent_via: None,
        expected: None,
    }
}

#[test]
fn a_send_stopped_after_the_journal_is_a_booked_draft_until_send_finishes_it() {
    let dir = scratch("accrual", "booked-draft");
    let (vault, books, mut index) = hooked(&dir);
    vault.fail_markdown.store(true, Ordering::SeqCst);
    let failed = books.send(&mut index, &send_108());
    assert!(
        matches!(failed, Err(BooksError::MarkdownNotWritten { .. })),
        "{failed:?}"
    );
    vault.fail_markdown.store(false, Ordering::SeqCst);
    let booked = one(
        &findings(&books, &index),
        ProblemCode::BookedDraft,
        Severity::Error,
    );
    assert_eq!(command(&booked), Some("send 108"));
    assert!(booked.message.contains("a send stopped"), "{booked}");
    assert_eq!(booked.file.as_ref().map(VaultPath::as_str), Some(INV_108));

    // Booked differently (the draft edited since): still found, no command.
    let draft = read(&dir, INV_108);
    write(
        &dir,
        INV_108,
        &replace_once(&draft, "Senior architect hours", "Architect hours"),
    );
    let (books, mut index) = reopen(books);
    let differs = one(
        &findings(&books, &index),
        ProblemCode::BookedDraft,
        Severity::Error,
    );
    assert_eq!(command(&differs), None);
    write(&dir, INV_108, &draft);
    let _ = index.refresh(books.vault(), &[VaultPath::new(INV_108).expect("path")]);

    // The offered fix finishes it, and the books checks are clean.
    let finished = books.send(&mut index, &send_108()).expect("finishes");
    assert!(!finished.journal_written && finished.markdown_written);
    assert!(findings(&books, &index).is_empty());
}

#[test]
fn a_void_stopped_after_the_journal_is_finished_by_void() {
    let dir = scratch("accrual", "void-stopped");
    let (vault, books, mut index) = hooked(&dir);
    vault.fail_markdown.store(true, Ordering::SeqCst);
    let void = Void {
        number: "106".into(),
        date: TODAY.into(),
        today: TODAY.into(),
        expected: None,
    };
    assert!(books.void(&mut index, &void).is_err());
    vault.fail_markdown.store(false, Ordering::SeqCst);
    let found = one(
        &findings(&books, &index),
        ProblemCode::VoidMismatch,
        Severity::Error,
    );
    assert_eq!(command(&found), Some("void 106 --date 2026-11-17"));
    books.void(&mut index, &void).expect("finishes");
    assert!(findings(&books, &index).is_empty());
}

#[test]
fn a_concurrent_journal_write_is_refused_whole_and_retried_once() {
    let dir = scratch("accrual", "concurrent");
    let (vault, books, mut index) = hooked(&dir);
    let journal = read(&dir, "books.journal");
    let draft = read(&dir, INV_108);

    // Ledgeline saves between the invoicer's load and its save.
    vault.race_journal.store(1, Ordering::SeqCst);
    let raced = books.send(&mut index, &send_108());
    assert_eq!(
        raced.as_ref().err().map(BooksError::code),
        Some("external-change"),
        "{raced:?}"
    );
    // Nothing of the invoicer's was written: only Ledgeline's edit is there.
    let after = read(&dir, "books.journal");
    assert_eq!(
        diff(&journal, &after).1,
        "\n; Ledgeline saved this while the invoicer was writing.\n"
    );
    assert!(!after.contains("Invoice 108"));
    assert_eq!(read(&dir, INV_108), draft);

    // Retried once: the second attempt reads Ledgeline's edit and lands.
    vault.race_journal.store(1, Ordering::SeqCst);
    let mut attempts = 0;
    let (sent, retried) = retry_once(|| {
        attempts += 1;
        books.send(&mut index, &send_108())
    });
    let sent = sent.expect("the retry lands");
    assert!(retried && sent.journal_written && sent.markdown_written);
    assert_eq!(attempts, 2);
    let landed = read(&dir, "books.journal");
    assert_eq!(landed.matches("Invoice 108  ; invoice: 108").count(), 1);
    assert_eq!(
        landed
            .matches("; Ledgeline saved this while the invoicer was writing.")
            .count(),
        2
    );
    assert!(read(&dir, INV_108).contains("status: sent\n"));

    // Only once: a second race in a row is returned, nothing written.
    vault.race_journal.store(2, Ordering::SeqCst);
    let void = Void {
        number: "106".into(),
        date: TODAY.into(),
        today: TODAY.into(),
        expected: None,
    };
    let before = read(&dir, INV_106);
    let (twice, retried) = retry_once(|| books.void(&mut index, &void));
    assert!(retried);
    assert_eq!(twice.err().map(|e| e.code()), Some("external-change"));
    assert!(!read(&dir, "books.journal").contains("Invoice 106 (void)"));
    assert_eq!(read(&dir, INV_106), before);
}

#[test]
fn a_mirror_whose_file_changed_meanwhile_is_not_written() {
    let dir = scratch("accrual", "mirror-race");
    write(
        &dir,
        INV_107,
        &replace_once(
            &read(&dir, INV_107),
            "paid: partial 2026-11-05 $1,000",
            "paid: no",
        ),
    );
    let (vault, books, mut index) = hooked(&dir);
    vault.race_markdown.store(1, Ordering::SeqCst);
    let report = run(&books, &mut index, false);
    let failed: Vec<(&str, &str)> = report
        .failed
        .iter()
        .map(|f| (f.number.as_str(), f.code))
        .collect();
    assert_eq!(failed, [("107", "external-change")]);
    let text = read(&dir, INV_107);
    assert!(text.contains("paid: no\n") && text.ends_with("Edited elsewhere.\n"));
    // The next run mirrors it, keeping the other program's edit.
    let (books, mut index) = reopen(books);
    let again = run(&books, &mut index, false);
    assert!(again.changed[0].written);
    let text = read(&dir, INV_107);
    assert!(
        text.contains("paid: partial 2026-11-05 $1,000\n") && text.ends_with("Edited elsewhere.\n")
    );
}

#[test]
fn recognize_posts_what_is_due_never_after_today() {
    let dir = scratch("accrual", "recognize");
    let (books, _) = open(&dir);
    let journal = read(&dir, "books.journal");
    let request = |through: Option<&str>, dry_run: bool, include_closed: bool| Recognize {
        today: TODAY.into(),
        through: through.map(str::to_string),
        include_closed,
        dry_run,
        expected: None,
    };
    let due = books.recognize(&request(None, true, false)).expect("lists");
    assert!(!due.written);
    assert_eq!(due.entries.len(), 10);
    assert_eq!(due.closed.len(), 1);
    assert_eq!(read(&dir, "books.journal"), journal);

    let refused = books.recognize(&request(Some("2026-11-18"), false, false));
    assert!(
        matches!(&refused, Err(BooksError::Refused(why)) if why.contains("never posted after today")),
        "{refused:?}"
    );

    let early = books
        .recognize(&request(Some("2026-11-02"), false, false))
        .expect("posts");
    assert!(early.written);
    assert_eq!(early.entries.len(), 5);
    let rest = books.recognize(&request(None, false, true)).expect("posts");
    assert_eq!(rest.entries.len(), 5);
    assert_eq!(rest.adjustments.len(), 1);
    assert!(rest.closed.is_empty());
    let again = books.recognize(&request(None, false, true)).expect("posts");
    assert!(!again.written && again.entries.is_empty());
    let posted = read(&dir, "books.journal");
    assert_eq!(
        posted.matches("Recognized  ; recognition:").count(),
        journal.matches("Recognized  ; recognition:").count() + 10
    );
}
