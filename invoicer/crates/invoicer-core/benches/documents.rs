//! Criterion coverage of the document engine on the §5.6 corpus
//! (`invoicer_corpus.rs`: 5,000 invoices, 500 customers, 10 years of
//! journal), against the ARCHITECTURE §5.6 budgets.
//!
//! | Bench                            | What it is                                                     | Budget it feeds                         |
//! |----------------------------------|----------------------------------------------------------------|-----------------------------------------|
//! | `invoicer/open`                  | walk, read and parse every note; index every document          | open → list shown (300 ms)              |
//! | `invoicer/list_json`             | the newest-first list, serialized (`list --json` on a warm index) | `list --json` (300 ms)               |
//! | `invoicer/check`                 | documents `check` on a warm index                              | `check` (1 s)                           |
//! | `invoicer/next_number`           | numbering over the index                                       | —                                       |
//! | `invoicer/refresh_one`           | one changed file re-read and re-indexed (a watcher event)      | save a draft (20 ms)                    |
//! | `invoicer/save_draft`            | verify, conditional atomic write, re-index                     | save a draft (20 ms)                    |
//! | `invoicer/preview`               | what a draft would book (the books panel; load the journal + derive) | keystroke → books panel (10 ms, warm in the app) |
//! | `invoicer/send`                  | send a draft: load, book, write the journal, update the markdown | send (300 ms, relaxed; goal 150 ms) |
//! | `invoicer/reconcile_change`      | a payment linked in Ledgeline: reload, book states, diff, mirror the one invoice | a journal change → `paid:` (100 ms) |
//! | `invoicer/books_check`           | the books checks over every invoice, on loaded books (`check`'s books half) | `check` (1 s)                |
//! | `ledgeline/journal_open`         | ledgeline-core opens the corpus journal                        | open; a journal change (100 ms)         |
//! | `ledgeline/journal_reopen`       | the reload a journal change triggers                           | a journal change → `paid:` (100 ms)     |
//!
//! ```text
//! nix develop -c cargo bench -p invoicer-core --bench documents
//! just invoicer-bench-gate     # every invoicer-core bench vs benches/baseline.json
//! ```

