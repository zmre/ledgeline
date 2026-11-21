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

    // 207 ($7,000: $12,000 of work less the $5,000 deposit 206 paid).
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

    // Half now: every row pro rata, the deposit row debited.
    let first = books
        .apply_payment(&index, &apply("207", &candidates[0].reference))
        .expect("applies");
    assert_eq!(first.remaining, Dec::new(350_000, 2));
    assert_eq!(
        first.applied,
        [
            ("revenue:consulting".to_string(), "4000.00".to_string()),
            (
                "revenue:consulting:delivery".to_string(),
                "2000.00".to_string()
            ),
            (
                "liabilities:customer-deposits".to_string(),
                "-2500.00".to_string()
            ),
        ]
    );
    assert_eq!(
        first.transaction.text,
        "2026-11-17 * WIRE CUSTOMER X INV 207  ; id: wx-7, invoice:207\n\
         \x20   assets:bank:checking   $3,500.00  ; bank-ref: 771\n\
         \x20   revenue:consulting             $-4000.00  ; customer: Customer X\n\
         \x20   revenue:consulting:delivery    $-2000.00  ; customer: Customer X\n\
         \x20   liabilities:customer-deposits  $2500.00  ; customer: Customer X\n"
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
        // 206's deposit and 207's two payments: $12,000 of revenue, the
        // liability back to zero.
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
fn a_final_invoice_its_deposit_covers_books_at_send_in_cash_books() {
    // Wingtip: a $1,000 deposit invoice (208), paid by wire and applied;
    // then a $1,000 final (209) the deposit covers. No payment will ever
    // arrive for 209, so sending it moves the revenue off the liability.
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
    let wire = books
        .payment_candidates(&index, "208", TODAY)
        .expect("lists")
        .into_iter()
        .next()
        .expect("the wire");
    books
        .apply_payment(&index, &apply("208", &wire.reference))
        .expect("applies");
    let open_deposits = books.open_deposits(&index, "209").expect("lists");
    assert_eq!(open_deposits.deposits.len(), 1);
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

    let before = read(&dir, "books.journal");
    let sent = books.send(&mut index, &send("209")).expect("sends");
    assert!(sent.journal_written && sent.markdown_written);
    let booked = sent.booked.expect("booked").text;
    assert_eq!(
        booked,
        "2026-11-17 Wingtip | Invoice 209  ; invoice: 209, document: invoices/2026-11-17 Wingtip Invoice 209.md, due: 2026-12-17\n\
         \x20   revenue:consulting             $-1000.00  ; customer: Wingtip, item: Phase 2 build\n\
         \x20   liabilities:customer-deposits  $1000.00  ; customer: Wingtip, item: Less deposit (Invoice 208)\n"
    );
    let (removed, added) = diff(&before, &read(&dir, "books.journal"));
    assert_eq!((removed.as_str(), added), ("", format!("\n{booked}")));
    // Nothing to pay, nothing left on the deposit; the bridge finds it.
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
    assert_eq!(
        books.preview(&index, "209").expect("previews").in_step,
        Some(true)
    );
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
