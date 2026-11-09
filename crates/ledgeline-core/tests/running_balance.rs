//! The journal view's running balance (plan 36).
//!
//! `fixtures/journal-view/errors/running/` holds a journal written out of date
//! order, with same-day ties, an `include`, posting-dated legs (one
//! transaction with two different `date:` tags), virtual and balanced-virtual
//! postings, a cost, and passing, failing and `==` zero-check assertions. Its
//! `reg-*.csv` files are `hledger reg -I -H -O csv 'assets:bank' cur:<c>`
//! captured from hledger 1.52 by `scripts/gen-golden.sh`; never regenerate
//! them from Rust.

mod common;

use common::fixtures_dir;
use ledgeline_core::assertions::{check_balance_assertions, posting_order};
use ledgeline_core::auto;
use ledgeline_core::model::{Commodity, Journal};
use ledgeline_core::reports::Interval;
use ledgeline_core::view::running::{AssertionMark, BalanceWindow, RunningBalance};
use ledgeline_core::{Dec, parse_journal};
use proptest::prelude::*;
use std::collections::BTreeMap;

const ROOT: &str = "assets:bank";

fn fixture() -> Journal {
    let path = fixtures_dir().join("journal-view/errors/running/main.journal");
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    parse_journal(&text, &path.to_string_lossy()).expect("fixture parses")
}

fn build(journal: &Journal, root: &str) -> RunningBalance {
    RunningBalance::build(journal, &posting_order(journal), root).expect("no overflow")
}

fn dec(text: &str) -> Dec {
    Dec::parse(text, '.').expect("a decimal")
}

/// The position of the transaction with `tindex`.
fn at(journal: &Journal, tindex: u32) -> u32 {
    let position = journal
        .transactions
        .iter()
        .position(|txn| txn.index.0 == tindex)
        .expect("tindex exists");
    u32::try_from(position).expect("small")
}

/// `(change, balance, touches, posted_on)` for the row with `tindex`.
fn cell(
    journal: &Journal,
    running: &RunningBalance,
    commodity: &str,
    tindex: u32,
) -> (Option<Dec>, Dec, bool, Option<String>) {
    let cell = running
        .cell(journal, commodity, at(journal, tindex))
        .expect("a cell");
    (
        cell.change,
        cell.balance,
        cell.touches,
        cell.posted_on.map(str::to_string),
    )
}

// ---------------------------------------------------------------------------
// hledger parity
// ---------------------------------------------------------------------------

/// One `hledger reg -O csv` line.
struct RegLine {
    tindex: u32,
    date: String,
    account: String,
    amount: Dec,
    total: Dec,
}

/// An hledger CSV amount (`$-4.50`, `100,00 EUR`, a bare `0`) as a decimal.
fn hledger_amount(text: &str, commodity: &str) -> Dec {
    let number = text.replace(commodity, "");
    let number = number.trim();
    let number = if commodity == "EUR" {
        number.replace('.', "").replace(',', ".")
    } else {
        number.replace(',', "")
    };
    dec(&number)
}

fn reg(file: &str, commodity: &str) -> Vec<RegLine> {
    let path = fixtures_dir()
        .join("journal-view/errors/running")
        .join(file);
    let text = std::fs::read_to_string(path).expect("golden readable");
    text.lines()
        .skip(1)
        .map(|line| {
            let fields: Vec<&str> = line.trim_matches('"').split("\",\"").collect::<Vec<_>>();
            assert_eq!(fields.len(), 7, "{line}");
            RegLine {
                tindex: fields[0].parse().expect("txnidx"),
                date: fields[1].to_string(),
                account: fields[4].trim_matches(['(', ')', '[', ']']).to_string(),
                amount: hledger_amount(fields[5], commodity),
                total: hledger_amount(fields[6], commodity),
            }
        })
        .collect()
}

