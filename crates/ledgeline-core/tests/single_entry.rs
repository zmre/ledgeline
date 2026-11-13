//! Single-entry app presets (plan 39 Phase 5) over `fixtures/import/single-entry/`.

mod common;

use common::fixtures_dir;
use ledgeline_core::Dec;
use ledgeline_core::grouped_csv::{self, GroupedCsvError};
use ledgeline_core::qb_journal::{ImportSource, ImportStatus, QbJournal, QbTransaction};
use ledgeline_core::single_entry::{self, Preset};
use std::collections::BTreeMap;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join("import/single-entry").join(name)).expect("fixture readable")
}

fn dec(text: &str) -> Dec {
    Dec::parse(text, '.').expect("literal")
}

fn sums(journal: &QbJournal) -> BTreeMap<String, Dec> {
    let mut totals: BTreeMap<String, Dec> = BTreeMap::new();
    for posting in journal.transactions.iter().flat_map(|t| &t.postings) {
        let entry = totals
            .entry(posting.account.clone())
            .or_insert_with(Dec::zero);
        *entry = entry.add(posting.amount).expect("no overflow");
    }
    totals
}

fn find<'a>(journal: &'a QbJournal, name: &str) -> &'a QbTransaction {
    journal
        .transactions
        .iter()
        .find(|t| t.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no transaction named {name}"))
}

// ---------------------------------------------------------------------------
// YNAB
// ---------------------------------------------------------------------------

#[test]
fn ynab_register_groups_splits_and_dedupes_transfers() {
    let parsed =
        single_entry::parse_preset(Preset::Ynab, &fixture("ynab-register.csv")).expect("parses");
    let journal = &parsed.journal;
    assert_eq!(journal.source, ImportSource::Ynab);
    assert_eq!(journal.transactions.len(), 9);
    assert_eq!(parsed.report.transfers_paired, 2);
    assert_eq!(parsed.report.transfers_unpaired, 1);

    let totals = sums(journal);
    assert_eq!(totals["Checking"], dec("2945.80"));
    assert_eq!(totals["Savings"], dec("5060"));
    assert_eq!(totals["Visa"], dec("-173.50"));
    assert_eq!(
        totals["Brokerage"],
        dec("1000"),
        "unpaired transfer posts to the named account"
    );
    assert_eq!(totals["Starting Balance"], dec("450"));
    assert_eq!(totals["Inflow: Ready to Assign"], dec("-9500"));

    // The split: one Checking posting for the total, one per line, memos clean.
    let split = find(journal, "Costco");
    assert_eq!(split.transaction_type, "Split");
    assert_eq!(split.date, "2026-01-10");
    let accounts: Vec<(&str, Dec, Option<&str>)> = split
        .postings
        .iter()
        .map(|p| (p.account.as_str(), p.amount, p.memo.as_deref()))
        .collect();
    assert_eq!(
        accounts,
        [
            ("Checking", dec("-200"), None),
            ("Everyday Expenses: Groceries", dec("100"), Some("food")),
            ("Monthly Bills: Household", dec("40"), Some("paper towels")),
            ("Savings", dec("60"), Some("cash back to savings")),
        ]
    );
    assert_eq!(split.status, ImportStatus::Cleared);
    assert_eq!(find(journal, "Pizza Place").status, ImportStatus::Unmarked);
    let card = find(journal, "Transfer : Visa");
    assert_eq!(card.transaction_type, "Transfer");
}

#[test]
fn ynab_ids_are_stable_and_prefixed() {
    let a = single_entry::parse(&fixture("ynab-register.csv")).expect("parses");
    let b = single_entry::parse(&fixture("ynab-register.csv")).expect("parses");
    assert_eq!(a, b);
    assert!(a.transactions.iter().all(|t| t.id.starts_with("ynab-")));
    let ids: std::collections::HashSet<_> = a.transactions.iter().map(|t| &t.id).collect();
    assert_eq!(ids.len(), a.transactions.len());
}

#[test]
fn ynab_memo_edits_do_not_change_ids() {
    let text = String::from_utf8(fixture("ynab-register.csv")).expect("utf8");
    let edited = text.replace("Monthly investment", "Monthly investment (edited)");
    let ids = |bytes: &[u8]| -> Vec<String> {
        single_entry::parse(bytes)
            .expect("parses")
            .transactions
            .into_iter()
            .map(|t| t.id)
            .collect()
    };
    assert_eq!(ids(text.as_bytes()), ids(edited.as_bytes()));
}

