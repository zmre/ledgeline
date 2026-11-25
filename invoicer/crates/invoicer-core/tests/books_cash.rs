//! The books bridge on the cash fixture (PLAN 2b, I17): sending writes the
//! markdown only, and Apply payment re-splits an imported deposit into the
//! invoice's rows, keeping its bank leg byte for byte. With
//! `LEDGELINE_HLEDGER_INVOICER_CHECK=1` (inside the dev shell), hledger reads
//! the result too.

mod books_common;

use books_common::*;
use invoicer_core::books::{ApplyPayment, Basis, BooksError, Send, Void};
use ledgeline_core::Dec;
use ledgeline_core::accrual::ops::OpsError;

const TODAY: &str = "2026-11-17";
const INV_203: &str = "invoices/2026-11-05 Wingtip Invoice 203.md";
const INV_204: &str = "invoices/2026-11-17 Customer X Invoice 204.md";

/// Bank imports, as Ledgeline's importer writes them: income on
/// `income:unknown`, one elided, one with a note under the bank leg.
const IMPORTS: &str = "
2026-11-16 * ACH WINGTIP PAYMENTS  ; id: ach-41
    assets:bank:checking  $1,200.00
    ; imported 2026-11-17
    income:unknown

2026-11-17 * WIRE CUSTOMER X INV 207  ; id: wx-7
    assets:bank:checking   $3,500.00  ; bank-ref: 771
    income:unknown        $-3,500.00

2026-11-17 * WIRE CUSTOMER X BALANCE  ; id: wx-8
    assets:bank:checking   $3,500.00
    income:unknown        $-3,500.00

2026-11-17 * WIRE CUSTOMER X LARGE  ; id: wx-9
    assets:bank:checking   $9,000.00
    income:unknown        $-9,000.00
";

fn apply(number: &str, reference: &str) -> ApplyPayment {
    ApplyPayment {
        number: number.into(),
        reference: reference.into(),
        today: TODAY.into(),
        expected: None,
    }
}

#[test]
fn sending_in_cash_books_writes_only_the_markdown() {
    let dir = scratch("cash", "cash-send");
    let (books, mut index) = open(&dir);
    let journal = read(&dir, "books.journal");
    let draft = read(&dir, INV_204);
    let preview = books.preview(&index, "204").expect("previews");
    assert_eq!(preview.basis, Basis::Cash);
    assert_eq!((preview.would_book, preview.booked), (None, None));
    let sent = books
        .send(
            &mut index,
            &Send {
                number: "204".into(),
                today: TODAY.into(),
                sent_via: Some("email".into()),
                expected: None,
            },
        )
        .expect("sends");
    assert_eq!(sent.basis, Basis::Cash);
    assert!(!sent.journal_written && sent.markdown_written && sent.booked.is_none());
    assert_eq!(read(&dir, "books.journal"), journal);
    assert_eq!(
        read(&dir, INV_204),
        replace_once(
            &replace_once(&draft, "status: draft\n", "status: sent\n"),
            "  Anywhere, CA 90000\n---\n",
            "  Anywhere, CA 90000\nsent_date: 2026-11-17\nsent_via: email\npaid: no\n---\n"
        )
    );
}

