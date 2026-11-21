//! The whole flow an invoicing app built on `ledgeline-core` walks, through
//! public `ledgeline_core::` paths only, on a temp copy of the accrual
//! fixture's books: create the recognition file, write an invoice with a
//! deferred line and its schedule, post what's due (idempotently), close the
//! books, edit the invoice and apply the restatement, record a payment and
//! link it with a fee — and an external write between load and save, which
//! writes nothing.
//!
//! The invoicer's books bridge (invoicer PLAN Phase 2a) runs here too: send,
//! partial pay (N:1), pay, re-book, void, deposit and final, receivables and
//! aging — and cash-books Apply payment with its undo. Those flows write
//! exactly `fixtures/receivables/*.journal`, whose hledger captures
//! (`fixtures/receivables/capture.sh` → `expected/`) Ledgeline's numbers are
//! checked against.
//!
//! `LEDGELINE_HLEDGER_ACCRUAL_CHECK=1` also checks the results with hledger
//! itself.

mod common;

use ledgeline_core::Dec;
use ledgeline_core::accrual::index::RecognitionIndex;
use ledgeline_core::accrual::invoice::{Deferral, Direction, DocumentLine, DocumentSpec, money};
use ledgeline_core::accrual::ops::{
    Books, OpsError, PostDue, SaveRequest, SettleLink, SourceChange,
};
use ledgeline_core::accrual::schedule::{Party, PartyKind, Proration, Term};
use ledgeline_core::accrual::settle::DEFAULT_TOLERANCE_BPS;
use ledgeline_core::accrual::{AccrualRole, AccrualRoles, Contract, RECOGNITION_TAG, tag_value};
use ledgeline_core::matching::{self, PostingRef};
use ledgeline_core::model::{Journal, Transaction};
use std::path::{Path, PathBuf};

/// A temp copy of the fixture's books, without its recognition file.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ll-accrual-lib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let fixture = common::fixtures_dir().join("accrual");
    // The fixture's journals reference its schedules; start from its chart of
    // accounts and opening balance only, so this flow owns every schedule.
    let books = std::fs::read_to_string(fixture.join("books.journal")).expect("reads");
    let header: String = books
        .split("include books-2025.journal")
        .next()
        .expect("header")
        .to_string();
    std::fs::write(
        dir.join("books.journal"),
        format!("{header}\n2026-01-01 Opening balance\n    assets:bank:checking  $50000.00\n    equity:opening\n"),
    )
    .expect("writes");
    dir
}

fn find(journal: &Journal, description: &str) -> (usize, Transaction) {
    journal
        .transactions
        .iter()
        .enumerate()
        .find(|(_, t)| t.description == description)
        .map(|(i, t)| (i, t.clone()))
        .unwrap_or_else(|| panic!("{description} is in the journal"))
}

fn receivable_posting(journal: &Journal, description: &str) -> PostingRef {
    let roles = AccrualRoles::of(journal);
    let (position, txn) = find(journal, description);
    let posting = txn
        .postings
        .iter()
        .position(|p| roles.role(&p.account.0) == Some(AccrualRole::Receivable))
        .expect("a receivable posting");
    PostingRef {
        position,
        txn: txn.index,
        posting,
    }
}

