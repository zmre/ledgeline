//! The time fixture (`fixtures/time/`, plan 46 Phase 1) against hledger 1.52's
//! own figures, captured by `fixtures/time/capture.sh` into `expected/`.
//!
//! hledger 1.52 rounds every unit amount (`20m` → 0.33) and every timeclock
//! session to 0.01 h; Ledgeline keeps the exact time (T4). So each figure is
//! compared in hours to within 0.005 h per rounded posting behind it (at
//! least 0.01 h), the most that rounding can move a sum.
//!
//! And T12: the books with the roll-up included read exactly as the books
//! alone in every money report.

mod common;

use common::fixtures_dir;
use ledgeline_core::Journal;
use ledgeline_core::model::PostingType;
use ledgeline_core::parse_journal;
use ledgeline_core::reports::{
    BudgetOpts, InsightsOpts, Interval, account_decls, balance_sheet, budget_report,
    declared_types, income_statement, insights,
};
use ledgeline_core::time::book::{self, TimeConfig};
use ledgeline_core::time::model::{TimeBook, TimeEntry};
use ledgeline_core::time::query::{self, Filter, GroupBy};
use ledgeline_core::time::{TagRole, TimeQty};
use ledgeline_core::vault::LocalVault;
use ledgeline_core::view::Classes;
use ledgeline_core::view::checks::{engine_findings, local_findings};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn dir() -> PathBuf {
    fixtures_dir().join("time")
}

fn load_book() -> TimeBook {
    let (vault, main) =
        LocalVault::for_journal(&dir().join("books.journal")).expect("fixture vault");
    book::load(&vault, &main, &TimeConfig::default(), None)
        .expect("roll-up reads")
        .expect("roll-up exists")
}

fn parse(name: &str) -> Journal {
    let path = dir().join(name);
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    parse_journal(&text, &path.to_string_lossy()).expect("fixture parses")
}

/// `(first column, hours)` rows of an hledger `bal` CSV, the `Total:` row
/// left out. A mixed cell (`33.00, 7.26h`) is the sum of its parts.
fn balances(name: &str) -> Vec<(String, f64)> {
    let path = dir().join("expected").join(name);
    let mut reader = csv::Reader::from_path(&path).expect("golden readable");
    reader
        .records()
        .map(|row| row.expect("row"))
        .filter(|row| &row[0] != "Total:")
        .map(|row| (row[0].to_string(), hours_cell(&row[1])))
        .collect()
}

fn hours_cell(cell: &str) -> f64 {
    cell.split(", ")
        .map(|part| part.trim().trim_end_matches('h'))
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f64>().expect("hours"))
        .sum()
}

/// Whether hledger 1.52 shows `entry` rounded (anything not exact to 0.01 h).
fn rounded(entry: &TimeEntry) -> bool {
    entry.quantity.hledger_152_hours() != entry.quantity.hours()
}

fn tolerance<'e>(entries: impl Iterator<Item = &'e TimeEntry>) -> f64 {
    let rounded = entries.filter(|entry| rounded(entry)).count();
    (0.005 * rounded as f64).max(0.01) + 1e-9
}

fn close(what: &str, ours: f64, theirs: f64, tolerance: f64) {
    assert!(
        (ours - theirs).abs() <= tolerance,
        "{what}: Ledgeline {ours:.4} h, hledger {theirs:.2} h (tolerance {tolerance:.3})"
    );
}

/// Compare a grouping of ours to an hledger pivot, key by key.
fn compare_groups(name: &str, ours: &BTreeMap<String, (f64, f64)>) {
    let theirs = balances(name);
    assert_eq!(
        theirs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        ours.keys().map(String::as_str).collect::<Vec<_>>(),
        "{name}: the groups differ"
    );
    for (key, hours) in theirs {
        let (mine, tol) = ours[&key];
        close(&format!("{name} {key:?}"), mine, hours, tol);
    }
}

#[test]
fn the_account_tree_matches_bal_tree() {
    let book = load_book();
    assert!(
        book.findings
            .iter()
            .all(|f| f.finding.severity < ledgeline_core::time::Severity::Warning),
        "{:?}",
        book.findings
    );
    let tree = query::account_tree(&book, &Filter::default());
    let ours: BTreeMap<&str, TimeQty> =
        tree.iter().map(|r| (r.account.as_str(), r.total)).collect();
    // `bal --tree` writes each name relative to its parent, indented two
    // no-break spaces (each followed by a space) per level, and folds a
    // single-child chain into one row (`globex:audit`).
    let mut stack: Vec<String> = Vec::new();
    let rows = balances("bal-tree.csv");
    assert!(rows.len() >= 10);
    for (name, hours) in rows {
        let depth = name.chars().filter(|c| *c == '\u{a0}').count() / 2;
        stack.truncate(depth);
        stack.push(name.trim().to_string());
        let account = stack.join(":");
        let total = ours
            .get(account.as_str())
            .unwrap_or_else(|| panic!("hledger shows {account}; Ledgeline doesn't"));
        let tol = tolerance(
            book.entries
                .iter()
                .filter(|e| ledgeline_core::time::model::is_under(&e.account, &account)),
        );
        close(&account, total.hours_f64(), hours, tol);
    }
}

