//! `qb_journal` over the committed corpus in `fixtures/import/qb-journal/`, plus
//! the detector's behaviour against every *other* import fixture in the repo.
//!
//! Three kinds of check live here and it matters that they stay separate:
//!
//! | Kind | What it proves |
//! | --- | --- |
//! | Per-fixture | One shape of the real export parses to the transactions we say it does |
//! | Refusal | Each way a damaged export is wrong gets its own named error, naming the group |
//! | Adversarial | The detector answers **no** on the rest of the corpus, including a file built to look like a hit |
//!
//! The third is the one worth reading. `near-miss.xlsx` carries `Account Name`,
//! `Debit` and `Credit` labels and a `Total` row, which is everything a detector
//! keyed on column names would want; a detector that stopped there would route an
//! ordinary bank export into a pipeline that cannot read it and would never show
//! the user the rules-matching screen they actually needed.

mod common;

use ledgeline_core::Dec;
use ledgeline_core::qb_journal::{self, QbJournal, QbJournalError};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Fixture access
// ---------------------------------------------------------------------------

fn import_fixture(relative: &str) -> Vec<u8> {
    let path: PathBuf = common::fixtures_dir().join("import").join(relative);
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {relative} should read: {error}"))
}

fn qb_fixture(name: &str) -> Vec<u8> {
    import_fixture(&format!("qb-journal/{name}"))
}

fn parse(name: &str) -> QbJournal {
    qb_journal::parse(&qb_fixture(name))
        .unwrap_or_else(|error| panic!("{name} should parse: {error}"))
}

fn refuse(name: &str) -> QbJournalError {
    qb_journal::parse(&qb_fixture(name)).expect_err(&format!("{name} should be refused"))
}

fn dec(text: &str) -> Dec {
    Dec::parse(text, '.').expect("test literal parses")
}

// ---------------------------------------------------------------------------
// Per-fixture: what the real export's shapes parse to
// ---------------------------------------------------------------------------

#[test]
fn a_two_posting_group_becomes_one_transaction_with_signed_amounts() {
    let journal = parse("simple.xlsx");
    assert_eq!(journal.transactions.len(), 2, "two markers, two groups");

    let deposit = &journal.transactions[0];
    assert_eq!(deposit.id, "441");
    assert_eq!(deposit.date, "2026-01-17");
    assert_eq!(deposit.transaction_type, "Deposit");
    assert_eq!(deposit.name.as_deref(), Some("Ridgeline Partners, LP"));
    assert_eq!(
        deposit.num, None,
        "an empty Num cell is None, never Some(\"\")"
    );

    // The sign check that needs no account-type knowledge: money arriving in the
    // bank account is a debit and becomes positive, the equity it came from is a
    // credit and becomes negative, and the two cancel.
    let amounts: Vec<(&str, &Dec)> = deposit
        .postings
        .iter()
        .map(|posting| (posting.account.as_str(), &posting.amount))
        .collect();
    assert_eq!(
        amounts,
        [
            ("Riverbank BUSINESS CHECKING (0002)", &dec("74999.71")),
            ("3000 Member Equity", &dec("-74999.71")),
        ]
    );

    // The same check from the liability side: charging a credit card credits the
    // card (negative, which is how hledger holds a liability) and debits the
    // expense (positive).
    let expense = &journal.transactions[1];
    assert_eq!(expense.id, "33");
    assert_eq!(expense.date, "2026-01-05");
    assert_eq!(expense.transaction_type, "Expense");
    let amounts: Vec<(&str, &Dec)> = expense
        .postings
        .iter()
        .map(|posting| (posting.account.as_str(), &posting.amount))
        .collect();
    assert_eq!(
        amounts,
        [
            ("2005 Northbank Credit Card", &dec("-79.99")),
            (
                "6000 Sales and Marketing:6001 Sales & Marketing Tools",
                &dec("79.99")
            ),
        ]
    );
}

