//! The QIF fixtures in `fixtures/import/qif/`, read end to end through
//! `book_import::detect_and_parse` (plan 39 Phase 2). Each fixture is
//! synthetic; see that directory's README for what each one exercises.

mod common;

use common::fixtures_dir;
use ledgeline_core::book_import::detect_and_parse;
use ledgeline_core::decimal::Dec;
use ledgeline_core::qb_journal::{ImportSource, ImportStatus, QbJournal};
use ledgeline_core::qif_import::{OPENING_BALANCES, UNCATEGORIZED};
use std::collections::{BTreeMap, HashSet};

fn read(name: &str) -> QbJournal {
    let bytes = std::fs::read(fixtures_dir().join("import/qif").join(name))
        .unwrap_or_else(|error| panic!("{name} reads: {error}"));
    detect_and_parse(name, &bytes)
        .unwrap_or_else(|| panic!("{name} is recognised"))
        .unwrap_or_else(|error| panic!("{name} imports: {error}"))
}

fn dec(text: &str) -> Dec {
    Dec::parse(text, '.').expect("test literal parses")
}

/// Each source account's total across the whole import.
fn balances(journal: &QbJournal) -> BTreeMap<String, Dec> {
    let mut totals: BTreeMap<String, Dec> = BTreeMap::new();
    for posting in journal.transactions.iter().flat_map(|t| &t.postings) {
        let total = totals
            .entry(posting.account.clone())
            .or_insert_with(Dec::zero);
        *total = total.add(posting.amount).expect("no overflow");
    }
    totals
}

/// Every transaction balances, every id is unique and `qif-`-prefixed, and
/// the source is QIF.
fn well_formed(journal: &QbJournal) {
    assert_eq!(journal.source, ImportSource::Qif);
    let ids: HashSet<_> = journal.transactions.iter().map(|t| &t.id).collect();
    assert_eq!(ids.len(), journal.transactions.len(), "ids are unique");
    for txn in &journal.transactions {
        assert!(txn.id.starts_with("qif-"), "{}", txn.id);
        let sum = txn
            .postings
            .iter()
            .try_fold(Dec::zero(), |sum, p| sum.add(p.amount))
            .expect("sums");
        assert!(sum.is_zero(), "{} does not balance", txn.id);
    }
}

#[test]
fn every_fixture_imports_balanced_and_reimports_identically() {
    for name in [
        "splits.qif",
        "transfers.qif",
        "multi-account.qif",
        "categories-classes.qif",
        "investment.qif",
        "banktivity.qif",
        "quicken-mac.qmtf",
        "ambiguous-dates.qif",
        "windows-1252.qif",
    ] {
        let first = read(name);
        well_formed(&first);
        assert_eq!(first, read(name), "{name}: a second read is identical");
    }
}

#[test]
fn splits_keep_each_line_and_payroll_nets_correctly() {
    let journal = read("splits.qif");
    assert_eq!(journal.transactions.len(), 4);
    let shop = &journal.transactions[1];
    assert_eq!(shop.num.as_deref(), Some("1041"));
    assert_eq!(shop.postings.len(), 4);
    assert_eq!(shop.postings[1].memo.as_deref(), Some("Produce and pantry"));
    let totals = balances(&journal);
    assert_eq!(
        totals["Checking"],
        dec("4147.63"),
        "named by its opening balance"
    );
    assert_eq!(totals["Salary"], dec("-2400"));
    assert_eq!(totals["Taxes:Federal"], dec("410"));
    assert_eq!(totals[OPENING_BALANCES], dec("-2500"));
}

#[test]
fn one_sided_transfers_name_the_other_account() {
    let journal = read("transfers.qif");
    let totals = balances(&journal);
    assert_eq!(totals["Savings"], dec("500"));
    assert_eq!(totals["Visa"], dec("750"));
    assert_eq!(totals["Checking"], dec("1725"));
}

#[test]
fn multi_account_registers_and_a_split_transfer() {
    let journal = read("multi-account.qif");
    assert_eq!(journal.transactions.len(), 8);
    let totals = balances(&journal);
    assert_eq!(totals["Checking"], dec("3380"));
    assert_eq!(totals["Wallet"], dec("187.50"));
    assert_eq!(totals["Visa"], dec("-181.60"));
    assert_eq!(totals["Car"], dec("18000"));
    assert_eq!(totals["Car Loan"], dec("-14620"));
    assert_eq!(totals["Auto:Loan Interest"], dec("40"));
}

#[test]
fn classes_become_posting_classes() {
    let journal = read("categories-classes.qif");
    let classes: Vec<_> = journal
        .transactions
        .iter()
        .flat_map(|t| &t.postings)
        .filter_map(|p| p.class.as_deref())
        .collect();
    assert_eq!(
        classes,
        vec!["Duplex A", "Duplex B", "Duplex A", "Duplex B"]
    );
    assert_eq!(balances(&journal)["Home:Repairs"], dec("310"));
}

#[test]
fn investment_actions_and_the_bank_side_collapse() {
    let journal = read("investment.qif");
    // Seven investment records; both bank records are mirrors.
    assert_eq!(journal.transactions.len(), 7);
    let totals = balances(&journal);
    assert_eq!(totals["Checking"], dec("-4450"));
    // The holding's cost; the buy's commission is an expense of its own.
    assert_eq!(totals["Brokerage:ACME Index Fund"], dec("1463.00"));
    assert_eq!(totals[ledgeline_core::qif_import::COMMISSIONS], dec("4.95"));
    assert_eq!(totals["Investment Income:Dividends"], dec("-31.40"));
    assert_eq!(totals["Investment Income:Interest"], dec("-2.10"));
    // 40 bought, 0.25 reinvested, 10 sold.
    let shares = journal
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .filter_map(|posting| posting.units.as_ref())
        .try_fold(dec("0"), |sum, units| sum.add(units.quantity))
        .expect("fits");
    assert_eq!(shares, dec("30.25"));
}

#[test]
fn banktivity_mirrors_collapse_and_repeated_lists_do_not_matter() {
    let journal = read("banktivity.qif");
    assert_eq!(journal.transactions.len(), 10);
    let totals = balances(&journal);
    assert_eq!(totals["Joint Checking"], dec("6197.34"));
    assert_eq!(totals["High Yield Savings"], dec("13291.07"));
    assert_eq!(totals["Rewards Card"], dec("-88.16"));
    assert_eq!(totals["Utilities:Electric"], dec("142.49"));
    let opening = &journal.transactions[0];
    assert_eq!(opening.status, ImportStatus::Cleared);
}

#[test]
fn qmtf_apostrophe_dates() {
    let journal = read("quicken-mac.qmtf");
    let dates: Vec<_> = journal
        .transactions
        .iter()
        .map(|t| t.date.as_str())
        .collect();
    assert_eq!(dates, vec!["2026-01-02", "2026-01-07", "2026-01-19"]);
    assert!(!journal.date_format.ambiguous);
    assert_eq!(
        journal.transactions[2].postings[0].memo.as_deref(),
        Some("Conference stay")
    );
}

#[test]
fn ambiguous_dates_are_flagged() {
    let journal = read("ambiguous-dates.qif");
    assert!(journal.date_format.ambiguous);
    assert_eq!(journal.transactions[0].date, "2026-01-02");
}

#[test]
fn windows_1252_payee() {
    let journal = read("windows-1252.qif");
    assert_eq!(
        journal.transactions[0].name.as_deref(),
        Some("Flat White Café")
    );
    assert_eq!(journal.transactions[0].postings[1].account, "Coffee");
    assert_ne!(journal.transactions[0].postings[1].account, UNCATEGORIZED);
}