#[test]
fn billable_customer_and_person_match_the_pivots() {
    let book = load_book();
    let group = |key: &dyn Fn(&TimeEntry) -> String| {
        let mut out: BTreeMap<String, Vec<&TimeEntry>> = BTreeMap::new();
        for entry in &book.entries {
            out.entry(key(entry)).or_default().push(entry);
        }
        out.into_iter()
            .map(|(k, entries)| {
                let hours = entries.iter().map(|e| e.quantity.hours_f64()).sum();
                (k, (hours, tolerance(entries.into_iter())))
            })
            .collect::<BTreeMap<_, _>>()
    };
    compare_groups(
        "bal-pivot-billable.csv",
        &group(&|e| {
            if query::is_billable(&book, e) {
                "yes"
            } else {
                "no"
            }
            .to_string()
        }),
    );
    compare_groups(
        "bal-pivot-customer.csv",
        &group(&|e| query::customer_of(&book, e).unwrap_or("").to_string()),
    );
    // hledger sees only the `person:` tag; Ledgeline also reads the folder.
    compare_groups(
        "bal-pivot-person.csv",
        &group(&|e| {
            book.tags
                .value(&e.tags, TagRole::Person)
                .unwrap_or("")
                .to_string()
        }),
    );
    let by_person = query::summary(&book, &Filter::default(), GroupBy::Person);
    assert!(
        by_person.iter().all(|(person, _)| !person.is_empty()),
        "{by_person:?}"
    );
    let sam = by_person.iter().find(|(p, _)| p == "sam").expect("sam");
    assert_eq!(
        sam.1,
        TimeQty::from_minutes(3 * 60 + 60 + 270 + 30 + 60)
            .plus(TimeQty::from_seconds(3 * 3600 + 30 * 60 + 30))
    );
}

/// `(date, account, hours)` rows of an hledger `reg` CSV.
fn register(name: &str) -> Vec<(String, String, f64)> {
    let mut reader =
        csv::Reader::from_path(dir().join("expected").join(name)).expect("golden readable");
    reader
        .records()
        .map(|row| row.expect("row"))
        .map(|row| {
            (
                row[1].to_string(),
                row[4].trim_matches(['(', ')']).to_string(),
                row[5]
                    .trim_start_matches('$')
                    .trim_end_matches('h')
                    .parse()
                    .expect("amount"),
            )
        })
        .collect()
}

