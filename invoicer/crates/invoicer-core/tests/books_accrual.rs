//! The books bridge on the accrual fixture (PLAN 2b): every operation on a
//! copy of `invoicer/fixtures/accrual` under `target/`, with the journal text
//! asserted exactly. With `LEDGELINE_HLEDGER_INVOICER_CHECK=1` (inside the
//! dev shell), hledger reads the result too.

mod books_common;

use books_common::*;
use invoicer_core::books::{ApplyDeposit, Basis, BooksError, InvoiceBooks, Rebook, Send, Void};
use invoicer_core::doc::Invoice;
use invoicer_core::table::words::Words;
use ledgeline_core::Dec;
use ledgeline_core::accrual::ops::OpsError;
use ledgeline_core::vault::{Entry, LocalVault, Revision, Vault, VaultError, VaultPath};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const TODAY: &str = "2026-11-17";
const INV_105: &str = "invoices/2026-11-10 Customer X Invoice 105.md";
const INV_106: &str = "invoices/2026-08-01 Wingtip Invoice 106.md";
const INV_108: &str = "invoices/2026-11-17 Customer X Invoice 108.md";
const INV_102: &str = "invoices/2026-09-15 Fabrikam Invoice 102.md";

/// What sending 108 books (`recognition:` ids blanked).
const BOOKED_108: &str = "\
2026-11-17 Customer X | Invoice 108  ; invoice: 108, document: invoices/2026-11-17 Customer X Invoice 108.md, due: 2026-12-17, reference: PO #PY75334
    assets:accounts receivable    $13600.00  ; customer: Customer X
    liabilities:deferred-revenue  $-12000.00  ; customer: Customer X, recognition: ID, service-start: 2026-12-01, term-months: 12, item: Platform subscription 1 year
    revenue:consulting            $-1600.00  ; customer: Customer X, item: Senior architect hours
";

fn send(number: &str) -> Send {
    Send {
        number: number.into(),
        today: TODAY.into(),
        sent_via: None,
        expected: None,
    }
}

fn rebook(number: &str) -> Rebook {
    Rebook {
        number: number.into(),
        today: TODAY.into(),
        expected: None,
    }
}

fn void(number: &str, date: &str) -> Void {
    Void {
        number: number.into(),
        date: date.into(),
        today: TODAY.into(),
        expected: None,
    }
}