#[test]
fn an_empty_string_cell_is_absent_and_never_an_empty_tag() {
    // Every unused text cell on a real posting row is `Data::String("")`, not
    // `Data::Empty`. Read by emptiness of the *variant* rather than of the text,
    // every posting here acquires `class: Some("")` and a `class:` tag with
    // nothing after it lands in the user's journal.
    let journal = parse("simple.xlsx");
    for transaction in &journal.transactions {
        for posting in &transaction.postings {
            assert_eq!(posting.class, None, "Item class is blank on every row");
        }
    }
    let deposit = &journal.transactions[0];
    assert_eq!(
        deposit.postings[0].customer.as_deref(),
        Some("Ridgeline Partners, LP")
    );
    assert_eq!(deposit.postings[0].vendor, None);

    let expense = &journal.transactions[1];
    assert_eq!(expense.postings[0].customer, None);
    assert_eq!(
        expense.postings[0].vendor.as_deref(),
        Some("Grasshopper Cloud")
    );
}

#[test]
fn a_memo_belongs_to_its_posting_and_not_to_the_transaction() {
    // The ten-line Journal Entry. Six of its ten postings carry a memo the
    // others do not, so flattening the group to one transaction-level
    // description keeps one and loses the rest.
    let journal = parse("many-postings.xlsx");
    let [entry] = &journal.transactions[..] else {
        panic!("one group, one transaction");
    };
    assert_eq!(entry.id, "612");
    assert_eq!(entry.transaction_type, "Journal Entry");
    assert_eq!(entry.postings.len(), 10);
    // `Num` repeats on every posting row rather than appearing only on the first.
    assert_eq!(entry.num.as_deref(), Some("2"));

    let memos: Vec<Option<&str>> = entry
        .postings
        .iter()
        .map(|posting| posting.memo.as_deref())
        .collect();
    assert_eq!(
        memos,
        [
            Some("Opening Balance Entry"),
            Some("Opening Balance Entry"),
            Some("Bulk Buy - Soda Machine"),
            Some("Riverbend - LCD Monitor"),
            Some("Monitor Arms"),
            Some("Riverbend"),
            Some("Opening Balance Entry"),
            Some("Opening Balance Entry - Accumulated Depreciation catchup"),
            Some("Opening Balance Entry - Accumulated Depreciation catchup"),
            Some("Opening Balance Entry - write off 2015 reconciling items"),
        ],
        "six distinct memos across ten postings"
    );

    // A sub-account arrives with QuickBooks' own colon in it, untouched. The
    // WP's Phase B sketch says these never contain a colon; ten of the real
    // export's eighteen accounts do.
    assert!(
        entry.postings.iter().any(|posting| posting.account
            == "1520 Computer & Office Equipment:1521 Computer & Equipment - Accum Depr"),
        "the sub-account keeps its colon"
    );
}

#[test]
fn the_total_rows_seventeen_stored_digits_are_read_as_the_cent_value() {
    // `many-postings.xlsx` stores its total as `70120.850000000006`, which is
    // what Excel wrote. Compared in `f64`, or read as the text in the file, that
    // disagrees with the postings' own sum by 6e-12 and the group is refused.
    // Shortest-round-trip printing is the only thing that recovers 70120.85.
    let journal = parse("many-postings.xlsx");
    let [entry] = &journal.transactions[..] else {
        panic!("one group");
    };
    let debits = entry
        .postings
        .iter()
        .filter(|posting| posting.amount > Dec::zero())
        .count();
    assert_eq!(debits, 7, "seven debit rows, three credit rows");

    // 70120.85 both ways, to the cent, from a stored 70120.850000000006.
    let debited = entry
        .postings
        .iter()
        .filter(|posting| posting.amount > Dec::zero())
        .try_fold(Dec::zero(), |total, posting| total.add(posting.amount))
        .expect("the debits sum");
    assert_eq!(debited, dec("70120.85"));
    let sum = entry
        .postings
        .iter()
        .try_fold(Dec::zero(), |total, posting| total.add(posting.amount))
        .expect("the postings sum");
    assert!(sum.is_zero(), "a group that balances sums to nothing");
}

