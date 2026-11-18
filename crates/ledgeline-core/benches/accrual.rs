//! Criterion coverage of plan 45's accrual recognition on the plan-26 corpus
//! with [`CONTRACTS`] schedules added: each a deferred-revenue payment tagged
//! `recognition:`, its rules in a recognition file, and about half of its
//! periods up to "today" posted.
//!
//! | Bench | What it is | Target at 200k |
//! |---|---|---|
//! | `accrual/index` | `RecognitionIndex::build` (roles included) | < 10 ms |
//! | `accrual/index_with_roles` | the same with roles cached per snapshot | < 10 ms |
//! | `accrual/book_parse` | the recognition file's parse | < 15 ms |
//! | `accrual/due` | the due set as of today | < 5 ms |
//! | `accrual/ties` | the D6 ties over the due set | < 5 ms |
//! | `accrual/schedule_build_5k` | building 5,000 schedules and their rules | — |
//! | `accrual/group_rows` | `RecognitionRows::group`, no filter | < 2 ms |
//! | `accrual/post_5k` | 5,000 entries through one `add_transactions` | < 600 ms |
//!
//! ```text
//! nix develop -c cargo bench -p ledgeline-core --bench accrual
//! LEDGELINE_BENCH_SIZES=200000 nix develop -c cargo bench -p ledgeline-core --bench accrual
//! ```

#[path = "corpus.rs"]
mod corpus;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ledgeline_core::accrual::AccrualRoles;
use ledgeline_core::accrual::book::RecognitionBook;
use ledgeline_core::accrual::due::{due, entries_to_post, ties};
use ledgeline_core::accrual::index::RecognitionIndex;
use ledgeline_core::accrual::schedule::{
    Party, PartyKind, Proration, ScheduleLine, ScheduleSpec, Term, build,
};
use ledgeline_core::edit::{InsertPosition, JournalEditor};
use ledgeline_core::model::{Amount, Journal};
use ledgeline_core::parse::JournalStyles;
use ledgeline_core::view::filter::display_order;
use ledgeline_core::view::rows::{RecognitionMode, RecognitionRows};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::hint::black_box;
use std::path::Path;
use std::time::Duration;

/// The corpus's last transaction date: "today".
const TODAY: &str = "2025-12-31";
/// Active schedules (consumer scale).
const CONTRACTS: usize = 5_000;
const DEFERRED: &str = "liabilities:deferred-revenue";

fn date_of(day: i64) -> String {
    // 2023-01-01 is day 0; 36 months of starts.
    ledgeline_core::reports::add_days("2023-01-01", day)
}

fn specs(journal: &Journal) -> Vec<ScheduleSpec> {
    let amount: Amount = ledgeline_core::accrual::invoice::money(
        journal,
        "$",
        ledgeline_core::Dec::new(-120_000, 2),
    );
    (0..CONTRACTS)
        .map(|i| {
            let start = date_of(i64::try_from(i * 1093 / CONTRACTS).unwrap_or(0));
            let mut total = amount.clone();
            total.quantity =
                ledgeline_core::Dec::new(-(120_000 + i128::try_from(i % 97).unwrap_or(0) * 100), 2);
            ScheduleSpec {
                id: format!("c{i:05}"),
                lines: vec![ScheduleLine {
                    source_account: DEFERRED.into(),
                    target_account: "revenue:subscriptions".into(),
                    total,
                    party: Some(Party {
                        kind: PartyKind::Customer,
                        name: format!("Customer {i}"),
                    }),
                }],
                start,
                term: Term::Months([12, 24, 36][i % 3]),
                proration: if i % 5 == 0 {
                    Proration::Calendar
                } else {
                    Proration::Anniversary
                },
                description: format!("Customer {i} Recognized"),
                header_tags: vec![],
            }
        })
        .collect()
}