/// The stream, posting by posting, is exactly `hledger reg`'s register:
/// same postings, same order, same amounts, same running totals.
#[test]
fn every_posting_matches_hledger_reg() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    for (file, commodity) in [("reg-usd.csv", "$"), ("reg-eur.csv", "EUR")] {
        let lines = reg(file, commodity);
        let entries = running.entries(commodity);
        assert_eq!(entries.len(), lines.len(), "{commodity}: posting count");
        let mut before = Dec::zero();
        for (entry, line) in entries.iter().zip(&lines) {
            let (txn, posting) = entry.at.resolve(&journal);
            let what = format!("{commodity} {} {}", line.tindex, line.account);
            assert_eq!(txn.index.0, line.tindex, "{what}: transaction");
            assert_eq!(posting.account.0, line.account, "{what}: account");
            assert_eq!(entry.at.date(&journal), line.date, "{what}: date");
            assert_eq!(
                entry.after.sub(before).expect("no overflow"),
                line.amount,
                "{what}: change"
            );
            assert_eq!(entry.after, line.total, "{what}: running total");
            before = entry.after;
        }
    }
}

/// A row's cell is hledger's last register line for that transaction, its
/// change the sum of the transaction's lines, and a posting-dated last line
/// names its date.
#[test]
fn every_touching_row_matches_its_last_register_line() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    for (file, commodity) in [("reg-usd.csv", "$"), ("reg-eur.csv", "EUR")] {
        let mut by_txn: BTreeMap<u32, (Dec, &RegLine)> = BTreeMap::new();
        let lines = reg(file, commodity);
        for line in &lines {
            let slot = by_txn.entry(line.tindex).or_insert((Dec::zero(), line));
            slot.0 = slot.0.add(line.amount).expect("no overflow");
            slot.1 = line;
        }
        for (tindex, (change, last)) in by_txn {
            let (got_change, balance, touches, posted_on) =
                cell(&journal, &running, commodity, tindex);
            let txn_date = &journal.transactions[at(&journal, tindex) as usize].date;
            assert!(touches, "{commodity} {tindex}");
            assert_eq!(got_change, Some(change), "{commodity} {tindex}: change");
            assert_eq!(balance, last.total, "{commodity} {tindex}: balance");
            assert_eq!(
                posted_on,
                (last.date != *txn_date).then(|| last.date.clone()),
                "{commodity} {tindex}: posted on"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------

#[test]
fn commodities_are_ranked_by_postings_and_zeroed_ones_remain() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    let ranked: Vec<(String, usize)> = running
        .commodities()
        .map(|(commodity, postings)| (commodity.0.clone(), postings))
        .collect();
    // EUR nets to exactly zero by the end and is still listed.
    assert_eq!(ranked, [("$".to_string(), 19), ("EUR".to_string(), 3)]);
    assert_eq!(
        running.resolve_commodity(None).map(|c| c.0.as_str()),
        Some("$")
    );
    assert_eq!(
        running.resolve_commodity(Some("EUR")).map(|c| c.0.as_str()),
        Some("EUR")
    );
    // A commodity the subtree never posts falls back to the default.
    assert_eq!(
        running.resolve_commodity(Some("GBP")).map(|c| c.0.as_str()),
        Some("$")
    );
    let eur = running.entries("EUR");
    assert_eq!(eur.last().map(|entry| entry.after), Some(Dec::zero()));
}

#[test]
fn an_unmatched_account_is_empty_and_a_name_prefix_is_not_a_subtree() {
    let journal = fixture();
    assert!(build(&journal, "assets:nowhere").is_empty());
    // `assets:ban` is a prefix of the NAME, not an ancestor.
    assert!(build(&journal, "assets:ban").is_empty());
    assert!(!build(&journal, "assets").is_empty());
}

#[test]
fn a_leaf_sums_only_itself() {
    let journal = fixture();
    let savings = build(&journal, "assets:bank:savings");
    // opening: 5000; transfer +200 (01-20); split leg -20 (01-22); virtual +2.
    assert_eq!(
        cell(&journal, &savings, "$", 1),
        (Some(dec("5000.00")), dec("5000.00"), true, None)
    );
    assert_eq!(
        cell(&journal, &savings, "$", 9),
        (Some(dec("2.00")), dec("5002.00"), true, None)
    );
    assert_eq!(
        cell(&journal, &savings, "$", 7),
        (
            Some(dec("-20.00")),
            dec("5182.00"),
            true,
            Some("2026-01-22".to_string())
        )
    );
    assert_eq!(savings.commodities().count(), 1);
}

#[test]
fn a_row_in_another_commodity_or_outside_the_subtree_carries_the_balance() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    // The euro deposit (01-10) touches the subtree, but not in $: the balance
    // after every $ posting keyed at or before (01-10, its position) — which
    // includes the included 01-05 transaction read after it.
    assert_eq!(
        cell(&journal, &running, "$", 5),
        (None, dec("5984.25"), false, None)
    );
    // Untouched by the subtree at all.
    assert_eq!(
        cell(&journal, &running, "$", 13),
        (None, dec("5402.25"), false, None)
    );
    // And in EUR, a $-only row before any EUR posting carries zero.
    assert_eq!(
        cell(&journal, &running, "EUR", 1),
        (None, Dec::zero(), false, None)
    );
    // A commodity the subtree never posts has no cell at all.
    assert!(running.cell(&journal, "GBP", 0).is_none());
}

/// Within a day the stream is read order, so the newest-first table (date
/// desc, tindex desc) reads correctly from top to bottom — the included
/// 01-05 transaction (tindex 16) is the last of its day.
#[test]
fn same_day_ties_are_read_order_including_an_include() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    let balance = |tindex| cell(&journal, &running, "$", tindex).1;
    assert_eq!(balance(2), dec("5995.50"));
    assert_eq!(balance(3), dec("5983.25"));
    assert_eq!(balance(16), dec("5984.25"));
    // 02-03: main's EUR transaction first, then the included one.
    assert_eq!(balance(14), dec("5402.25"));
    assert_eq!(balance(15), dec("5502.25"));
}