#[test]
fn an_invoicing_app_walks_the_whole_flow() {
    let dir = scratch("flow");
    let main = dir.join("books.journal");
    let books = Books::open(&main).expect("opens");

    // 1. The recognition file, explicitly.
    let path = books
        .load()
        .expect("loads")
        .create_recognition_file()
        .expect("creates");
    assert!(dir.join(path.as_str()).exists());
    assert!(matches!(
        books.load().expect("loads").create_recognition_file(),
        Err(OpsError::FileExists(_))
    ));

    // 2. An invoice with a deferred line, and its schedule, in one save.
    let mut loaded = books.load().expect("loads");
    let usd = |cents: i128| money(loaded.journal(), "$", Dec::new(cents, 2));
    let invoice = DocumentSpec {
        direction: Direction::Revenue,
        date: "2026-01-15".into(),
        description: "Acme Invoice 5001".into(),
        number: Some("5001".into()),
        party: Some(Party {
            kind: PartyKind::Customer,
            name: "Acme".into(),
        }),
        settlement_account: "assets:accounts receivable".into(),
        lines: vec![DocumentLine {
            account: "liabilities:deferred-revenue".into(),
            amount: usd(1_200_000),
            deferral: Some(Deferral {
                target_account: "revenue:subscriptions".into(),
                service_start: None,
                term: Term::Months(12),
                proration: Proration::Anniversary,
                contract: Some(Contract::New),
            }),
            tags: vec![],
        }],
        tags: vec![],
    };
    let generation = loaded.generation();
    let (_, plan) = loaded
        .write_document(&invoice, "2026-01-15", false, Some(&generation))
        .expect("writes the invoice");
    assert_eq!(plan.appended.len(), 1);
    let id = plan.appended[0].id.clone();

    // 3. Post what's due on 2026-04-20: January through April. Twice.
    let post = PostDue {
        today: "2026-04-20".into(),
        ..PostDue::default()
    };
    let posted = books.load().expect("loads").post_due(&post).expect("posts");
    let dates: Vec<&str> = posted.entries.iter().map(|(_, d)| d.as_str()).collect();
    assert_eq!(
        dates,
        vec!["2026-01-15", "2026-02-15", "2026-03-15", "2026-04-15"]
    );
    assert!(
        books
            .load()
            .expect("loads")
            .post_due(&post)
            .expect("posts")
            .entries
            .is_empty()
    );
    assert!(matches!(
        books.load().expect("loads").post_due(&PostDue {
            through: Some("2026-04-21".into()),
            ..post.clone()
        }),
        Err(OpsError::FutureDate { .. })
    ));

    // 4. Close February, then cut the invoice to $11,400 and restate.
    books
        .set_closed_through(Some("2026-02-28"))
        .expect("closes");
    let loaded = books.load().expect("loads");
    assert_eq!(
        loaded.config().closed_through.as_deref(),
        Some("2026-02-28")
    );
    let index = loaded.index();
    let mut spec = index.spec_of(&id, loaded.book()).expect("readable");
    spec.lines[0].total.quantity = Dec::new(-1_140_000, 2);
    let (_, source) = find(loaded.journal(), "Acme Invoice 5001");
    let mut edited = source.clone();
    for posting in &mut edited.postings {
        let sign = if posting.amounts[0].quantity.mantissa < 0 {
            -1
        } else {
            1
        };
        posting.amounts[0].quantity = Dec::new(sign * 1_140_000, 2);
    }
    let fingerprint = loaded
        .editor()
        .transaction_fingerprint(source.index)
        .expect("fingerprint");
    let request = SaveRequest {
        source: SourceChange::Replace {
            tindex: source.index,
            fingerprint,
            transaction: Box::new(edited),
        },
        schedules: vec![spec],
        today: "2026-04-20".into(),
        create_file: false,
        expected: Some(loaded.generation()),
    };
    let preview = loaded.preview(&request).expect("previews");
    let restated = &preview.restatements[0];
    assert_eq!(restated.rewrites.len(), 2, "March and April are open");
    let adjustment = restated.adjustment.as_ref().expect("closed months differ");
    assert_eq!(adjustment.date, "2026-04-20");
    books
        .load()
        .expect("loads")
        .save(&request)
        .expect("restates");

    let loaded = books.load().expect("loads");
    let index = RecognitionIndex::build(loaded.journal(), loaded.book());
    let issues = index.issues(loaded.journal(), loaded.book());
    assert!(issues.is_empty(), "{issues:?}");
    assert!(loaded.due("2026-04-20").is_empty());
    // A stale preview is refused.
    assert!(matches!(
        books.load().expect("loads").save(&request),
        Err(OpsError::Stale)
    ));

    // 5. A payment $30 short, linked with a fee.
    let mut loaded = books.load().expect("loads");
    let payment = {
        let journal = loaded.journal();
        let mut txn = find(journal, "Opening balance").1;
        txn.date = "2026-05-02".into();
        txn.description = "Acme payment INV 5001".into();
        txn.postings[0].amounts[0].quantity = Dec::new(1_137_000, 2);
        txn.postings[1].account.0 = "assets:accounts receivable".into();
        txn.postings[1].amounts = vec![money(journal, "$", Dec::new(-1_137_000, 2))];
        txn.postings[1].elided = false;
        txn
    };
    loaded
        .save(&SaveRequest {
            source: SourceChange::Add(Box::new(payment)),
            schedules: vec![],
            today: "2026-05-02".into(),
            create_file: false,
            expected: None,
        })
        .expect("adds the payment");
    let mut loaded = books.load().expect("loads");
    let journal = loaded.journal().clone();
    let link = loaded
        .link_settlement(
            receivable_posting(&journal, "Acme Invoice 5001"),
            receivable_posting(&journal, "Acme payment INV 5001"),
            Some("expenses:payment fees"),
            DEFAULT_TOLERANCE_BPS,
        )
        .expect("links");
    let journal = books.load().expect("loads").journal().clone();
    let links = matching::links(&journal);
    assert!(links.is_settlement(&link) && links.broken().is_empty());
    let (_, paid) = find(&journal, "Acme payment INV 5001");
    assert!(
        paid.postings
            .iter()
            .any(|p| p.account.0 == "expenses:payment fees")
    );

    // The schedule is still whole, and every entry carries its id.
    let loaded = books.load().expect("loads");
    assert!(
        loaded
            .index()
            .issues(loaded.journal(), loaded.book())
            .is_empty()
    );
    assert!(
        loaded
            .journal()
            .transactions
            .iter()
            .filter(|t| t.description == "Acme Recognized")
            .all(|t| tag_value(&t.tags, RECOGNITION_TAG) == Some(id.as_str()))
    );

    if std::env::var("LEDGELINE_HLEDGER_ACCRUAL_CHECK").is_ok() {
        hledger_agrees(
            &dir,
            &id,
            &loaded.index().spec_of(&id, loaded.book()).expect("spec"),
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn hledger_agrees(dir: &Path, id: &str, spec: &ledgeline_core::accrual::schedule::ScheduleSpec) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("hledger")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("hledger runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8")
    };
    run(&[
        "-s",
        "-f",
        "books.journal",
        "-f",
        "recognition.journal",
        "check",
    ]);
    let query = format!("tag:recognition=^{id}$");
    let printed = run(&[
        "-f",
        "books.journal",
        "-f",
        "recognition.journal",
        "print",
        "--forecast=2026-01-01..2028-01-01",
        &query,
        "tag:_generated-transaction",
    ]);
    let dates: Vec<&str> = printed
        .lines()
        .filter(|l| l.starts_with("20"))
        .map(|l| &l[..10])
        .collect();
    let journal =
        ledgeline_core::parse_journal("commodity $1,000.00\n", "x.journal").expect("parses");
    let schedule = ledgeline_core::accrual::schedule::build(
        spec,
        &ledgeline_core::parse::JournalStyles::of(&journal),
    )
    .expect("builds");
    let ours: Vec<&str> = schedule.periods.iter().map(|p| p.date.as_str()).collect();
    assert_eq!(dates, ours);
}

#[test]
fn an_external_write_between_load_and_save_writes_nothing() {
    let dir = scratch("race");
    let main = dir.join("books.journal");
    let books = Books::open(&main).expect("opens");
    books
        .load()
        .expect("loads")
        .create_recognition_file()
        .expect("creates");
    let mut loaded = books.load().expect("loads");
    let usd = money(loaded.journal(), "$", Dec::new(120_000, 2));
    loaded
        .write_document(
            &DocumentSpec {
                direction: Direction::Expense,
                date: "2026-01-01".into(),
                description: "Northwind Insurance".into(),
                number: Some("B-1".into()),
                party: Some(Party {
                    kind: PartyKind::Vendor,
                    name: "Northwind Insurance".into(),
                }),
                settlement_account: "liabilities:accounts payable".into(),
                lines: vec![DocumentLine {
                    account: "assets:prepaid".into(),
                    amount: usd,
                    deferral: Some(Deferral {
                        target_account: "expenses:insurance:d-and-o".into(),
                        service_start: None,
                        term: Term::Months(12),
                        proration: Proration::Anniversary,
                        contract: None,
                    }),
                    tags: vec![],
                }],
                tags: vec![],
            },
            "2026-01-01",
            false,
            None,
        )
        .expect("writes");

    let mut loaded = books.load().expect("loads");
    let external = format!(
        "{}\n; edited elsewhere\n",
        std::fs::read_to_string(&main).expect("reads")
    );
    std::fs::write(&main, &external).expect("writes");
    let result = loaded.post_due(&PostDue {
        today: "2026-03-01".into(),
        ..PostDue::default()
    });
    assert!(
        matches!(result, Err(OpsError::ExternalChange)),
        "{result:?}"
    );
    assert_eq!(std::fs::read_to_string(&main).expect("reads"), external);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `pairs` exact invoice ↔ payment pairs (each its own amount, so none is
/// ambiguous), plus one invoice paid $30 short: a synthetic receivables book.
fn receivables_book(pairs: usize) -> String {
    let mut text = String::from(
        "account assets:accounts receivable  ; type: A, kpi: receivable\naccount assets:bank  ; type: C\naccount revenue:services  ; type: R\naccount expenses:fees  ; type: X\n",
    );
    for n in 0..pairs {
        let (number, amount) = (1001 + n, 100 + n);
        text.push_str(&format!(
            "\n2026-10-01 Invoice {number}  ; invoice: {number}\n    assets:accounts receivable  ${amount}.00  ; customer: Client {n}\n    revenue:services\n\n2026-10-15 Payment {number}\n    assets:bank  ${amount}.00\n    assets:accounts receivable  $-{amount}.00  ; customer: Client {n}\n"
        ));
    }
    text.push_str("\n2026-10-02 Invoice 2001  ; invoice: 2001\n    assets:accounts receivable  $1000.00  ; customer: Short\n    revenue:services\n\n2026-10-16 Payment 2001\n    assets:bank  $970.00\n    assets:accounts receivable  $-970.00  ; customer: Short\n");
    text
}

#[test]
fn link_settlements_links_a_batch_in_one_write_or_not_at_all() {
    use ledgeline_core::accrual::settle::{self, SettleSettings};
    use ledgeline_core::vault::{MemVault, Vault, VaultPath};
    use std::sync::Arc;

    let pairs = 8;
    let vault = Arc::new(MemVault::new().with_file("books.journal", receivables_book(pairs)));
    let main = VaultPath::new("books.journal").expect("path");
    let books = Books::open_in(Arc::clone(&vault) as Arc<dyn Vault>, main);
    let clear_links = |journal: &Journal| -> Vec<SettleLink> {
        let roles = AccrualRoles::of(journal);
        settle::propose(journal, &roles, &SettleSettings::default(), "2026-11-17")
            .into_iter()
            .filter(|p| p.clear)
            .map(|p| SettleLink {
                invoice: p.invoice,
                payment: p.payment,
                fee_account: None,
            })
            .collect()
    };

    // All or nothing: one pair that isn't a pair (an invoice with the wrong
    // payment) refuses the whole batch, and nothing is written.
    let mut loaded = books.load().expect("loads");
    let mut links = clear_links(loaded.journal());
    assert_eq!(links.len(), pairs, "every exact pair is clear");
    let before = vault.write_count();
    let text = vault.text("books.journal");
    let mut crossed = links.clone();
    let wrong_payment = crossed[1].payment;
    crossed[0].payment = wrong_payment;
    crossed.remove(1);
    assert!(matches!(
        loaded.link_settlements(&crossed, DEFAULT_TOLERANCE_BPS),
        Err(OpsError::Settle(_))
    ));
    // A posting in two links is refused too.
    let mut doubled = links.clone();
    doubled.push(links[0].clone());
    assert!(matches!(
        loaded.link_settlements(&doubled, DEFAULT_TOLERANCE_BPS),
        Err(OpsError::Settle(_))
    ));
    assert_eq!(
        vault.write_count(),
        before,
        "a refused batch writes nothing"
    );
    assert_eq!(vault.text("books.journal"), text);

    // The batch: every link, one write, the ids in order.
    let mut loaded = books.load().expect("loads");
    links = clear_links(loaded.journal());
    let ids = loaded
        .link_settlements(&links, DEFAULT_TOLERANCE_BPS)
        .expect("links");
    assert_eq!(ids.len(), pairs);
    assert_eq!(vault.write_count(), before + 1, "N links are ONE write");
    let written = vault.text("books.journal").expect("journal");
    for id in &ids {
        assert_eq!(
            written.matches(&format!("match:{id}")).count(),
            2,
            "{id}: {written}"
        );
    }
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "every link has its own id");

    // The short pair still links on its own, with its fee (one more write).
    let mut loaded = books.load().expect("loads");
    let short = settle::propose(
        loaded.journal(),
        &AccrualRoles::of(loaded.journal()),
        &SettleSettings::default(),
        "2026-11-17",
    )
    .into_iter()
    .next()
    .expect("the short pair is left");
    let id = loaded
        .link_settlement(
            short.invoice,
            short.payment,
            Some("expenses:fees"),
            DEFAULT_TOLERANCE_BPS,
        )
        .expect("links with a fee");
    assert_eq!(vault.write_count(), before + 2);
    assert!(
        vault
            .text("books.journal")
            .expect("journal")
            .contains(&format!("match:{id}"))
    );
}

// ---------------------------------------------------------------------------
// The invoicer's books bridge (invoicer PLAN Phase 2a, L1–L4)
// ---------------------------------------------------------------------------

/// A small service business's chart of accounts, in memory.
const SERVICE_BOOKS: &str = "\
commodity $1,000.00

account assets:bank  ; type: C
account assets:accounts receivable  ; type: A, kpi: receivable
account liabilities:customer-deposits  ; type: L
account revenue:consulting  ; type: R
account revenue:consulting:delivery  ; type: R
account revenue:discounts  ; type: R
account expenses:payment fees  ; type: X
account equity:opening  ; type: E

2026-01-01 Opening balance
    assets:bank  $1000.00
    equity:opening
";

fn tag(name: &str, value: &str) -> (String, String) {
    (name.to_string(), value.to_string())
}

fn customer(name: &str) -> Option<Party> {
    Some(Party {
        kind: PartyKind::Customer,
        name: name.into(),
    })
}

/// An invoice to `who`, numbered `number`, with `lines` of `(account,
/// cents, item)`, due 30 days after `date`.
fn invoice_spec(
    journal: &Journal,
    number: &str,
    date: &str,
    who: &str,
    lines: &[(&str, i128, &str)],
) -> DocumentSpec {
    DocumentSpec {
        direction: Direction::Revenue,
        date: date.into(),
        description: format!("{who} | Invoice {number}"),
        number: Some(number.into()),
        party: customer(who),
        settlement_account: "assets:accounts receivable".into(),
        lines: lines
            .iter()
            .map(|(account, cents, item)| DocumentLine {
                tags: vec![tag("item", item)],
                ..DocumentLine::new(*account, money(journal, "$", Dec::new(*cents, 2)))
            })
            .collect(),
        tags: vec![
            tag(
                "document",
                &format!("invoices/{date} {who} Invoice {number}.md"),
            ),
            tag("due", &ledgeline_core::reports::add_days(date, 30)),
        ],
    }
}

fn mem_books(text: &str) -> (std::sync::Arc<ledgeline_core::vault::MemVault>, Books) {
    use ledgeline_core::vault::{MemVault, Vault, VaultPath};
    let vault = std::sync::Arc::new(MemVault::new().with_file("books.journal", text));
    let books = Books::open_in(
        std::sync::Arc::clone(&vault) as std::sync::Arc<dyn Vault>,
        VaultPath::new("books.journal").expect("path"),
    );
    (vault, books)
}

#[test]
fn documents_are_found_by_number_rebooked_and_voided() {
    use ledgeline_core::accrual::ops::Generation;
    let (vault, books) = mem_books(SERVICE_BOOKS);
    let journal_text = || vault.text("books.journal").expect("journal");

    // A stale generation writes nothing.
    let mut loaded = books.load().expect("loads");
    let spec = invoice_spec(
        loaded.journal(),
        "105",
        "2026-02-01",
        "Customer X",
        &[("revenue:consulting", 720_000, "Senior architect hours")],
    );
    let before = journal_text();
    assert!(matches!(
        loaded.write_document(
            &spec,
            "2026-02-01",
            false,
            Some(&Generation::from_token("stale"))
        ),
        Err(OpsError::Stale)
    ));
    assert_eq!(journal_text(), before);

    // Booked once; a second send of the same number is refused.
    let generation = loaded.generation();
    loaded
        .write_document(&spec, "2026-02-01", false, Some(&generation))
        .expect("books 105");
    assert!(matches!(
        books
            .load()
            .expect("loads")
            .write_document(&spec, "2026-02-01", false, None),
        Err(OpsError::DuplicateDocument(n)) if n == "105"
    ));

    // Found by its exact number only, as an invoice only.
    let loaded = books.load().expect("loads");
    let found = loaded.find_document("105").expect("found");
    assert_eq!(found.transaction.description, "Customer X | Invoice 105");
    assert_eq!(found.settlement.posting, 0);
    assert_eq!(
        Some(found.fingerprint.clone()),
        loaded.editor().transaction_fingerprint(found.tindex)
    );
    assert!(loaded.find_document("10").is_none());
    assert!(loaded.find_document("1050").is_none());
    assert!(
        loaded
            .find_document_as(AccrualRole::Payable, "105")
            .is_none()
    );

    // Re-booked: the amount and lines change, still one document.
    let mut loaded = books.load().expect("loads");
    let mut edited = spec.clone();
    edited.lines.push(DocumentLine {
        tags: vec![tag("item", "Early payment discount")],
        ..DocumentLine::new(
            "revenue:discounts",
            money(loaded.journal(), "$", Dec::new(-20_000, 2)),
        )
    });
    loaded
        .rebook_document(&edited, "2026-02-03", false, None)
        .expect("re-books");
    let loaded = books.load().expect("loads");
    let found = loaded.find_document("105").expect("found");
    assert_eq!(
        found.transaction.postings[0].amounts[0].quantity,
        Dec::new(700_000, 2)
    );
    assert_eq!(found.transaction.postings.len(), 3);
    assert!(loaded.documents().duplicates().is_empty());

    // An edit elsewhere between load and re-book: nothing is written.
    let mut loaded = books.load().expect("loads");
    vault.modify_externally("books.journal", format!("{}\n; touched\n", journal_text()));
    let touched = journal_text();
    assert!(matches!(
        loaded.rebook_document(&spec, "2026-02-03", false, None),
        Err(OpsError::ExternalChange)
    ));
    assert_eq!(journal_text(), touched);

    // Closed through its date: re-booking is refused (I8).
    books
        .set_closed_through(Some("2026-02-01"))
        .expect("closes");
    assert!(matches!(
        books
            .load()
            .expect("loads")
            .rebook_document(&spec, "2026-02-03", false, None),
        Err(OpsError::Refused(_))
    ));
    books.set_closed_through(None).expect("reopens");

    // Voided: a reversal linked to the invoice, which is still the document.
    let voided = books
        .load()
        .expect("loads")
        .void_document(AccrualRole::Receivable, "105", "2026-02-10", None)
        .expect("voids");
    let loaded = books.load().expect("loads");
    let journal = loaded.journal();
    let links = matching::links(journal);
    assert!(links.is_settlement(&voided.link) && links.broken().is_empty());
    let found = loaded.find_document("105").expect("still found");
    assert_eq!(found.transaction.description, "Customer X | Invoice 105");
    assert_eq!(
        tag_value(&voided.transaction.tags, "void"),
        Some("2026-02-10")
    );
    // A voided (linked) invoice can't be voided again.
    assert!(matches!(
        books.load().expect("loads").void_document(
            AccrualRole::Receivable,
            "105",
            "2026-02-11",
            None
        ),
        Err(OpsError::Refused(_))
    ));
    assert!(matches!(
        books.load().expect("loads").void_document(
            AccrualRole::Receivable,
            "999",
            "2026-02-11",
            None
        ),
        Err(OpsError::DocumentNotFound(_))
    ));
}

/// Cash books: deposits as a bank import writes them, hand-aligned, one
/// with a processor fee and a note under the bank leg.
const CASH_BOOKS: &str = "\
account assets:bank  ; type: C
account income:unknown  ; type: R
account revenue:consulting  ; type: R
account revenue:consulting:delivery  ; type: R
account expenses:payment fees  ; type: X

2026-03-02 * STRIPE PAYOUT ACME  ; id: tx-77
    assets:bank                          $1,940.00
    ; imported 2026-03-03
    expenses:payment fees                   $60.00
    income:unknown

2026-03-10 * WIRE CUSTOMER X INV 106
    assets:bank        $500.00  ; bank-ref: 9921
    income:unknown    $-500.00
";

#[test]
fn cash_books_apply_payment_keeps_the_bank_leg_and_undoes_exactly() {
    use ledgeline_core::accrual::apply::{self, ApplyPayment, CashInvoice};
    use ledgeline_core::accrual::settle::{Fit, SettleSettings};
    use ledgeline_core::edit::EditOp;
    let (vault, books) = mem_books(CASH_BOOKS);
    let text = || vault.text("books.journal").expect("journal");
    let original = text();
    let split = |pairs: &[(&str, i128)]| -> Vec<(String, Dec)> {
        pairs
            .iter()
            .map(|(a, c)| ((*a).to_string(), Dec::new(*c, 2)))
            .collect()
    };
    let customer_x = customer("Customer X");

    // The candidates for invoice 105 ($2,000): the Stripe payout is exact.
    let loaded = books.load().expect("loads");
    let invoice = CashInvoice {
        number: "105".into(),
        party: customer_x.clone(),
        date: "2026-03-01".into(),
        due: money(loaded.journal(), "$", Dec::new(200_000, 2)),
    };
    let found = apply::candidates(
        loaded.journal(),
        &invoice,
        &SettleSettings::default(),
        "2026-03-31",
    );
    assert_eq!(found[0].fit, Fit::Exact, "{found:?}");
    let (_, stripe) = find(loaded.journal(), "STRIPE PAYOUT ACME");
    assert_eq!(found[0].tindex, stripe.index);

    // Apply: the income side becomes the invoice's rows; nothing else moves.
    let mut loaded = books.load().expect("loads");
    let fingerprint = loaded
        .editor()
        .transaction_fingerprint(stripe.index)
        .expect("fingerprint");
    let request = ApplyPayment {
        tindex: stripe.index,
        fingerprint: fingerprint.clone(),
        number: "105".into(),
        party: customer_x.clone(),
        split: split(&[
            ("revenue:consulting", 120_000),
            ("revenue:consulting:delivery", 80_000),
        ]),
    };
    let applied = loaded.apply_payment(&request, None).expect("applies");
    assert!(!applied.partial);
    let after = text();
    for kept in [
        "2026-03-02 * STRIPE PAYOUT ACME  ; id: tx-77, invoice:105\n",
        "    revenue:consulting           $-1200.00  ; customer: Customer X\n",
        "    assets:bank                          $1,940.00\n    ; imported 2026-03-03\n    expenses:payment fees                   $60.00\n",
    ] {
        assert!(after.contains(kept), "{kept:?} in\n{after}");
    }
    assert!(!after.contains("income:unknown\n\n"), "{after}");
    let loaded = books.load().expect("loads");
    let (_, paid) = find(loaded.journal(), "STRIPE PAYOUT ACME");
    let lines: Vec<(&str, Dec, Option<&str>)> = paid
        .postings
        .iter()
        .map(|p| {
            (
                p.account.0.as_str(),
                p.amounts[0].quantity,
                tag_value(&p.tags, "customer"),
            )
        })
        .collect();
    assert_eq!(
        lines,
        vec![
            ("assets:bank", Dec::new(194_000, 2), None),
            ("expenses:payment fees", Dec::new(6_000, 2), None),
            (
                "revenue:consulting",
                Dec::new(-120_000, 2),
                Some("Customer X")
            ),
            (
                "revenue:consulting:delivery",
                Dec::new(-80_000, 2),
                Some("Customer X")
            ),
        ]
    );
    // It is never a booked document.
    assert!(loaded.find_document("105").is_none());
    // Applied twice: refused.
    assert!(matches!(
        books.load().expect("loads").apply_payment(
            &ApplyPayment {
                fingerprint: loaded
                    .editor()
                    .transaction_fingerprint(paid.index)
                    .expect("fp"),
                ..request.clone()
            },
            None
        ),
        Err(OpsError::Refused(_))
    ));

    // Undo restores the deposit's text exactly.
    books
        .load()
        .expect("loads")
        .undo_apply_payment(&applied.undo, None)
        .expect("undoes");
    assert_eq!(text(), original);
    // A second undo finds nothing with that fingerprint.
    assert!(matches!(
        books
            .load()
            .expect("loads")
            .undo_apply_payment(&applied.undo, None),
        Err(OpsError::Stale)
    ));

    // A partial payment ($500 of $2,000) splits 300 / 200, pro rata.
    let mut loaded = books.load().expect("loads");
    let (_, wire) = find(loaded.journal(), "WIRE CUSTOMER X INV 106");
    let partial = loaded
        .apply_payment(
            &ApplyPayment {
                tindex: wire.index,
                fingerprint: loaded
                    .editor()
                    .transaction_fingerprint(wire.index)
                    .expect("fp"),
                number: "106".into(),
                party: customer_x.clone(),
                split: split(&[
                    ("revenue:consulting", 120_000),
                    ("revenue:consulting:delivery", 80_000),
                ]),
            },
            None,
        )
        .expect("applies");
    assert!(partial.partial);
    assert_eq!(
        partial.applied,
        split(&[
            ("revenue:consulting", 30_000),
            ("revenue:consulting:delivery", 20_000)
        ])
    );
    assert!(text().contains("    assets:bank        $500.00  ; bank-ref: 9921\n"));

    // The deposit edited elsewhere after its fingerprint was taken: refused,
    // nothing written.
    let mut loaded = books.load().expect("loads");
    let stale = loaded
        .editor()
        .transaction_fingerprint(stripe.index)
        .expect("fp");
    let edited = text().replace("STRIPE PAYOUT ACME", "STRIPE PAYOUT ACME CORP");
    vault.modify_externally("books.journal", &edited);
    let mut loaded_fresh = books.load().expect("loads");
    assert!(matches!(
        loaded_fresh.apply_payment(
            &ApplyPayment {
                fingerprint: stale.clone(),
                ..request.clone()
            },
            None
        ),
        Err(OpsError::ExternalChange)
    ));
    assert!(matches!(
        loaded.apply_payment(
            &ApplyPayment {
                fingerprint: stale,
                ..request
            },
            None
        ),
        Err(OpsError::ExternalChange)
    ));
    assert_eq!(text(), edited);

    // A restore text that smuggles a directive in is refused.
    let mut editor = books.load().expect("loads").into_editor();
    let (_, txn) = find(editor.journal(), "STRIPE PAYOUT ACME CORP");
    let fp = editor.transaction_fingerprint(txn.index).expect("fp");
    let smuggled = "2026-03-02 x\n    assets:bank  $1.00\n    income:unknown\naccount evil\n";
    assert!(
        editor
            .apply_batch(&[(txn.index, fp, vec![EditOp::SetText(smuggled.into())])])
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// The whole invoicer flow, written to `fixtures/receivables` and checked
// against hledger's own reading of it (`fixtures/receivables/capture.sh`).
// ---------------------------------------------------------------------------

/// The accrual books the flow starts from: `fixtures/receivables/books.journal`
/// is exactly what it leaves (match ids aside).
const RECEIVABLES_START: &str = "\
; Receivables fixture: the invoicer's books bridge (invoicer PLAN Phase 2a).
; Synthetic — every name and figure is invented. Written by
; tests/accrual_library.rs (the_invoicer_flow_on_accrual_books). Clock: 2026-11-17.
commodity $1,000.00

account assets:bank:checking           ; type: C
account assets:accounts receivable     ; type: A, kpi: receivable
account liabilities:customer-deposits  ; type: L
account revenue:consulting             ; type: R
account revenue:consulting:delivery    ; type: R
account revenue:discounts              ; type: R
account equity:opening                 ; type: E

2026-01-01 Opening balance
    assets:bank:checking  $10000.00
    equity:opening
";

/// The cash books the cash half starts from (`fixtures/receivables/cash.journal`
/// is what it leaves).
const CASH_START: &str = "\
; Receivables fixture, cash books: Apply payment (invoicer I17). Synthetic.
; Written by tests/accrual_library.rs (the_invoicer_flow_on_cash_books).
commodity $1,000.00

account assets:bank:checking          ; type: C
account income:unknown                ; type: R
account revenue:consulting            ; type: R
account revenue:consulting:delivery   ; type: R
account expenses:payment fees         ; type: X

2026-11-03 * STRIPE PAYOUT CONTOSO  ; id: st-1
    assets:bank:checking       $2,910.00
    expenses:payment fees         $90.00
    income:unknown

2026-11-12 * WIRE FABRIKAM INV 202  ; id: wf-9
    assets:bank:checking         $500.00
    income:unknown
";

const RECEIVABLES_TODAY: &str = "2026-11-17";
const OPT_IN: &str = "LEDGELINE_HLEDGER_ACCRUAL_CHECK";
const WRITE_FIXTURE: &str = "LEDGELINE_RECEIVABLES_FIXTURE_WRITE";

fn receivables_dir() -> PathBuf {
    common::fixtures_dir().join("receivables")
}

/// `text` with every `match:` id renamed by first appearance (`id1`, `id2`
/// …): ids are random, the books aren't.
fn normalize_ids(text: &str) -> String {
    let mut names: Vec<String> = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("match:") {
        let (head, tail) = rest.split_at(at + "match:".len());
        out.push_str(head);
        let spaces = tail.len() - tail.trim_start_matches(' ').len();
        out.push_str(&tail[..spaces]);
        let tail = &tail[spaces..];
        let end = tail
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(tail.len());
        let id = &tail[..end];
        let n = names.iter().position(|n| n == id).unwrap_or_else(|| {
            names.push(id.to_string());
            names.len() - 1
        });
        out.push_str(&format!("id{}", n + 1));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// A bank import: `amount` into checking, against `account`, tagged.
fn bank_import(
    books: &Books,
    date: &str,
    description: &str,
    header: &str,
    account: &str,
    cents: i128,
    posting_comment: &str,
) {
    let mut loaded = books.load().expect("loads");
    let text = format!(
        "commodity $1,000.00\n\n{date} {description}{header}\n    assets:bank:checking  ${}\n    {account}  ${}{posting_comment}\n",
        ledgeline_core::edit::render_dec(Dec::new(cents, 2), '.'),
        ledgeline_core::edit::render_dec(Dec::new(-cents, 2), '.'),
    );
    let txn = ledgeline_core::parse_journal(&text, "import.journal")
        .expect("parses")
        .transactions[0]
        .clone();
    loaded
        .save(&SaveRequest {
            source: SourceChange::Add(Box::new(txn)),
            schedules: vec![],
            today: date.into(),
            create_file: false,
            expected: None,
        })
        .expect("imports");
}

/// The receivable posting of the transaction described `description`.
fn ar_of(books: &Books, description: &str) -> PostingRef {
    receivable_posting(books.load().expect("loads").journal(), description)
}

fn state_of(books: &Books, number: &str) -> ledgeline_core::accrual::receivables::DocumentStatus {
    use ledgeline_core::accrual::receivables::ReceivablesOptions;
    books
        .load()
        .expect("loads")
        .receivables(&ReceivablesOptions::default())
        .by_number(AccrualRole::Receivable, number)
        .cloned()
        .unwrap_or_else(|| panic!("invoice {number}"))
}

fn usd_of(cents: i128) -> Dec {
    Dec::new(cents, 2)
}

#[test]
fn the_invoicer_flow_on_accrual_books() {
    use ledgeline_core::accrual::invoice::tag_value_from;
    use ledgeline_core::accrual::receivables::{AgingBucket, PaymentState, ReceivablesOptions};
    let dir = std::env::temp_dir().join(format!("ll-receivables-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let main = dir.join("books.journal");
    std::fs::write(&main, RECEIVABLES_START).expect("writes");
    let books = Books::open(&main).expect("opens");
    let send = |spec: &DocumentSpec| {
        let mut loaded = books.load().expect("loads");
        let generation = loaded.generation();
        loaded
            .write_document(spec, &spec.date, false, Some(&generation))
            .expect("sends");
    };
    let spec = |number: &str, date: &str, who: &str, lines: &[(&str, i128, &str)]| {
        invoice_spec(
            books.load().expect("loads").journal(),
            number,
            date,
            who,
            lines,
        )
    };

    // 101: sent, paid in two parts (N:1).
    send(&spec(
        "101",
        "2026-09-01",
        "Contoso",
        &[("revenue:consulting", 400_000, "Architecture review")],
    ));
    assert_eq!(state_of(&books, "101").state, PaymentState::Open);
    bank_import(
        &books,
        "2026-09-20",
        "CONTOSO ACH",
        "  ; invoice: 101",
        "assets:accounts receivable",
        150_000,
        "  ; customer: Contoso",
    );
    let link = books
        .load()
        .expect("loads")
        .link_payments(
            ar_of(&books, "Contoso | Invoice 101"),
            &[ar_of(&books, "CONTOSO ACH")],
            None,
        )
        .expect("links the first part");
    let first = state_of(&books, "101");
    assert_eq!(
        (first.state, first.outstanding),
        (PaymentState::Partial, usd_of(250_000))
    );
    assert!(matching::links(books.load().expect("loads").journal()).is_open(&link));
    bank_import(
        &books,
        "2026-10-05",
        "CONTOSO ACH 2",
        "  ; invoice: 101",
        "assets:accounts receivable",
        250_000,
        "  ; customer: Contoso",
    );
    let same = books
        .load()
        .expect("loads")
        .link_payments(
            ar_of(&books, "Contoso | Invoice 101"),
            &[ar_of(&books, "CONTOSO ACH 2")],
            None,
        )
        .expect("links the rest");
    assert_eq!(same, link, "one invoice, one link");
    let paid = state_of(&books, "101");
    assert_eq!(paid.state, PaymentState::Paid);
    assert_eq!(paid.settled_on(), Some("2026-10-05"));
    assert!(!matching::links(books.load().expect("loads").journal()).is_open(&link));

    // 102: sent, re-booked with a discount, paid exactly (plan 45's 1:1).
    let mut sent = spec(
        "102",
        "2026-09-15",
        "Fabrikam",
        &[
            ("revenue:consulting", 200_000, "Discovery workshop"),
            ("revenue:consulting:delivery", 100_000, "Implementation"),
        ],
    );
    send(&sent);
    sent.lines[1].amount.quantity = usd_of(120_000);
    sent.lines.push(DocumentLine {
        tags: vec![tag("item", "Loyalty discount")],
        ..DocumentLine::new(
            "revenue:discounts",
            money(books.load().expect("loads").journal(), "$", usd_of(-30_000)),
        )
    });
    books
        .load()
        .expect("loads")
        .rebook_document(&sent, "2026-09-16", false, None)
        .expect("re-books");
    assert_eq!(state_of(&books, "102").amount, usd_of(290_000));
    bank_import(
        &books,
        "2026-10-20",
        "FABRIKAM PAYMENT",
        "  ; invoice: 102",
        "assets:accounts receivable",
        290_000,
        "  ; customer: Fabrikam",
    );
    books
        .load()
        .expect("loads")
        .link_settlement(
            ar_of(&books, "Fabrikam | Invoice 102"),
            ar_of(&books, "FABRIKAM PAYMENT"),
            None,
            DEFAULT_TOLERANCE_BPS,
        )
        .expect("links exactly");
    assert_eq!(state_of(&books, "102").state, PaymentState::Paid);

    // 103: sent, then voided.
    send(&spec(
        "103",
        "2026-10-01",
        "Tailspin",
        &[("revenue:consulting", 80_000, "Site visit")],
    ));
    books
        .load()
        .expect("loads")
        .void_document(AccrualRole::Receivable, "103", "2026-10-10", None)
        .expect("voids");
    assert_eq!(state_of(&books, "103").state, PaymentState::Void);

    // 104 + 105: a deposit invoice, paid, then the final invoice applying it.
    let mut deposit = spec(
        "104",
        "2026-10-01",
        "Customer X",
        &[(
            "liabilities:customer-deposits",
            835_000,
            "Deposit: 50% of proposal P-12",
        )],
    );
    deposit.tags.push(tag("reference", "PO #PY75334"));
    send(&deposit);
    bank_import(
        &books,
        "2026-10-08",
        "CUSTOMER X WIRE",
        "  ; invoice: 104",
        "assets:accounts receivable",
        835_000,
        "  ; customer: Customer X",
    );
    books
        .load()
        .expect("loads")
        .link_payments(
            ar_of(&books, "Customer X | Invoice 104"),
            &[ar_of(&books, "CUSTOMER X WIRE")],
            None,
        )
        .expect("links the deposit");
    let mut last = spec(
        "105",
        "2026-11-10",
        "Customer X",
        &[
            ("revenue:consulting", 720_000, "Senior architect hours"),
            ("revenue:consulting:delivery", 950_000, "Junior engineer"),
            (
                "liabilities:customer-deposits",
                -835_000,
                "Less deposit (Invoice 104)",
            ),
        ],
    );
    last.tags.push(tag(
        "reference",
        &tag_value_from("PO #PY75334, rev. 2").expect("value"),
    ));
    send(&last);
    let open = state_of(&books, "105");
    assert_eq!(
        (open.state, open.amount, open.due.as_str()),
        (PaymentState::Open, usd_of(835_000), "2026-12-10")
    );

    // 106: old and unpaid; and a receipt nobody has matched.
    send(&spec(
        "106",
        "2026-08-01",
        "Wingtip",
        &[("revenue:consulting", 120_000, "Retainer")],
    ));
    bank_import(
        &books,
        "2026-11-12",
        "GAMMA LLC DEPOSIT",
        "",
        "assets:accounts receivable",
        25_000,
        "  ; customer: Gamma",
    );

    // The receivables as of the clock.
    let loaded = books.load().expect("loads");
    let r = loaded.receivables(&ReceivablesOptions::default());
    let states: Vec<(&str, PaymentState)> = r
        .documents()
        .iter()
        .filter_map(|d| Some((d.number.as_deref()?, d.state)))
        .collect();
    assert_eq!(
        states,
        vec![
            ("106", PaymentState::Open),
            ("101", PaymentState::Paid),
            ("102", PaymentState::Paid),
            ("103", PaymentState::Void),
            ("104", PaymentState::Paid),
            ("105", PaymentState::Open),
        ]
    );
    assert_eq!(r.unapplied().len(), 1);
    let aging = r.aging(AccrualRole::Receivable, RECEIVABLES_TODAY);
    assert_eq!(aging.len(), 1);
    let row = &aging[0];
    assert_eq!(row.buckets[AgingBucket::Current as usize], usd_of(835_000));
    assert_eq!(
        row.buckets[AgingBucket::Days61To90 as usize],
        usd_of(120_000)
    );
    assert_eq!(row.total, usd_of(955_000));
    assert_eq!(row.oldest_overdue.as_deref(), Some("2026-08-31"));
    // DSO, 90 days: $9,550 outstanding over $23,600 billed (101, 102, 104,
    // 105; 106 is older, 103 was voided) × 90 = 36.4.
    let dso = r.dso(AccrualRole::Receivable, RECEIVABLES_TODAY, 90);
    assert_eq!(
        (dso[0].outstanding, dso[0].billed, dso[0].value),
        (usd_of(955_000), usd_of(2_360_000), Some(Dec::new(364, 1)))
    );
    // Days to pay: 101 in 34, 102 in 35, 104 in 7 — 25.3 on average.
    let pay = r.average_days_to_pay(AccrualRole::Receivable);
    assert_eq!((pay.documents, pay.average), (3, Some(Dec::new(253, 1))));
    // Every booked number is found, exactly once.
    for number in ["101", "102", "103", "104", "105", "106"] {
        assert!(loaded.find_document(number).is_some(), "{number}");
    }
    assert!(loaded.documents().duplicates().is_empty());
    assert!(matching::links(loaded.journal()).broken().is_empty());

    let written = std::fs::read_to_string(&main).expect("reads");
    if std::env::var(WRITE_FIXTURE).is_ok() {
        std::fs::write(receivables_dir().join("books.journal"), &written).expect("writes");
    }
    let fixture =
        std::fs::read_to_string(receivables_dir().join("books.journal")).expect("fixture");
    assert_eq!(
        normalize_ids(&written),
        normalize_ids(&fixture),
        "the flow no longer writes fixtures/receivables/books.journal; rerun with \
         {WRITE_FIXTURE}=1 and then fixtures/receivables/capture.sh if that's intended"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_invoicer_flow_on_cash_books() {
    use ledgeline_core::accrual::apply::{self, ApplyPayment, CashInvoice};
    use ledgeline_core::accrual::settle::{Fit, SettleSettings};
    let dir = std::env::temp_dir().join(format!("ll-receivables-cash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let main = dir.join("cash.journal");
    std::fs::write(&main, CASH_START).expect("writes");
    let books = Books::open(&main).expect("opens");
    let split = |pairs: &[(&str, i128)]| -> Vec<(String, Dec)> {
        pairs
            .iter()
            .map(|(a, c)| ((*a).to_string(), usd_of(*c)))
            .collect()
    };
    let apply_to = |number: &str, who: &str, date: &str, rows: &[(&str, i128)]| {
        let mut loaded = books.load().expect("loads");
        let total: i128 = rows.iter().map(|(_, c)| c).sum();
        let invoice = CashInvoice {
            number: number.into(),
            party: customer(who),
            date: date.into(),
            due: money(loaded.journal(), "$", usd_of(total)),
        };
        let best = apply::candidates(
            loaded.journal(),
            &invoice,
            &SettleSettings::default(),
            RECEIVABLES_TODAY,
        )
        .into_iter()
        .next()
        .expect("a candidate");
        let fingerprint = loaded
            .editor()
            .transaction_fingerprint(best.tindex)
            .expect("fingerprint");
        let applied = loaded
            .apply_payment(
                &ApplyPayment {
                    tindex: best.tindex,
                    fingerprint,
                    number: number.into(),
                    party: customer(who),
                    split: split(rows),
                },
                None,
            )
            .expect("applies");
        (best, applied)
    };

    // 201: the Stripe payout pays it exactly (gross of the fee).
    let (best, applied) = apply_to(
        "201",
        "Contoso",
        "2026-11-01",
        &[
            ("revenue:consulting", 180_000),
            ("revenue:consulting:delivery", 120_000),
        ],
    );
    assert_eq!(best.fit, Fit::Exact);
    assert!(!applied.partial);
    // Undo restores the import byte for byte; applying again lands the same.
    books
        .load()
        .expect("loads")
        .undo_apply_payment(&applied.undo, None)
        .expect("undoes");
    assert_eq!(std::fs::read_to_string(&main).expect("reads"), CASH_START);
    apply_to(
        "201",
        "Contoso",
        "2026-11-01",
        &[
            ("revenue:consulting", 180_000),
            ("revenue:consulting:delivery", 120_000),
        ],
    );

    // 202: the wire names it and pays a quarter: split 375 / 125.
    let (best, applied) = apply_to(
        "202",
        "Fabrikam",
        "2026-11-05",
        &[
            ("revenue:consulting", 150_000),
            ("revenue:consulting:delivery", 50_000),
        ],
    );
    assert!(best.by_number && matches!(best.fit, Fit::Short { .. }));
    assert!(applied.partial);
    assert_eq!(
        applied.applied,
        split(&[
            ("revenue:consulting", 37_500),
            ("revenue:consulting:delivery", 12_500)
        ])
    );

    let written = std::fs::read_to_string(&main).expect("reads");
    for bank in [
        "    assets:bank:checking       $2,910.00\n    expenses:payment fees         $90.00\n",
        "    assets:bank:checking         $500.00\n",
    ] {
        assert!(
            written.contains(bank),
            "the bank legs are untouched:\n{written}"
        );
    }
    if std::env::var(WRITE_FIXTURE).is_ok() {
        std::fs::write(receivables_dir().join("cash.journal"), &written).expect("writes");
    }
    let fixture = std::fs::read_to_string(receivables_dir().join("cash.journal")).expect("fixture");
    assert_eq!(written, fixture, "rerun with {WRITE_FIXTURE}=1 if intended");
    let _ = std::fs::remove_dir_all(&dir);
}

/// One `hledger bal -O csv` capture: account → balance, with the total row
/// and zero balances (hledger omits them) left out.
fn hledger_balances(csv: &str) -> std::collections::BTreeMap<String, Dec> {
    csv.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.trim_matches('"').split("\",\"").collect();
            let [account, balance] = fields.as_slice() else {
                panic!("not a two-column row: {line}");
            };
            if *account == "Total:" {
                return None;
            }
            let number = balance.replace(['$', ','], "");
            let value = Dec::parse(&number, '.').unwrap_or_else(|_| panic!("{balance}"));
            Some((account.to_string(), value.normalized()))
        })
        .filter(|(_, value)| !value.is_zero())
        .collect()
}

/// Ledgeline's own per-account sums over the transactions `keep` selects —
/// what `hledger bal QUERY` reports for a header-tag query.
fn our_balances(
    journal: &Journal,
    keep: impl Fn(&Transaction) -> bool,
) -> std::collections::BTreeMap<String, Dec> {
    let mut sums: std::collections::BTreeMap<String, Dec> = std::collections::BTreeMap::new();
    for txn in journal.transactions.iter().filter(|t| keep(t)) {
        for posting in &txn.postings {
            let sum = sums.entry(posting.account.0.clone()).or_insert(Dec::zero());
            *sum = sum.add(posting.amounts[0].quantity).expect("sums");
        }
    }
    sums.into_iter()
        .map(|(account, sum)| (account, sum.normalized()))
        .filter(|(_, sum)| !sum.is_zero())
        .collect()
}

fn expected(name: &str) -> String {
    std::fs::read_to_string(receivables_dir().join("expected").join(name))
        .unwrap_or_else(|e| panic!("expected/{name}: {e}"))
}

#[test]
fn the_receivables_fixture_agrees_with_hledger() {
    use ledgeline_core::accrual::receivables::ReceivablesOptions;
    let loaded = Books::open(&receivables_dir().join("books.journal"))
        .expect("opens")
        .load()
        .expect("loads");
    let journal = loaded.journal();
    let r = loaded.receivables(&ReceivablesOptions::default());
    let ar = "assets:accounts receivable";
    fn invoice_is(n: &str) -> impl Fn(&Transaction) -> bool + '_ {
        move |t: &Transaction| tag_value(&t.tags, "invoice") == Some(n)
    }

    // Each invoice's own transactions, account by account; and its
    // receivable line is exactly what Ledgeline says is still outstanding.
    for n in ["101", "102", "103", "104", "105", "106"] {
        let theirs = hledger_balances(&expected(&format!("invoice-{n}.csv")));
        assert_eq!(our_balances(journal, invoice_is(n)), theirs, "invoice {n}");
        let outstanding = r
            .by_number(AccrualRole::Receivable, n)
            .expect("booked")
            .outstanding
            .normalized();
        assert_eq!(
            theirs.get(ar).copied().unwrap_or(Dec::zero()).normalized(),
            outstanding,
            "invoice {n}'s receivable"
        );
    }

    // The receivable: what is outstanding less what is unapplied; and the
    // part no link covers: unlinked documents less unapplied receipts.
    let sum = |values: &mut dyn Iterator<Item = Dec>| {
        values.fold(Dec::zero(), |s, v| s.add(v).expect("sums"))
    };
    let unapplied = sum(&mut r.unapplied().iter().map(|u| u.amount));
    let outstanding = sum(&mut r.open(AccrualRole::Receivable).map(|d| d.outstanding));
    let unlinked = sum(&mut r
        .documents()
        .iter()
        .filter(|d| d.link.is_none())
        .map(|d| d.amount));
    assert_eq!(
        hledger_balances(&expected("receivable.csv"))[ar],
        outstanding.sub(unapplied).expect("subtracts").normalized()
    );
    assert_eq!(
        hledger_balances(&expected("unlinked.csv"))[ar],
        unlinked.sub(unapplied).expect("subtracts").normalized()
    );
    // Every balance, the deposit liability netted to zero by invoice 105.
    let all = hledger_balances(&expected("bal.csv"));
    assert_eq!(our_balances(journal, |_| true), all);
    assert!(!all.contains_key("liabilities:customer-deposits"));

    // Cash books: each applied deposit, as hledger reads it.
    let cash = Books::open(&receivables_dir().join("cash.journal"))
        .expect("opens")
        .load()
        .expect("loads");
    for n in ["201", "202"] {
        assert_eq!(
            our_balances(cash.journal(), invoice_is(n)),
            hledger_balances(&expected(&format!("cash-invoice-{n}.csv"))),
            "cash invoice {n}"
        );
    }

    if std::env::var(OPT_IN).is_ok() {
        hledger_still_says_so();
    }
}

/// Re-run `capture.sh`'s commands against the fixture and compare with the
/// committed captures, byte for byte.
fn hledger_still_says_so() {
    let run = |journal: &str, args: &[&str]| {
        let out = std::process::Command::new("hledger")
            .current_dir(receivables_dir())
            .env_remove("LEDGER_FILE")
            .args(["-f", journal])
            .args(args)
            .output()
            .expect("hledger runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8")
    };
    run("books.journal", &["-s", "check"]);
    run("cash.journal", &["-s", "check"]);
    let csv = ["--flat", "-O", "csv"];
    let mut checks: Vec<(String, &str, Vec<String>)> = vec![
        ("bal.csv".into(), "books.journal", vec!["bal".into()]),
        (
            "receivable.csv".into(),
            "books.journal",
            vec!["bal".into(), "tag:kpi=receivable".into()],
        ),
        (
            "unlinked.csv".into(),
            "books.journal",
            vec![
                "bal".into(),
                "tag:kpi=receivable".into(),
                "not:tag:match".into(),
            ],
        ),
    ];
    for n in ["101", "102", "103", "104", "105", "106"] {
        checks.push((
            format!("invoice-{n}.csv"),
            "books.journal",
            vec!["bal".into(), format!("tag:invoice=^{n}$")],
        ));
    }
    for n in ["201", "202"] {
        checks.push((
            format!("cash-invoice-{n}.csv"),
            "cash.journal",
            vec!["bal".into(), format!("tag:invoice=^{n}$")],
        ));
    }
    for (name, journal, args) in checks {
        let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
        args.extend(csv);
        assert_eq!(run(journal, &args), expected(&name), "{name}");
    }
}
