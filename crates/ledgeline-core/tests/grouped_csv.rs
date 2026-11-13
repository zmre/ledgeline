//! Posting-row CSV presets (plan 39 Phase 4) over `fixtures/import/grouped-csv/`.
//!
//! `hledger-print.csv` and `hledger-print.bal.csv` were captured from
//! `hledger 1.52 --no-conf` over `roundtrip.journal` (2026-11-12). With
//! `LEDGELINE_HLEDGER_GROUPED_CSV_CHECK=1` the round trip also runs live against
//! the `hledger` on `PATH`, over several fixture journals.

mod common;

use common::fixtures_dir;
use ledgeline_core::Dec;
use ledgeline_core::grouped_csv::{self, GroupedCsvError, Preset};
use ledgeline_core::qb_journal::{ImportSource, ImportStatus, QbJournal};
use ledgeline_core::single_entry;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

const OPT_IN: &str = "LEDGELINE_HLEDGER_GROUPED_CSV_CHECK";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join("import/grouped-csv").join(name)).expect("fixture readable")
}

fn dec(text: &str) -> Dec {
    Dec::parse(text, '.').expect("literal")
}

/// Per-account totals, zero totals dropped (hledger `bal` omits them).
fn sums(journal: &QbJournal) -> BTreeMap<String, Dec> {
    let mut totals: BTreeMap<String, Dec> = BTreeMap::new();
    for posting in journal.transactions.iter().flat_map(|t| &t.postings) {
        let entry = totals
            .entry(posting.account.clone())
            .or_insert_with(Dec::zero);
        *entry = entry.add(posting.amount).expect("no overflow");
    }
    totals.retain(|_, total| !total.is_zero());
    totals
        .into_iter()
        .map(|(a, t)| (a, t.normalized()))
        .collect()
}

/// `hledger bal --flat --no-total -O csv` as account → amount.
fn bal_csv(text: &str) -> BTreeMap<String, Dec> {
    let mut reader = csv::ReaderBuilder::new().from_reader(text.as_bytes());
    reader
        .records()
        .map(|record| record.expect("bal csv"))
        .map(|record| {
            let number: String = record[1]
                .chars()
                .filter(|c| c.is_ascii_digit() || matches!(c, '-' | '.' | ','))
                .collect();
            let mark = if number.contains('.') { '.' } else { ',' };
            let amount = Dec::parse(&number, mark).expect("bal amount");
            (record[0].to_string(), amount.normalized())
        })
        .filter(|(_, amount)| !amount.is_zero())
        .collect()
}

// ---------------------------------------------------------------------------
// hledger print -O csv
// ---------------------------------------------------------------------------

#[test]
fn hledger_csv_round_trips_to_hledger_balances() {
    let journal = grouped_csv::parse(&fixture("hledger-print.csv")).expect("parses");
    assert_eq!(journal.source, ImportSource::HledgerCsv);
    assert_eq!(journal.transactions.len(), 8);
    let expected = bal_csv(&String::from_utf8(fixture("hledger-print.bal.csv")).expect("utf8"));
    assert_eq!(sums(&journal), expected);
}

#[test]
fn hledger_csv_carries_ids_status_code_and_memos() {
    let journal = grouped_csv::parse(&fixture("hledger-print.csv")).expect("parses");
    let txns = &journal.transactions;
    assert_eq!(txns[0].id, "open-2026");
    assert_eq!(txns[1].id, "chk-1042");
    assert_eq!(txns[1].num.as_deref(), Some("1042"));
    assert_eq!(txns[1].name.as_deref(), Some("Corner Grocer | weekly shop"));
    assert_eq!(txns[1].status, ImportStatus::Cleared);
    assert_eq!(txns[1].postings[0].memo.as_deref(), Some("organic produce"));
    assert_eq!(txns[1].postings[2].amount, dec("-100"));
    assert_eq!(txns[2].status, ImportStatus::Pending);
    assert_eq!(txns[3].status, ImportStatus::Unmarked);
    // No `id:` tag: a content hash, and the two identical Visa payments are
    // told apart by their occurrence index.
    assert!(txns[2].id.starts_with("hledger-"), "{}", txns[2].id);
    let (a, b) = (&txns[5].id, &txns[6].id);
    assert_eq!(a.strip_suffix("-0"), b.strip_suffix("-1"));
}