#[test]
fn posting_dated_legs_post_on_their_own_date() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    // The cheque is dated 01-10; its bank leg clears 01-25.
    assert_eq!(
        cell(&journal, &running, "$", 6),
        (
            Some(dec("-500.00")),
            dec("5402.25"),
            true,
            Some("2026-01-25".to_string())
        )
    );
    // Two legs with two different dates: the change is both, the balance is
    // after the later one.
    assert_eq!(
        cell(&journal, &running, "$", 7),
        (
            Some(dec("-30.00")),
            dec("5902.25"),
            true,
            Some("2026-01-22".to_string())
        )
    );
}

#[test]
fn virtual_postings_count_and_costs_are_ignored() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    // `(checking) $1` + `[savings] $2`.
    assert_eq!(
        cell(&journal, &running, "$", 9),
        (Some(dec("3.00")), dec("5922.25"), true, None)
    );
    // `50,00 EUR @ $1.10` is 50 EUR in the EUR column and nothing in $; the
    // $ leg is the explicit $-55.
    assert_eq!(
        cell(&journal, &running, "EUR", 8),
        (Some(dec("50.00")), dec("150.00"), true, None)
    );
    assert_eq!(cell(&journal, &running, "$", 8).0, Some(dec("-55.00")));
}

#[test]
fn generated_postings_count_when_the_journal_has_them() {
    let text = "\
= expenses:food
    (budget:food)   *-1

2026-01-01 groceries
    expenses:food   $10.00
    assets:cash

2026-01-02 more groceries
    expenses:food   $5.00
    assets:cash
";
    let raw = parse_journal(text, "/tmp/running-auto.journal").expect("parses");
    assert!(build(&raw, "budget").is_empty());
    let applied = auto::apply_journal(&raw);
    let running = build(&applied, "budget");
    assert_eq!(
        cell(&applied, &running, "$", 2),
        (Some(dec("-5.00")), dec("-15.00"), true, None)
    );
}

// ---------------------------------------------------------------------------
// tindex order is position order
// ---------------------------------------------------------------------------

