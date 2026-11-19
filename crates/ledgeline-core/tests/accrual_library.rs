//! The whole flow an invoicing app built on `ledgeline-core` walks, through
//! public `ledgeline_core::` paths only, on a temp copy of the accrual
//! fixture's books: create the recognition file, write an invoice with a
//! deferred line and its schedule, post what's due (idempotently), close the
//! books, edit the invoice and apply the restatement, record a payment and
//! link it with a fee — and an external write between load and save, which
//! writes nothing.
//!
//! `LEDGELINE_HLEDGER_ACCRUAL_CHECK=1` also checks the result with hledger.

mod common;

use ledgeline_core::Dec;
use ledgeline_core::accrual::index::RecognitionIndex;
use ledgeline_core::accrual::invoice::{Deferral, Direction, DocumentLine, DocumentSpec, money};
use ledgeline_core::accrual::ops::{Books, OpsError, PostDue, SaveRequest, SourceChange};
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
        }],
    };
    let (_, plan) = loaded
        .write_document(&invoice, "2026-01-15", false)
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
                }],
            },
            "2026-01-01",
            false,
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