/// `text` with every `recognition:` id replaced by `ID` (ids are fresh).
fn ids_blanked(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("recognition: ") {
        let (head, tail) = rest.split_at(at + "recognition: ".len());
        out.push_str(head);
        out.push_str("ID");
        let end = tail.find([',', '\n']).unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// The document after recomputing its table.
fn recalculated(text: &str, books: &InvoiceBooks) -> String {
    let loaded = books.load().expect("loads");
    let money = invoicer_core::doc::Money::of_journal(loaded.journal());
    Invoice::parse(text, &Words::default())
        .recalc(&Words::default(), &money)
        .expect("recalcs")
        .unwrap_or_else(|| text.to_string())
}

#[test]
fn the_fixture_books_are_in_step_with_their_files() {
    let dir = scratch("accrual", "in-step");
    let (books, index) = open(&dir);
    for number in ["101", "102", "103", "104", "105", "106", "107"] {
        let preview = books.preview(&index, number).expect("previews");
        assert_eq!(preview.basis, Basis::Accrual);
        assert_eq!(
            preview.in_step,
            Some(true),
            "{number}: booked\n{}\nwould book\n{}",
            preview.booked.as_ref().map_or("", |b| b.text.as_str()),
            preview.would_book.as_deref().unwrap_or("")
        );
        assert_eq!(
            preview.receivable.as_deref(),
            Some("assets:accounts receivable")
        );
    }
    // Linked: paid, partly paid, void.
    for (number, linked) in [("101", true), ("105", false), ("106", false), ("107", true)] {
        assert_eq!(
            books.preview(&index, number).expect("previews").linked,
            linked
        );
    }
    let draft = books.preview(&index, "108").expect("previews");
    assert_eq!(draft.booked, None);
    assert_eq!(draft.in_step, None);
    assert_eq!(
        ids_blanked(draft.would_book.as_deref().expect("would book")),
        BOOKED_108
    );
}

#[test]
fn sending_books_exactly_what_the_fixture_holds() {
    // 105, un-booked and set back to draft, sends back to the fixture's own
    // text: the journal and the markdown, byte for byte.
    let dir = scratch("accrual", "resend-105");
    let journal = read(&dir, "books.journal");
    let markdown = read(&dir, INV_105);
    let (cut, original) = cut_transaction(&journal, "2026-11-10 Customer X | Invoice 105");
    write(&dir, "books.journal", &cut);
    write(
        &dir,
        INV_105,
        &replace_once(&markdown, "status: sent\n", "status: draft\n"),
    );
    let (books, mut index) = open(&dir);
    let sent = books.send(&mut index, &send("105")).expect("sends");
    assert!(sent.journal_written && sent.markdown_written);
    assert_eq!(sent.booked.expect("booked").text, original);
    assert_eq!(read(&dir, INV_105), markdown);
    assert_eq!(
        diff(&journal, &read(&dir, "books.journal")),
        (String::new(), String::new())
    );
    // Again: it is sent.
    let again = books.send(&mut index, &send("105"));
    assert!(matches!(again, Err(BooksError::Status { .. })), "{again:?}");
}

#[test]
fn sending_into_a_closed_period_is_refused() {
    // 106 is dated 2026-08-01; the books are closed through 2026-09-30.
    let dir = scratch("accrual", "resend-106");
    let journal = read(&dir, "books.journal");
    let (cut, _) = cut_transaction(&journal, "2026-08-01 Wingtip | Invoice 106");
    write(&dir, "books.journal", &cut);
    let markdown = replace_once(&read(&dir, INV_106), "status: sent\n", "status: draft\n");
    write(&dir, INV_106, &markdown);
    let (books, mut index) = open(&dir);
    let refused = books.send(&mut index, &send("106"));
    assert!(
        matches!(&refused, Err(BooksError::Refused(why)) if why.contains("closed-through 2026-09-30")),
        "{refused:?}"
    );
    assert_eq!(read(&dir, "books.journal"), cut);
    assert_eq!(read(&dir, INV_106), markdown);
}

#[test]
fn send_rebook_and_void_on_accrual_books() {
    let dir = scratch("accrual", "lifecycle");
    let (books, mut index) = open(&dir);
    let before = read(&dir, "books.journal");
    let recognition_before = read(&dir, "recognition.journal");
    let draft = read(&dir, INV_108);

    // Send 108: one transaction, its schedule in recognition.journal, and
    // four frontmatter lines; the custom field is kept byte for byte.
    let sent = books
        .send(
            &mut index,
            &Send {
                sent_via: Some("portal".into()),
                ..send("108")
            },
        )
        .expect("sends");
    assert!(sent.journal_written && sent.markdown_written);
    let booked = sent.booked.expect("booked").text;
    assert_eq!(ids_blanked(&booked), BOOKED_108);
    let (removed, added) = diff(&before, &read(&dir, "books.journal"));
    assert_eq!(removed, "");
    assert_eq!(added, format!("\n{booked}"));
    let id = booked
        .split("recognition: ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .expect("an id");
    let (removed, added) = diff(&recognition_before, &read(&dir, "recognition.journal"));
    assert_eq!(removed, "");
    assert_eq!(
        added.replace(id, "ID"),
        "\n; ── Customer X · ID · 2026-11-17 Customer X | Invoice 108\n\
         ~ monthly from 2026-12-01 to 2027-11-02  Customer X Recognized  ; recognition: ID, renewal-date: 2027-12-01\n\
         \x20   liabilities:deferred-revenue   $1000.00  ; customer: Customer X\n\
         \x20   revenue:saas                  $-1000.00  ; customer: Customer X\n"
    );
    assert_eq!(
        read(&dir, INV_108),
        replace_once(
            &replace_once(&draft, "status: draft\n", "status: sent\n"),
            "kept byte for byte\n---\n",
            "kept byte for byte\nsent_date: 2026-11-17\nsent_via: portal\npaid: no\n---\n"
        ),
        "status changes in place; the mirrors go after the last key"
    );
    let sent_md = read(&dir, INV_108);

    // Re-book with nothing changed: nothing written.
    let same = books.rebook(&index, &rebook("108")).expect("rebooks");
    assert!(!same.changed);
    assert_eq!(
        same.booked.as_ref().map(|b| b.text.clone()),
        Some(booked.clone())
    );

    // Edit: 3 hours, not 2. Re-book replaces the transaction in place.
    let edited = recalculated(
        &replace_once(
            &sent_md,
            "| 2   | h    | $800    | $1,600.00  |",
            "| 3   | h    | $800    | $1,600.00  |",
        ),
        &books,
    );
    write(&dir, INV_108, &edited);
    let (books, index) = reopen(books);
    let journal = read(&dir, "books.journal");
    let rebooked = books.rebook(&index, &rebook("108")).expect("rebooks");
    assert!(rebooked.changed);
    assert!(rebooked.warnings.is_empty());
    let (removed, added) = diff(&journal, &read(&dir, "books.journal"));
    assert_eq!(
        (removed.replace(id, "ID"), added.replace(id, "ID")),
        (
            "    assets:accounts receivable    $13600.00  ; customer: Customer X\n\
             \x20   liabilities:deferred-revenue  $-12000.00  ; customer: Customer X, recognition: ID, service-start: 2026-12-01, term-months: 12, item: Platform subscription 1 year\n\
             \x20   revenue:consulting            $-1600.00  ; customer: Customer X, item: Senior architect hours\n"
                .to_string(),
            "    assets:accounts receivable    $14400.00  ; customer: Customer X\n\
             \x20   liabilities:deferred-revenue  $-12000.00  ; customer: Customer X, recognition: ID, service-start: 2026-12-01, term-months: 12, item: Platform subscription 1 year\n\
             \x20   revenue:consulting            $-2400.00  ; customer: Customer X, item: Senior architect hours\n"
                .to_string()
        ),
        "re-booked in place, keeping its schedule id"
    );

    // RC-7: the check a caller runs before writing the file evaluates the
    // text it will write. Dropping the deferred row would drop its schedule:
    // refused, and nothing is written. Another hour is fine, and is still
    // only checked.
    let dropped: String = edited
        .lines()
        .filter(|line| !line.starts_with("| SAAS-Y"))
        .map(|line| format!("{line}\n"))
        .collect();
    let dropped = recalculated(&dropped, &books);
    let more = recalculated(
        &replace_once(
            &edited,
            "| Senior architect hours        | 3   |",
            "| Senior architect hours        | 4   |",
        ),
        &books,
    );
    let (books, index) = reopen(books);
    let journal = read(&dir, "books.journal");
    let rules = read(&dir, "recognition.journal");
    let checked = books.check_rebook(&index, "108", &dropped, TODAY);
    assert!(
        matches!(&checked, Err(BooksError::Ops(OpsError::Refused(why))) if why.contains("drop its schedule")),
        "{checked:?}"
    );
    assert!(books.check_rebook(&index, "108", &more, TODAY).is_ok());
    assert_eq!(read(&dir, "books.journal"), journal);
    assert_eq!(read(&dir, "recognition.journal"), rules);
    assert_eq!(read(&dir, INV_108), edited);

    // Void 108: it has a deferred line, so Ledgeline refuses; nothing moves.
    let journal = read(&dir, "books.journal");
    let (books, mut index) = reopen(books);
    let refused = books.void(&mut index, &void("108", TODAY));
    assert!(
        matches!(&refused, Err(BooksError::Ops(OpsError::Refused(why))) if why.contains("deferred")),
        "{refused:?}"
    );
    assert_eq!(read(&dir, "books.journal"), journal);
    assert_eq!(read(&dir, INV_108), edited);

    // Void 106: the reversal, linked to the invoice's receivable.
    let md_106 = read(&dir, INV_106);
    let voided = books.void(&mut index, &void("106", TODAY)).expect("voids");
    assert!(voided.journal_written && voided.markdown_written);
    let link = voided.link.clone().expect("a link");
    let reversal = format!(
        "2026-11-17 Wingtip | Invoice 106 (void)  ; invoice: 106, void: 2026-11-17\n\
         \x20   assets:accounts receivable  $-1200.00  ; customer: Wingtip, match: {link}\n\
         \x20   revenue:consulting          $1200.00  ; customer: Wingtip, item: Retainer\n"
    );
    assert_eq!(voided.reversal.as_ref().expect("reversal").text, reversal);
    let ar_106 = "    assets:accounts receivable  $1200.00  ; customer: Wingtip\n";
    assert_eq!(
        read(&dir, "books.journal"),
        format!(
            "{}\n{reversal}",
            replace_once(
                &journal,
                ar_106,
                &format!(
                    "    assets:accounts receivable  $1200.00  ; customer: Wingtip, match:{link}\n"
                )
            )
        ),
        "the original gains the link; the reversal goes last (the latest date)"
    );
    assert_eq!(
        read(&dir, INV_106),
        replace_once(
            &replace_once(&md_106, "status: sent\n", "status: void\n"),
            "paid: no\n---\n",
            "void_date: 2026-11-17\n---\n"
        ),
        "voiding removes the paid: mirror (P1)"
    );
    let again = books.void(&mut index, &void("106", TODAY));
    assert!(matches!(again, Err(BooksError::Status { .. })), "{again:?}");

    // 102 is dated inside the closed period: an edit can't be re-booked.
    let md_102 = read(&dir, INV_102);
    let edited_102 = recalculated(
        &replace_once(&md_102, "| 6   | $200   |", "| 7   | $200   |"),
        &books,
    );
    write(&dir, INV_102, &edited_102);
    let (books, index) = reopen(books);
    let journal = read(&dir, "books.journal");
    // The check a caller runs before rewriting the markdown says so first.
    let checked = books.check_rebook(&index, "102", &edited_102, TODAY);
    assert!(
        matches!(&checked, Err(BooksError::Refused(why)) if why.contains("closed-through 2026-09-30")),
        "{checked:?}"
    );
    assert!(
        books
            .check_rebook(&index, "105", &read(&dir, INV_105), TODAY)
            .is_ok()
    );
    assert!(matches!(
        books.check_rebook(&index, "106", &read(&dir, INV_106), TODAY),
        Err(BooksError::Status { .. })
    ));
    let refused = books.rebook(&index, &rebook("102"));
    assert!(
        matches!(&refused, Err(BooksError::Ops(OpsError::Refused(why))) if why.contains("closed-through")),
        "{refused:?}"
    );
    assert_eq!(read(&dir, "books.journal"), journal);

    if let Some(out) = hledger(
        &dir,
        &["books.journal", "recognition.journal"],
        &["-s", "check"],
    ) {
        assert_eq!(out, "");
    }
    if let Some(out) = hledger(
        &dir,
        &["books.journal"],
        &["bal", "tag:invoice=^10[68]$", "--flat", "-O", "csv"],
    ) {
        // 106 and its void net to nothing; 108 as re-booked.
        assert_eq!(
            out,
            "\"account\",\"balance\"\n\
             \"assets:accounts receivable\",\"$14400.00\"\n\
             \"liabilities:deferred-revenue\",\"$-12000.00\"\n\
             \"revenue:consulting\",\"$-2400.00\"\n\
             \"Total:\",\"0\"\n"
        );
    }
}

/// A vault whose markdown writes fail while `fail` is set: the second write
/// of every operation, after the journal's.
#[derive(Debug)]
struct FailingMarkdown {
    inner: LocalVault,
    fail: AtomicBool,
}

impl FailingMarkdown {
    fn failing(&self, path: &VaultPath) -> Result<(), VaultError> {
        if self.fail.load(Ordering::SeqCst) && path.as_str().ends_with(".md") {
            Err(VaultError::from_io(
                path,
                std::io::Error::other("injected failure"),
            ))
        } else {
            Ok(())
        }
    }
}

impl Vault for FailingMarkdown {
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
        self.failing(path)?;
        self.inner.replace(path, bytes, expected)
    }
    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.failing(path)?;
        self.inner.write_atomic(path, bytes)
    }
    fn create_new(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.failing(path)?;
        self.inner.create_new(path, bytes)
    }
    fn create_new_shared(&self, path: &VaultPath, bytes: &[u8]) -> Result<Revision, VaultError> {
        self.failing(path)?;
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

#[test]
fn a_failed_markdown_write_is_finished_by_running_it_again() {
    let dir = scratch("accrual", "crash");
    let (vault, main) = LocalVault::for_journal(&dir.join("books.journal")).expect("vault");
    let failing = Arc::new(FailingMarkdown {
        inner: vault,
        fail: AtomicBool::new(true),
    });
    let (books, mut index) = reopen(InvoiceBooks::open_in(failing.clone(), main));
    let draft = read(&dir, INV_108);
    let before = read(&dir, "books.journal");

    // Send: the journal is written, the markdown isn't.
    let failed = books.send(&mut index, &send("108"));
    assert!(
        matches!(&failed, Err(BooksError::MarkdownNotWritten { number, .. }) if number == "108"),
        "{failed:?}"
    );
    assert_eq!(
        failed.as_ref().err().map(BooksError::code),
        Some("markdown-not-written")
    );
    let booked = read(&dir, "books.journal");
    assert_ne!(booked, before);
    assert_eq!(read(&dir, INV_108), draft);
    // The books panel sees a booked draft, in step with its file.
    let preview = books.preview(&index, "108").expect("previews");
    assert_eq!(
        (preview.status.as_str(), preview.in_step),
        ("draft", Some(true))
    );

    // Run it again: only the markdown is written; nothing is booked twice.
    failing.fail.store(false, Ordering::SeqCst);
    let finished = books.send(&mut index, &send("108")).expect("finishes");
    assert!(!finished.journal_written && finished.markdown_written);
    assert_eq!(read(&dir, "books.journal"), booked);
    assert!(read(&dir, INV_108).contains("status: sent\n"));

    // Void the same way.
    failing.fail.store(true, Ordering::SeqCst);
    let failed = books.void(&mut index, &void("106", TODAY));
    assert!(
        matches!(failed, Err(BooksError::MarkdownNotWritten { .. })),
        "{failed:?}"
    );
    let reversed = read(&dir, "books.journal");
    assert_eq!(reversed.matches("Invoice 106 (void)").count(), 1);
    failing.fail.store(false, Ordering::SeqCst);
    let finished = books
        .void(&mut index, &void("106", TODAY))
        .expect("finishes");
    assert!(!finished.journal_written && finished.markdown_written);
    assert!(finished.reversal.is_some());
    assert_eq!(read(&dir, "books.journal"), reversed);
    assert!(read(&dir, INV_106).contains("status: void\n"));
}

#[test]
fn a_stale_preview_writes_nothing() {
    let dir = scratch("accrual", "stale");
    let (books, mut index) = open(&dir);
    let generation = books.preview(&index, "108").expect("previews").generation;
    // Ledgeline saves between the preview and the send.
    let journal = format!("{}\n; edited in Ledgeline\n", read(&dir, "books.journal"));
    write(&dir, "books.journal", &journal);
    let draft = read(&dir, INV_108);
    let stale = books.send(
        &mut index,
        &Send {
            expected: Some(generation),
            ..send("108")
        },
    );
    assert_eq!(
        stale.as_ref().err().map(BooksError::code),
        Some("stale"),
        "{stale:?}"
    );
    assert_eq!(read(&dir, "books.journal"), journal);
    assert_eq!(read(&dir, INV_108), draft);
    // Previewed again, it goes through.
    let generation = books.preview(&index, "108").expect("previews").generation;
    books
        .send(
            &mut index,
            &Send {
                expected: Some(generation),
                ..send("108")
            },
        )
        .expect("sends");
}

#[test]
fn a_deposit_invoice_and_the_final_invoice_applying_it() {
    let dir = scratch("accrual", "deposit");
    write(
        &dir,
        "invoices/2026-11-12 Wingtip Invoice 109.md",
        &wingtip(
            "109",
            "2026-11-12",
            "draft",
            "| Deposit on phase 2      | 1   | $1,500 | $1,500.00 | liabilities:customer-deposits |\n",
            "$1,500.00",
        ),
    );
    let final_path = "invoices/2026-11-17 Wingtip Invoice 110.md";
    write(
        &dir,
        final_path,
        &wingtip(
            "110",
            "2026-11-17",
            "draft",
            "| Phase 2 build           | 1   | $4,000 | $4,000.00 | revenue:consulting            |\n",
            "$4,000.00",
        ),
    );
    let (books, mut index) = open(&dir);
    // Nothing open for Wingtip yet; Customer X's 104 is used up by 105.
    assert!(
        books
            .open_deposits(&index, "110")
            .expect("lists")
            .deposits
            .is_empty()
    );
    let customer_x = books.open_deposits(&index, "108").expect("lists");
    assert_eq!(customer_x.account, "liabilities:customer-deposits");
    assert!(customer_x.deposits.is_empty(), "{customer_x:?}");
    assert_eq!(customer_x.balance, Dec::zero());

    books
        .send(&mut index, &send("109"))
        .expect("sends the deposit");
    let open = books.open_deposits(&index, "110").expect("lists");
    assert_eq!(open.balance, Dec::new(150_000, 2));
    assert_eq!(open.deposits.len(), 1);
    assert_eq!(open.deposits[0].number.as_deref(), Some("109"));
    assert_eq!(open.deposits[0].remaining, Dec::new(150_000, 2));

    // Apply it: the row lands above the total, which is recomputed.
    let applied = books
        .apply_deposit(
            &mut index,
            &ApplyDeposit {
                number: "110".into(),
                deposit: "109".into(),
                amount: None,
            },
        )
        .expect("applies");
    assert_eq!(applied.amount, Dec::new(150_000, 2));
    assert_eq!(applied.total, Dec::new(250_000, 2));
    assert_eq!(applied.row, "Less deposit (Invoice 109)");
    assert_eq!(
        read(&dir, final_path).split("---\n\n").nth(1),
        Some(
            "| Description                | Qty | Rate          | Amount        | *Account*                     |\n\
             |:---------------------------|----:|--------------:|--------------:|:------------------------------|\n\
             | Phase 2 build              | 1   | $4,000        | $4,000.00     | revenue:consulting            |\n\
             | Less deposit (Invoice 109) |     |               | -$1,500.00    | liabilities:customer-deposits |\n\
             |                            |     | **Total due** | **$2,500.00** |                               |\n"
        )
    );
    // Applied in a draft, it is used up for every other invoice too.
    let wingtip_open = books.open_deposits(&index, "106").expect("lists");
    assert!(wingtip_open.deposits.is_empty(), "{wingtip_open:?}");
    assert_eq!(wingtip_open.balance, Dec::zero());
    // Only drafts take a deposit row.
    let sent_already = books.apply_deposit(
        &mut index,
        &ApplyDeposit {
            number: "106".into(),
            deposit: "109".into(),
            amount: None,
        },
    );
    assert!(
        matches!(sent_already, Err(BooksError::Status { .. })),
        "{sent_already:?}"
    );

    let sent = books.send(&mut index, &send("110")).expect("sends");
    assert_eq!(
        sent.booked.expect("booked").text,
        "2026-11-17 Wingtip | Invoice 110  ; invoice: 110, document: invoices/2026-11-17 Wingtip Invoice 110.md, due: 2026-12-17\n\
         \x20   assets:accounts receivable     $2500.00  ; customer: Wingtip\n\
         \x20   revenue:consulting             $-4000.00  ; customer: Wingtip, item: Phase 2 build\n\
         \x20   liabilities:customer-deposits  $1500.00  ; customer: Wingtip, item: Less deposit (Invoice 109)\n"
    );
    assert!(
        books
            .open_deposits(&index, "106")
            .expect("lists")
            .deposits
            .is_empty()
    );
    if let Some(out) = hledger(
        &dir,
        &["books.journal"],
        &[
            "bal",
            "tag:customer=Wingtip",
            "liabilities:customer-deposits",
            "--flat",
            "-O",
            "csv",
        ],
    ) {
        // The deposit is used up: nothing left on the liability.
        assert_eq!(out, "\"account\",\"balance\"\n\"Total:\",\"0\"\n");
    }
}

/// A $1,500 Wingtip deposit (109, sent) and a $1,500 final (110, a draft)
/// that its deposit covers in full once applied.
fn fully_covered(
    test: &str,
) -> (
    std::path::PathBuf,
    InvoiceBooks,
    invoicer_core::index::DocumentIndex,
) {
    let dir = scratch("accrual", test);
    write(
        &dir,
        "invoices/2026-11-12 Wingtip Invoice 109.md",
        &wingtip(
            "109",
            "2026-11-12",
            "draft",
            "| Deposit on phase 2      | 1   | $1,500 | $1,500.00 | liabilities:customer-deposits |\n",
            "$1,500.00",
        ),
    );
    write(
        &dir,
        FINAL_110,
        &wingtip(
            "110",
            "2026-11-17",
            "draft",
            "| Phase 2 build           | 1   | $1,500 | $1,500.00 | revenue:consulting            |\n",
            "$1,500.00",
        ),
    );
    let (books, mut index) = open(&dir);
    books
        .send(&mut index, &send("109"))
        .expect("sends the deposit");
    let applied = books
        .apply_deposit(
            &mut index,
            &ApplyDeposit {
                number: "110".into(),
                deposit: "109".into(),
                amount: None,
            },
        )
        .expect("applies");
    assert_eq!(applied.total, Dec::zero());
    (dir, books, index)
}

const FINAL_110: &str = "invoices/2026-11-17 Wingtip Invoice 110.md";

/// 110 as booked when its deposit covers it: the rows against the deposit
/// liability, and no receivable posting.
const ZERO_110: &str = "\
2026-11-17 Wingtip | Invoice 110  ; invoice: 110, document: invoices/2026-11-17 Wingtip Invoice 110.md, due: 2026-12-17
    revenue:consulting             $-1500.00  ; customer: Wingtip, item: Phase 2 build
    liabilities:customer-deposits  $1500.00  ; customer: Wingtip, item: Less deposit (Invoice 109)
";

/// `text` with the rate of the row described `row` changed from `from` to
/// `to`.
fn with_rate(text: &str, row: &str, from: &str, to: &str) -> String {
    text.lines()
        .map(|line| {
            if line.contains(row) {
                line.replacen(&format!("{from} "), &format!("{to} "), 1)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn a_final_invoice_its_deposit_covers_books_with_no_receivable() {
    let (dir, books, mut index) = fully_covered("zero-total");
    let before = read(&dir, "books.journal");
    let sent = books.send(&mut index, &send("110")).expect("sends");
    assert!(sent.journal_written && sent.markdown_written);
    let booked = sent.booked.expect("booked").text;
    assert_eq!(booked, ZERO_110);
    let (removed, added) = diff(&before, &read(&dir, "books.journal"));
    assert_eq!((removed.as_str(), added), ("", format!("\n{booked}")));
    // Not a receivable document; the bridge still finds it, in step.
    assert!(books.load().expect("loads").find_document("110").is_none());
    let preview = books.preview(&index, "110").expect("previews");
    assert_eq!(preview.in_step, Some(true));
    assert_eq!(preview.booked.map(|b| b.text), Some(booked.clone()));
    assert!(!preview.linked);
    // Booked once: again is refused, and nothing is left on the deposit.
    assert!(matches!(
        books.send(&mut index, &send("110")),
        Err(BooksError::Status { .. })
    ));
    assert!(
        books
            .open_deposits(&index, "106")
            .expect("lists")
            .deposits
            .is_empty()
    );

    // Void: the rows reversed, no link (no receivable to net).
    let journal = read(&dir, "books.journal");
    let voided = books.void(&mut index, &void("110", TODAY)).expect("voids");
    assert!(voided.journal_written && voided.link.is_none());
    let reversal = voided.reversal.expect("reversal").text;
    assert_eq!(
        reversal,
        "2026-11-17 Wingtip | Invoice 110 (void)  ; invoice: 110, void: 2026-11-17\n\
         \x20   revenue:consulting             $1500.00  ; customer: Wingtip, item: Phase 2 build\n\
         \x20   liabilities:customer-deposits  $-1500.00  ; customer: Wingtip, item: Less deposit (Invoice 109)\n"
    );
    let (removed, added) = diff(&journal, &read(&dir, "books.journal"));
    assert_eq!((removed.as_str(), added), ("", format!("\n{reversal}")));
    // Voiding it frees the deposit again.
    let open = books.open_deposits(&index, "106").expect("lists");
    assert_eq!(open.balance, Dec::new(150_000, 2));

    if let Some(out) = hledger(
        &dir,
        &["books.journal", "recognition.journal"],
        &["-s", "check"],
    ) {
        assert_eq!(out, "");
    }
    if let Some(out) = hledger(
        &dir,
        &["books.journal"],
        &["bal", "tag:invoice=^110$", "--flat", "-O", "csv"],
    ) {
        // The booking and its void net to nothing.
        assert_eq!(out, "\"account\",\"balance\"\n\"Total:\",\"0\"\n");
    }
}

#[test]
fn a_rebook_moves_between_a_zero_and_a_positive_total() {
    let (dir, books, mut index) = fully_covered("zero-rebook");
    books.send(&mut index, &send("110")).expect("sends");
    let zero = books
        .preview(&index, "110")
        .expect("previews")
        .booked
        .expect("booked")
        .text;
    assert_eq!(zero, ZERO_110);

    // More work: $2,000 less the $1,500 deposit leaves $500 to receive.
    let more = recalculated(
        &with_rate(&read(&dir, FINAL_110), "Phase 2 build", "$1,500", "$2,000"),
        &books,
    );
    write(&dir, FINAL_110, &more);
    let (books, index) = reopen(books);
    let rebooked = books.rebook(&index, &rebook("110")).expect("rebooks");
    assert!(rebooked.changed);
    let positive = rebooked.booked.expect("booked").text;
    assert_eq!(
        positive,
        "2026-11-17 Wingtip | Invoice 110  ; invoice: 110, document: invoices/2026-11-17 Wingtip Invoice 110.md, due: 2026-12-17\n\
         \x20   assets:accounts receivable     $500.00  ; customer: Wingtip\n\
         \x20   revenue:consulting             $-2000.00  ; customer: Wingtip, item: Phase 2 build\n\
         \x20   liabilities:customer-deposits  $1500.00  ; customer: Wingtip, item: Less deposit (Invoice 109)\n"
    );
    let loaded = books.load().expect("loads");
    assert!(loaded.find_document("110").is_some(), "a receivable now");
    assert_eq!(rebooked.previous.map(|b| b.text), Some(zero.clone()));

    // And back: the receivable (unlinked) is dropped again.
    let back = recalculated(
        &with_rate(&read(&dir, FINAL_110), "Phase 2 build", "$2,000", "$1,500"),
        &books,
    );
    write(&dir, FINAL_110, &back);
    let (books, index) = reopen(books);
    let rebooked = books.rebook(&index, &rebook("110")).expect("rebooks");
    assert!(rebooked.changed);
    assert_eq!(rebooked.booked.map(|b| b.text), Some(zero));
    assert!(books.load().expect("loads").find_document("110").is_none());
    if let Some(out) = hledger(
        &dir,
        &["books.journal", "recognition.journal"],
        &["-s", "check"],
    ) {
        assert_eq!(out, "");
    }
}
