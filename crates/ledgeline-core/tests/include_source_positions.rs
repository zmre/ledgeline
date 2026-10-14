//! Source positions inside `include`d files name the INCLUDED file.
//!
//! hledger reports the file a transaction, balance assertion or `account`
//! directive was actually read from, so `tsourcepos`, `baposition` and
//! `adisourcepos` from an included file carry that file's path, while anything
//! in the main file keeps the main journal's name exactly as it was opened.
//!
//! Every expectation below was read off real hledger 1.52 output over these same
//! committed fixtures — `hledger print -O json` for the transactions and
//! assertions, `hledger-web --serve-api`'s `/accounts` for the declarations —
//! not derived from our own parser.

use ledgeline_core::parse_journal;
use ledgeline_core::wire::{journal_to_accounts_value, journal_to_value};
use serde_json::Value;
use std::path::PathBuf;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// `(sourceLine, sourceColumn, sourceName)` of one wire position.
type Pos = (u64, u64, String);

/// A layout fixture's directory, resolved (hledger resolves included paths, so
/// comparing against resolved paths is comparing like with like).
fn layout(name: &str) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/import/layouts")
            .join(name),
    )
}

/// Parse `dir/main.journal` under its absolute path.
fn open(
    dir: &std::path::Path,
) -> Result<(String, ledgeline_core::model::Journal), Box<dyn std::error::Error>> {
    let main = dir.join("main.journal");
    let name = main.to_string_lossy().into_owned();
    let journal = parse_journal(&std::fs::read_to_string(&main)?, &name)?;
    Ok((name, journal))
}

/// One wire position as a [`Pos`].
fn pos(value: &Value) -> Pos {
    (
        value["sourceLine"].as_u64().unwrap_or_default(),
        value["sourceColumn"].as_u64().unwrap_or_default(),
        value["sourceName"].as_str().unwrap_or_default().to_string(),
    )
}

/// `aname` → `adisourcepos` for every declared node of `/accounts`.
fn declarations(accounts: &Value) -> Vec<(String, Pos)> {
    accounts
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| !node["adeclarationinfo"].is_null())
        .map(|node| {
            (
                node["aname"].as_str().unwrap_or_default().to_string(),
                pos(&node["adeclarationinfo"]["adisourcepos"]),
            )
        })
        .collect()
}

/// `split-year/` declares every account in `accounts.journal`, an include.
#[test]
fn an_included_account_declaration_names_its_own_file() -> TestResult {
    let dir = layout("split-year")?;
    let (_, journal) = open(&dir)?;
    let accounts_file = dir.join("accounts.journal").to_string_lossy().into_owned();

    let actual = declarations(&journal_to_accounts_value(&journal)?);
    let expected: Vec<(String, Pos)> = [
        ("assets:bank:checking", 4),
        ("equity:opening balances", 6),
        ("expenses:groceries", 7),
        ("income:salary", 8),
    ]
    .into_iter()
    .map(|(name, line)| (name.to_string(), (line, 1, accounts_file.clone())))
    .collect();
    assert_eq!(actual, expected);
    Ok(())
}

/// `split-year-assert/` declares its accounts in the main file — those keep the
/// main journal's name — and keeps its transactions, one with a balance
/// assertion, in two included year files.
#[test]
fn included_transactions_and_assertions_name_their_own_file() -> TestResult {
    let dir = layout("split-year-assert")?;
    let (main_name, journal) = open(&dir)?;
    let y2025 = dir.join("2025/2025.journal").to_string_lossy().into_owned();
    let y2026 = dir.join("2026/2026.journal").to_string_lossy().into_owned();

    let declared = declarations(&journal_to_accounts_value(&journal)?);
    assert!(!declared.is_empty());
    assert!(
        declared
            .iter()
            .all(|(_, (_, _, source))| *source == main_name),
        "main-file declarations keep the main journal's name: {declared:?}"
    );
    let checking = declared
        .iter()
        .find(|(name, _)| name == "assets:bank:checking")
        .map(|(_, position)| position.0);
    assert_eq!(checking, Some(23));

    let transactions = journal_to_value(&journal)?;
    let spans: Vec<(String, Pos, Pos)> = transactions
        .as_array()
        .into_iter()
        .flatten()
        .map(|txn| {
            (
                txn["tdescription"].as_str().unwrap_or_default().to_string(),
                pos(&txn["tsourcepos"][0]),
                pos(&txn["tsourcepos"][1]),
            )
        })
        .collect();
    let expected: Vec<(String, Pos, Pos)> = [
        ("Opening balance", 4, 7, &y2025),
        ("Grocery store", 8, 11, &y2025),
        ("start of year", 12, 14, &y2026),
        ("Coffee", 15, 18, &y2026),
    ]
    .into_iter()
    .map(|(description, first, last, file)| {
        (
            description.to_string(),
            (first, 1, file.clone()),
            (last, 1, file.clone()),
        )
    })
    .collect();
    assert_eq!(spans, expected);

    let assertions: Vec<Pos> = transactions
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|txn| txn["tpostings"].as_array().into_iter().flatten())
        .filter(|posting| !posting["pbalanceassertion"].is_null())
        .map(|posting| pos(&posting["pbalanceassertion"]["baposition"]))
        .collect();
    assert_eq!(assertions, vec![(13, 38, y2026)]);
    Ok(())
}
