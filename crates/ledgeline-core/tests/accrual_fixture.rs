//! Plan 45 on `fixtures/accrual` (synthetic; clock 2026-11-17): due set,
//! ties, gate, findings, settlement proposals, renewal prefill, credit-back
//! restatement, guards, and parity with hledger's own output captured in
//! `fixtures/accrual/expected/` by `capture.sh`.
//!
//! `LEDGELINE_HLEDGER_ACCRUAL_CHECK=1` opts in to re-running hledger and
//! comparing it with the committed captures.

mod common;

use ledgeline_core::accrual::book::{RecognitionBook, is_included};
use ledgeline_core::accrual::due::{due, ties};
use ledgeline_core::accrual::index::{Issue, RecognitionIndex};
use ledgeline_core::accrual::ops::Books;
use ledgeline_core::accrual::restate::{RowAction, restatement};
use ledgeline_core::accrual::schedule::{Term, build, periods_of};
use ledgeline_core::accrual::settle::{Fit, SettleSettings, propose};
use ledgeline_core::accrual::{AccrualRoles, is_accrual_ledger, is_posted_entry};
use ledgeline_core::model::{Journal, PeriodicTransaction};
use ledgeline_core::parse::JournalStyles;
use ledgeline_core::view::checks::recognition_findings;
use ledgeline_core::{Dec, parse_journal};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const TODAY: &str = "2026-11-17";
const CLOSED: &str = "2026-09-30";
const OPT_IN: &str = "LEDGELINE_HLEDGER_ACCRUAL_CHECK";

fn dir() -> PathBuf {
    common::fixtures_dir().join("accrual")
}

/// The fixture through the public load path, read once per test binary and
/// shared by every test (each used to re-read and re-parse it).
fn load() -> &'static (Journal, RecognitionBook, RecognitionIndex) {
    static LOADED: OnceLock<(Journal, RecognitionBook, RecognitionIndex)> = OnceLock::new();
    LOADED.get_or_init(|| {
        let loaded = Books::open(&dir().join("books.journal"))
            .expect("opens")
            .load()
            .expect("loads");
        assert_eq!(loaded.config().closed_through.as_deref(), Some(CLOSED));
        let journal = loaded.journal().clone();
        let book = loaded.book().clone();
        let index = RecognitionIndex::build(&journal, &book);
        (journal, book, index)
    })
}

#[test]
fn november_is_due_and_september_is_closed() {
    let (_, book, index) = load();
    let set = due(index, book, TODAY, Some(CLOSED));
    let open: Vec<(&str, &str)> = set
        .open
        .iter()
        .map(|e| (e.recognized.as_str(), e.id.as_str()))
        .collect();
    assert_eq!(
        open,
        vec![
            ("2026-11-01", "ad0401"),
            ("2026-11-01", "ct0801"),
            ("2026-11-01", "lw0517"),
            ("2026-11-01", "nw0301"),
            ("2026-11-01", "tt2001"),
            ("2026-11-03", "cy0203"),
            ("2026-11-10", "as0710"),
            ("2026-11-15", "ad1015"),
            ("2026-11-15", "pw0615"),
            ("2026-11-17", "cx1217"),
        ]
    );
    let closed: Vec<(&str, &str)> = set
        .closed
        .iter()
        .map(|e| (e.recognized.as_str(), e.id.as_str()))
        .collect();
    assert_eq!(closed, vec![("2026-09-10", "as0710")]);
    // Nothing after today, whatever "today" the caller says.
    assert!(
        due(index, book, "2026-11-16", Some(CLOSED))
            .open
            .iter()
            .all(|e| e.recognized.as_str() <= "2026-11-16")
    );
}