#[test]
fn hledger_csv_ids_do_not_depend_on_txnidx() {
    let original = grouped_csv::parse(&fixture("hledger-print.csv")).expect("parses");
    let text = String::from_utf8(fixture("hledger-print.csv")).expect("utf8");
    // A later export numbers the same transactions differently.
    let renumbered: String = text
        .lines()
        .map(
            |line| match line.strip_prefix('"').and_then(|rest| rest.split_once('"')) {
                Some((index, rest)) if index.parse::<u32>().is_ok() => {
                    format!("\"{}\"{rest}\n", index.parse::<u32>().unwrap_or(0) + 500)
                }
                _ => format!("{line}\n"),
            },
        )
        .collect();
    let again = grouped_csv::parse(renumbered.as_bytes()).expect("parses");
    let ids = |j: &QbJournal| {
        j.transactions
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&original), ids(&again));
}

#[test]
fn hledger_csv_reads_columns_by_name_not_position() {
    // hledger 1.99.4's order: debit before credit; and no `amount` column.
    let text = "\"txnidx\",\"date\",\"description\",\"account\",\"commodity\",\"debit\",\"credit\",\"posting-comment\"\n\
                \"1\",\"2026-02-01\",\"Shop\",\"expenses:food\",\"$\",\"5.00\",\"\",\"\"\n\
                \"1\",\"2026-02-01\",\"Shop\",\"assets:cash\",\"$\",\"\",\"5.00\",\"\"\n";
    let journal = grouped_csv::parse(text.as_bytes()).expect("parses");
    assert_eq!(journal.transactions[0].postings[0].amount, dec("5"));
    assert_eq!(journal.transactions[0].postings[1].amount, dec("-5"));
}

#[test]
fn hledger_csv_refuses_mixed_commodities() {
    let text = "txnidx,date,account,amount,commodity,posting-comment\n\
                1,2026-02-01,assets:broker,10,AAPL,\n\
                1,2026-02-01,assets:cash,-1500,$,\n";
    assert_eq!(
        grouped_csv::parse(text.as_bytes()),
        Err(GroupedCsvError::MixedCommodities {
            commodities: vec!["$".to_string(), "AAPL".to_string()]
        })
    );
}

#[test]
fn hledger_csv_reads_decimal_comma_commodities() {
    let text = "txnidx,date,account,amount,commodity,posting-comment\n\
                1,2026-02-01,expenses:travel,\"645,00\",EUR,\n\
                1,2026-02-01,assets:wise,\"-645,00\",EUR,\n";
    let journal = grouped_csv::parse(text.as_bytes()).expect("parses");
    assert_eq!(journal.transactions[0].postings[0].amount, dec("645"));
}

/// Live: `hledger print -O csv` → parse → per-account sums equal `hledger bal`.
#[test]
fn hledger_print_csv_round_trip_live() {
    if std::env::var(OPT_IN).is_err() {
        eprintln!("skipped: set {OPT_IN}=1");
        return;
    }
    let run = |file: &Path, args: &[&str]| -> String {
        let out = Command::new("hledger")
            .arg("-f")
            .arg(file)
            .arg("--no-conf")
            .args(args)
            .output()
            .expect("hledger runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf8")
    };
    let root = fixtures_dir();
    for name in [
        "import/grouped-csv/roundtrip.journal",
        "matching.journal",
        "corpus/status.journal",
        "corpus/dates.journal",
        "account-types/non-english.journal",
        "budget/basic.journal",
        "projections-expense-as-asset/fixed.journal",
    ] {
        let file = root.join(name);
        let printed = run(&file, &["print", "-O", "csv"]);
        let journal = grouped_csv::parse(printed.as_bytes())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let expected = bal_csv(&run(&file, &["bal", "--flat", "--no-total", "-O", "csv"]));
        assert_eq!(sums(&journal), expected, "{name}");
    }
    // A multi-commodity book is refused, not summed across commodities.
    let printed = run(&root.join("sample.journal"), &["print", "-O", "csv"]);
    assert!(matches!(
        grouped_csv::parse(printed.as_bytes()),
        Err(GroupedCsvError::MixedCommodities { .. })
    ));
}

