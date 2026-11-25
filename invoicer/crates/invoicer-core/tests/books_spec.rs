//! What an invoice books (PLAN 2b, ARCHITECTURE §4.2), on an in-memory
//! books folder: §3.3's worked example, the header tags, the customer, and
//! the setup errors.

mod common;

use invoicer_core::books::spec::{self, tag_text};
use invoicer_core::books::{BooksError, InvoiceBooks};
use invoicer_core::doc::Money;
use invoicer_core::index::DocumentIndex;
use invoicer_core::settings::Settings;
use ledgeline_core::Dec;
use ledgeline_core::vault::{MemVault, VaultPath};
use std::sync::Arc;

const JOURNAL: &str = "\
commodity $1,000.00

account assets:bank:checking            ; type: C
account assets:accounts receivable      ; type: A, kpi: receivable
account liabilities:deferred-revenue    ; type: L, kpi: deferred-revenue
account liabilities:customer-deposits   ; type: L
account liabilities:sales-tax           ; type: L
account revenue:projects                ; type: R
account revenue:consulting              ; type: R
account revenue:support                 ; type: R
";

const ACME: &str = "---\ntitle: Acme\ntype: organization\ntags: [customer]\nledger_customer: Acme Corp\nrevenue_account: revenue:consulting\n---\n";
const ANN: &str =
    "---\ntitle: Ann Lee\ntype: person\ntags: [customer]\ncompany: \"[[Acme]]\"\n---\n";

/// An invoice to `customer` with `table` as its bill table.
fn invoice(number: &str, customer: &str, reference: &str, table: &str) -> String {
    format!(
        "---\ntitle: Invoice {number}\ntype: invoice\ntemplate: /invoices/tpl/x.html\nstatus: draft\n\
         invoice_num: {number}\ndate: 2026-11-17\nterms: 15\ncustomer: \"[[{customer}]]\"\n\
         invoicee_name: {customer}\ninvoicer: \"[[Me]]\"\ninvoicer_name: Me\n{reference}---\n\n{table}"
    )
}

fn books(files: &[(&str, &str)], journal: &str) -> (InvoiceBooks, DocumentIndex) {
    let vault = files.iter().fold(
        MemVault::new()
            .with_file("books.journal", journal)
            .with_file("customers/Acme.md", ACME)
            .with_file("customers/Ann Lee.md", ANN),
        |vault, (path, text)| vault.with_file(path, text),
    );
    let books = InvoiceBooks::open_in(
        Arc::new(vault),
        VaultPath::new("books.journal").expect("path"),
    );
    let loaded = books.load().expect("loads");
    let settings = Settings::read(books.vault()).expect("settings");
    let index = DocumentIndex::open(books.vault(), settings, Money::of_journal(loaded.journal()));
    (books, index)
}

