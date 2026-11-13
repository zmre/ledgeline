//! QIF export goldens and loss-report cases (plan 39 Phase 1).
//!
//! Each fixture's export — every per-commodity `.qif` file and the rendered
//! loss report (`loss.txt`) — is compared byte for byte against
//! `fixtures/export/qif/<fixture>/`. The goldens were reviewed by hand when
//! written; to regenerate after an intended change, run
//! `LEDGELINE_BLESS=1 cargo test -p ledgeline-core --test export_qif` and
//! review the diff.
//!
//! Every file is also checked for the property a QIF importer relies on:
//! each record's split lines add up to its total.

mod common;

use common::fixtures_dir;
use ledgeline_core::Dec;
use ledgeline_core::export::load_journal;
use ledgeline_core::export::qif::{
    HOLDING_ACCOUNT, QifExport, QifOptions, TransferPlacement, export_qif,
};
use std::path::Path;

/// Fixture journal (relative to `fixtures/`) → golden directory name under
/// `fixtures/export/qif/`.
const GOLDENS: [(&str, &str); 5] = [
    ("export/splits-transfers.journal", "splits-transfers"),
    ("export/multi-currency.journal", "multi-currency"),
    ("export/holding.journal", "holding"),
    ("account-types/non-english.journal", "non-english"),
    ("sample.journal", "sample"),
];

fn export(journal: &str, name: &str, transfers: TransferPlacement) -> QifExport {
    let journal = load_journal(&fixtures_dir().join(journal)).expect("fixture parses");
    export_qif(
        &journal,
        &QifOptions {
            name: name.to_string(),
            transfers,
        },
    )
    .expect("exports")
}