/// The table's tie-break is `tindex`; the stream's is position. They agree
/// because tindexes are assigned in read order, `include`d files at their
/// directive — nested includes too.
#[test]
fn tindex_order_is_position_order_across_nested_includes() {
    let dir =
        std::env::temp_dir().join(format!("ledgeline-running-include-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("deeper")).expect("temp dir");
    std::fs::write(
        dir.join("deeper/b.journal"),
        "2026-01-01 b1\n    a   $1\n    c\n\n2025-12-31 b2\n    a   $1\n    c\n",
    )
    .expect("write b");
    std::fs::write(
        dir.join("a.journal"),
        "2026-01-01 a1\n    a   $1\n    c\n\ninclude deeper/b.journal\n\n2026-01-01 a2\n    a   $1\n    c\n",
    )
    .expect("write a");
    let main = dir.join("main.journal");
    let text = "2026-01-02 m1\n    a   $1\n    c\n\ninclude a.journal\n\n2026-01-01 m2\n    a   $1\n    c\n";
    std::fs::write(&main, text).expect("write main");
    let journal = parse_journal(text, &main.to_string_lossy()).expect("parses");
    let order: Vec<(&str, u32)> = journal
        .transactions
        .iter()
        .map(|txn| (txn.description.as_str(), txn.index.0))
        .collect();
    assert_eq!(
        order,
        [
            ("m1", 1),
            ("a1", 2),
            ("b1", 3),
            ("b2", 4),
            ("a2", 5),
            ("m2", 6)
        ]
    );
    for fixture in [&journal, &fixture()] {
        for (position, txn) in fixture.transactions.iter().enumerate() {
            assert_eq!(txn.index.0 as usize, position + 1, "{}", txn.description);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Assertions
// ---------------------------------------------------------------------------

fn marks(journal: &Journal, running: &RunningBalance) -> BTreeMap<Commodity, Vec<AssertionMark>> {
    let failures = check_balance_assertions(journal).expect("no overflow");
    running.assertion_marks(journal, &failures)
}

/// `(tindex, account, op, asserted, calculated, ok, zero_check)`.
type Summary = (u32, String, &'static str, Dec, Option<Dec>, bool, bool);

fn summarize(journal: &Journal, marks: &[AssertionMark]) -> Vec<Summary> {
    marks
        .iter()
        .map(|mark| {
            (
                journal.transactions[mark.at.position as usize].index.0,
                mark.account.clone(),
                mark.operator,
                mark.asserted,
                mark.calculated,
                mark.ok,
                mark.zero_check,
            )
        })
        .collect()
}

#[test]
fn assertion_marks_follow_the_checker_in_stream_order() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    let marks = marks(&journal, &running);
    let usd = summarize(&journal, &marks[&Commodity("$".into())]);
    let s = String::from;
    assert_eq!(
        usd,
        [
            (
                3,
                s("assets:bank:checking"),
                "=",
                dec("983.25"),
                None,
                true,
                false
            ),
            (
                10,
                s("assets:bank:checking"),
                "=",
                dec("220.25"),
                None,
                true,
                false
            ),
            (
                10,
                s("assets:bank:savings"),
                "=*",
                dec("5182.00"),
                None,
                true,
                false
            ),
            (
                10,
                s("assets:bank"),
                "=*",
                dec("5402.25"),
                None,
                true,
                false
            ),
            (
                11,
                s("assets:bank:checking"),
                "=",
                dec("300.00"),
                Some(dec("220.25")),
                false,
                false
            ),
            // `==` holds in $ ...
            (
                12,
                s("assets:bank:checking"),
                "==",
                dec("220.25"),
                None,
                true,
                false
            ),
        ]
    );
    let eur = summarize(&journal, &marks[&Commodity("EUR".into())]);
    assert_eq!(
        eur,
        [
            // ... but asserts EUR is zero, and 150 EUR is left.
            (
                12,
                s("assets:bank:checking"),
                "==",
                Dec::zero(),
                Some(dec("150.00")),
                false,
                true
            ),
            (
                14,
                s("assets:bank:checking"),
                "=",
                Dec::zero(),
                None,
                true,
                false
            ),
        ]
    );
}

#[test]
fn assertions_outside_the_subtree_are_not_marked() {
    let journal = fixture();
    let running = build(&journal, "assets:bank:savings");
    let marks = marks(&journal, &running);
    assert_eq!(marks.len(), 1);
    let usd = summarize(&journal, &marks[&Commodity("$".into())]);
    assert_eq!(usd.len(), 1);
    assert_eq!(usd[0].1, "assets:bank:savings");
}

/// A passing `=*` on the pinned root asserts exactly what the column shows on
/// that row.
#[test]
fn a_passing_inclusive_assertion_on_the_root_equals_the_column() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    for mark in &marks(&journal, &running)[&Commodity("$".into())] {
        if mark.ok && mark.account == ROOT && mark.operator == "=*" {
            let cell = running
                .cell(&journal, "$", mark.at.position)
                .expect("a cell");
            assert_eq!(cell.balance, mark.asserted);
        }
    }
}

// ---------------------------------------------------------------------------
// Balance over time (amendment A1)
// ---------------------------------------------------------------------------

fn window(from: &str, to: &str, interval: Interval) -> BalanceWindow {
    BalanceWindow {
        from: from.to_string(),
        to: to.to_string(),
        interval,
    }
}

/// `(key, end, balance, last tindex)` per point.
fn points(
    journal: &Journal,
    running: &RunningBalance,
    commodity: &str,
    window: &BalanceWindow,
) -> Vec<(String, String, Dec, Option<u32>)> {
    running
        .series(journal, commodity, window, &[])
        .expect("a series")
        .into_iter()
        .map(|point| {
            (
                point.key,
                point.end,
                point.balance,
                point
                    .last
                    .map(|position| journal.transactions[position as usize].index.0),
            )
        })
        .collect()
}

/// Every day's point is hledger's running total after its last register line
/// dated on or before that day — same-day ties and posting-dated legs
/// included, since the register is in posting-date order.
#[test]
fn every_daily_point_is_hledgers_total_at_that_date() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    for (file, commodity) in [("reg-usd.csv", "$"), ("reg-eur.csv", "EUR")] {
        let lines = reg(file, commodity);
        let series = points(
            &journal,
            &running,
            commodity,
            &window("2025-12-30", "2026-02-05", Interval::Daily),
        );
        assert_eq!(series.len(), 38);
        for (key, end, balance, _) in &series {
            assert_eq!(key, end, "a daily period ends on itself");
            let expected = lines
                .iter()
                .rev()
                .find(|line| line.date <= *end)
                .map_or_else(Dec::zero, |line| line.total);
            assert_eq!(*balance, expected, "{commodity} {end}");
        }
    }
}

#[test]
fn period_ends_land_on_the_right_entry() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    let series = points(
        &journal,
        &running,
        "$",
        &window("2026-01-01", "2026-02-10", Interval::Weekly),
    );
    let s = String::from;
    assert_eq!(
        series,
        [
            // ISO weeks; the first is clamped to start at `from` and ends Sunday.
            // 01-05 (a Monday) is in the next week: just the opening.
            (s("2026-W01"), s("2026-01-04"), dec("6000.00"), Some(1)),
            // 01-05's three same-day rows, the included one read last; the
            // cheque dated 01-10 has its bank leg on 01-25, so not here.
            (s("2026-W02"), s("2026-01-11"), dec("5984.25"), Some(16)),
            // The 01-12 split's 01-14 leg, then the 01-15 pair.
            (s("2026-W03"), s("2026-01-18"), dec("5922.25"), Some(9)),
            // The split's savings leg (01-22) and the cheque's (01-25).
            (s("2026-W04"), s("2026-01-25"), dec("5402.25"), Some(6)),
            (s("2026-W05"), s("2026-02-01"), dec("5402.25"), Some(12)),
            (s("2026-W06"), s("2026-02-08"), dec("5502.25"), Some(15)),
            // The last week is clamped to end at `to`, and nothing posted.
            (s("2026-W07"), s("2026-02-10"), dec("5502.25"), None),
        ]
    );
}

#[test]
fn an_empty_window_is_flat_at_the_carried_balance_and_before_the_first_is_zero() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    let quiet = points(
        &journal,
        &running,
        "$",
        &window("2026-03-01", "2026-05-31", Interval::Monthly),
    );
    assert_eq!(quiet.len(), 3);
    assert!(
        quiet
            .iter()
            .all(|(_, _, balance, last)| *balance == dec("5502.25") && last.is_none())
    );
    let before = points(
        &journal,
        &running,
        "$",
        &window("2025-10-01", "2025-12-31", Interval::Monthly),
    );
    assert_eq!(before.len(), 3);
    assert!(before.iter().all(|(_, _, balance, _)| balance.is_zero()));
    // A commodity the subtree never posts is a zero line, not an error.
    let gbp = points(
        &journal,
        &running,
        "GBP",
        &window("2026-01-01", "2026-01-31", Interval::Monthly),
    );
    assert_eq!(gbp.len(), 1);
    assert!(gbp[0].2.is_zero());
}

#[test]
fn an_open_window_runs_from_the_first_posting_to_today() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    // The included 01-05 transaction is read last but is not the first date;
    // the opening is.
    let first = running.first_date(&journal).expect("not empty");
    assert_eq!(first, "2026-01-01");
    let open = BalanceWindow::resolve(first, None, None, "2026-11-09", None);
    assert_eq!(open, window("2026-01-01", "2026-11-09", Interval::Monthly));
    // One side open: it follows the closed one.
    assert_eq!(
        BalanceWindow::resolve(first, Some("2026-03-01"), None, "2026-11-09", None).to,
        "2026-11-09"
    );
    assert_eq!(
        BalanceWindow::resolve(first, None, Some("2026-01-20"), "2026-11-09", None).from,
        "2026-01-01"
    );
    // A future-only range runs to its own start rather than inverting, and a
    // range that ends before the first posting starts at its end.
    let future = BalanceWindow::resolve(first, Some("2027-01-01"), None, "2026-11-09", None);
    assert_eq!(
        (future.from.as_str(), future.to.as_str()),
        ("2027-01-01", "2027-01-01")
    );
    let early = BalanceWindow::resolve(first, None, Some("2025-06-30"), "2026-11-09", None);
    assert_eq!(
        (early.from.as_str(), early.to.as_str()),
        ("2025-06-30", "2025-06-30")
    );
    // The first point is the period containing the first posting; the last
    // ends on `today`.
    let series = running.series(&journal, "$", &open, &[]).expect("a series");
    assert_eq!(series.first().map(|p| p.key.as_str()), Some("2026-01"));
    assert_eq!(series.last().map(|p| p.end.as_str()), Some("2026-11-09"));
}