#[test]
fn apply_payment_in_full_in_parts_and_undone() {
    let dir = scratch("cash", "cash-apply");
    let start = format!("{}{IMPORTS}", read(&dir, "books.journal"));
    write(&dir, "books.journal", &start);
    let (books, mut index) = open(&dir);

    // 203 ($1,200, Wingtip): the ACH pays it exactly and ranks first.
    let candidates = books
        .payment_candidates(&index, "203", TODAY)
        .expect("lists");
    let best = &candidates[0];
    assert_eq!(best.description, "ACH WINGTIP PAYMENTS");
    assert!(best.exact);
    assert_eq!(best.received, Dec::new(120_000, 2));
    let applied = books
        .apply_payment(&index, &apply("203", &best.reference))
        .expect("applies");
    assert_eq!(applied.remaining, Dec::zero());
    assert_eq!(
        applied.transaction.text,
        "2026-11-16 * ACH WINGTIP PAYMENTS  ; id: ach-41, invoice:203\n\
         \x20   assets:bank:checking  $1,200.00\n\
         \x20   ; imported 2026-11-17\n\
         \x20   revenue:consulting  $-1200.00  ; customer: Wingtip\n"
    );
    let journal = read(&dir, "books.journal");
    assert_eq!(
        diff(&start, &journal),
        (
            "2026-11-16 * ACH WINGTIP PAYMENTS  ; id: ach-41\n\
             \x20   assets:bank:checking  $1,200.00\n\
             \x20   ; imported 2026-11-17\n\
             \x20   income:unknown\n"
                .into(),
            applied.transaction.text.clone()
        ),
        "only the deposit changes, and its bank leg and note stay as written"
    );
    // Paid: no more candidates.
    assert!(
        books
            .payment_candidates(&index, "203", TODAY)
            .expect("lists")
            .is_empty()
    );
    // A paid invoice can't be voided; undo the payment first.
    let refused = books.void(
        &mut index,
        &Void {
            number: "203".into(),
            date: TODAY.into(),
            today: TODAY.into(),
            expected: None,
        },
    );
    assert!(
        matches!(refused, Err(BooksError::Refused(_))),
        "{refused:?}"
    );

    // Undo restores the import byte for byte; a second undo finds nothing.
    let undo: invoicer_core::books::PaymentUndo =
        serde_json::from_str(&serde_json::to_string(&applied.undo).expect("json"))
            .expect("round-trips through JSON, for a later process");
    books.undo_payment(&undo, None).expect("undoes");
    assert_eq!(read(&dir, "books.journal"), start);
    assert!(matches!(
        books.undo_payment(&undo, None),
        Err(BooksError::Ops(OpsError::Stale))
    ));
    books
        .apply_payment(&index, &apply("203", &best.reference))
        .expect("applies again");
    assert_eq!(read(&dir, "books.journal"), journal);

    // 207 ($7,000: $12,000 of work less the $5,000 deposit 206 paid). In
    // cash books the deposit was income when it arrived (into
    // revenue:consulting), so the deposit row books nothing. By default
    // (match-rows) the $7,000 goes where the rows still lack it:
    // consulting $8,000 − $5,000 = $3,000, delivery $4,000.
    // The $9,000 wire is more than is due: never a candidate.
    let candidates = books
        .payment_candidates(&index, "207", TODAY)
        .expect("lists");
    let names: Vec<&str> = candidates.iter().map(|c| c.description.as_str()).collect();
    assert_eq!(
        names,
        ["WIRE CUSTOMER X INV 207", "WIRE CUSTOMER X BALANCE"]
    );
    assert!(!candidates[0].exact && candidates[0].by_number);

    // Half now: pro rata over what each row still lacks (3,000 : 4,000).
    let first = books
        .apply_payment(&index, &apply("207", &candidates[0].reference))
        .expect("applies");
    assert_eq!(first.remaining, Dec::new(350_000, 2));
    assert_eq!(
        first.applied,
        [
            ("revenue:consulting".to_string(), "1500.00".to_string()),
            (
                "revenue:consulting:delivery".to_string(),
                "2000.00".to_string()
            ),
        ]
    );
    assert_eq!(
        first.transaction.text,
        "2026-11-17 * WIRE CUSTOMER X INV 207  ; id: wx-7, invoice:207\n\
         \x20   assets:bank:checking   $3,500.00  ; bank-ref: 771\n\
         \x20   revenue:consulting           $-1500.00  ; customer: Customer X\n\
         \x20   revenue:consulting:delivery  $-2000.00  ; customer: Customer X\n"
    );
    // The rest: each row's remainder; it is paid.
    let rest = books
        .payment_candidates(&index, "207", TODAY)
        .expect("lists");
    assert_eq!(rest.len(), 1);
    assert!(rest[0].exact);
    let second = books
        .apply_payment(&index, &apply("207", &rest[0].reference))
        .expect("applies");
    assert_eq!(second.remaining, Dec::zero());
    assert_eq!(
        second.applied,
        [
            ("revenue:consulting".to_string(), "1500.00".to_string()),
            (
                "revenue:consulting:delivery".to_string(),
                "2000.00".to_string()
            ),
        ]
    );
    assert!(
        books
            .payment_candidates(&index, "207", TODAY)
            .expect("lists")
            .is_empty()
    );

    // 206's deposit is used up by 207; nothing is open for Customer X.
    let open = books.open_deposits(&index, "204").expect("lists");
    assert!(open.deposits.is_empty(), "{open:?}");
    assert_eq!(open.balance, Dec::zero());

    // A reference that no longer names a candidate writes nothing.
    let journal = read(&dir, "books.journal");
    let gone = books.apply_payment(&index, &apply("203", &best.reference));
    assert!(matches!(gone, Err(BooksError::Refused(_))), "{gone:?}");
    assert_eq!(read(&dir, "books.journal"), journal);

    if let Some(out) = hledger(&dir, &["books.journal"], &["-s", "check"]) {
        assert_eq!(out, "");
    }
    if let Some(out) = hledger(
        &dir,
        &["books.journal"],
        &["bal", "tag:invoice=^20[67]$", "--flat", "-O", "csv"],
    ) {
        // 206's deposit (income on receipt) and 207's two payments: $12,000
        // in the bank, and revenue matching 207's rows; the liability never
        // moves.
        assert_eq!(
            out,
            "\"account\",\"balance\"\n\
             \"assets:bank:checking\",\"$12000.00\"\n\
             \"revenue:consulting\",\"$-8000.00\"\n\
             \"revenue:consulting:delivery\",\"$-4000.00\"\n\
             \"Total:\",\"0\"\n"
        );
    }
}