/// The corpus with the contracts' sources and half their due entries; the
/// recognition file's text; and 5,000 still-due entries to post.
fn variant(corpus: &str) -> (String, String) {
    let base = ledgeline_core::parse_journal(corpus, "base.journal").expect("corpus parses");
    let styles = JournalStyles::of(&base);
    let mut journal = String::with_capacity(corpus.len() * 2);
    let _ = writeln!(
        journal,
        "account {DEFERRED}  ; type: L, kpi: deferred-revenue\naccount revenue:subscriptions  ; type: R\n"
    );
    journal.push_str(corpus);
    let mut rules = String::from("commodity $1,000.00\n\n");
    for spec in specs(&base) {
        let schedule = build(&spec, &styles).expect("builds");
        let line = &spec.lines[0];
        let _ = write!(
            journal,
            "\n{} {} Payment\n    {DEFERRED}  {}  ; customer: {}, recognition: {}, term-months: {}{}\n    assets:bank:chase:checking\n",
            spec.start,
            line.party.as_ref().map_or("", |p| p.name.as_str()),
            ledgeline_core::edit::render_amount(&line.total),
            line.party.as_ref().map_or("", |p| p.name.as_str()),
            spec.id,
            match spec.term {
                Term::Months(n) => n,
                Term::Through(_) => 0,
            },
            if spec.proration == Proration::Calendar {
                ", proration: calendar"
            } else {
                ""
            },
        );
        let _ = write!(rules, "; ── {}\n{}\n", spec.id, schedule.rules_text());
    }
    (journal, rules)
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
    let (unposted, rules) = variant(&text);
    let stem = corpus_path
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("stem");
    let main = corpus_path.with_file_name(format!("{stem}-accrual.journal"));
    let rules_path = corpus_path.with_file_name(format!("{stem}-accrual-recognition.journal"));
    write(&rules_path, &rules);

    // Post about half of each schedule's due periods (the earlier half).
    let base = ledgeline_core::parse_journal(&unposted, &main.to_string_lossy()).expect("parses");
    let book =
        RecognitionBook::parse(&rules, &rules_path, &base.commodity_styles).expect("rules parse");
    let index = RecognitionIndex::build(&base, &book);
    let all_due = due(&index, &book, TODAY, None);
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let counts = all_due
        .open
        .iter()
        .fold(std::collections::HashMap::new(), |mut m, e| {
            *m.entry(e.id.as_str()).or_insert(0usize) += 1;
            m
        });
    let first_half: Vec<_> = all_due
        .open
        .iter()
        .filter(|e| {
            let n = seen.entry(e.id.as_str()).or_insert(0);
            *n += 1;
            *n <= counts[e.id.as_str()] / 2
        })
        .cloned()
        .collect();
    let mut posted_text = unposted.clone();
    for txn in entries_to_post(&book, &first_half) {
        posted_text.push('\n');
        posted_text.push_str(&ledgeline_core::edit::format_transaction(&txn));
    }
    write(&main, &posted_text);
    let journal =
        ledgeline_core::parse_journal(&posted_text, &main.to_string_lossy()).expect("parses");
    let roles = AccrualRoles::of(&journal);
    let index = RecognitionIndex::build_with(&journal, &book, roles.clone());
    let set = due(&index, &book, TODAY, None);
    let to_post: Vec<_> = set.open.iter().take(5_000).cloned().collect();
    eprintln!(
        "accrual/{label}: {} transactions, {} schedules, {} posted entries, {} due",
        journal.transactions.len(),
        index.schedules().len(),
        first_half.len(),
        set.open.len()
    );

    let mut group = c.benchmark_group("accrual");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function(BenchmarkId::new("index", &label), |b| {
        b.iter(|| RecognitionIndex::build(black_box(&journal), &book));
    });
    group.bench_function(BenchmarkId::new("index_with_roles", &label), |b| {
        b.iter(|| RecognitionIndex::build_with(black_box(&journal), &book, roles.clone()));
    });
    group.bench_function(BenchmarkId::new("book_parse", &label), |b| {
        b.iter(|| {
            RecognitionBook::parse(black_box(&rules), &rules_path, &journal.commodity_styles)
        });
    });
    group.bench_function(BenchmarkId::new("due", &label), |b| {
        b.iter(|| due(black_box(&index), &book, TODAY, None));
    });
    group.bench_function(BenchmarkId::new("ties", &label), |b| {
        b.iter(|| ties(black_box(&journal), &index, &book, &set));
    });
    let specs = specs(&journal);
    let styles = JournalStyles::of(&journal);
    group.bench_function(BenchmarkId::new("schedule_build_5k", &label), |b| {
        b.iter(|| {
            specs
                .iter()
                .map(|spec| build(black_box(spec), &styles).expect("builds"))
                .collect::<Vec<_>>()
        });
    });
    let rows = RecognitionRows::of(&journal);
    let order = display_order(&journal);
    let none = HashSet::new();
    group.bench_function(BenchmarkId::new("group_rows", &label), |b| {
        b.iter(|| rows.group(&journal, black_box(&order), RecognitionMode::Group, &none));
    });
    let entries = entries_to_post(&book, &to_post);
    let editor = JournalEditor::open(&main).expect("opens");
    group.bench_function(BenchmarkId::new("post_5k", &label), |b| {
        b.iter_batched(
            || editor.reopen().expect("reopens"),
            |mut editor| {
                editor
                    .add_transactions(black_box(&entries), InsertPosition::DateOrdered)
                    .expect("posts")
            },
            criterion::BatchSize::PerIteration,
        );
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