/// The interval is the holdings chart's: months from six of them, weeks from
/// six weeks, days below that, coarsening past ten years. An explicit one wins.
#[test]
fn the_interval_switches_at_the_thresholds() {
    let pick = |from: &str, to: &str| {
        BalanceWindow::resolve(from, Some(from), Some(to), to, None).interval
    };
    assert_eq!(pick("2026-01-01", "2026-01-31"), Interval::Daily);
    // Five ISO weeks (W01..W05) is still daily; six is weekly.
    assert_eq!(pick("2026-01-01", "2026-02-01"), Interval::Daily);
    assert_eq!(pick("2026-01-01", "2026-02-02"), Interval::Weekly);
    // Five months (Jan..May) is weekly; six is monthly.
    assert_eq!(pick("2026-01-01", "2026-05-31"), Interval::Weekly);
    assert_eq!(pick("2026-01-01", "2026-06-01"), Interval::Monthly);
    assert_eq!(pick("2016-01-01", "2026-01-01"), Interval::Quarterly);
    assert_eq!(
        BalanceWindow::resolve(
            "2026-01-01",
            None,
            None,
            "2026-01-31",
            Some(Interval::Monthly)
        )
        .interval,
        Interval::Monthly
    );
}

#[test]
fn failing_assertions_sit_in_their_period_with_the_gap_from_the_line() {
    let journal = fixture();
    let running = build(&journal, ROOT);
    let marks = marks(&journal, &running);
    let january = window("2026-01-01", "2026-02-28", Interval::Monthly);
    let usd = running
        .series(&journal, "$", &january, &marks[&Commodity("$".into())])
        .expect("a series");
    // Only the failure is placed; the five passing $ assertions are not.
    let jan: Vec<(u32, Dec, Dec)> = usd[0]
        .failures
        .iter()
        .map(|failure| {
            (
                journal.transactions[failure.mark.at.position as usize]
                    .index
                    .0,
                failure.mark.asserted,
                failure.expected,
            )
        })
        .collect();
    // checking `= $300` found $220.25: the line, $5,402.25 at Jan 31, would
    // be $79.75 higher.
    assert_eq!(jan, [(11, dec("300.00"), dec("5482.00"))]);
    assert!(usd[1].failures.is_empty());
    // The `==` zero-check failure in EUR: 150 EUR found where 0 was asserted,
    // so the line (150 EUR) should have been at zero.
    let eur = running
        .series(&journal, "EUR", &january, &marks[&Commodity("EUR".into())])
        .expect("a series");
    assert_eq!(eur[0].balance, dec("150.00"));
    assert_eq!(eur[0].failures.len(), 1);
    assert_eq!(eur[0].failures[0].expected, Dec::zero());
    // A window that does not contain the failure's date does not show it.
    let february = window("2026-02-01", "2026-02-28", Interval::Monthly);
    assert!(
        running
            .series(&journal, "$", &february, &marks[&Commodity("$".into())])
            .expect("a series")
            .iter()
            .all(|point| point.failures.is_empty())
    );
}