// ---------------------------------------------------------------------------
// Xero
// ---------------------------------------------------------------------------

#[test]
fn xero_journal_report_groups_by_journal_id() {
    let parsed =
        grouped_csv::parse_preset(Preset::Xero, &fixture("xero-journal.csv")).expect("parses");
    let journal = &parsed.journal;
    assert_eq!(journal.source, ImportSource::Xero);
    let ids: Vec<&str> = journal.transactions.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        ["xero-101", "xero-102", "xero-103", "xero-104", "xero-105"]
    );
    let first = &journal.transactions[0];
    assert_eq!(first.date, "2026-01-03");
    assert_eq!(first.transaction_type, "Spend Money");
    assert_eq!(first.num.as_deref(), Some("INV-0042"));
    assert_eq!(first.postings[0].account, "090 Business Bank Account");
    assert_eq!(first.postings[0].amount, dec("-1250"));
    assert_eq!(first.postings[0].memo.as_deref(), Some("Office chairs"));
    // The reversal journal is imported as-is, so the edit nets out.
    let totals = sums(journal);
    assert_eq!(totals["090 Business Bank Account"], dec("-1300"));
    assert_eq!(totals["710 Office Equipment"], dec("1181.82"));
    // The report's Total row is skipped, not refused.
    assert_eq!(parsed.report.skipped_rows, 1);
}

#[test]
fn xero_gl_detail_groups_lines_sorted_by_account() {
    let parsed =
        grouped_csv::parse_preset(Preset::Xero, &fixture("xero-gl-detail.csv")).expect("parses");
    let txns = &parsed.journal.transactions;
    assert_eq!(txns.len(), 2);
    assert_eq!(txns[0].id, "xero-201");
    assert_eq!(txns[0].date, "2026-01-03");
    assert_eq!(txns[0].name.as_deref(), Some("Office Supplies Ltd"));
    assert_eq!(txns[0].postings.len(), 3);
    assert_eq!(txns[1].date, "2026-01-17");
    assert_eq!(parsed.report.skipped_rows, 8);
}

#[test]
fn xero_unbalanced_journal_is_refused_with_its_row() {
    assert_eq!(
        grouped_csv::parse(&fixture("xero-unbalanced.csv")),
        Err(GroupedCsvError::Unbalanced {
            row: 4,
            total: dec("-1.00")
        })
    );
}

#[test]
fn xero_without_a_journal_column_groups_balanced_runs() {
    let text = "Date,Account,Description,Debit,Credit\n\
                5 Feb 2026,Bank,Coffee,,4.50\n\
                5 Feb 2026,General Expenses,Coffee,4.50,\n\
                6 Feb 2026,Bank,Lunch,,12.00\n\
                6 Feb 2026,General Expenses,Lunch,12.00,\n";
    let parsed = grouped_csv::parse_preset(Preset::Xero, text.as_bytes()).expect("parses");
    assert_eq!(parsed.journal.transactions.len(), 2);
    assert!(parsed.journal.transactions[0].id.starts_with("xero-"));
    // Sorted by account (GL Detail without Journal ID): never balances.
    let sorted = "Date,Account,Debit,Credit\n\
                  5 Feb 2026,Bank,,4.50\n\
                  6 Feb 2026,Bank,,12.00\n\
                  5 Feb 2026,General Expenses,4.50,\n\
                  6 Feb 2026,General Expenses,12.00,\n";
    assert!(matches!(
        grouped_csv::parse_preset(Preset::Xero, sorted.as_bytes()),
        Err(GroupedCsvError::Unbalanced { row: 2, .. })
    ));
}

#[test]
fn xero_refusals_name_the_row() {
    let both = "Journal ID,Date,Account,Debit,Credit\n1,5 Feb 2026,Bank,4.50,4.50\n";
    assert_eq!(
        grouped_csv::parse(both.as_bytes()),
        Err(GroupedCsvError::AmountNotSplit { row: 2 })
    );
    let bad =
        "Journal ID,Date,Account,Debit,Credit\n1,5 Feb 2026,Bank,4.50,\n1,5 Feb 2026,Fees,,four\n";
    assert_eq!(
        grouped_csv::parse(bad.as_bytes()),
        Err(GroupedCsvError::MalformedAmount {
            row: 3,
            cell: "four".to_string()
        })
    );
    let no_key = "Journal ID,Date,Account,Debit,Credit\n,5 Feb 2026,Bank,4.50,\n";
    assert_eq!(
        grouped_csv::parse(no_key.as_bytes()),
        Err(GroupedCsvError::MissingKey { row: 2 })
    );
}