fn compare_or_bless(path: &Path, actual: &str, mismatched: &mut Vec<String>) {
    if std::env::var_os("LEDGELINE_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("mkdir");
        std::fs::write(path, actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_default();
    if expected != actual {
        mismatched.push(path.display().to_string());
    }
}

#[test]
fn matches_the_goldens() {
    let root = fixtures_dir().join("export/qif");
    let mut mismatched = Vec::new();
    for (journal, name) in GOLDENS {
        let out = export(journal, name, TransferPlacement::Once);
        let dir = root.join(name);
        for file in &out.files {
            compare_or_bless(&dir.join(&file.file_name), &file.text, &mut mismatched);
        }
        compare_or_bless(&dir.join("loss.txt"), &out.loss.render(), &mut mismatched);
        // No stale golden for a file the export no longer writes.
        let expected_files: Vec<String> = out
            .files
            .iter()
            .map(|file| file.file_name.clone())
            .chain(std::iter::once("loss.txt".to_string()))
            .collect();
        for entry in std::fs::read_dir(&dir).expect("golden dir").flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            assert!(
                expected_files.contains(&file_name),
                "{name}: stale golden {file_name}"
            );
        }
    }
    assert!(
        mismatched.is_empty(),
        "QIF output changed for {mismatched:?}; if intended, re-run with LEDGELINE_BLESS=1 and \
         review the diff"
    );
}

/// Parse a QIF amount (`-1234.50`) exactly.
fn amount(text: &str) -> Dec {
    Dec::parse(text, '.').expect("a QIF amount")
}

/// Every record's `$` split lines sum to its `T` total, in every file of every
/// fixture, under both transfer placements.
#[test]
fn every_split_record_adds_up() {
    for placement in [TransferPlacement::Once, TransferPlacement::Mirrored] {
        for (journal, name) in GOLDENS {
            for file in export(journal, name, placement).files {
                for record in file.text.split("^\n") {
                    let total = record.lines().find_map(|line| line.strip_prefix('T'));
                    let splits: Vec<Dec> = record
                        .lines()
                        .filter_map(|line| line.strip_prefix('$'))
                        .map(amount)
                        .collect();
                    let (Some(total), false) = (total, splits.is_empty()) else {
                        continue;
                    };
                    let sum = splits
                        .into_iter()
                        .try_fold(Dec::zero(), Dec::add)
                        .expect("no overflow");
                    assert_eq!(sum, amount(total), "{name}/{}: {record}", file.file_name);
                }
            }
        }
    }
}

/// QIF has no currency field: one file per commodity, the `D` default first,
/// and costs written as the cash they cost.
#[test]
fn multi_currency_writes_one_file_per_commodity() {
    let out = export(
        "export/multi-currency.journal",
        "books",
        TransferPlacement::Once,
    );
    let names: Vec<&str> = out.files.iter().map(|f| f.file_name.as_str()).collect();
    assert_eq!(names, ["books-dollar.qif", "books-EUR.qif"]);
    let loss = &out.loss;
    assert_eq!(loss.extra_commodity_files(), 1);
    // The AAPL buy (`@`) into a brokerage account is written at cost.
    assert_eq!(loss.cost_postings, 1);
    // The wire into the euro account (`@@`, a bank: the euros are kept) and
    // the swap with no written cost.
    assert_eq!(loss.conversions, 2);
    let euro = &out.files[1].text;
    assert!(
        euro.contains("D01/05/2026\nT500.00\nPWire to euro account\nMcommodity conversion\n"),
        "{euro}"
    );
    assert_eq!(loss.price_directives, 2);
    // Opening balances in both currencies.
    assert_eq!(loss.opening_balances, 2);
    assert!(!out.files.iter().any(|f| f.commodity.0 == "AAPL"));
    let text = loss.render();
    assert!(
        text.contains("1 extra commodity written to a file of its own"),
        "{text}"
    );
}

/// What has no bank-like account goes through the holding account, and every
/// dropped feature is counted.
#[test]
fn holding_account_and_dropped_features_are_reported() {
    let out = export("export/holding.journal", "h", TransferPlacement::Once);
    let loss = &out.loss;
    // The reclassification (no register at all), the owner draw (equity) and
    // nothing else: the opening is an Opening Balance record.
    assert_eq!(loss.holding_account_transactions, 2);
    assert_eq!(loss.opening_balances, 1);
    assert_eq!(loss.balance_assertions, 1);
    assert_eq!(loss.periodic_transactions, 1);
    assert_eq!(loss.auto_posting_rules, 1);
    // Two postings on the one matching transaction.
    assert_eq!(loss.auto_postings_applied, 2);
    assert_eq!(loss.virtual_postings, 1);
    assert_eq!(loss.secondary_dates, 1);
    // Counted where written, not again on the generated postings that
    // inherit it.
    assert_eq!(loss.posting_dates, 1);
    assert_eq!(loss.transaction_tags, 1);
    assert_eq!(loss.account_tags, 1);
    assert_eq!(loss.untyped_accounts, ["misc/shared"]);
    assert_eq!(
        loss.renamed_accounts,
        [("misc/shared".to_string(), "misc-shared".to_string())]
    );
    let text = &out.files[0].text;
    assert!(
        text.contains(&format!(
            "!Account\nN{HOLDING_ACCOUNT}\nTOth A\n^\n!Type:Oth A\n"
        )),
        "{text}"
    );
}

/// Investments fall back to cash movement at cost: the broker account holds
/// the dollars the shares cost, and the report says the units are gone.
#[test]
fn investments_fall_back_to_cash_at_cost() {
    let out = export("sample.journal", "s", TransferPlacement::Once);
    assert!(out.loss.cost_postings > 0);
    assert!(
        out.loss
            .render()
            .contains("written as the cash amount; units, lots and prices are not carried")
    );
    let commodities: Vec<&str> = out.files.iter().map(|f| f.commodity.0.as_str()).collect();
    assert!(!commodities.contains(&"AAPL"), "{commodities:?}");
}

/// Accounts are classified by resolved type, so a Spanish chart exports the
/// same shapes an English one does.
#[test]
fn non_english_accounts_are_classified_by_type() {
    let out = export(
        "account-types/non-english.journal",
        "n",
        TransferPlacement::Once,
    );
    assert!(out.loss.untyped_accounts.is_empty(), "{:?}", out.loss);
    let text = &out.files[0].text;
    assert!(text.contains("TBank\n"), "{text}");
    assert!(text.contains("\nI\n^\n"), "{text}");
}

/// Mirrored placement writes the other side of each transfer as its own
/// record, so it has more records than the default.
#[test]
fn mirrored_placement_adds_the_other_side() {
    let once = export(
        "export/splits-transfers.journal",
        "m",
        TransferPlacement::Once,
    );
    let mirrored = export(
        "export/splits-transfers.journal",
        "m",
        TransferPlacement::Mirrored,
    );
    // Savings transfer, payroll's auto-save, card payment, loan payment.
    assert_eq!(mirrored.loss.records, once.loss.records + 4);
}