// ---------------------------------------------------------------------------
// Property: the column agrees with the assertion checker
// ---------------------------------------------------------------------------

/// One generated posting: account (0 = `a`, 1 = `a:x`, 2 = `a:x:y`, 3 = `b`
/// outside), amount in cents, commodity (0 = `$`, 1 = `EUR`), optional
/// posting-date offset in days, virtual.
type GenPosting = (u8, i32, u8, Option<u8>, bool);

const ACCOUNTS: [&str; 4] = ["a", "a:x", "a:x:y", "b"];
const COMMODITIES: [&str; 2] = ["$", "EUR"];

fn date(day: u8) -> String {
    format!("2026-01-{:02}", day + 1)
}

/// A journal text from generated transactions `(day, postings)`; each
/// transaction balances against `equity`. `extra` appends a posting to a
/// transaction (by position).
fn render(txns: &[(u8, Vec<GenPosting>)], extra: &dyn Fn(usize) -> Option<String>) -> String {
    let mut out = String::new();
    for (position, (day, postings)) in txns.iter().enumerate() {
        out.push_str(&format!("{} t{position}\n", date(*day)));
        for (account, cents, commodity, shift, virtual_) in postings {
            let account = ACCOUNTS[*account as usize];
            let account = if *virtual_ {
                format!("({account})")
            } else {
                account.to_string()
            };
            let amount = format!(
                "{}{:.2}",
                COMMODITIES[*commodity as usize],
                f64::from(*cents) / 100.0
            );
            let amount = if *commodity == 1 {
                format!("{:.2} EUR", f64::from(*cents) / 100.0)
            } else {
                amount
            };
            out.push_str(&format!("    {account}  {amount}"));
            if let Some(shift) = shift {
                out.push_str(&format!("  ; date:{}", date(day + shift)));
            }
            out.push('\n');
            if !*virtual_ {
                let balancing = if *commodity == 1 {
                    format!("{:.2} EUR", -f64::from(*cents) / 100.0)
                } else {
                    format!("${:.2}", -f64::from(*cents) / 100.0)
                };
                out.push_str(&format!("    equity  {balancing}\n"));
            }
        }
        if let Some(line) = extra(position) {
            out.push_str(&line);
        }
        out.push('\n');
    }
    out
}