#[test]
fn the_worked_example_books_as_the_architecture_says() {
    let (books, index) = books(
        &[(
            "invoices/106.md",
            &invoice("106", "Acme", "", &common::worked_example_3_3()),
        )],
        JOURNAL,
    );
    let preview = books.preview(&index, "106").expect("previews");
    let lines: Vec<(&str, Option<&str>, Dec, Option<&str>)> = preview
        .booking
        .lines
        .iter()
        .map(|l| {
            (
                l.account.as_str(),
                l.recognize_into.as_deref(),
                l.amount,
                l.item.as_deref(),
            )
        })
        .collect();
    // §3.3: the 2% spreads over the item-derived lines only; tax and the
    // deposit keep their own accounts.
    assert_eq!(
        lines,
        [
            (
                "revenue:projects",
                None,
                Dec::new(1_176_000, 2),
                Some("Website redesign")
            ),
            (
                "revenue:consulting",
                None,
                Dec::new(220_500, 2),
                Some("Senior architect hours")
            ),
            (
                "liabilities:deferred-revenue",
                Some("revenue:support"),
                Dec::new(588_000, 2),
                Some("Annual support plan")
            ),
            (
                "liabilities:sales-tax",
                None,
                Dec::new(163_721, 2),
                Some("Sales tax")
            ),
            (
                "liabilities:customer-deposits",
                None,
                Dec::new(-500_000, 2),
                Some("Less deposit (Invoice 104)")
            ),
        ]
    );
    assert_eq!(preview.booking.total, Dec::new(1_648_221, 2));
    let would = preview.would_book.expect("accrual books");
    let would = would
        .split("recognition: ")
        .enumerate()
        .map(|(n, part)| {
            if n == 0 {
                part.to_string()
            } else {
                format!(
                    "recognition: ID{}",
                    &part[part.find(',').expect("terms follow")..]
                )
            }
        })
        .collect::<String>();
    assert_eq!(
        would,
        "2026-11-17 Acme | Invoice 106  ; invoice: 106, document: invoices/106.md, due: 2026-12-02\n\
         \x20   assets:accounts receivable     $16482.21  ; customer: Acme Corp\n\
         \x20   revenue:projects               $-11760.00  ; customer: Acme Corp, item: Website redesign\n\
         \x20   revenue:consulting             $-2205.00  ; customer: Acme Corp, item: Senior architect hours\n\
         \x20   liabilities:deferred-revenue   $-5880.00  ; customer: Acme Corp, recognition: ID, service-start: 2026-12-01, term-months: 12, item: Annual support plan\n\
         \x20   liabilities:sales-tax          $-1637.21  ; customer: Acme Corp, item: Sales tax\n\
         \x20   liabilities:customer-deposits  $5000.00  ; customer: Acme Corp, item: Less deposit (Invoice 104)\n"
    );
}

#[test]
fn header_tags_items_and_the_customer() {
    let table = "\
| Description          | Qty | Rate   | Amount    | *Account* |
|:---------------------|----:|-------:|----------:|:----------|
| Design, phase 1      | 1   | $1,000 | $1,000.00 |           |
| Design \\| review     | 1   | $500   | $500.00   |           |
|                      |     | **Total due** | **$1,500.00** | |
";
    let (books, index) = books(
        &[(
            "invoices/Acme, Inc 107.md",
            &invoice("107", "Ann Lee", "reference: \"PO #1, rev. 2\"\n", table),
        )],
        JOURNAL,
    );
    let preview = books.preview(&index, "107").expect("previews");
    let booking = &preview.booking;
    // Ann buys for Acme: the books say Acme, by its ledger name. Blank cells
    // take Acme's revenue account, and merge into one line naming both rows.
    assert_eq!(booking.customer, "Acme Corp");
    assert_eq!(booking.description, "Ann Lee | Invoice 107");
    assert_eq!(booking.lines.len(), 1);
    assert_eq!(booking.lines[0].account, "revenue:consulting");
    assert_eq!(
        booking.lines[0].item.as_deref(),
        Some("Design phase 1 + Design | review")
    );
    // Commas never reach a tag value; `due:` is date + terms.
    assert_eq!(
        booking.tags,
        [
            (
                "document".to_string(),
                "invoices/Acme Inc 107.md".to_string()
            ),
            ("due".to_string(), "2026-12-02".to_string()),
            ("reference".to_string(), "PO #1 rev. 2".to_string()),
        ]
    );
    assert_eq!(tag_text("$16,700, net 30"), Some("$16700 net 30".into()));
    assert_eq!(tag_text(" , "), None);
}

/// RC-3: Ledgeline refuses `;` and `#` in a description, so a payee or
/// number holding one is written without it rather than blocking the send.
#[test]
fn a_payee_or_number_with_a_comment_mark_still_books() {
    let table = "\
| Description | Qty | Rate   | Amount    |
|:------------|----:|-------:|----------:|
| Design      | 1   | $1,000 | $1,000.00 |
|             |     | **Total due** | **$1,000.00** |
";
    let text = invoice("108", "Ann Lee", "", table)
        .replace("invoice_num: 108", "invoice_num: \"#108\"")
        .replace("invoicee_name: Ann Lee", "invoicee_name: \"Studio #9; LLC\"");
    let (books, index) = books(&[("invoices/Studio 108.md", &text)], JOURNAL);
    let preview = books.preview(&index, "#108").expect("previews");
    assert_eq!(preview.booking.description, "Studio 9 LLC | Invoice 108");
    assert!(preview.would_book.is_some(), "{preview:?}");
}

