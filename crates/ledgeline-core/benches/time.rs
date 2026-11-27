//! Criterion coverage of plan 46's time engine against its Performance
//! budgets, on a generated book of 10 years × 5 people × 8 lines a day
//! (~146k entry lines in 600 monthly timedot files, ~165k lines in all).
//!
//! | Bench | What it is | Budget |
//! |---|---|---|
//! | `time/load` | `book::load`: the roll-up, every glob-included file read and parsed losslessly, accounts resolved | < 150 ms |
//! | `time/unbilled` | `query::unbilled` over everything | < 100 ms |
//! | `time/dashboard` | the dashboard's figures for one month: tree, by customer, by billable, by person | < 100 ms |
//! | `time/suggest` | suggested monthly periods for every customer | < 100 ms |
//! | `time/recents` | slots 6–9 | — |
//!
//! ```text
//! nix develop -c cargo bench -p ledgeline-core --bench time
//! ```

use criterion::{Criterion, criterion_group, criterion_main};
use ledgeline_core::reports::add_days;
use ledgeline_core::time::book::{self, TimeConfig};
use ledgeline_core::time::query::{self, Cadence, Filter, GroupBy, Period};
use ledgeline_core::vault::{MemVault, VaultPath};
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Duration;

const PEOPLE: [&str; 5] = ["pat", "sam", "ann", "lee", "kim"];
const YEARS: i64 = 10;
const LINES_PER_DAY: usize = 8;
const CUSTOMERS: usize = 40;

fn account(i: usize) -> String {
    match i % 10 {
        0 => "internal:admin".to_string(),
        1 => "internal:sales".to_string(),
        n => format!(
            "clients:c{:02}:{}",
            (i / 10 + n) % CUSTOMERS,
            ["advisory", "audit", "tax"][n % 3]
        ),
    }
}

fn corpus() -> (MemVault, usize) {
    let vault = MemVault::with_root("/books");
    vault.insert_file("books.journal", "");
    vault.insert_file(
        "time.journal",
        "commodity \"\"\ninclude time-accounts.journal\ninclude time/**/*.timedot\n",
    );
    let mut accounts =
        String::from("account clients  ; billable: yes\naccount internal  ; billable: no\n");
    for c in 0..CUSTOMERS {
        let _ = writeln!(
            accounts,
            "account clients:c{c:02}  ; customer: Customer {c}"
        );
    }
    vault.insert_file("time-accounts.journal", &accounts);
    let mut lines = 0;
    for person in PEOPLE {
        for month in 0..YEARS * 12 {
            let (year, mm) = (2016 + month / 12, month % 12 + 1);
            let first = format!("{year:04}-{mm:02}-01");
            let mut text = format!("# Time — {person} — {year}-{mm:02}\n\n");
            let mut day = first.clone();
            while day.starts_with(&first[..7]) {
                let _ = writeln!(text, "{day} Mon  ; person: {person}");
                for n in 0..LINES_PER_DAY {
                    let i = (month as usize) * 31 + n * 7 + day.len() + person.len();
                    let tag = match i % 9 {
                        0 => "  ; invoice: 1001",
                        1 => "  ; a note on the work",
                        2 => "  ; nobill: courtesy",
                        _ => "",
                    };
                    let _ = writeln!(text, "    {:<30} {:>4}m{tag}", account(i + n), 5 + (i % 90));
                    lines += 1;
                }
                text.push('\n');
                lines += 2;
                day = add_days(&day, 1);
            }
            vault.insert_file(&format!("time/{person}/{year:04}-{mm:02}.timedot"), &text);
        }
    }
    (vault, lines)
}

fn all(c: &mut Criterion) {
    let (vault, lines) = corpus();
    let main = VaultPath::new("books.journal").expect("main");
    let config = TimeConfig::default();
    let book = book::load(&vault, &main, &config, None)
        .expect("load")
        .expect("book");
    eprintln!(
        "time corpus: {lines} lines, {} entries, {} files",
        book.entries.len(),
        book.sources.len()
    );
    let mut group = c.benchmark_group("time");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(8));
    group.bench_function("load", |b| {
        b.iter(|| book::load(black_box(&vault), &main, &config, None).expect("load"));
    });
    group.bench_function("unbilled", |b| {
        b.iter(|| query::unbilled(black_box(&book), None));
    });
    let month = Filter {
        period: Period::between("2025-11-01", "2025-12-01"),
        ..Filter::default()
    };
    group.bench_function("dashboard", |b| {
        b.iter(|| {
            (
                query::account_tree(black_box(&book), &month),
                query::summary(&book, &month, GroupBy::Customer),
                query::summary(&book, &month, GroupBy::Billable),
                query::summary(&book, &month, GroupBy::Person),
            )
        });
    });
    let unbilled = query::unbilled(&book, None);
    group.bench_function("suggest", |b| {
        b.iter(|| {
            unbilled
                .customers
                .iter()
                .map(|(_, batch)| {
                    query::suggest_periods(&book, batch, Cadence::Monthly, "2025-12-15").len()
                })
                .sum::<usize>()
        });
    });
    group.bench_function("recents", |b| {
        b.iter(|| query::recents(black_box(&book), 4, &[]));
    });
    group.finish();
}

criterion_group!(benches, all);
criterion_main!(benches);