/// RC-9: a partial payment can leave the largest row exactly paid while a
/// smaller one still lacks a cent; the last payment's split must leave the
/// paid row out (Ledgeline refuses a zero row), not send it as zero.
#[test]
fn paying_the_rest_after_a_row_is_exactly_paid_skips_that_row() {
    let dir = scratch("cash", "cash-apply-zero-row");
    let wires = "
2026-11-17 * WIRE CUSTOMER X MOST  ; id: wx-1
    assets:bank:checking   $6,999.99
    income:unknown        $-6,999.99

2026-11-17 * WIRE CUSTOMER X CENT  ; id: wx-2
    assets:bank:checking   $0.01
    income:unknown        $-0.01
";
    let start = format!("{}{wires}", read(&dir, "books.journal"));
    write(&dir, "books.journal", &start);
    let (books, index) = open(&dir);
    // 207 lacks $3,000 on consulting and $4,000 on delivery: $6,999.99
    // truncates consulting to $2,999.99 and delivery takes exactly $4,000.
    let most = books
        .payment_candidates(&index, "207", TODAY)
        .expect("lists")
        .into_iter()
        .find(|c| c.description == "WIRE CUSTOMER X MOST")
        .expect("a candidate");
    let first = books
        .apply_payment(&index, &apply("207", &most.reference))
        .expect("applies");
    assert_eq!(first.remaining, Dec::new(1, 2));
    let cent = books
        .payment_candidates(&index, "207", TODAY)
        .expect("lists")
        .into_iter()
        .find(|c| c.description == "WIRE CUSTOMER X CENT")
        .expect("a candidate");
    let last = books
        .apply_payment(&index, &apply("207", &cent.reference))
        .expect("the last cent applies");
    assert_eq!(last.remaining, Dec::zero());
    assert_eq!(
        last.applied,
        [("revenue:consulting".to_string(), "0.01".to_string())]
    );
}

#[test]
fn apply_payment_is_for_sent_invoices_in_cash_books() {
    let dir = scratch("cash", "cash-refusals");
    let (books, index) = open(&dir);
    let draft = books.payment_candidates(&index, "204", TODAY);
    assert!(matches!(draft, Err(BooksError::Status { .. })), "{draft:?}");
    let void = books.payment_candidates(&index, "205", TODAY);
    assert!(matches!(void, Err(BooksError::Status { .. })), "{void:?}");

    let dir = scratch("accrual", "accrual-apply");
    let (books, index) = open(&dir);
    let accrual = books.payment_candidates(&index, "106", TODAY);
    assert!(
        matches!(accrual, Err(BooksError::Refused(_))),
        "{accrual:?}"
    );
}

