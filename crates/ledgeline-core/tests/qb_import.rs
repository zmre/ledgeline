//! `qb_import` against the real QuickBooks Journal fixtures (WP-17 Phase B),
//! rather than the constructed shapes `qb_import.rs`'s own inline tests use.
//!
//! `many-postings.xlsx` is the one fixture in the corpus whose transaction has
//! no `Name` at all (group 612 is a manual Journal Entry) — see
//! `fixtures/import/qb-journal/README.md` — which makes it the fixture that
//! proves the description fallback chain against real data rather than a
//! hand-built stand-in.

mod common;

use ledgeline_core::model::Journal;
use ledgeline_core::parse_journal;
use ledgeline_core::qb_import::{
    description_for, plain_aliases, resolve_account, unmapped_accounts,
};
use ledgeline_core::qb_journal;
use std::path::PathBuf;

fn qb_fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = common::fixtures_dir().join("import/qb-journal").join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {name} should read: {error}"))
}

fn journal(text: &str) -> Journal {
    parse_journal(text, "t.journal").expect("the fixture parses")
}

#[test]
fn the_manual_journal_entry_has_no_name_and_falls_back_to_type_and_first_memo() {
    let parsed = qb_journal::parse(&qb_fixture("many-postings.xlsx")).expect("parses");
    let [transaction] = &parsed.transactions[..] else {
        panic!("one group");
    };
    assert_eq!(transaction.name, None, "group 612 carries no payee");
    assert_eq!(
        description_for(transaction),
        "Journal Entry: Opening Balance Entry"
    );
}

#[test]
fn the_deposit_and_expense_groups_use_their_payee_name() {
    let parsed = qb_journal::parse(&qb_fixture("simple.xlsx")).expect("parses");
    let descriptions: Vec<String> = parsed.transactions.iter().map(description_for).collect();
    assert_eq!(
        descriptions,
        vec![
            "Ridgeline Partners, LP".to_string(),
            "Grasshopper Cloud".to_string()
        ]
    );
}

#[test]
fn a_parent_alias_maps_every_real_account_in_simple_xlsx() {
    let parsed = qb_journal::parse(&qb_fixture("simple.xlsx")).expect("parses");
    let journal = journal(
        "alias Riverbank BUSINESS CHECKING (0002) = assets:checking\n\
         alias 3000 Member Equity = equity:member\n\
         alias 2005 Northbank Credit Card = liabilities:card\n\
         alias 6000 Sales and Marketing = expenses:marketing\n",
    );
    let aliases = plain_aliases(&journal);
    assert_eq!(
        unmapped_accounts(&aliases, &parsed.transactions),
        Vec::<String>::new(),
        "every account in the fixture must resolve"
    );
    // The cascade: an alias on the parent account also covers the sub-account
    // the real export writes as `6000 Sales and Marketing:6001 Sales & Marketing
    // Tools`, preserving the child segment.
    assert_eq!(
        resolve_account(
            "6000 Sales and Marketing:6001 Sales & Marketing Tools",
            &aliases
        )
        .as_deref(),
        Some("expenses:marketing:6001 Sales & Marketing Tools")
    );
}

#[test]
fn the_many_postings_fixtures_sub_account_is_unmapped_without_a_parent_alias() {
    let parsed = qb_journal::parse(&qb_fixture("many-postings.xlsx")).expect("parses");
    let journal = journal("");
    let aliases = plain_aliases(&journal);
    let unmapped = unmapped_accounts(&aliases, &parsed.transactions);
    assert!(
        unmapped.iter().any(|account| account
            == "1520 Computer & Office Equipment:1521 Computer & Equipment - Accum Depr"),
        "the colon-containing sub-account must be reported by its own full name: {unmapped:?}"
    );
}