#[test]
fn the_stock_column_set_parses_to_what_the_customized_one_does() {
    // Four columns fewer, and `Memo/Description`/`Account` where the other file
    // says `Description`/`Account Name`. Everything that is not QuickBooks
    // dimensional metadata must come out identical.
    let custom = parse("simple.xlsx");
    let stock = parse("default-columns.xlsx");
    assert_eq!(custom.transactions.len(), stock.transactions.len());
    for (custom, stock) in custom.transactions.iter().zip(&stock.transactions) {
        assert_eq!(custom.id, stock.id);
        assert_eq!(custom.date, stock.date);
        assert_eq!(custom.transaction_type, stock.transaction_type);
        assert_eq!(custom.name, stock.name);
        assert_eq!(custom.postings.len(), stock.postings.len());
        for (custom, stock) in custom.postings.iter().zip(&stock.postings) {
            assert_eq!(custom.account, stock.account);
            assert_eq!(custom.amount, stock.amount);
            assert_eq!(custom.memo, stock.memo);
        }
    }
    // The columns that are genuinely absent are absent, not empty.
    for transaction in &stock.transactions {
        for posting in &transaction.postings {
            assert_eq!(posting.customer, None);
            assert_eq!(posting.vendor, None);
            assert_eq!(posting.class, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Round trip: the whole report
// ---------------------------------------------------------------------------

#[test]
fn every_group_in_the_full_report_is_parsed_and_balances() {
    let journal = parse("report.xlsx");

    // The count anyone can reproduce by searching the spreadsheet for the
    // closing rows, rather than by re-running this parser.
    assert_eq!(
        journal.transactions.len(),
        45,
        "one transaction per `Total for ` row"
    );
    assert_eq!(
        journal
            .transactions
            .iter()
            .map(|transaction| transaction.postings.len())
            .sum::<usize>(),
        100,
        "43 groups of two, one of four, one of ten"
    );

    for transaction in &journal.transactions {
        let sum = transaction
            .postings
            .iter()
            .try_fold(Dec::zero(), |total, posting| total.add(posting.amount))
            .expect("postings sum");
        assert!(
            sum.is_zero(),
            "group {} must balance to nothing",
            transaction.id
        );
        assert!(
            transaction.postings.len() >= 2,
            "group {} has at least two postings",
            transaction.id
        );
        assert!(
            transaction.date.starts_with("2026-01-"),
            "group {} normalised its date, got {}",
            transaction.id,
            transaction.date
        );
    }

    // Dates run past the 12th, so month-first is a measurement and not a guess.
    assert_eq!(journal.date_format.format, "%m/%d/%Y");
    assert!(
        !journal.date_format.ambiguous,
        "a day above 12 rules out day-first"
    );
}

// ---------------------------------------------------------------------------
// Refusals: one named error per way a damaged export is wrong
// ---------------------------------------------------------------------------

#[test]
fn a_group_closed_by_another_groups_total_is_refused_by_id() {
    // The real export's own damage. Its four postings balance perfectly, so
    // every arithmetic check passes and only the id says rows were deleted. A
    // parser that pairs marker to total by position imports this silently.
    match refuse("truncated-tail.xlsx") {
        QbJournalError::TotalIdMismatch {
            opened,
            closed,
            row,
        } => {
            assert_eq!(opened, "6");
            assert_eq!(closed, "11024");
            assert_eq!(row, 15, "1-based, so it is the row number Excel shows");
        }
        other => panic!("expected TotalIdMismatch, got {other:?}"),
    }
}

#[test]
fn a_ref_error_in_a_total_is_refused_by_name() {
    // `convert::spreadsheet` renders `Data::Error` as an empty string, which
    // here reads as a total of zero against postings of 533.94 — a silent,
    // wrong, balanced-looking answer. It has to be a refusal.
    match refuse("malformed-total.xlsx") {
        QbJournalError::MalformedTotal { id, cell } => {
            assert_eq!(id, "6");
            assert_eq!(cell, "#REF!");
        }
        other => panic!("expected MalformedTotal, got {other:?}"),
    }
}

#[test]
fn a_total_that_disagrees_with_its_own_postings_is_refused_with_both_numbers() {
    match refuse("mismatched-total.xlsx") {
        QbJournalError::MismatchedTotal {
            id,
            computed,
            reported,
        } => {
            assert_eq!(id, "6");
            assert_eq!(computed, dec("533.94"));
            assert_eq!(reported, dec("500.00"));
        }
        other => panic!("expected MismatchedTotal, got {other:?}"),
    }
}

#[test]
fn a_total_row_with_no_marker_above_it_is_refused() {
    match refuse("orphan-total.xlsx") {
        QbJournalError::OrphanTotal { id, row } => {
            assert_eq!(id, "99");
            assert_eq!(row, 6);
        }
        other => panic!("expected OrphanTotal, got {other:?}"),
    }
}

#[test]
fn a_file_that_is_not_a_quickbooks_journal_is_refused_before_anything_else() {
    // The near-miss has the header triple and no grouping structure at all.
    assert!(matches!(
        qb_journal::parse(&qb_fixture("near-miss.xlsx")),
        Err(QbJournalError::NoHeader)
    ));
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn every_quickbooks_journal_shape_is_detected() {
    for name in [
        "simple.xlsx",
        "default-columns.xlsx",
        "many-postings.xlsx",
        "report.xlsx",
        "mismatched-total.xlsx",
        "orphan-total.xlsx",
    ] {
        assert!(
            qb_journal::detect(&qb_fixture(name)),
            "{name} is a QuickBooks Journal export"
        );
    }
}

#[test]
fn a_damaged_export_is_still_detected_as_one() {
    // Detection and parsing answer different questions. A truncated export is
    // still unmistakably a QuickBooks Journal, and saying "no" here would route
    // the user to the CSV rules screen instead of to the named refusal that
    // tells them their file lost rows.
    for name in ["truncated-tail.xlsx", "malformed-total.xlsx"] {
        assert!(qb_journal::detect(&qb_fixture(name)), "{name} is detected");
        assert!(
            qb_journal::parse(&qb_fixture(name)).is_err(),
            "{name} refused"
        );
    }
}

#[test]
fn the_near_miss_workbook_is_not_detected() {
    // `Account Name`, `Debit`, `Credit` and a `Total` row, and still not one.
    assert!(!qb_journal::detect(&qb_fixture("near-miss.xlsx")));
}

#[test]
fn nothing_else_in_the_import_corpus_is_detected_as_a_quickbooks_journal() {
    // Every other fixture the New Transactions tab accepts. A false positive
    // here costs the user their working rules-file flow, so the list is
    // deliberately the whole corpus and not a sample of it.
    let corpus = [
        // Workbooks, including the one real (scrubbed) brokerage export.
        "spreadsheet/simple.xlsx",
        "spreadsheet/multi-sheet.xlsx",
        "spreadsheet/preamble.xlsx",
        "spreadsheet/trailer.xlsx",
        "spreadsheet/single-column.xlsx",
        "spreadsheet/no-table.xlsx",
        "spreadsheet/legacy.xls",
        "spreadsheet/sheet.ods",
        "spreadsheet/real-brokerage-preamble.xlsx",
        // Delimited text, including the two that are not UTF-8 at all.
        "delimited/tab.tsv",
        "delimited/semicolon.ssv",
        "delimited/preamble.csv",
        "delimited/ragged.csv",
        "delimited/trailer.csv",
        "delimited/padded-prose.csv",
        "delimited/trailing-delimiter.csv",
        "delimited/quoted.csv",
        "delimited/latin1.csv",
        "delimited/utf16le-bom.csv",
        // The rules generator's corpus. `capitalone-card.csv` and
        // `ambiguous-dates.csv` both carry Debit AND Credit columns, and
        // `quickbooks-label.csv` carries an unnamed first column holding one
        // isolated cell — the closest thing in the repo to a marker row.
        "generate/headers/capitalone-card.csv",
        "generate/headers/ambiguous-dates.csv",
        "generate/headers/chase-checking.csv",
        "generate/headers/uk-current-account.csv",
        "generate/headers/euro-decimal-comma.csv",
        "generate/headers/paypal-activity.csv",
        "generate/headers/thousands-trap.csv",
        "generate/isolated/quickbooks-label.csv",
        "generate/isolated/check-number.csv",
    ];
    for name in corpus {
        assert!(
            !qb_journal::detect(&import_fixture(name)),
            "{name} must not be read as a QuickBooks Journal export"
        );
    }
}

#[test]
fn detection_declines_rather_than_panicking_on_anything() {
    for bytes in [
        b"".to_vec(),
        b"not a workbook at all".to_vec(),
        b"PK\x03\x04 truncated zip".to_vec(),
        vec![0u8; 512],
    ] {
        assert!(!qb_journal::detect(&bytes));
    }
}