/// The dry run shows exactly what the write then writes, and writes
/// nothing; it refuses what the write refuses.
#[test]
fn preview_payment_shows_what_apply_payment_writes() {
    let dir = scratch("cash", "cash-preview");
    let start = format!("{}{IMPORTS}", read(&dir, "books.journal"));
    write(&dir, "books.journal", &start);
    let (books, index) = open(&dir);
    let candidates = books
        .payment_candidates(&index, "207", TODAY)
        .expect("lists");
    let half = apply("207", &candidates[0].reference);

    let preview = books.preview_payment(&index, &half).expect("previews");
    assert_eq!(
        read(&dir, "books.journal"),
        start,
        "a preview writes nothing"
    );
    assert_eq!(preview.deposit, candidates[0]);
    assert!(preview.partial);
    assert_eq!(preview.received, Dec::new(350_000, 2));
    assert_eq!(preview.remaining, Dec::new(350_000, 2));

    let applied = books.apply_payment(&index, &half).expect("applies");
    assert_eq!(preview.applied, applied.applied);
    assert_eq!(preview.received, applied.received);
    assert_eq!(preview.remaining, applied.remaining);

    // Applied, it is no longer a candidate: the preview refuses it too.
    let again = books.preview_payment(&index, &half);
    assert!(matches!(again, Err(BooksError::Refused(_))), "{again:?}");
    // The last one pays the rest: not partial.
    let rest = books
        .payment_candidates(&index, "207", TODAY)
        .expect("lists");
    let last = books
        .preview_payment(&index, &apply("207", &rest[0].reference))
        .expect("previews");
    assert!(!last.partial);
    assert_eq!(last.remaining, Dec::zero());

    let dir = scratch("accrual", "accrual-preview");
    let (books, index) = open(&dir);
    let accrual = books.preview_payment(&index, &apply("106", "abcdef"));
    assert!(
        matches!(accrual, Err(BooksError::Refused(_))),
        "{accrual:?}"
    );
}

#[test]
fn voiding_an_unpaid_cash_invoice_writes_only_the_markdown() {
    let dir = scratch("cash", "cash-void");
    let (books, mut index) = open(&dir);
    let journal = read(&dir, "books.journal");
    let voided = books
        .void(
            &mut index,
            &Void {
                number: "203".into(),
                date: TODAY.into(),
                today: TODAY.into(),
                expected: None,
            },
        )
        .expect("voids");
    assert!(!voided.journal_written && voided.markdown_written);
    assert_eq!(read(&dir, "books.journal"), journal);
    assert!(read(&dir, INV_203).contains("status: void\n"));
    assert!(read(&dir, INV_203).contains("void_date: 2026-11-17\n---\n"));
}