#[test]
fn prepaid_ties_and_deferred_revenue_is_off_by_the_unscheduled_lines() {
    let (journal, book, index) = load();
    let set = due(index, book, TODAY, Some(CLOSED));
    let tie = ties(journal, index, book, &set);
    let prepaid: Vec<_> = tie
        .iter()
        .filter(|t| t.account == "assets:prepaid")
        .collect();
    assert!(
        !prepaid.is_empty() && prepaid.iter().all(|t| t.ties),
        "{prepaid:#?}"
    );
    // The account holds Fourth Coffee's unscheduled $-1,800 and Adatum's
    // $1,000 credit (net $-800), misses the unposted closed September
    // occurrence ($-100 against the schedule), and carries the duplicate
    // entry ($+1,000): $+100 off.
    let deferred = tie
        .iter()
        .find(|t| t.account == "liabilities:deferred-revenue")
        .expect("deferred revenue is checked");
    assert!(!deferred.ties);
    let difference = deferred.after.sub(deferred.scheduled).expect("subtracts");
    assert_eq!(difference.normalized(), Dec::new(100, 0), "{deferred:#?}");
}

#[test]
fn the_gate_opens_here_and_not_on_the_household_sample() {
    let (journal, _, _) = load();
    assert!(is_accrual_ledger(journal, None));
    assert!(!is_accrual_ledger(&common::fixture_journal(), None));
}

#[test]
fn the_only_issue_is_the_duplicate_and_its_findings_are_anchored() {
    let (journal, book, index) = load();
    let issues = index.issues();
    assert!(
        matches!(issues, [Issue::Duplicate { id, recognized, positions }]
            if id == "cx1217" && recognized == "2026-03-17" && positions.len() == 2),
        "{issues:#?}"
    );
    let set = due(index, book, TODAY, Some(CLOSED));
    let findings = recognition_findings(index, book, &set.closed, false);
    let rules: Vec<&str> = findings.iter().map(|f| f.rule).collect();
    assert_eq!(
        rules,
        vec![
            "recognition-duplicate",
            "recognition-duplicate",
            "recognition-closed-due"
        ]
    );
    let needs: Vec<&str> = index
        .needs_schedule()
        .iter()
        .map(|at| journal.transactions[at.position].description.as_str())
        .collect();
    assert_eq!(needs, vec!["Fourth Coffee Invoice 2004"]);
}

#[test]
fn settlements_pair_by_number_with_a_fee_and_exactly() {
    let (journal, _, _) = load();
    let roles = AccrualRoles::of(journal);
    // The D&O bill (March) is outside the default 180-day lookback.
    let recent = propose(journal, &roles, &SettleSettings::default(), TODAY);
    assert_eq!(recent.len(), 2);
    let all = SettleSettings {
        lookback_days: 0,
        ..SettleSettings::default()
    };
    let proposals = propose(journal, &roles, &all, TODAY);
    let described: Vec<(&str, &str, bool, bool)> = proposals
        .iter()
        .map(|p| {
            (
                journal.transactions[p.invoice.position]
                    .description
                    .as_str(),
                journal.transactions[p.payment.position]
                    .description
                    .as_str(),
                p.by_number,
                p.clear,
            )
        })
        .collect();
    assert_eq!(
        described,
        vec![
            (
                "Northwind Insurance",
                "Northwind Insurance B-77",
                true,
                true
            ),
            (
                "Tailspin Invoice 2001",
                "Tailspin payment INV 2001",
                true,
                false
            ),
            (
                "Wingtip Invoice 2002",
                "Wingtip Toys payment 2002",
                true,
                true
            ),
        ]
    );
    assert_eq!(
        proposals[1].fit,
        Fit::Short {
            fee: Dec::new(3000, 2)
        }
    );
}

#[test]
fn a_renewal_prefills_from_the_latest_schedule() {
    let (_, book, index) = load();
    let source = index.schedule("cx1217").expect("indexed").sources[0].clone();
    let party = source.party.expect("Customer X");
    assert_eq!(
        index.latest_schedule_of(&party).map(|s| s.id.as_str()),
        Some("cx1217")
    );
    let spec = index.spec_of("cx1217", book).expect("readable");
    assert_eq!(spec.term, Term::Months(12));
    assert_eq!(spec.lines[0].target_account, "revenue:subscriptions");
}

