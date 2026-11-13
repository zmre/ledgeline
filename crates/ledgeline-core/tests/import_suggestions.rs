//! The mapping step's suggested journal accounts (plan 39: the account tree is
//! derived from the export, then reviewed by the user before any alias is
//! saved), over real importer output.

mod common;

use common::fixtures_dir;
use ledgeline_core::qb_import::suggest_accounts;
use ledgeline_core::qb_journal::{QbJournal, SourceAccountKind};
use std::collections::BTreeMap;

fn read(path: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(path)).expect("fixture exists")
}

fn accounts(parsed: &QbJournal) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for posting in parsed.transactions.iter().flat_map(|txn| &txn.postings) {
        if !seen.contains(&posting.account) {
            seen.push(posting.account.clone());
        }
    }
    seen
}

fn suggestions(parsed: &QbJournal, existing: &[&str]) -> BTreeMap<String, String> {
    let existing: Vec<String> = existing.iter().map(|name| (*name).to_string()).collect();
    suggest_accounts(parsed, &accounts(parsed), &existing)
}

#[test]
fn qif_registers_take_their_declared_kind_and_every_account_gets_a_suggestion() {
    let parsed = ledgeline_core::qif_import::parse_and_import(
        "multi-account.qif",
        &read("import/qif/multi-account.qif"),
    )
    .expect("imports");
    assert_eq!(
        parsed.account_kinds.get("Visa"),
        Some(&SourceAccountKind::CreditCard)
    );
    assert_eq!(
        parsed.account_kinds.get("Wallet"),
        Some(&SourceAccountKind::Cash)
    );
    assert_eq!(
        parsed.account_kinds.get("Car Loan"),
        Some(&SourceAccountKind::OtherLiability)
    );

    let suggested = suggestions(&parsed, &[]);
    assert_eq!(suggested["Checking"], "assets:bank:Checking");
    assert_eq!(suggested["Visa"], "liabilities:credit cards:Visa");
    assert_eq!(suggested["Wallet"], "assets:cash:Wallet");
    assert_eq!(
        suggested["Equity:Opening Balances"],
        "equity:Opening Balances"
    );
    let unsuggested: Vec<String> = accounts(&parsed)
        .into_iter()
        .filter(|account| !suggested.contains_key(account))
        .collect();
    assert!(unsuggested.is_empty(), "no suggestion for {unsuggested:?}");
}

#[test]
fn an_existing_account_with_the_same_last_segment_is_preferred_when_it_is_the_only_one() {
    let parsed = ledgeline_core::qif_import::parse_and_import(
        "multi-account.qif",
        &read("import/qif/multi-account.qif"),
    )
    .expect("imports");
    let suggested = suggestions(
        &parsed,
        &[
            "assets:bank:chase:checking",
            "assets:wallet",
            "expenses:gifts:wallet",
        ],
    );
    assert_eq!(suggested["Checking"], "assets:bank:chase:checking");
    // Two existing accounts end in `wallet`: neither is guessed.
    assert_eq!(suggested["Wallet"], "assets:cash:Wallet");
}

#[test]
fn single_entry_tracked_accounts_and_categories_split_by_sign() {
    let parsed = ledgeline_core::single_entry::parse(&read("import/single-entry/monarch.csv"))
        .expect("parses");
    let suggested = suggestions(&parsed, &[]);
    assert_eq!(
        suggested["Chase Checking (...1234)"],
        "assets:Chase Checking (...1234)"
    );
    assert_eq!(
        suggested["Restaurants & Bars"],
        "expenses:Restaurants & Bars"
    );
    assert!(
        suggested
            .values()
            .any(|account| account.starts_with("income:")),
        "a paycheck category reads as income: {suggested:?}"
    );
}

#[test]
fn hierarchical_sources_are_suggested_as_written() {
    let parsed =
        ledgeline_core::grouped_csv::parse(&read("import/grouped-csv/gnucash-multiline.csv"))
            .expect("parses");
    let suggested = suggestions(&parsed, &[]);
    for account in accounts(&parsed) {
        assert_eq!(suggested.get(&account), Some(&account));
    }
}
