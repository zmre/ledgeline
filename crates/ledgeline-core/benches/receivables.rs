//! Criterion coverage of the invoicer's books bridge (invoicer PLAN Phase 2a,
//! ARCHITECTURE §5.6) on the plan-26 corpus with [`INVOICES`] invoices added
//! for [`CUSTOMERS`] customers over 2021–2025: about 60% paid 1:1, 10% paid
//! in two parts, 10% part-paid (an open N:1 link), 5% voided, the rest
//! open; every payment tagged with its invoice.
//!
//! | Bench | What it is |
//! |---|---|
//! | `receivables/document_index` | `DocumentIndex::build`: one pass |
//! | `receivables/find_document_all` | a fresh `Loaded`, then `find_document` for every invoice: the index and every fingerprint built once, then 5,000 lookups |
//! | `receivables/find_document_one` | one lookup on a warm index |
//! | `receivables/build` | `Receivables::build` (links included) |
//! | `receivables/aging` | the aging table as of today, on a built `Receivables` |
//! | `receivables/metrics` | DSO (90 days) and average days to pay |
//!
//! ```text
//! nix develop -c cargo bench -p ledgeline-core --bench receivables
//! LEDGELINE_BENCH_SIZES=200000 nix develop -c cargo bench -p ledgeline-core --bench receivables
//! ```

#[path = "corpus.rs"]
mod corpus;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ledgeline_core::accrual::documents::DocumentIndex;
use ledgeline_core::accrual::ops::Books;
use ledgeline_core::accrual::receivables::{Receivables, ReceivablesOptions};
use ledgeline_core::accrual::{AccrualRole, AccrualRoles};
use ledgeline_core::edit::JournalEditor;
use std::fmt::Write as _;
use std::hint::black_box;
use std::path::Path;
use std::time::Duration;

/// The corpus's last transaction date: "today".
const TODAY: &str = "2025-12-31";
/// Invoices (the invoicer's §5.6 corpus).
const INVOICES: usize = 5_000;
/// Customers.
const CUSTOMERS: usize = 500;
const AR: &str = "assets:accounts receivable";
const BANK: &str = "assets:bank:chase:checking";

fn date_of(day: i64) -> String {
    ledgeline_core::reports::add_days("2021-01-01", day)
}

fn cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("${sign}{}.{:02}", abs / 100, abs % 100)
}

/// The corpus with the invoices, their payments and voids appended.
fn variant(corpus: &str) -> String {
    let mut journal = String::with_capacity(corpus.len() + INVOICES * 400);
    let _ = writeln!(
        journal,
        "account {AR}  ; type: A, kpi: receivable\naccount revenue:consulting  ; type: R\n"
    );
    journal.push_str(corpus);
    for i in 0..INVOICES {
        let n = 10_001 + i;
        let day = i64::try_from(i * 1_800 / INVOICES).unwrap_or(0);
        let (date, due) = (date_of(day), date_of(day + 30));
        let amount = 50_000 + i64::try_from(i % 997).unwrap_or(0) * 100;
        let who = format!("Customer {}", i % CUSTOMERS);
        let link = format!("r{i:05}");
        let kind = i % 20;
        let tag = match kind {
            0..=14 => format!(", match:{link}"),
            15 => format!(", match:v{i:05}"),
            _ => String::new(),
        };
        let _ = write!(
            journal,
            "\n{date} {who} | Invoice {n}  ; invoice: {n}, due: {due}\n    {AR}  {}  ; customer: {who}{tag}\n    revenue:consulting  {}  ; customer: {who}, item: Services\n",
            cents(amount),
            cents(-amount)
        );
        let pay = |journal: &mut String, offset: i64, part: i64| {
            let _ = write!(
                journal,
                "\n{} {who} payment  ; invoice: {n}\n    {BANK}  {}\n    {AR}  {}  ; customer: {who}, match:{link}\n",
                date_of(day + offset),
                cents(part),
                cents(-part)
            );
        };
        match kind {
            // Paid 1:1.
            0..=11 => pay(&mut journal, 25, amount),
            // Paid in two parts.
            12 | 13 => {
                pay(&mut journal, 20, amount / 2);
                pay(&mut journal, 40, amount - amount / 2);
            }
            // Part-paid: an open N:1 link.
            14 => pay(&mut journal, 20, amount / 3),
            // Voided.
            15 => {
                let _ = write!(
                    journal,
                    "\n{} {who} | Invoice {n} (void)  ; invoice: {n}, void: {}\n    {AR}  {}  ; customer: {who}, match:v{i:05}\n    revenue:consulting  {}\n",
                    date_of(day + 5),
                    date_of(day + 5),
                    cents(-amount),
                    cents(amount)
                );
            }
            // Open.
            _ => {}
        }
    }
    journal
}