fn gen_posting() -> impl Strategy<Value = GenPosting> {
    (
        0u8..4,
        -50_000i32..50_000,
        0u8..2,
        prop::option::weighted(0.2, 0u8..5),
        prop::bool::weighted(0.15),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Give every transaction an `a  $0 =* <the column's balance>` posting
    /// (dated on the row's `posted_on` when it has one): the checker must
    /// accept every one. That is the column and the checker agreeing on
    /// order, scope and arithmetic, row by row, touching or not.
    #[test]
    fn the_column_is_what_an_inclusive_assertion_on_the_root_checks(
        txns in prop::collection::vec(
            (0u8..20, prop::collection::vec(gen_posting(), 1..4)),
            1..25,
        ),
        commodity in 0u8..2,
    ) {
        let symbol = COMMODITIES[commodity as usize];
        let plain = parse_journal(&render(&txns, &|_| None), "/tmp/running-prop.journal")
            .expect("generated journal parses");
        let running = build(&plain, "a");
        prop_assume!(!running.is_empty());
        let cells: Vec<Option<(Dec, Option<String>)>> = (0..txns.len())
            .map(|position| {
                running
                    .cell(&plain, symbol, u32::try_from(position).expect("small"))
                    .map(|cell| (cell.balance, cell.posted_on.map(str::to_string)))
            })
            .collect();
        prop_assume!(cells.iter().all(Option::is_some));
        let asserted = render(&txns, &|position| {
            let (balance, posted_on) = cells[position].clone().expect("checked");
            let zero = if commodity == 1 { "0 EUR".to_string() } else { "$0".to_string() };
            let quantity = ledgeline_core::view::wire::plain_decimal(balance);
            let value = if commodity == 1 { format!("{quantity} EUR") } else { format!("${quantity}") };
            let dated = posted_on.map_or_else(String::new, |date| format!("  ; date:{date}"));
            Some(format!("    a  {zero} =* {value}{dated}\n"))
        });
        let checked = parse_journal(&asserted, "/tmp/running-prop.journal")
            .expect("asserted journal parses");
        let failures = check_balance_assertions(&checked).expect("no overflow");
        prop_assert!(failures.is_empty(), "{}\n{}", asserted, failures.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"));
    }

    /// The series agrees with the column: at every day's end the point is the
    /// Balance cell of the newest row (date desc, then read order) dated on or
    /// before it — or zero before the first row. Posting-dated legs are left
    /// out here, because there the column is deliberately not a function of
    /// the row's date (its leg counts on its own day; see the module docs) —
    /// the hledger parity test above covers them for the series.
    #[test]
    fn the_series_is_the_column_of_the_newest_row_at_each_period_end(
        txns in prop::collection::vec(
            (0u8..20, prop::collection::vec(gen_posting(), 1..4)),
            1..25,
        ),
        commodity in 0u8..2,
    ) {
        let txns: Vec<(u8, Vec<GenPosting>)> = txns
            .into_iter()
            .map(|(day, postings)| {
                (day, postings.into_iter().map(|(a, c, k, _, v)| (a, c, k, None, v)).collect())
            })
            .collect();
        let symbol = COMMODITIES[commodity as usize];
        let journal = parse_journal(&render(&txns, &|_| None), "/tmp/running-series.journal")
            .expect("generated journal parses");
        let running = build(&journal, "a");
        prop_assume!(!running.is_empty());
        let series = running
            .series(&journal, symbol, &window(&date(0), &date(21), Interval::Daily), &[])
            .expect("a series");
        prop_assert_eq!(series.len(), 22);
        for point in &series {
            let newest = journal
                .transactions
                .iter()
                .enumerate()
                .filter(|(_, txn)| txn.date <= point.end)
                .max_by(|(a_at, a), (b_at, b)| a.date.cmp(&b.date).then(a_at.cmp(b_at)))
                .map(|(position, _)| u32::try_from(position).expect("small"));
            let column = newest.map_or_else(Dec::zero, |position| {
                running
                    .cell(&journal, symbol, position)
                    .map_or_else(Dec::zero, |cell| cell.balance)
            });
            prop_assert_eq!(point.balance, column, "{}", point.end);
        }
    }
}
