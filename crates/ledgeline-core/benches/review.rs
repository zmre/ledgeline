//! Criterion coverage of the transaction-review engine (plan 32 D14) on the
//! plan-26 corpus, against its budget:
//!
//! | Bench | Budget at 200k |
//! |---|---|
//! | `review/queue_{recents,unknowns}` | < 20 ms |
//! | `review/similar`, `review/suggest` | < 5 ms |
//! | `review/batch_1` | < `review/patch_1` (one surgical PATCH today) |
//! | `review/batch_50` | < 2 × `review/patch_1` |
//!
//! `review/build` is the per-generation index the server caches; it is paid
//! once per journal change, off the request path.
//!
//! # Running
//!
//! ```text
//! nix develop -c cargo bench -p ledgeline-core --bench review
//! LEDGELINE_BENCH_SIZES=200000 nix develop -c cargo bench -p ledgeline-core --bench review
//! ```
//!
//! The corpus has no `expenses:unknown` postings, so the Unknowns queue is
//! measured as the full scan it is (an empty result costs the same walk), and
//! `similar`/`suggest` target the transaction with the BIGGEST similarity
//! bucket — the worst case, not a typical one.

#[path = "corpus.rs"]
mod corpus;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ledgeline_core::edit::{EditOp, JournalEditor};
use ledgeline_core::model::{Journal, Tindex};
use ledgeline_core::review::{QueueKind, QueueOpts, ReviewIndex, similarity_key};
use ledgeline_core::view::Classes;
use std::collections::HashMap;
use std::hint::black_box;
use std::time::{Duration, Instant};

/// The corpus's last transaction date: the queue's "today".
const TODAY: &str = "2025-12-31";

/// The transaction whose similarity key is shared by the most others, newest
/// of them.
fn biggest_bucket(journal: &Journal) -> Tindex {
    let mut buckets: HashMap<String, Vec<Tindex>> = HashMap::new();
    for txn in &journal.transactions {
        if let Some(key) = similarity_key(&txn.description) {
            buckets.entry(key).or_default().push(txn.index);
        }
    }
    buckets
        .into_values()
        .max_by_key(Vec::len)
        .and_then(|bucket| bucket.last().copied())
        .expect("the corpus has descriptions")
}

fn bench_queries(c: &mut Criterion, label: &str, journal: &Journal) {
    let classes = Classes::of(journal);
    let mut group = c.benchmark_group("review");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function(BenchmarkId::new("build", label), |b| {
        b.iter(|| ReviewIndex::build(black_box(journal), &classes, |_| true));
    });

    let index = ReviewIndex::build(journal, &classes, |_| true);
    for (name, kind) in [
        ("queue_recents", QueueKind::Recents),
        ("queue_unknowns", QueueKind::Unknowns),
    ] {
        let opts = QueueOpts::new(kind, TODAY);
        group.bench_function(BenchmarkId::new(name, label), |b| {
            b.iter(|| index.queue(black_box(journal), &opts));
        });
    }

    let target = biggest_bucket(journal);
    group.bench_function(BenchmarkId::new("similar", label), |b| {
        b.iter(|| index.similar(black_box(journal), &classes, target));
    });
    // The first call detects the journal's subscriptions (the recurring
    // source) and builds the prefix table; both are cached per index, so
    // this measures every call after it.
    let _ = index.suggest(journal, &classes, target);
    group.bench_function(BenchmarkId::new("suggest", label), |b| {
        b.iter(|| index.suggest(black_box(journal), &classes, target));
    });
    group.finish();
}

/// The newest `count` transactions, the shape of a review save.
fn newest(editor: &JournalEditor, count: usize) -> Vec<Tindex> {
    let transactions = &editor.journal().transactions;
    transactions[transactions.len() - count..]
        .iter()
        .map(|txn| txn.index)
        .collect()
}

/// A recategorise-and-review save of `targets`, alternating values by
/// `round` so every iteration really changes the text.
fn review_batch(
    editor: &JournalEditor,
    targets: &[Tindex],
    round: u64,
) -> Vec<(Tindex, ledgeline_core::Fingerprint, Vec<EditOp>)> {
    let fingerprints = editor.transaction_fingerprints(targets);
    targets
        .iter()
        .zip(fingerprints)
        .map(|(&index, fingerprint)| {
            let account = if round.is_multiple_of(2) {
                "expenses:review:a"
            } else {
                "expenses:review:b"
            };
            let ops = vec![
                EditOp::SetPostingAccount {
                    posting: 0,
                    account: account.to_string(),
                },
                EditOp::SetTxnTag {
                    name: "reviewed".to_string(),
                    value: Some(format!("2026-10-{:02}", 1 + round % 2)),
                },
            ];
            (index, fingerprint.expect("target exists"), ops)
        })
        .collect()
}

fn bench_writes(c: &mut Criterion, label: &str, path: &std::path::Path) {
    let mut editor = JournalEditor::open(path).expect("corpus opens");
    let mut group = c.benchmark_group("review");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));

    // Today's single surgical PATCH: one account, one reparse.
    let one = newest(&editor, 1)[0];
    group.bench_function(BenchmarkId::new("patch_1", label), |b| {
        let mut round = 0u64;
        b.iter(|| {
            round += 1;
            let account = if round.is_multiple_of(2) {
                "expenses:review:a"
            } else {
                "expenses:review:b"
            };
            editor
                .set_posting_account(one, 0, account)
                .expect("patch applies");
        });
    });

    for count in [1usize, 50] {
        let targets = newest(&editor, count);
        group.bench_function(BenchmarkId::new(format!("batch_{count}"), label), |b| {
            let mut round = 0u64;
            b.iter_custom(|iters| {
                let mut spent = Duration::ZERO;
                for _ in 0..iters {
                    round += 1;
                    // Fingerprinting is the caller's (it happens when the
                    // queue is served), so only the batch itself is timed.
                    let edits = review_batch(&editor, &targets, round);
                    let started = Instant::now();
                    editor.apply_batch(&edits).expect("batch applies");
                    spent += started.elapsed();
                }
                spent
            });
        });
    }
    group.finish();
}

fn all(c: &mut Criterion) {
    for txns in corpus::bench_sizes() {
        let path = corpus::ensure_journal(txns);
        let label = corpus::shape_id(&corpus::V1.with_txns(txns));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let journal = ledgeline_core::parse_journal(&text, &path.to_string_lossy())
            .unwrap_or_else(|e| panic!("generated corpus must parse: {e}"));
        bench_queries(c, &label, &journal);
        bench_writes(c, &label, &path);
    }
}

criterion_group!(benches, all);
criterion_main!(benches);