fn write(path: &Path, text: &str) {
    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        std::fs::write(path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }
}

fn bench_size(c: &mut Criterion, txns: usize) {
    let corpus_path = corpus::ensure_journal(txns);
    let label = corpus::shape_id(&corpus::V1.with_txns(txns));
    let text = std::fs::read_to_string(&corpus_path).expect("corpus");
    let stem = corpus_path
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("stem");
    let main = corpus_path.with_file_name(format!("{stem}-receivables.journal"));
    write(&main, &variant(&text));

    let books = Books::open(&main).expect("opens");
    let editor = JournalEditor::open(&main).expect("opens");
    let loaded = books.load_from(&editor).expect("loads");
    let journal = loaded.journal().clone();
    let roles = AccrualRoles::of(&journal);
    let options = ReceivablesOptions::default();
    let receivables = Receivables::build(&journal, &roles, &options);
    let numbers: Vec<String> = (0..INVOICES).map(|i| (10_001 + i).to_string()).collect();
    let links = ledgeline_core::matching::links(&journal);
    assert!(links.broken().is_empty(), "{:?}", &links.broken()[..1]);
    assert_eq!(
        receivables
            .documents()
            .iter()
            .filter(|d| d.number.is_some())
            .count(),
        INVOICES
    );
    assert!(numbers.iter().all(|n| loaded.find_document(n).is_some()));
    eprintln!(
        "receivables/{label}: {} transactions, {} documents, {} open, {} links",
        journal.transactions.len(),
        receivables.documents().len(),
        receivables.open(AccrualRole::Receivable).count(),
        links.pairs().len() + links.groups().len(),
    );

    let mut group = c.benchmark_group("receivables");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function(BenchmarkId::new("document_index", &label), |b| {
        b.iter(|| DocumentIndex::build(black_box(&journal), &roles));
    });
    group.bench_function(BenchmarkId::new("find_document_all", &label), |b| {
        b.iter_batched(
            || books.load_from(&editor).expect("loads"),
            |loaded| {
                numbers
                    .iter()
                    .filter(|n| loaded.find_document(black_box(n)).is_some())
                    .count()
            },
            criterion::BatchSize::PerIteration,
        );
    });
    group.bench_function(BenchmarkId::new("find_document_one", &label), |b| {
        b.iter(|| loaded.find_document(black_box("12345")));
    });
    group.bench_function(BenchmarkId::new("build", &label), |b| {
        b.iter(|| Receivables::build(black_box(&journal), &roles, &options));
    });
    group.bench_function(BenchmarkId::new("aging", &label), |b| {
        b.iter(|| receivables.aging(AccrualRole::Receivable, black_box(TODAY)));
    });
    group.bench_function(BenchmarkId::new("metrics", &label), |b| {
        b.iter(|| {
            (
                receivables.dso(AccrualRole::Receivable, black_box(TODAY), 90),
                receivables.average_days_to_pay(AccrualRole::Receivable),
            )
        });
    });
    group.finish();
}

fn all(c: &mut Criterion) {
    for txns in corpus::bench_sizes() {
        bench_size(c, txns);
    }
}

criterion_group!(benches, all);
criterion_main!(benches);