#[test]
fn ynab_refuses_a_malformed_amount_by_row() {
    let text = "Account,Flag,Date,Payee,Category Group/Category,Category Group,Category,Memo,Outflow,Inflow,Cleared\n\
                Checking,,01/02/2026,Shop,Food: Groceries,Food,Groceries,,$5.00,$0.00,Cleared\n\
                Checking,,01/03/2026,Shop,Food: Groceries,Food,Groceries,,five,$0.00,Cleared\n";
    assert_eq!(
        single_entry::parse(text.as_bytes()),
        Err(GroupedCsvError::MalformedAmount {
            row: 3,
            cell: "five".to_string()
        })
    );
}

// ---------------------------------------------------------------------------
// Monarch
// ---------------------------------------------------------------------------

#[test]
fn monarch_pairs_transfers_across_accounts_within_the_window() {
    let parsed =
        single_entry::parse_preset(Preset::Monarch, &fixture("monarch.csv")).expect("parses");
    let journal = &parsed.journal;
    assert_eq!(journal.source, ImportSource::Monarch);
    assert_eq!(journal.transactions.len(), 10);
    assert_eq!(parsed.report.transfers_paired, 2);
    assert_eq!(
        parsed.report.transfers_unpaired, 3,
        "Venmo plus a pair 13 days apart"
    );

    let totals = sums(journal);
    assert_eq!(totals["Chase Checking (...1234)"], dec("1342.73"));
    assert_eq!(totals["Ally Savings"], dec("750"));
    assert_eq!(totals["Chase Sapphire (...4321)"], dec("295.04"));
    assert_eq!(totals["Transfer"], dec("600"));
    assert_eq!(totals["Groceries"], dec("84.12"), "negative = spend");
    assert_eq!(totals["Paycheck"], dec("-3100"));
    assert_eq!(totals["Uncategorized"], dec("3"));

    // Dated on the outflow side, named after it.
    let pair = journal
        .transactions
        .iter()
        .find(|t| t.name.as_deref() == Some("Transfer to Savings") && t.date == "2026-01-07")
        .expect("the paired transfer");
    assert_eq!(pair.transaction_type, "Transfer");
    assert_eq!(pair.date, "2026-01-07");
    assert_eq!(pair.postings.len(), 2);
    assert_eq!(pair.postings[0].account, "Chase Checking (...1234)");
    assert_eq!(pair.postings[1].account, "Ally Savings");

    // No merchant: the statement text is the payee.
    assert_eq!(
        find(journal, "ATM FEE").postings[1].account,
        "Uncategorized"
    );
    assert_eq!(
        find(journal, "Bistro 9").postings[1].memo.as_deref(),
        Some("date night")
    );
    assert!(
        journal
            .transactions
            .iter()
            .all(|t| t.id.starts_with("monarch-"))
    );
}

// ---------------------------------------------------------------------------
// Copilot
// ---------------------------------------------------------------------------

#[test]
fn copilot_skips_excluded_rows_and_pairs_internal_transfers() {
    let parsed =
        single_entry::parse_preset(Preset::Copilot, &fixture("copilot.csv")).expect("parses");
    let journal = &parsed.journal;
    assert_eq!(journal.source, ImportSource::Copilot);
    assert_eq!(parsed.report.skipped_transactions, 1);
    assert_eq!(parsed.report.transfers_paired, 1);
    assert_eq!(journal.transactions.len(), 6);

    let totals = sums(journal);
    assert_eq!(
        totals["Chase Checking 1234"],
        dec("2515.88"),
        "positive = spend"
    );
    assert_eq!(totals["Ally Savings 9876"], dec("500"));
    assert_eq!(totals["Chase Sapphire 4321"], dec("0.01"));
    assert_eq!(find(journal, "Coffee").status, ImportStatus::Pending);
    assert!(
        journal
            .transactions
            .iter()
            .all(|t| t.id.starts_with("copilot-"))
    );
}

#[test]
fn copilot_sign_follows_income_rows() {
    // Income positive here, so positive means money in.
    let text = "date,name,amount,type,category,account,account mask,excluded\n\
                2026-01-06,Employer,3100.00,income,Paycheck,Checking,1234,false\n\
                2026-01-07,Shop,-20.00,regular,Shopping,Checking,1234,false\n";
    let journal = single_entry::parse(text.as_bytes()).expect("parses");
    assert_eq!(sums(&journal)["Checking 1234"], dec("3080"));
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn detects_each_preset_and_nothing_else() {
    for (name, preset) in [
        ("ynab-register.csv", Preset::Ynab),
        ("monarch.csv", Preset::Monarch),
        ("copilot.csv", Preset::Copilot),
    ] {
        assert_eq!(
            single_entry::detect_preset(name, &fixture(name)),
            Some(preset),
            "{name}"
        );
        assert!(!grouped_csv::detect(name, &fixture(name)), "{name}");
    }
    // A bank CSV that happens to have Account and Amount columns.
    let bank = b"Date,Account,Description,Category,Amount\n2026-01-01,Checking,Shop,Food,-5\n";
    assert!(!single_entry::detect("bank.csv", bank));
}