#[test]
fn a_deposit_is_income_on_receipt_and_a_covered_final_books_a_zero_marker() {
    // Wingtip: a $1,000 deposit invoice (208), paid by wire; then a $1,000
    // final (209) the deposit covers in full.
    let dir = scratch("cash", "cash-zero");
    write(
        &dir,
        "invoices/2026-11-10 Wingtip Invoice 208.md",
        &wingtip(
            "208",
            "2026-11-10",
            "draft",
            "| Deposit on phase 2      | 1   | $1,000 | $1,000.00 | liabilities:customer-deposits |\n",
            "$1,000.00",
        ),
    );
    let final_209 = "invoices/2026-11-17 Wingtip Invoice 209.md";
    write(
        &dir,
        final_209,
        &wingtip(
            "209",
            "2026-11-17",
            "draft",
            "| Phase 2 build           | 1   | $1,000 | $1,000.00 | revenue:consulting            |\n",
            "$1,000.00",
        ),
    );
    let journal = format!(
        "{}\n2026-11-12 * WIRE WINGTIP DEPOSIT  ; id: ww-1\n    assets:bank:checking  $1,000.00\n    income:unknown\n",
        read(&dir, "books.journal")
    );
    write(&dir, "books.journal", &journal);
    let (books, mut index) = open(&dir);
    let send = |number: &str| Send {
        number: number.into(),
        today: TODAY.into(),
        sent_via: None,
        expected: None,
    };
    books.send(&mut index, &send("208")).expect("sends");

    // The wire is income the day it arrives: Wingtip has no revenue_account,
    // so `[invoicer] revenue-account` takes it, not the deposit liability.
    let wire = books
        .payment_candidates(&index, "208", TODAY)
        .expect("lists")
        .into_iter()
        .next()
        .expect("the wire");
    let paid = books
        .apply_payment(&index, &apply("208", &wire.reference))
        .expect("applies");
    assert_eq!(
        paid.transaction.text,
        "2026-11-12 * WIRE WINGTIP DEPOSIT  ; id: ww-1, invoice:208\n\
         \x20   assets:bank:checking  $1,000.00\n\
         \x20   revenue:consulting  $-1000.00  ; customer: Wingtip\n"
    );
    // Still a credit for Wingtip to apply: what 208 was paid.
    let open_deposits = books.open_deposits(&index, "209").expect("lists");
    assert_eq!(open_deposits.deposits.len(), 1);
    assert_eq!(open_deposits.deposits[0].number.as_deref(), Some("208"));
    assert_eq!(open_deposits.deposits[0].remaining, Dec::new(100_000, 2));
    let applied = books
        .apply_deposit(
            &mut index,
            &invoicer_core::books::ApplyDeposit {
                number: "209".into(),
                deposit: "208".into(),
                amount: None,
            },
        )
        .expect("applies the deposit");
    assert_eq!(applied.total, Dec::zero());

    // Send 209: a $0 marker, so the books show it was sent; paid on its date.
    let before = read(&dir, "books.journal");
    let markdown = read(&dir, final_209);
    let sent = books.send(&mut index, &send("209")).expect("sends");
    assert!(sent.journal_written && sent.markdown_written);
    let booked = sent.booked.expect("booked").text;
    assert_eq!(
        booked,
        "2026-11-17 Wingtip | Invoice 209  ; invoice: 209, document: invoices/2026-11-17 Wingtip Invoice 209.md, due: 2026-12-17\n\
         \x20   revenue:consulting             $0.00  ; customer: Wingtip, item: Phase 2 build\n\
         \x20   liabilities:customer-deposits  $0.00  ; customer: Wingtip, item: Less deposit (Invoice 208)\n"
    );
    let (removed, added) = diff(&before, &read(&dir, "books.journal"));
    assert_eq!((removed.as_str(), added), ("", format!("\n{booked}")));
    assert_eq!(
        read(&dir, final_209),
        replace_once(
            &replace_once(&markdown, "status: draft\n", "status: sent\n"),
            "  1234 Background Blvd.\n---\n",
            "  1234 Background Blvd.\nsent_date: 2026-11-17\npaid: 2026-11-17\n---\n"
        )
    );
    assert!(
        books
            .payment_candidates(&index, "209", TODAY)
            .expect("lists")
            .is_empty()
    );
    assert!(
        books
            .open_deposits(&index, "203")
            .expect("lists")
            .deposits
            .is_empty()
    );
    let preview = books.preview(&index, "209").expect("previews");
    assert_eq!(preview.in_step, Some(true));
    assert_eq!(preview.receivable, None);
    // An ordinary cash-books invoice still books nothing at send.
    assert!(
        books
            .preview(&index, "203")
            .expect("previews")
            .would_book
            .is_none()
    );
    if let Some(out) = hledger(&dir, &["books.journal"], &["-s", "check"]) {
        assert_eq!(out, "");
    }

    // Void 209: the marker reversed. The deposit is Wingtip's again, for a
    // reissue.
    let journal = read(&dir, "books.journal");
    let voided = books
        .void(
            &mut index,
            &Void {
                number: "209".into(),
                date: TODAY.into(),
                today: TODAY.into(),
                expected: None,
            },
        )
        .expect("voids");
    assert!(voided.journal_written && voided.link.is_none());
    let reversal = voided.reversal.expect("reversal").text;
    assert_eq!(
        reversal,
        "2026-11-17 Wingtip | Invoice 209 (void)  ; invoice: 209, void: 2026-11-17\n\
         \x20   revenue:consulting             $0.00  ; customer: Wingtip, item: Phase 2 build\n\
         \x20   liabilities:customer-deposits  $0.00  ; customer: Wingtip, item: Less deposit (Invoice 208)\n"
    );
    let (removed, added) = diff(&journal, &read(&dir, "books.journal"));
    assert_eq!((removed.as_str(), added), ("", format!("\n{reversal}")));
    let open_again = books.open_deposits(&index, "203").expect("lists");
    assert_eq!(open_again.balance, Dec::new(100_000, 2));

    if let Some(out) = hledger(&dir, &["books.journal"], &["-s", "check"]) {
        assert_eq!(out, "");
    }
    if let Some(out) = hledger(
        &dir,
        &["books.journal"],
        &["bal", "tag:invoice=^20[89]$", "--flat", "-O", "csv"],
    ) {
        // The deposit was revenue on receipt; the marker and its void move
        // nothing.
        assert_eq!(
            out,
            "\"account\",\"balance\"\n\
             \"assets:bank:checking\",\"$1000.00\"\n\
             \"revenue:consulting\",\"$-1000.00\"\n\
             \"Total:\",\"0\"\n"
        );
    }
}

