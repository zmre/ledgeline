//! The one thing `new_ledger`'s unit tests cannot prove hermetically: that the
//! files the File → New wizard writes are a journal the REAL `hledger` accepts,
//! and that hledger gives every account the same type Ledgeline does.
//!
//! Gated behind `LEDGELINE_HLEDGER_NEW_LEDGER_CHECK=1`, like the other
//! `LEDGELINE_HLEDGER_*_CHECK` suites, so `cargo test` stays hermetic. Run by
//! `just hledger-checks`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use ledgeline_core::new_ledger::{
    ACCOUNTS_FILE, Currency, DecimalMark, LedgerKind, MAIN_FILE, Month, NewLedger, SymbolSide,
    chart, render, write_ledger,
};
use ledgeline_core::parse::parse_journal_in;
use ledgeline_core::reports::account_types::{
    AccountType, account_decls, declared_types, resolve_account_type,
};
use ledgeline_core::vault::{LocalVault, VaultPath};

const OPT_IN: &str = "LEDGELINE_HLEDGER_NEW_LEDGER_CHECK";

/// A scratch folder that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ledgeline-new-ledger-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hledger(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("hledger")
        .current_dir(dir)
        .arg("-f")
        .arg(MAIN_FILE)
        .args(args)
        .env("LEDGER_FILE", "/dev/null")
        .arg("--no-conf")
        .output()
        .expect("hledger runs (set it up on PATH, e.g. `nix develop`)");
    assert!(
        output.status.success(),
        "hledger {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// hledger's letter for one of our types.
fn letter(ty: AccountType) -> &'static str {
    match ty {
        AccountType::Asset => "A",
        AccountType::Liability => "L",
        AccountType::Equity => "E",
        AccountType::Revenue => "R",
        AccountType::Expense => "X",
        AccountType::Cash => "C",
        AccountType::Conversion => "V",
        AccountType::Gain => "G",
    }
}

#[test]
fn every_template_passes_hledger_check_and_hledger_agrees_on_types() {
    if std::env::var_os(OPT_IN).is_none() {
        eprintln!("skipped; set {OPT_IN}=1 to run it");
        return;
    }
    let currencies = [
        Currency::usd(),
        Currency {
            commodity: "€".to_string(),
            side: SymbolSide::Right,
            spaced: true,
            decimals: 2,
            decimal_mark: DecimalMark::Comma,
        },
        Currency {
            commodity: "JPY".to_string(),
            side: SymbolSide::Right,
            spaced: true,
            decimals: 0,
            decimal_mark: DecimalMark::Period,
        },
    ];
    for (index, kind) in [
        LedgerKind::Individual,
        LedgerKind::Business,
        LedgerKind::Empty,
    ]
    .into_iter()
    .enumerate()
    {
        for (cindex, currency) in currencies.iter().enumerate() {
            let scratch = Scratch::new(&format!("{index}-{cindex}"));
            let files = render(&NewLedger {
                title: format!("{kind:?} books"),
                currency: currency.clone(),
                fiscal_year_start: Month::new(4).unwrap(),
                accounts: chart(kind),
            })
            .expect("renders");
            let vault = LocalVault::new(&scratch.0);
            write_ledger(&vault, &files).expect("writes");
            assert!(scratch.0.join(ACCOUNTS_FILE).is_file());

            hledger(&scratch.0, &["check"]);
            // A transaction in the base currency, written the way the wizard
            // styled it, must parse to the number we meant.
            let amount = currency.sample_amount();
            std::fs::write(
                scratch.0.join(MAIN_FILE),
                format!(
                    "{}2026-01-01 opening\n    assets:x  {amount}\n    equity:y\n",
                    files.main
                ),
            )
            .unwrap();
            let printed = hledger(&scratch.0, &["bal", "assets:x", "-N", "-O", "csv"]);
            let digits: String = printed.chars().filter(char::is_ascii_digit).collect();
            assert!(
                digits.starts_with("1000"),
                "{kind:?}/{amount}: hledger read {printed}"
            );
            std::fs::write(scratch.0.join(MAIN_FILE), &files.main).unwrap();

            // hledger's own type for every declared account.
            let listed = hledger(&scratch.0, &["accounts", "--types"]);
            let theirs: BTreeMap<String, String> = listed
                .lines()
                .filter_map(|line| {
                    let (name, rest) = line.split_once(';')?;
                    let ty = rest.trim().strip_prefix("type:")?.trim();
                    Some((name.trim().to_string(), ty.to_string()))
                })
                .collect();

            let main = VaultPath::root().child(MAIN_FILE).unwrap();
            let journal = parse_journal_in(&vault, &main, &HashMap::new())
                .expect("parses natively")
                .journal;
            let declared = declared_types(&account_decls(&journal));
            for account in chart(kind) {
                let ours = resolve_account_type(&account.name, &declared)
                    .map(letter)
                    .unwrap_or("-");
                assert_eq!(
                    theirs.get(&account.name).map(String::as_str),
                    Some(ours),
                    "{kind:?}: {} — hledger and Ledgeline disagree",
                    account.name
                );
            }
        }
    }
}