#[test]
fn ending_the_old_adatum_plan_leaves_exactly_the_credit() {
    let (journal, book, index) = load();
    let styles = JournalStyles::of(journal);
    let old_spec = index.spec_of("ad0401", book).expect("readable");
    let old = build(&old_spec, &styles).expect("builds");
    let mut ended = old_spec.clone();
    ended.term = Term::Through("2026-10-31".into());
    ended.lines[0].total.quantity = Dec::new(-140_000, 2);
    let new = build(&ended, &styles).expect("builds");
    assert_eq!(new.periods.len(), 7);
    assert!(
        new.periods
            .iter()
            .all(|p| p.amounts[0] == Dec::new(-20000, 2))
    );
    let r = restatement(
        &old,
        &new,
        journal,
        index.schedule("ad0401").expect("indexed"),
        Some(CLOSED),
        TODAY,
        &styles,
    )
    .expect("restates");
    let actions: Vec<RowAction> = r.rows.iter().map(|row| row.action).collect();
    assert_eq!(&actions[..6], &[RowAction::Kept; 6]);
    assert_eq!(actions[6], RowAction::Rewrite, "renewal-date moves");
    assert!(r.deletes.is_empty() && r.adjustment.is_none());
}

fn rules_of<'b>(book: &'b RecognitionBook, id: &str) -> Vec<&'b PeriodicTransaction> {
    book.rules_of(id)
}

/// `print` output parsed back: date → amount on the source account.
fn captured(id: &str, source: &str) -> Vec<(String, Dec)> {
    let path = dir().join(format!("expected/forecast-{id}.journal"));
    let text = std::fs::read_to_string(&path).expect("captured");
    let journal =
        parse_journal(&format!("commodity $1,000.00\n{text}"), "c.journal").expect("parses");
    journal
        .transactions
        .iter()
        .map(|txn| {
            let sum = txn
                .postings
                .iter()
                .filter(|p| p.account.0 == source)
                .flat_map(|p| &p.amounts)
                .try_fold(Dec::zero(), |s, a| s.add(a.quantity))
                .expect("sums");
            (txn.date.clone(), sum.neg().expect("negates").normalized())
        })
        .collect()
}

#[test]
fn every_schedule_reads_back_as_hledger_generates_it() {
    let (journal, book, index) = load();
    let styles = JournalStyles::of(journal);
    for id in book.ids() {
        let spec = index.spec_of(id, book).expect("readable");
        let schedule = build(&spec, &styles).expect("builds");
        let source = spec.lines[0].source_account.as_str();
        let ours: Vec<(String, Dec)> = periods_of(&rules_of(book, id), &[source])
            .expect("bounded")
            .into_iter()
            .map(|o| (o.date, o.amounts[0].normalized()))
            .collect();
        let built: Vec<(String, Dec)> = schedule
            .periods
            .iter()
            .map(|p| {
                let total = p
                    .amounts
                    .iter()
                    .try_fold(Dec::zero(), |s, a| s.add(*a))
                    .expect("sums");
                (p.date.clone(), total.normalized())
            })
            .collect();
        assert_eq!(ours, built, "{id}: rules vs built");
        assert_eq!(ours, captured(id, source), "{id}: rules vs hledger");
        assert!(!schedule.rules_text().is_empty());
    }
}

fn csv_totals(text: &str) -> BTreeMap<String, Dec> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split("\",\"").map(|f| f.trim_matches('"')).collect();
            let [account, value] = fields.as_slice() else {
                return None;
            };
            (*account != "Total:").then(|| {
                let digits = value.replace(['$', ','], "");
                (
                    (*account).to_string(),
                    Dec::parse(&digits, '.').expect("number").normalized(),
                )
            })
        })
        .collect()
}