// ---------------------------------------------------------------------------
// GnuCash
// ---------------------------------------------------------------------------

#[test]
fn gnucash_multiline_carries_the_transaction_down() {
    let parsed = grouped_csv::parse_preset(Preset::GnuCash, &fixture("gnucash-multiline.csv"))
        .expect("parses");
    let journal = &parsed.journal;
    assert_eq!(journal.source, ImportSource::GnuCash);
    assert_eq!(parsed.report.skipped_transactions, 1, "the voided one");
    let txns = &journal.transactions;
    assert_eq!(txns.len(), 3);
    assert_eq!(txns[0].id, "gnucash-5f1c0a9e2b7d4c3a9e8f7a6b5c4d3e21");
    assert_eq!(txns[0].date, "2026-01-05");
    assert_eq!(txns[0].num.as_deref(), Some("101"));
    assert_eq!(txns[0].name.as_deref(), Some("Corner Grocer"));
    assert_eq!(txns[0].status, ImportStatus::Cleared);
    assert_eq!(txns[0].postings[0].account, "Expenses:Groceries");
    assert_eq!(txns[0].postings[0].memo.as_deref(), Some("organic produce"));
    assert_eq!(txns[1].postings.len(), 4);
    assert_eq!(txns[1].postings[0].amount, dec("3104.55"));
    assert_eq!(txns[2].status, ImportStatus::Unmarked);
}

#[test]
fn gnucash_simple_layout_with_european_numbers() {
    let journal = grouped_csv::parse(&fixture("gnucash-simple.csv")).expect("parses");
    assert_eq!(journal.transactions.len(), 2);
    assert_eq!(journal.transactions[0].date, "2026-01-05");
    assert_eq!(journal.transactions[1].postings[0].amount, dec("3104.55"));
}

#[test]
fn gnucash_split_before_any_transaction_is_refused() {
    let text = "Date,Transaction ID,Description,Full Account Name,Value Num.\n\
                ,,,Expenses:Food,5.00\n";
    assert_eq!(
        grouped_csv::parse(text.as_bytes()),
        Err(GroupedCsvError::MissingKey { row: 2 })
    );
}

#[test]
fn gnucash_refuses_more_than_one_currency() {
    let text = "Date,Transaction ID,Commodity/Currency,Full Account Name,Value Num.\n\
                01/05/2026,a,CURRENCY::USD,,\n,,,X,1\n,,,Y,-1\n\
                01/06/2026,b,CURRENCY::EUR,,\n,,,X,1\n,,,Y,-1\n";
    assert!(matches!(
        grouped_csv::parse(text.as_bytes()),
        Err(GroupedCsvError::MixedCommodities { .. })
    ));
}

// ---------------------------------------------------------------------------
// Detection and stability
// ---------------------------------------------------------------------------

#[test]
fn detects_each_preset() {
    for (name, preset) in [
        ("hledger-print.csv", Preset::HledgerCsv),
        ("xero-journal.csv", Preset::Xero),
        ("xero-gl-detail.csv", Preset::Xero),
        ("xero-unbalanced.csv", Preset::Xero),
        ("gnucash-multiline.csv", Preset::GnuCash),
        ("gnucash-simple.csv", Preset::GnuCash),
    ] {
        assert_eq!(
            grouped_csv::detect_preset(name, &fixture(name)),
            Some(preset),
            "{name}"
        );
        assert!(!single_entry::detect(name, &fixture(name)), "{name}");
    }
    // Not a delimited name: never claimed.
    assert!(!grouped_csv::detect(
        "export.pdf",
        &fixture("hledger-print.csv")
    ));
    assert!(!grouped_csv::detect(
        "journal",
        &fixture("hledger-print.csv")
    ));
}