#[test]
fn what_cannot_be_booked_says_why() {
    let table = |amount: &str, account: &str| {
        format!(
            "| Description | Amount | *Account* |\n|:--|--:|:--|\n| Work | {amount} | {account} |\n"
        )
    };
    // No receivable account in accrual books.
    let no_ar = JOURNAL.replace(
        "account assets:accounts receivable      ; type: A, kpi: receivable\n",
        "",
    );
    let no_ar =
        format!("{no_ar}account liabilities:deferred-revenue2  ; type: L, kpi: deferred-revenue\n");
    let (books_, index) = books(
        &[(
            "invoices/1.md",
            &invoice("1", "Acme", "", &table("$10.00", "revenue:projects")),
        )],
        &no_ar,
    );
    let setup = books_.preview(&index, "1");
    assert!(
        matches!(&setup, Err(BooksError::Setup(why)) if why.contains("receivable")),
        "{setup:?}"
    );

    // A total of zero or less is a credit note's job.
    let (books_, index) = books(
        &[(
            "invoices/2.md",
            &invoice("2", "Acme", "", &table("-$10.00", "revenue:projects")),
        )],
        JOURNAL,
    );
    let negative = books_.preview(&index, "2");
    assert!(
        matches!(negative, Err(BooksError::Refused(_))),
        "{negative:?}"
    );

    // A table that doesn't add up: recalc first.
    let wrong = "| Description | Qty | Rate | Amount | *Account* |\n|:--|--:|--:|--:|:--|\n| Work | 2 | $10 | $25.00 | revenue:projects |\n";
    let (books_, index) = books(
        &[("invoices/3.md", &invoice("3", "Acme", "", wrong))],
        JOURNAL,
    );
    let invalid = books_.preview(&index, "3");
    assert!(
        matches!(invalid, Err(BooksError::Invalid { .. })),
        "{invalid:?}"
    );
    assert_eq!(invalid.err().map(|e| e.code()), Some("invalid"));

    // An unknown number, and one used twice.
    assert!(matches!(
        books_.preview(&index, "99"),
        Err(BooksError::NotFound(_))
    ));
    let (books_, index) = books(
        &[
            (
                "invoices/4a.md",
                &invoice("4", "Acme", "", &table("$10.00", "")),
            ),
            (
                "invoices/4b.md",
                &invoice("4", "Acme", "", &table("$10.00", "")),
            ),
        ],
        JOURNAL,
    );
    assert!(matches!(
        books_.preview(&index, "4"),
        Err(BooksError::Ambiguous { .. })
    ));
}

#[test]
fn the_deposit_account_and_inferred_revenue() {
    let journal = ledgeline_core::parse_journal(JOURNAL, "books.journal").expect("parses");
    let settings = Settings::default();
    // Declared kpi: deferred-revenue wins over the I11 default name.
    assert_eq!(
        spec::deposit_account(&settings, &journal),
        "liabilities:deferred-revenue"
    );
    let bare = ledgeline_core::parse_journal("account revenue:only  ; type: R\n", "b.journal")
        .expect("parses");
    assert_eq!(
        spec::deposit_account(&settings, &bare),
        spec::DEFAULT_DEPOSIT_ACCOUNT
    );
    assert_eq!(
        spec::inferred_revenue_account(&bare).as_deref(),
        Some("revenue:only")
    );
    // Several revenue accounts: no single inference.
    assert_eq!(spec::inferred_revenue_account(&journal), None);
    let kpi = ledgeline_core::parse_journal(
        "account revenue:a  ; type: R\naccount revenue:b  ; type: R, kpi: revenue\n",
        "k.journal",
    )
    .expect("parses");
    assert_eq!(
        spec::inferred_revenue_account(&kpi).as_deref(),
        Some("revenue:b")
    );
}