#[test]
fn invoice_101_matches_reg() {
    let book = load_book();
    let ours: Vec<(String, String, f64)> = book
        .entries
        .iter()
        .filter(|e| book.tags.value(&e.tags, TagRole::Invoice) == Some("101"))
        .map(|e| (e.date.clone(), e.account.clone(), e.quantity.hours_f64()))
        .collect();
    assert_eq!(ours, register("reg-invoice-101.csv"));

    // Across books and time: the journal reads the time files (T12) into
    // their own list, and the money side is untouched.
    let journal = parse("with-time.journal");
    let tagged = |tags: &[(String, String)]| tags.iter().any(|(n, v)| n == "invoice" && v == "101");
    let mut rows: Vec<(String, String, f64)> = journal
        .time_transactions
        .iter()
        .chain(&journal.transactions)
        .flat_map(|txn| {
            txn.postings
                .iter()
                .filter(move |p| tagged(&txn.tags) || tagged(&p.tags))
                .map(move |p| {
                    (
                        txn.date.clone(),
                        p.account.0.clone(),
                        p.amounts[0].quantity.floating_point(),
                    )
                })
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(rows, register("reg-invoice-101-with-books.csv"));
}

#[test]
fn time_transactions_match_print() {
    let ours = parse("time.journal");
    assert!(
        ours.transactions.is_empty(),
        "time is never a money transaction"
    );
    let golden = {
        let path = dir().join("expected/print.journal");
        let text = std::fs::read_to_string(&path).expect("golden");
        parse_journal(&text, &path.to_string_lossy()).expect("golden parses")
    };
    let mut mine = ours.time_transactions.clone();
    // hledger prints in date order, stable.
    mine.sort_by(|a, b| a.date.cmp(&b.date));
    assert_eq!(mine.len(), golden.transactions.len());
    for (a, b) in mine.iter().zip(&golden.transactions) {
        assert_eq!(
            (a.date.as_str(), a.description.as_str()),
            (b.date.as_str(), b.description.as_str())
        );
        assert_eq!(a.status, b.status);
        let tags = |t: &[(String, String)]| {
            let mut t = t.to_vec();
            t.sort();
            t
        };
        assert_eq!(tags(&a.tags), tags(&b.tags), "{} {}", a.date, a.description);
        assert_eq!(
            a.postings.len(),
            b.postings.len(),
            "{} {}",
            a.date,
            a.description
        );
        for (p, q) in a.postings.iter().zip(&b.postings) {
            assert_eq!(p.account, q.account);
            assert_eq!(p.ptype, PostingType::Virtual);
            assert_eq!(q.ptype, PostingType::Virtual);
            assert_eq!(p.amounts[0].commodity, q.amounts[0].commodity);
            assert_eq!(tags(&p.tags), tags(&q.tags), "{} {}", a.date, p.account.0);
            close(
                &format!("{} {}", a.date, p.account.0),
                p.amounts[0].quantity.floating_point(),
                q.amounts[0].quantity.floating_point(),
                0.005 + 1e-9,
            );
        }
    }
}

#[test]
fn including_the_roll_up_leaves_every_money_report_alone() {
    let books = parse("books.journal");
    let with_time = parse("with-time.journal");
    assert!(!with_time.time_transactions.is_empty());
    assert!(books.time_transactions.is_empty());
    assert_eq!(books.transactions, with_time.transactions);
    assert_eq!(books.prices, with_time.prices);
    assert_eq!(books.periodic_transactions, with_time.periodic_transactions);

    let reports = |journal: &Journal| -> Vec<String> {
        let declared = declared_types(&account_decls(journal));
        let classes = Classes::of(journal);
        vec![
            format!(
                "{:?}",
                balance_sheet(&journal.transactions, "2026-11-30", 3, &declared)
            ),
            format!(
                "{:?}",
                income_statement(
                    &journal.transactions,
                    "2026-09-01",
                    "2026-11-30",
                    3,
                    &declared
                )
            ),
            format!(
                "{:?}",
                insights(
                    journal,
                    &InsightsOpts {
                        start: "2026-09-01",
                        end: "2026-11-30",
                        cost_exclude: &[],
                        change_min: ledgeline_core::Dec::zero(),
                    },
                )
            ),
            format!(
                "{:?}",
                budget_report(
                    &journal.transactions,
                    &journal.periodic_transactions,
                    &BudgetOpts {
                        end: "2026-11-30",
                        interval: Interval::Monthly,
                        count: 3,
                        depth: 9,
                        budget_desc: None,
                    },
                )
            ),
            format!("{:?}", engine_findings(journal)),
            format!("{:?}", local_findings(journal, &classes, "2026-11-30")),
            format!("{:?}", ledgeline_core::wire::journal_to_value(journal).ok()),
        ]
    };
    let (a, b) = (reports(&books), reports(&with_time));
    for (n, (a, b)) in a.iter().zip(&b).enumerate() {
        assert_eq!(a, b, "report {n} changed when the roll-up was included");
    }
    // hledger agrees, once the time commodities are filtered out.
    assert_eq!(
        std::fs::read_to_string(dir().join("expected/books-bal.csv")).expect("golden"),
        std::fs::read_to_string(dir().join("expected/with-time-money-bal.csv")).expect("golden"),
    );
}

#[test]
fn unbilled_and_suggested_periods_on_the_fixture() {
    let book = load_book();
    let report = query::unbilled(&book, None);
    let customers: Vec<&str> = report.customers.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(customers, vec!["Acme Corp", "Globex", "Initech"]);
    let acme = &report.customers[0].1;
    // Not billed: the aab letters (45m), Oct 30 advisory (1h), Nov 2 (90m),
    // Nov 25 (130m), Nov 26 call (20m) and advisory (1h), Sam's Nov 24 (1h).
    // Travel is not billable; the 20m courtesy call is nobill.
    assert_eq!(
        acme.quantity,
        TimeQty::from_minutes(45 + 60 + 90 + 130 + 20 + 60 + 60)
    );
    let periods = query::suggest_periods(&book, acme, query::Cadence::Monthly, "2026-11-26");
    assert_eq!(periods.len(), 1);
    assert_eq!(
        (periods[0].start.as_str(), periods[0].end.as_str()),
        ("2026-10-01", "2026-10-31")
    );
    let unpaid = query::unpaid(&book, None);
    let sam = unpaid.iter().find(|(p, _)| p == "sam").expect("sam");
    // Sam's unpaid: the Nov 4 lines and Nov 24, plus the imported session.
    assert_eq!(
        sam.1.quantity,
        TimeQty::from_minutes(270 + 30 + 60).plus(TimeQty::from_seconds(3 * 3600 + 30 * 60 + 30))
    );
}