/// Cash books with `extra` added to `[invoicer]` and `accounts` declared:
/// Wingtip's deposit invoice 208 for `deposit` (a wire, applied), and a
/// final 209 with `rows`, the deposit applied, sent; then a wire paying it.
fn deposit_then_final(
    test: &str,
    extra: &str,
    accounts: &str,
    deposit: &str,
    rows: &str,
    subtotal: &str,
) -> (
    std::path::PathBuf,
    invoicer_core::books::InvoiceBooks,
    invoicer_core::index::DocumentIndex,
) {
    let dir = scratch("cash", test);
    write(
        &dir,
        "ledgeline.toml",
        &format!("{}{extra}", read(&dir, "ledgeline.toml")),
    );
    write(
        &dir,
        "invoices/2026-11-10 Wingtip Invoice 208.md",
        &wingtip(
            "208",
            "2026-11-10",
            "draft",
            &format!(
                "| Deposit on phase 2      | 1   | {deposit} | {deposit} | liabilities:customer-deposits |\n"
            ),
            deposit,
        ),
    );
    write(
        &dir,
        "invoices/2026-11-17 Wingtip Invoice 209.md",
        &wingtip("209", "2026-11-17", "draft", rows, subtotal),
    );
    let journal = read(&dir, "books.journal").replacen(
        "account equity:opening",
        &format!("{accounts}account equity:opening"),
        1,
    );
    write(
        &dir,
        "books.journal",
        &format!(
            "{journal}\n2026-11-12 * WIRE WINGTIP DEPOSIT  ; id: ww-1\n    assets:bank:checking  {deposit}\n    income:unknown\n"
        ),
    );
    let (books, mut index) = open(&dir);
    let send = |number: &str| Send {
        number: number.into(),
        today: TODAY.into(),
        sent_via: None,
        expected: None,
    };
    books.send(&mut index, &send("208")).expect("sends 208");
    let wire = books
        .payment_candidates(&index, "208", TODAY)
        .expect("lists")
        .into_iter()
        .next()
        .expect("the deposit wire");
    books
        .apply_payment(&index, &apply("208", &wire.reference))
        .expect("applies the deposit");
    books
        .apply_deposit(
            &mut index,
            &invoicer_core::books::ApplyDeposit {
                number: "209".into(),
                deposit: "208".into(),
                amount: None,
            },
        )
        .expect("applies the deposit row");
    books.send(&mut index, &send("209")).expect("sends 209");
    (dir, books, index)
}

/// Pay 209 with a wire of `amount` (text) dated today.
fn pay_209(
    dir: &std::path::Path,
    books: invoicer_core::books::InvoiceBooks,
    amount: &str,
    id: &str,
) -> (
    invoicer_core::books::InvoiceBooks,
    invoicer_core::books::PaymentApplied,
) {
    write(
        dir,
        "books.journal",
        &format!(
            "{}\n2026-11-17 * WIRE WINGTIP INV 209 {id}  ; id: {id}\n    assets:bank:checking  {amount}\n    income:unknown\n",
            read(dir, "books.journal")
        ),
    );
    let (books, index) = reopen(books);
    let wire = books
        .payment_candidates(&index, "209", TODAY)
        .expect("lists")
        .into_iter()
        .find(|c| c.description.ends_with(id))
        .expect("the wire");
    let paid = books
        .apply_payment(&index, &apply("209", &wire.reference))
        .expect("applies");
    (books, paid)
}