/// Every other import fixture (bank CSVs, OFX, spreadsheets, QuickBooks) must
/// stay with the pipeline it was built for.
#[test]
fn no_other_import_fixture_is_detected() {
    let root = fixtures_dir().join("import");
    let mut checked = 0;
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                let leaf = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if !matches!(leaf, "grouped-csv" | "single-entry") {
                    stack.push(path);
                }
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let bytes = std::fs::read(&path).expect("readable");
            assert!(!grouped_csv::detect(&name, &bytes), "{}", path.display());
            assert!(!single_entry::detect(&name, &bytes), "{}", path.display());
            checked += 1;
        }
    }
    assert!(checked > 20, "walked {checked} fixtures");
}

#[test]
fn reparsing_gives_the_same_ids() {
    for name in [
        "hledger-print.csv",
        "xero-journal.csv",
        "gnucash-multiline.csv",
    ] {
        let a = grouped_csv::parse(&fixture(name)).expect("parses");
        let b = grouped_csv::parse(&fixture(name)).expect("parses");
        assert_eq!(a, b, "{name}");
    }
}

#[test]
fn unrecognised_and_empty_files_are_refused() {
    assert_eq!(
        grouped_csv::parse(b"Date,Description,Amount\n2026-01-01,X,1\n"),
        Err(GroupedCsvError::NotRecognised)
    );
    assert_eq!(grouped_csv::parse(b""), Err(GroupedCsvError::Empty));
    let header_only = "txnidx,date,account,amount,posting-comment\n";
    assert_eq!(
        grouped_csv::parse(header_only.as_bytes()),
        Err(GroupedCsvError::UnreadableDates)
    );
}

// ---------------------------------------------------------------------------
// Hostile input
// ---------------------------------------------------------------------------

/// Deterministic byte mutations of every fixture in both directories: parse and
/// detect must return, never panic.
#[test]
fn mutated_fixtures_never_panic() {
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let root = fixtures_dir().join("import");
    let files: Vec<Vec<u8>> = ["grouped-csv", "single-entry"]
        .iter()
        .flat_map(|dir| std::fs::read_dir(root.join(dir)).expect("dir").flatten())
        .map(|entry| std::fs::read(entry.path()).expect("readable"))
        .collect();
    let alphabet = b",;\"\n\r-().$0123456789/: \x00\xff";
    for original in &files {
        for _ in 0..300 {
            let mut bytes = original.clone();
            for _ in 0..(1 + next() % 6) {
                if bytes.is_empty() {
                    break;
                }
                let at = (next() as usize) % bytes.len();
                match next() % 4 {
                    0 => bytes[at] = alphabet[(next() as usize) % alphabet.len()],
                    1 => bytes.insert(at, alphabet[(next() as usize) % alphabet.len()]),
                    2 => {
                        bytes.remove(at);
                    }
                    _ => bytes.truncate(at),
                }
            }
            let _ = grouped_csv::parse(&bytes);
            let _ = single_entry::parse(&bytes);
            for preset in Preset::ALL {
                let _ = grouped_csv::parse_preset(preset, &bytes);
            }
            for preset in single_entry::Preset::ALL {
                let _ = single_entry::parse_preset(preset, &bytes);
            }
            let _ = grouped_csv::detect("x.csv", &bytes);
        }
    }
}

proptest::proptest! {
    #[test]
    fn arbitrary_rows_never_panic(rows in proptest::collection::vec(
        proptest::collection::vec("[-0-9a-zA-Z.,() $/:]{0,12}", 0..8), 0..12)) {
        for header in [
            "txnidx,date,account,amount,commodity,posting-comment",
            "Journal ID,Date,Account,Debit,Credit",
            "Date,Transaction ID,Description,Full Account Name,Value Num.,Void Reason",
            "Account,Date,Payee,Category Group/Category,Memo,Outflow,Inflow",
            "Date,Merchant,Category,Account,Original Statement,Amount",
            "date,name,amount,type,category,account,account mask,excluded",
        ] {
            let body: String = rows.iter().map(|row| row.join(",") + "\n").collect();
            let text = format!("{header}\n{body}");
            let _ = grouped_csv::parse(text.as_bytes());
            let _ = single_entry::parse(text.as_bytes());
        }
    }
}
