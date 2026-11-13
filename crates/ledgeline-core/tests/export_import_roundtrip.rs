//! Export → import equivalence (plan 39, Verification): what Ledgeline exports
//! as QIF or posting-row CSV, Ledgeline's own importers read back with the same
//! per-account totals.
//!
//! The comparison is on the supported subset each format documents:
//!
//! * **CSV** carries every real posting, so every account's total must
//!   survive. Virtual postings (`(a)`, `[a]`) are written by the export (as
//!   hledger does) but left out by the importer, which has no virtual postings;
//!   both formats leave them out of this comparison.
//! * **QIF** carries registers (bank-like accounts) and categories (revenue and
//!   expense accounts). Equity legs become opening balances or go through the
//!   "Ledgeline Adjustments" holding register, so equity is not compared; every
//!   other account the journal touches must come back with its own total.
//!
//! Account names are compared as the importer offers them to the alias-mapping
//! step: the QIF exporter keeps hledger's `:` hierarchy and full account names,
//! renaming only the characters QIF reserves ([`qif_name`]).

mod common;

use common::fixtures_dir;
use ledgeline_core::Dec;
use ledgeline_core::export::csv::export_csv;
use ledgeline_core::export::load_journal;
use ledgeline_core::export::qif::{QifOptions, TransferPlacement, export_qif, qif_name};
use ledgeline_core::model::{Journal, PostingType};
use ledgeline_core::qb_journal::QbJournal;
use ledgeline_core::reports::{AccountType, AccountTypes, account_decls, declared_types};
use std::collections::BTreeMap;

/// Single-commodity fixtures (the importers refuse a file mixing commodities,
/// since the multi-posting shape has no commodity per posting).
const FIXTURES: [&str; 3] = [
    "export/splits-transfers.journal",
    "export/holding.journal",
    "account-types/non-english.journal",
];

type Totals = BTreeMap<String, Dec>;

fn add(totals: &mut Totals, account: &str, amount: Dec) {
    let entry = totals.entry(account.to_string()).or_insert(Dec::zero());
    *entry = entry.add(amount).expect("fixture totals fit");
}

/// Per-account totals of the journal's real postings, auto-postings applied
/// (both exporters apply them first).
fn journal_totals(journal: &Journal) -> Totals {
    let applied = ledgeline_core::auto::apply_journal(journal);
    let mut totals = Totals::new();
    for txn in &applied.transactions {
        for posting in txn
            .postings
            .iter()
            .filter(|posting| posting.ptype == PostingType::Regular)
        {
            for amount in &posting.amounts {
                add(&mut totals, &posting.account.0, amount.quantity);
            }
        }
    }
    totals.retain(|_, total| !total.is_zero());
    totals
}

fn imported_totals(imported: &[QbJournal]) -> Totals {
    let mut totals = Totals::new();
    for book in imported {
        for txn in &book.transactions {
            for posting in &txn.postings {
                add(&mut totals, &posting.account, posting.amount);
            }
        }
    }
    totals.retain(|_, total| !total.is_zero());
    totals
}

#[test]
fn a_posting_row_csv_export_reads_back_with_every_account_total() {
    for fixture in FIXTURES {
        let journal = load_journal(&fixtures_dir().join(fixture)).expect("fixture parses");
        let csv = export_csv(&journal).text;
        let parsed = ledgeline_core::grouped_csv::parse(csv.as_bytes())
            .unwrap_or_else(|error| panic!("{fixture}: re-import refused: {error}"));
        assert_eq!(
            parsed.source,
            ledgeline_core::qb_journal::ImportSource::HledgerCsv
        );
        assert_eq!(
            imported_totals(&[parsed]),
            journal_totals(&journal),
            "{fixture}: per-account totals differ after a CSV round trip"
        );
    }
}

#[test]
fn a_qif_export_reads_back_with_every_register_and_category_total() {
    for placement in [TransferPlacement::Once, TransferPlacement::Mirrored] {
        for fixture in FIXTURES {
            let journal = load_journal(&fixtures_dir().join(fixture)).expect("fixture parses");
            let options = QifOptions {
                name: "roundtrip".to_string(),
                transfers: placement,
            };
            let export = export_qif(&journal, &options).expect("fixture exports");
            let imported: Vec<QbJournal> = export
                .files
                .iter()
                .map(|file| {
                    ledgeline_core::qif_import::parse_and_import(
                        &file.file_name,
                        file.text.as_bytes(),
                    )
                    .unwrap_or_else(|error| panic!("{fixture}: re-import refused: {error}"))
                })
                .collect();

            let types = AccountTypes::from_declared(declared_types(&account_decls(&journal)));
            let mut expected = journal_totals(&journal);
            expected.retain(|account, _| !types.is_type(account, AccountType::Equity));
            let actual = imported_totals(&imported);
            for (account, total) in &expected {
                assert_eq!(
                    actual.get(&qif_name(account)),
                    Some(total),
                    "{fixture} ({placement:?}): {account} total differs after a QIF round trip"
                );
            }
        }
    }
}

/// Units per commodity across every posting of the journal (auto-postings
/// applied), for the commodities in `wanted`.
fn journal_units(journal: &Journal, wanted: &[&str]) -> Totals {
    let applied = ledgeline_core::auto::apply_journal(journal);
    let mut totals = Totals::new();
    for posting in applied
        .transactions
        .iter()
        .flat_map(|txn| &txn.postings)
        .filter(|posting| posting.ptype == PostingType::Regular)
    {
        for amount in posting
            .amounts
            .iter()
            .filter(|amount| wanted.contains(&amount.commodity.0.as_str()))
        {
            add(&mut totals, &amount.commodity.0, amount.quantity);
        }
    }
    totals.retain(|_, total| !total.is_zero());
    totals
}

fn imported_units(imported: &[QbJournal]) -> Totals {
    let mut totals = Totals::new();
    for units in imported
        .iter()
        .flat_map(|book| &book.transactions)
        .flat_map(|txn| &txn.postings)
        .filter_map(|posting| posting.units.as_ref())
    {
        add(&mut totals, &units.commodity, units.quantity);
    }
    totals.retain(|_, total| !total.is_zero());
    totals
}

#[test]
fn a_qif_export_reads_back_with_every_share_count() {
    let cases: [(&str, &[&str]); 2] = [
        ("export/investments.journal", &["AAPL", "VTI", "FXAIX"]),
        ("sample.journal", &["AAPL", "VTI", "NVDA", "TSLA", "GLD"]),
    ];
    for placement in [TransferPlacement::Once, TransferPlacement::Mirrored] {
        for (fixture, securities) in cases {
            let journal = load_journal(&fixtures_dir().join(fixture)).expect("fixture parses");
            let options = QifOptions {
                name: "roundtrip".to_string(),
                transfers: placement,
            };
            let export = export_qif(&journal, &options).expect("fixture exports");
            let imported: Vec<QbJournal> = export
                .files
                .iter()
                .map(|file| {
                    ledgeline_core::qif_import::parse_and_import(
                        &file.file_name,
                        file.text.as_bytes(),
                    )
                    .unwrap_or_else(|error| panic!("{fixture}: re-import refused: {error}"))
                })
                .collect();
            assert_eq!(
                imported_units(&imported),
                journal_units(&journal, securities),
                "{fixture} ({placement:?}): share counts differ after a QIF round trip"
            );
        }
    }
}
