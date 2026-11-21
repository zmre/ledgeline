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
use invoicer_core::doc::Money;
use invoicer_core::draft;
use invoicer_core::index::{DocumentIndex, ListFilter};
use invoicer_core::problem::ProblemCode;
use invoicer_core::settings::Settings;
use ledgeline_core::edit::JournalEditor;
use ledgeline_core::vault::{LocalVault, Vault};
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