fn applied(paid: &invoicer_core::books::PaymentApplied) -> Vec<(&str, &str)> {
    paid.applied
        .iter()
        .map(|(a, d)| (a.as_str(), d.as_str()))
        .collect()
}

#[test]
fn pro_rata_allocation_splits_every_payment_like_the_rows() {
    // `payment-allocation = "pro-rata"`: 207's $7,000 over its $8,000 and
    // $4,000 of work, whatever the deposit already booked.
    let dir = scratch("cash", "cash-pro-rata");
    write(
        &dir,
        "ledgeline.toml",
        &format!(
            "{}payment-allocation = \"pro-rata\"\n",
            read(&dir, "ledgeline.toml")
        ),
    );
    write(
        &dir,
        "books.journal",
        &format!("{}{IMPORTS}", read(&dir, "books.journal")),
    );
    let (books, index) = open(&dir);
    let candidates = books
        .payment_candidates(&index, "207", TODAY)
        .expect("lists");
    let first = books
        .apply_payment(&index, &apply("207", &candidates[0].reference))
        .expect("applies");
    assert_eq!(
        applied(&first),
        [
            ("revenue:consulting", "2333.34"),
            ("revenue:consulting:delivery", "1166.66")
        ]
    );
    let rest = books
        .payment_candidates(&index, "207", TODAY)
        .expect("lists");
    let second = books
        .apply_payment(&index, &apply("207", &rest[0].reference))
        .expect("applies");
    assert_eq!(
        applied(&second),
        [
            ("revenue:consulting", "2333.33"),
            ("revenue:consulting:delivery", "1166.67")
        ]
    );
    if let Some(out) = hledger(
        &dir,
        &["books.journal"],
        &["bal", "tag:invoice=^20[67]$", "--flat", "-O", "csv"],
    ) {
        assert_eq!(
            out,
            "\"account\",\"balance\"\n\
             \"assets:bank:checking\",\"$12000.00\"\n\
             \"revenue:consulting\",\"$-9666.67\"\n\
             \"revenue:consulting:delivery\",\"$-2333.33\"\n\
             \"Total:\",\"0\"\n"
        );
    }
}

#[test]
fn a_deposit_in_an_account_no_row_uses_stays_there() {
    // The $1,000 deposit went to revenue:consulting; 209's rows are
    // delivery $3,000 and support $1,000, so $3,000 is due. Consulting keeps
    // the deposit; the $1,000 the rows give up comes off them pro rata:
    // delivery $2,250, support $750. Paid in two parts.
    let (dir, books, _) = deposit_then_final(
        "cash-unused-account",
        "",
        "account revenue:support                ; type: R\n",
        "$1,000.00",
        "| Phase 2 delivery        | 1   | $3,000 | $3,000.00 | revenue:consulting:delivery   |\n\
         | Support                 | 1   | $1,000 | $1,000.00 | revenue:support               |\n",
        "$4,000.00",
    );
    let (books, first) = pay_209(&dir, books, "$1,200.00", "w1");
    assert_eq!(
        applied(&first),
        [
            ("revenue:consulting:delivery", "900.00"),
            ("revenue:support", "300.00")
        ]
    );
    let (_, second) = pay_209(&dir, books, "$1,800.00", "w2");
    assert_eq!(second.remaining, Dec::zero());
    assert_eq!(
        applied(&second),
        [
            ("revenue:consulting:delivery", "1350.00"),
            ("revenue:support", "450.00")
        ]
    );
    if let Some(out) = hledger(&dir, &["books.journal"], &["-s", "check"]) {
        assert_eq!(out, "");
    }
    if let Some(out) = hledger(
        &dir,
        &["books.journal"],
        &["bal", "tag:invoice=^20[89]$", "--flat", "-O", "csv"],
    ) {
        assert_eq!(
            out,
            "\"account\",\"balance\"\n\
             \"assets:bank:checking\",\"$4000.00\"\n\
             \"revenue:consulting\",\"$-1000.00\"\n\
             \"revenue:consulting:delivery\",\"$-2250.00\"\n\
             \"revenue:support\",\"$-750.00\"\n\
             \"Total:\",\"0\"\n"
        );
    }
}