#[test]
fn posted_totals_match_hledger() {
    let (journal, _, _) = load();
    let mut ours: BTreeMap<String, Dec> = BTreeMap::new();
    for txn in journal.transactions.iter().filter(|t| is_posted_entry(t)) {
        for posting in &txn.postings {
            for amount in &posting.amounts {
                let sum = ours.entry(posting.account.0.clone()).or_insert(Dec::zero());
                *sum = sum.add(amount.quantity).expect("adds");
            }
        }
    }
    let ours: BTreeMap<String, Dec> = ours.into_iter().map(|(k, v)| (k, v.normalized())).collect();
    let text = std::fs::read_to_string(dir().join("expected/posted.csv")).expect("captured");
    assert_eq!(ours, csv_totals(&text));
}

#[test]
fn an_included_recognition_file_is_flagged_and_never_a_goal() {
    let scratch = std::env::temp_dir().join(format!("ll-accrual-incl-{}", std::process::id()));
    copy_fixture(&scratch);
    let main = scratch.join("books.journal");
    let text = std::fs::read_to_string(&main).expect("reads");
    std::fs::write(&main, format!("{text}\ninclude recognition.journal\n")).expect("writes");
    let journal = parse_journal(
        &std::fs::read_to_string(&main).expect("reads"),
        &main.to_string_lossy(),
    )
    .expect("parses");
    let key = std::fs::canonicalize(scratch.join("recognition.journal")).expect("exists");
    assert!(is_included(&journal, &key));
    assert!(!journal.periodic_transactions.is_empty());
    let book = RecognitionBook::parse(
        &std::fs::read_to_string(&key).expect("reads"),
        &key,
        &journal.commodity_styles,
    )
    .expect("parses");
    let index = RecognitionIndex::build(&journal, &book);
    let findings = recognition_findings(&index, &book, &[], true);
    assert_eq!(findings[0].rule, "recognition-file-included");
    // The budget report sees no goal from it.
    let report = ledgeline_core::reports::budget::budget_report(
        &journal.transactions,
        &journal.periodic_transactions,
        &ledgeline_core::reports::budget::BudgetOpts {
            end: "2026-11-30",
            interval: ledgeline_core::reports::Interval::Monthly,
            count: 3,
            depth: 9,
            budget_desc: None,
        },
    )
    .expect("reports");
    assert!(
        report
            .rows
            .iter()
            .all(|row| row.cells.iter().all(|c| c.goal.is_none())),
        "no goals"
    );
    // Discovery leaves the file out once excluded.
    let discovery = ledgeline_core::projections::Discovery::without_headers(&main);
    assert!(
        discovery
            .files
            .iter()
            .any(|f| f.id == "recognition.journal")
    );
    let excluded = discovery.excluding(&[key]);
    assert!(excluded.files.iter().all(|f| f.id != "recognition.journal"));
    assert!(excluded.resolve("recognition.journal").is_none());
    let _ = std::fs::remove_dir_all(&scratch);
}

fn copy_fixture(to: &Path) {
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to).expect("dir");
    for name in [
        "books.journal",
        "books-2025.journal",
        "recognition.journal",
        "ledgeline.toml",
    ] {
        std::fs::copy(dir().join(name), to.join(name)).expect("copies");
    }
}

#[test]
fn hledger_still_says_what_was_captured() {
    if std::env::var(OPT_IN).is_err() {
        return;
    }
    let run = |args: &[&str]| -> String {
        let out = std::process::Command::new("hledger")
            .current_dir(dir())
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
    assert_eq!(
        run(&[
            "-f",
            "books.journal",
            "bal",
            "tag:recognized",
            "--flat",
            "-O",
            "csv"
        ]),
        std::fs::read_to_string(dir().join("expected/posted.csv")).expect("captured")
    );
    let (_, book, _) = load();
    for id in book.ids() {
        let query = format!("tag:recognition=^{id}$");
        let live = run(&[
            "-f",
            "books.journal",
            "-f",
            "recognition.journal",
            "print",
            "--forecast=2025-01-01..2029-01-01",
            &query,
            "tag:_generated-transaction",
        ]);
        let committed =
            std::fs::read_to_string(dir().join(format!("expected/forecast-{id}.journal")))
                .expect("captured");
        assert_eq!(live, committed, "{id}");
    }
}