#[path = "../../../../crates/ledgeline-core/benches/corpus.rs"]
mod corpus;
mod invoicer_corpus;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use invoicer_core::books::{Basis, InvoiceBooks, Send};
use invoicer_core::doc::{Money, Status};
use invoicer_core::draft;
use invoicer_core::index::{DocumentIndex, ListFilter};
use invoicer_core::problem::ProblemCode;
use invoicer_core::reconcile::{self, BookStates, ReconcileOptions};
use invoicer_core::settings::Settings;
use ledgeline_core::edit::JournalEditor;
use ledgeline_core::vault::{LocalVault, Vault, VaultPath};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn benches(c: &mut Criterion) {
    let dir = invoicer_corpus::ensure();
    let main = dir.join("books.journal");
    let label = format!("{}", invoicer_corpus::INVOICES);

    let started = Instant::now();
    let editor = JournalEditor::open(&main).expect("the corpus journal opens");
    let opened = started.elapsed();
    let journal = editor.journal().clone();
    let vault = LocalVault::new(&dir);
    let settings = Settings::read(&vault).expect("settings");
    let money = Money::of_journal(&journal);
    let mut index = DocumentIndex::open(&vault, settings.clone(), money.clone());

    // The corpus is what it says, and it is clean: every table adds up and
    // every relation resolves, so `check` measures a full pass.
    assert_eq!(index.docs().count(), invoicer_corpus::INVOICES);
    assert_eq!(index.customers().len(), invoicer_corpus::CUSTOMERS);
    let problems = index.check(&vault);
    assert!(
        problems.iter().all(|p| p.code != ProblemCode::Arithmetic),
        "{:?}",
        problems.first()
    );
    assert!(
        problems.is_empty(),
        "{} problems, first {:?}",
        problems.len(),
        problems.first()
    );
    eprintln!(
        "invoicer corpus: {} notes, {} documents, {} customers; journal {} transactions ({} bytes), opened in {opened:?}",
        index.site().len(),
        index.docs().count(),
        invoicer_corpus::CUSTOMERS,
        journal.transactions.len(),
        std::fs::metadata(&main).map(|m| m.len()).unwrap_or(0),
    );

    let mut group = c.benchmark_group("invoicer");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function(BenchmarkId::new("open", &label), |b| {
        b.iter(|| DocumentIndex::open(black_box(&vault), settings.clone(), money.clone()));
    });
    group.bench_function(BenchmarkId::new("list_json", &label), |b| {
        b.iter(|| {
            let rows = index.list(black_box(&ListFilter::default()));
            serde_json::to_string(&rows).unwrap_or_default()
        });
    });
    group.bench_function(BenchmarkId::new("check", &label), |b| {
        b.iter(|| index.check(black_box(&vault)));
    });
    group.bench_function(BenchmarkId::new("next_number", &label), |b| {
        b.iter(|| index.next_number(black_box([])));
    });

    // One invoice edited back and forth: each iteration's refresh sees a
    // real change, so it re-reads, re-hashes and re-parses.
    let target = index
        .list(&ListFilter::default())
        .first()
        .map(|d| d.path.clone())
        .expect("a document");
    let original = vault.read_to_string(&target).expect("reads");
    let edited = original.replace("Hi, here is", "Hello, here is");
    assert_ne!(original, edited);
    group.bench_function(BenchmarkId::new("refresh_one", &label), |b| {
        b.iter_custom(|iters| {
            (0..iters)
                .map(|n| {
                    let text = if n % 2 == 0 { &edited } else { &original };
                    vault
                        .write_atomic(&target, text.as_bytes())
                        .expect("writes");
                    let start = Instant::now();
                    black_box(index.refresh(&vault, std::slice::from_ref(&target)));
                    start.elapsed()
                })
                .sum()
        });
    });
    group.bench_function(BenchmarkId::new("save_draft", &label), |b| {
        b.iter_custom(|iters| {
            (0..iters)
                .map(|n| {
                    let text = if n % 2 == 0 { &edited } else { &original };
                    let (_, revision) = vault.read_pinned(&target, None).expect("reads");
                    let start = Instant::now();
                    black_box(
                        draft::save(&mut index, &vault, &target, text, &revision).expect("saves"),
                    );
                    start.elapsed()
                })
                .sum()
        });
    });
    vault
        .write_atomic(&target, original.as_bytes())
        .expect("restores");

    // Send (PLAN 2b): the newest draft, booked and marked sent; the journal
    // and the file are put back between iterations (untimed).
    let books = InvoiceBooks::open(&main).expect("the corpus books open");
    let draft_doc = index
        .list(&ListFilter {
            status: Some(Status::Draft),
            ..ListFilter::default()
        })
        .first()
        .map(|d| (d.path.clone(), d.number().unwrap_or_default().to_string()))
        .expect("a draft");
    let (draft_path, draft_number) = draft_doc;
    let draft_text = vault.read_to_string(&draft_path).expect("reads");
    let journal_path = VaultPath::new("books.journal").expect("path");
    let journal_text = vault.read_to_string(&journal_path).expect("reads");
    group.bench_function(BenchmarkId::new("preview", &label), |b| {
        b.iter(|| {
            books
                .preview(black_box(&index), &draft_number)
                .expect("previews")
        });
    });
    let request = Send {
        number: draft_number.clone(),
        today: "2036-12-31".into(),
        sent_via: None,
        expected: None,
    };
    group.bench_function(BenchmarkId::new("send", &label), |b| {
        b.iter_custom(|iters| {
            (0..iters)
                .map(|_| {
                    vault
                        .write_atomic(&journal_path, journal_text.as_bytes())
                        .expect("restores the journal");
                    vault
                        .write_atomic(&draft_path, draft_text.as_bytes())
                        .expect("restores the draft");
                    index.refresh(&vault, std::slice::from_ref(&draft_path));
                    let start = Instant::now();
                    black_box(books.send(&mut index, &request).expect("sends"));
                    start.elapsed()
                })
                .sum()
        });
    });
    vault
        .write_atomic(&journal_path, journal_text.as_bytes())
        .expect("restores the journal");
    vault
        .write_atomic(&draft_path, draft_text.as_bytes())
        .expect("restores the draft");
    index.refresh(&vault, std::slice::from_ref(&draft_path));

    // A journal change (PLAN 2c): Ledgeline links the rest of a part-paid
    // invoice's payment. The reconciler reloads the books, diffs their
    // payment state against the last, and mirrors `paid:` for what moved.
    let loaded = books.load().expect("loads");
    let before = BookStates::build(loaded.journal(), Basis::of(&loaded));
    let part_paid = index
        .docs()
        .find(|d| {
            d.header.status == Some(Status::Sent)
                && matches!(
                    d.header.paid,
                    Some(invoicer_core::doc::Paid::Partial { .. })
                )
        })
        .expect("a part-paid invoice");
    let (part_path, part_number) = (
        part_paid.path.clone(),
        part_paid.number().unwrap_or_default().to_string(),
    );
    let part_text = vault.read_to_string(&part_path).expect("reads");
    let receivable = before
        .standing(&part_number)
        .and_then(|s| s.booking())
        .and_then(|b| b.receivable.clone())
        .expect("booked");
    let header = format!("; invoice: {part_number}, document:");
    let link = journal_text[journal_text.find(&header).expect("booked")..]
        .lines()
        .nth(1)
        .and_then(|l| l.split("match:").nth(1))
        .map(|id| id.trim().to_string())
        .expect("its link");
    let paid_journal = format!(
        "{journal_text}\n2025-12-31 Rest of {part_number}  ; invoice: {part_number}\n    \
         assets:bank:operating  {rest}\n    assets:accounts receivable  -{rest}  ; match:{link}\n",
        rest = format!(
            "${}",
            ledgeline_core::edit::render_dec(receivable.outstanding, '.')
        ),
    );
    let all_changed = |states: &BookStates| states.changed_since(&before);
    {
        // Where the time goes, once (not part of the measurement).
        vault
            .write_atomic(&journal_path, paid_journal.as_bytes())
            .expect("writes the journal");
        let t = Instant::now();
        let loaded = books.load().expect("loads");
        let load = t.elapsed();
        let t = Instant::now();
        let states = BookStates::build(loaded.journal(), Basis::of(&loaded));
        let build = t.elapsed();
        let t = Instant::now();
        let only = all_changed(&states);
        let diff = t.elapsed();
        let t = Instant::now();
        let done = reconcile::reconcile_with(
            &books,
            &mut index,
            &loaded,
            &states,
            &ReconcileOptions {
                only: Some(only),
                dry_run: false,
            },
        );
        let mirror = t.elapsed();
        assert_eq!(done.changed.len(), 1);
        eprintln!(
            "reconcile_change: load {load:?}, book states {build:?}, diff {diff:?}, mirror + audit {mirror:?}"
        );
    }
    group.bench_function(BenchmarkId::new("reconcile_change", &label), |b| {
        b.iter_custom(|iters| {
            (0..iters)
                .map(|_| {
                    vault
                        .write_atomic(&journal_path, paid_journal.as_bytes())
                        .expect("writes the journal");
                    vault
                        .write_atomic(&part_path, part_text.as_bytes())
                        .expect("restores the invoice");
                    index.refresh(&vault, std::slice::from_ref(&part_path));
                    let start = Instant::now();
                    let loaded = books.load().expect("loads");
                    let states = BookStates::build(loaded.journal(), Basis::of(&loaded));
                    let only = all_changed(&states);
                    let done = reconcile::reconcile_with(
                        &books,
                        &mut index,
                        &loaded,
                        &states,
                        &ReconcileOptions {
                            only: Some(only),
                            dry_run: false,
                        },
                    );
                    let elapsed = start.elapsed();
                    assert_eq!(done.changed.len(), 1, "{:?}", done.changed);
                    assert!(done.changed[0].written);
                    elapsed
                })
                .sum()
        });
    });
    vault
        .write_atomic(&journal_path, journal_text.as_bytes())
        .expect("restores the journal");
    vault
        .write_atomic(&part_path, part_text.as_bytes())
        .expect("restores the invoice");
    index.refresh(&vault, std::slice::from_ref(&part_path));

    // `check`'s books half: every invoice against the books (the journal
    // already loaded, as `check` has it from opening).
    let loaded = books.load().expect("loads");
    let states = BookStates::build(loaded.journal(), Basis::of(&loaded));
    let found = reconcile::audit(&books, &index, &loaded, &states, None);
    assert!(
        found.is_empty(),
        "{} findings, first {:?}",
        found.len(),
        found.first()
    );
    group.bench_function(BenchmarkId::new("books_check", &label), |b| {
        b.iter(|| reconcile::audit(&books, black_box(&index), &loaded, &states, None));
    });
    group.finish();

    let mut group = c.benchmark_group("ledgeline");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(15));
    group.bench_function(BenchmarkId::new("journal_open", &label), |b| {
        b.iter(|| JournalEditor::open(black_box(&main)).expect("opens"));
    });
    group.bench_function(BenchmarkId::new("journal_reopen", &label), |b| {
        b.iter(|| editor.reopen().expect("reopens"));
    });
    group.finish();
}

criterion_group!(documents, benches);
criterion_main!(documents);