#[test]
fn a_deposit_larger_than_its_row_comes_off_the_others() {
    // The $5,000 deposit went to revenue:consulting, but 209 bills only
    // $3,000 of consulting (and $9,000 of delivery): consulting gets nothing
    // more, never a negative share, and the whole $7,000 goes to delivery.
    // Consulting ends $2,000 over its row: the deposit can't be moved.
    let (dir, books, _) = deposit_then_final(
        "cash-deposit-over-row",
        "",
        "",
        "$5,000.00",
        "| Phase 2 build           | 1   | $3,000 | $3,000.00 | revenue:consulting            |\n\
         | Phase 2 delivery        | 1   | $9,000 | $9,000.00 | revenue:consulting:delivery   |\n",
        "$12,000.00",
    );
    let (_, paid) = pay_209(&dir, books, "$7,000.00", "w1");
    assert_eq!(applied(&paid), [("revenue:consulting:delivery", "7000.00")]);
    if let Some(out) = hledger(
        &dir,
        &["books.journal"],
        &["bal", "tag:invoice=^20[89]$", "--flat", "-O", "csv"],
    ) {
        assert_eq!(
            out,
            "\"account\",\"balance\"\n\
             \"assets:bank:checking\",\"$12000.00\"\n\
             \"revenue:consulting\",\"$-5000.00\"\n\
             \"revenue:consulting:delivery\",\"$-7000.00\"\n\
             \"Total:\",\"0\"\n"
        );
    }
}

#[test]
fn liability_until_final_holds_the_deposit_and_the_final_clears_it() {
    // `cash-deposits = "liability-until-final"`: 208's wire goes to the
    // liability; 209's payment is split over its rows with the deposit row
    // debiting the liability (the rows match by construction).
    let (dir, books, index) = deposit_then_final(
        "cash-liability",
        "cash-deposits = \"liability-until-final\"\n",
        "",
        "$1,000.00",
        "| Phase 2 build           | 1   | $3,000 | $3,000.00 | revenue:consulting            |\n",
        "$3,000.00",
    );
    assert!(
        read(&dir, "books.journal").contains(
            "2026-11-12 * WIRE WINGTIP DEPOSIT  ; id: ww-1, invoice:208\n    assets:bank:checking  $1,000.00\n    liabilities:customer-deposits  $-1000.00  ; customer: Wingtip\n"
        ),
        "the deposit is held in the liability"
    );
    // Applied by 209's file: nothing left open.
    assert!(
        books
            .open_deposits(&index, "203")
            .expect("lists")
            .deposits
            .is_empty()
    );
    let (_, paid) = pay_209(&dir, books, "$2,000.00", "w1");
    assert_eq!(
        applied(&paid),
        [
            ("revenue:consulting", "3000.00"),
            ("liabilities:customer-deposits", "-1000.00")
        ]
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
        assert_eq!(out, "\"account\",\"balance\"\n\"Total:\",\"0\"\n");
    }
}

#[test]
fn liability_until_final_books_a_covered_final_against_the_liability() {
    let (dir, books, index) = deposit_then_final(
        "cash-liability-zero",
        "cash-deposits = \"liability-until-final\"\n",
        "",
        "$1,000.00",
        "| Phase 2 build           | 1   | $1,000 | $1,000.00 | revenue:consulting            |\n",
        "$1,000.00",
    );
    let booked = books
        .preview(&index, "209")
        .expect("previews")
        .booked
        .expect("booked at send")
        .text;
    assert_eq!(
        booked,
        "2026-11-17 Wingtip | Invoice 209  ; invoice: 209, document: invoices/2026-11-17 Wingtip Invoice 209.md, due: 2026-12-17\n\
         \x20   revenue:consulting             $-1000.00  ; customer: Wingtip, item: Phase 2 build\n\
         \x20   liabilities:customer-deposits  $1000.00  ; customer: Wingtip, item: Less deposit (Invoice 208)\n"
    );
    if let Some(out) = hledger(&dir, &["books.journal"], &["-s", "check"]) {
        assert_eq!(out, "");
    }
}
