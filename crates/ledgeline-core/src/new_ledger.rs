//! The files a brand-new ledger starts with (plan 39 Phase 6, File → New).
//!
//! Pure: answers in, file contents out. Writing them is [`write_ledger`], which
//! goes through a [`Vault`] so a test can use [`crate::vault::MemVault`] and the
//! desktop a [`crate::vault::LocalVault`] rooted at the folder the user picked.
//!
//! # The layout
//!
//! Two files, and nothing else:
//!
//! - **`main.journal`** — the file Ledgeline opens. A title comment on the
//!   first line (what [`crate::title`] shows as the ledger's name), the fiscal
//!   year as a comment, the `decimal-mark`, `commodity` and `D` directives for
//!   the base currency, `include accounts.journal`, and then nothing: new
//!   transactions are appended below.
//! - **`accounts.journal`** — the chart of accounts: one `account` line per
//!   account, each with its `type:` and any report tags. The same file, with
//!   the same header, that Settings → Accounts creates for a journal that has
//!   none (`plans/18-account-list-editor.md`), and it is rendered by the same
//!   [`AccountDoc`] editor that tab saves through, so a name or tag the editor
//!   would refuse is refused here too.
//!
//! Keeping the declarations in their own file is what the accounts editor
//! already expects ("a file that holds no transactions"), and keeps the
//! transaction file free of a hundred lines of setup.
//!
//! # Fiscal year
//!
//! Neither hledger nor Ledgeline has a fiscal-year setting today: every report
//! period is a calendar one. So the answer is recorded as a comment in
//! `main.journal`, where a person (or a later version) can read it, and is
//! never written to `ledgeline.toml`, which Ledgeline never creates.
//!
//! # The charts
//!
//! The individual and business charts are a small common core of the GnuCash,
//! QuickBooks and Xero default charts and of the 108 public plain-text
//! accounting repositories surveyed for `plans/19-account-balances.md`, spelled
//! in the lowercase, colon-separated style those repositories overwhelmingly
//! use. Every root carries an explicit `type:` so nothing depends on English
//! name inference, and a few tags are seeded where a report reads them:
//! `bsgroup:` for the cash and investment lines, `bsterm: noncurrent` on
//! long-term debt and fixed assets, `issection:` and a handful of `kpi:` roles
//! on the business chart, and a `transfers` clearing account for transfer
//! matching (`docs/journal/transfer-matching.md`).

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use thiserror::Error;

use crate::accounts::{AccountDoc, AccountEdit, AccountError, AccountPlan};
use crate::parse::parse_journal_in;
use crate::vault::{Vault, VaultError, VaultPath};

/// The file a new ledger's journal lives in — what Ledgeline opens.
pub const MAIN_FILE: &str = "main.journal";

/// The file the chart of accounts lives in. Shared with Settings → Accounts'
/// "Create accounts.journal", so both paths produce the same file.
pub const ACCOUNTS_FILE: &str = "accounts.journal";

/// The header every `accounts.journal` Ledgeline creates starts with.
pub const ACCOUNTS_FILE_HEADER: &str = "\
; Chart of accounts.
;
; Each `account` line below declares one account and its tags (`type:`,
; `bsgroup:`, `holdings:`, `valuation:`, or any of your own). Ledgeline edits
; this file from the Settings \u{2192} Accounts tab.
";

/// The longest title accepted, in characters. A title is one comment line.
pub const MAX_TITLE_CHARS: usize = 120;

/// The most accounts one new chart may declare. The bundled charts hold about
/// fifty; this only stops a runaway client.
pub const MAX_ACCOUNTS: usize = 500;

/// The longest commodity symbol accepted, in characters.
const MAX_COMMODITY_CHARS: usize = 8;

/// Currency symbols accepted as a commodity alongside letters. hledger reads
/// any of these unquoted; the list is deliberately closed rather than "any
/// non-alphanumeric character", which would admit `;`, `@` and `"`.
const CURRENCY_SYMBOLS: &str = "$€£¥₹₩₽₪₫₱₦₴₸₺₼₾฿¢₡₲₵";

/// Which starting chart of accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerKind {
    /// Personal or household books.
    Individual,
    /// A small business's books.
    Business,
    /// No accounts at all; the user adds their own (or an import does).
    Empty,
}

/// Which side of the number the commodity symbol goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolSide {
    /// `$1,000.00`
    Left,
    /// `1.000,00 €`
    Right,
}

/// The decimal mark the ledger's amounts use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalMark {
    /// `1,000.00`
    Period,
    /// `1.000,00`
    Comma,
}

impl DecimalMark {
    fn mark(self) -> char {
        match self {
            Self::Period => '.',
            Self::Comma => ',',
        }
    }

    fn group(self) -> char {
        match self {
            Self::Period => ',',
            Self::Comma => '.',
        }
    }
}

/// The base currency and how its amounts are written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Currency {
    /// The commodity symbol: `$`, `€`, `USD`, `CHF`.
    pub commodity: String,
    /// Which side of the number it is written.
    pub side: SymbolSide,
    /// Whether a space separates it from the number.
    pub spaced: bool,
    /// Decimal places, 0 to 4.
    pub decimals: u8,
    /// The decimal mark.
    pub decimal_mark: DecimalMark,
}

impl Currency {
    /// US dollars, `$1,000.00` — the default the wizard opens on.
    #[must_use]
    pub fn usd() -> Self {
        Self {
            commodity: "$".to_string(),
            side: SymbolSide::Left,
            spaced: false,
            decimals: 2,
            decimal_mark: DecimalMark::Period,
        }
    }

    /// One thousand in this currency's style, e.g. `$1,000.00` or `1.000,00 €`
    /// — the sample amount a `commodity` / `D` directive is written with.
    ///
    /// A currency with no decimals still ends in its decimal mark (`¥1,000.`):
    /// hledger refuses a `commodity` directive without one, because `1,000`
    /// alone could be read either way.
    #[must_use]
    pub fn sample_amount(&self) -> String {
        let number = if self.decimals == 0 {
            format!(
                "1{}000{}",
                self.decimal_mark.group(),
                self.decimal_mark.mark()
            )
        } else {
            format!(
                "1{}000{}{}",
                self.decimal_mark.group(),
                self.decimal_mark.mark(),
                "0".repeat(usize::from(self.decimals))
            )
        };
        let gap = if self.spaced { " " } else { "" };
        match self.side {
            SymbolSide::Left => format!("{}{gap}{number}", self.commodity),
            SymbolSide::Right => format!("{number}{gap}{}", self.commodity),
        }
    }

    fn validate(&self) -> Result<(), NewLedgerError> {
        let count = self.commodity.chars().count();
        let well_formed = (1..=MAX_COMMODITY_CHARS).contains(&count)
            && self
                .commodity
                .chars()
                .all(|ch| ch.is_alphabetic() || CURRENCY_SYMBOLS.contains(ch));
        if !well_formed {
            return Err(NewLedgerError::Commodity(self.commodity.clone()));
        }
        if self.decimals > 4 {
            return Err(NewLedgerError::Decimals(self.decimals));
        }
        Ok(())
    }
}

/// A calendar month, 1 (January) to 12 (December).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Month(u8);

impl Month {
    /// January — a calendar fiscal year.
    pub const JANUARY: Self = Self(1);

    /// The month numbered `number`, 1 to 12.
    ///
    /// # Errors
    /// [`NewLedgerError::Month`] for anything outside 1–12.
    pub fn new(number: u8) -> Result<Self, NewLedgerError> {
        (1..=12)
            .contains(&number)
            .then_some(Self(number))
            .ok_or(NewLedgerError::Month(number))
    }

    /// The month's English name.
    #[must_use]
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        NAMES[usize::from(self.0 - 1)]
    }
}

impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One account a new chart declares: its full name and the tags on its
/// `account` line, in the order they are written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateAccount {
    /// The full account name, `assets:bank:checking`.
    pub name: String,
    /// `(tag, value)` pairs; a value-less tag such as `clearing:` has `""`.
    pub tags: Vec<(String, String)>,
}

/// Everything a new ledger is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLedger {
    /// The ledger's name, written as the first line of `main.journal`.
    pub title: String,
    /// The base currency.
    pub currency: Currency,
    /// The month the fiscal year starts in.
    pub fiscal_year_start: Month,
    /// The chart of accounts, in the order it is written.
    pub accounts: Vec<TemplateAccount>,
}

/// The two files a new ledger is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerFiles {
    /// `main.journal`'s contents.
    pub main: String,
    /// `accounts.journal`'s contents.
    pub accounts: String,
}

/// Why a new ledger could not be built or written.
#[derive(Debug, Error)]
pub enum NewLedgerError {
    /// The title is empty, too long, or spans lines.
    #[error(
        "the ledger's name must be one line of 1 to {MAX_TITLE_CHARS} characters, with no control \
         characters"
    )]
    Title,
    /// The commodity is not one Ledgeline will write unquoted.
    #[error(
        "\"{0}\" can't be used as the currency: use letters (such as USD) or one currency symbol \
         (such as $ or €), at most 8 characters"
    )]
    Commodity(String),
    /// More than four decimal places.
    #[error("a currency can have at most 4 decimal places, not {0}")]
    Decimals(u8),
    /// A month outside 1–12.
    #[error("{0} is not a month; use 1 (January) to 12 (December)")]
    Month(u8),
    /// The same account twice.
    #[error("{0} is in the list twice")]
    DuplicateAccount(String),
    /// Too many accounts.
    #[error("a new chart can hold at most {MAX_ACCOUNTS} accounts")]
    TooManyAccounts,
    /// An account name or tag the accounts editor refuses.
    #[error("{0}")]
    Account(#[from] AccountError),
    /// The rendered files do not parse — a bug, never the user's input.
    #[error("the new ledger would not be readable ({0}), so nothing was written")]
    Unreadable(String),
    /// The folder holds something other than dotfiles.
    #[error(
        "that folder isn't empty. Choose an empty folder, or make a new one, so nothing of yours \
         is mixed in with the new ledger"
    )]
    FolderNotEmpty,
    /// The folder could not be read or written.
    #[error("{0}")]
    Io(#[from] VaultError),
}

/// The chart of accounts a `kind` of ledger starts with.
#[must_use]
pub fn chart(kind: LedgerKind) -> Vec<TemplateAccount> {
    let rows: &[(&str, &[(&str, &str)])] = match kind {
        LedgerKind::Individual => INDIVIDUAL,
        LedgerKind::Business => BUSINESS,
        LedgerKind::Empty => &[],
    };
    rows.iter()
        .map(|(name, tags)| TemplateAccount {
            name: (*name).to_string(),
            tags: tags
                .iter()
                .map(|(tag, value)| ((*tag).to_string(), (*value).to_string()))
                .collect(),
        })
        .collect()
}

/// The value-less tag that marks a clearing account.
const CLEARING: (&str, &str) = ("clearing", "");
/// The balance-sheet line hledger's own cash accounts sit on.
const CASH_LINE: (&str, &str) = ("bsgroup", "Cash and cash equivalents");
const NONCURRENT: (&str, &str) = ("bsterm", "noncurrent");

/// Personal books: the accounts nearly every household chart in the survey
/// has, and few that only some do.
const INDIVIDUAL: &[(&str, &[(&str, &str)])] = &[
    ("assets", &[("type", "A")]),
    ("assets:bank", &[("type", "C"), CASH_LINE]),
    ("assets:bank:checking", &[]),
    ("assets:bank:savings", &[]),
    ("assets:cash", &[("type", "C"), CASH_LINE]),
    (
        "assets:investments",
        &[("type", "A"), ("bsgroup", "Investments")],
    ),
    ("assets:investments:brokerage", &[]),
    ("assets:investments:retirement", &[NONCURRENT]),
    ("assets:property", &[("type", "A"), NONCURRENT]),
    ("liabilities", &[("type", "L")]),
    ("liabilities:credit-card", &[]),
    ("liabilities:loans", &[]),
    ("liabilities:loans:mortgage", &[NONCURRENT]),
    ("liabilities:loans:student", &[NONCURRENT]),
    ("equity", &[("type", "E")]),
    ("equity:opening-balances", &[]),
    ("transfers", &[("type", "A"), CLEARING]),
    ("transfers:internal-bank", &[]),
    ("transfers:creditcard-payment", &[]),
    ("income", &[("type", "R")]),
    ("income:salary", &[]),
    ("income:interest", &[]),
    ("income:dividends", &[]),
    ("income:gifts", &[]),
    ("income:other", &[]),
    ("expenses", &[("type", "X")]),
    ("expenses:housing:rent", &[]),
    ("expenses:housing:utilities", &[]),
    ("expenses:housing:maintenance", &[]),
    ("expenses:food:groceries", &[]),
    ("expenses:food:dining", &[]),
    ("expenses:transport:fuel", &[]),
    ("expenses:transport:public", &[]),
    ("expenses:transport:car", &[]),
    ("expenses:health", &[]),
    ("expenses:insurance", &[]),
    ("expenses:clothing", &[]),
    ("expenses:household", &[]),
    ("expenses:entertainment", &[]),
    ("expenses:subscriptions", &[]),
    ("expenses:travel", &[]),
    ("expenses:education", &[]),
    ("expenses:gifts", &[]),
    ("expenses:charity", &[]),
    ("expenses:fees", &[]),
    ("expenses:interest", &[]),
    ("expenses:taxes", &[]),
    ("expenses:other", &[]),
];

/// A small business's books: the QuickBooks/Xero default chart, trimmed to
/// what a sole trader or small company uses from day one.
const BUSINESS: &[(&str, &[(&str, &str)])] = &[
    ("assets", &[("type", "A")]),
    ("assets:bank", &[("type", "C"), CASH_LINE]),
    ("assets:bank:checking", &[]),
    ("assets:bank:savings", &[]),
    ("assets:petty-cash", &[("type", "C"), CASH_LINE]),
    (
        "assets:receivable",
        &[("bsgroup", "Accounts receivable"), ("kpi", "receivable")],
    ),
    ("assets:inventory", &[("bsgroup", "Inventory")]),
    ("assets:prepaid-expenses", &[]),
    ("assets:fixed", &[NONCURRENT, ("bsgroup", "Fixed assets")]),
    ("assets:fixed:equipment", &[]),
    ("assets:fixed:accumulated-depreciation", &[]),
    ("liabilities", &[("type", "L")]),
    (
        "liabilities:payable",
        &[("bsgroup", "Accounts payable"), ("kpi", "payable")],
    ),
    ("liabilities:credit-card", &[]),
    ("liabilities:payroll", &[]),
    ("liabilities:sales-tax", &[]),
    (
        "liabilities:deferred-revenue",
        &[("kpi", "deferred-revenue")],
    ),
    (
        "liabilities:loans",
        &[NONCURRENT, ("bsgroup", "Long-term debt")],
    ),
    ("equity", &[("type", "E")]),
    ("equity:opening-balances", &[]),
    ("equity:owner-contributions", &[]),
    ("equity:owner-draws", &[]),
    ("equity:retained-earnings", &[]),
    ("transfers", &[("type", "A"), CLEARING]),
    ("transfers:internal-bank", &[]),
    ("transfers:creditcard-payment", &[]),
    ("revenue", &[("type", "R"), ("issection", "revenue")]),
    ("revenue:sales", &[]),
    ("revenue:services", &[]),
    ("revenue:other", &[("issection", "other")]),
    ("expenses", &[("type", "X"), ("issection", "opex")]),
    ("expenses:cost-of-goods-sold", &[("issection", "cogs")]),
    ("expenses:advertising", &[("kpi", "sales-marketing")]),
    ("expenses:bank-fees", &[]),
    ("expenses:contractors", &[]),
    ("expenses:insurance", &[]),
    ("expenses:meals", &[]),
    ("expenses:office", &[("kpi", "ga")]),
    ("expenses:payroll:wages", &[]),
    ("expenses:payroll:taxes", &[]),
    ("expenses:professional-fees", &[("kpi", "ga")]),
    ("expenses:rent", &[]),
    ("expenses:software", &[]),
    ("expenses:travel", &[]),
    ("expenses:utilities", &[]),
    ("expenses:depreciation", &[("issection", "depreciation")]),
    ("expenses:interest", &[("issection", "interest")]),
    ("expenses:taxes", &[("issection", "tax")]),
];

/// Render a new ledger's two files, and prove they parse together.
///
/// # Errors
/// [`NewLedgerError`] for a title, currency, month or account the wizard should
/// not have sent; [`NewLedgerError::Unreadable`] if the result fails to parse
/// (a bug, but checked rather than trusted).
pub fn render(ledger: &NewLedger) -> Result<LedgerFiles, NewLedgerError> {
    let title = clean_title(&ledger.title)?;
    ledger.currency.validate()?;
    let accounts = render_accounts(&ledger.accounts)?;
    let main = render_main(&title, &ledger.currency, ledger.fiscal_year_start);
    let files = LedgerFiles { main, accounts };
    prove_readable(&files)?;
    Ok(files)
}

/// The title as written, trimmed; refused when empty, long or multi-line.
fn clean_title(raw: &str) -> Result<String, NewLedgerError> {
    let title = raw.trim();
    let ok = !title.is_empty()
        && title.chars().count() <= MAX_TITLE_CHARS
        && !title.chars().any(char::is_control);
    ok.then(|| title.to_string()).ok_or(NewLedgerError::Title)
}

fn render_main(title: &str, currency: &Currency, fiscal_year_start: Month) -> String {
    let sample = currency.sample_amount();
    let mark = currency.decimal_mark.mark();
    format!(
        "; {title}\n\
         ;\n\
         ; Created by Ledgeline's New ledger wizard. The chart of accounts is in\n\
         ; {ACCOUNTS_FILE}; transactions go at the end of this file.\n\
         ;\n\
         ; Fiscal year starts in {fiscal_year_start}.\n\
         \n\
         decimal-mark {mark}\n\
         commodity {sample}\n\
         D {sample}\n\
         \n\
         include {ACCOUNTS_FILE}\n\
         \n\
         ; Transactions\n"
    )
}

/// `accounts.journal`: the shared header, then one declaration per account,
/// rendered — and round-trip verified — by the accounts editor itself.
fn render_accounts(accounts: &[TemplateAccount]) -> Result<String, NewLedgerError> {
    if accounts.len() > MAX_ACCOUNTS {
        return Err(NewLedgerError::TooManyAccounts);
    }
    let mut seen = BTreeSet::new();
    if let Some(duplicate) = accounts
        .iter()
        .find(|account| !seen.insert(account.name.as_str()))
    {
        return Err(NewLedgerError::DuplicateAccount(duplicate.name.clone()));
    }
    let doc = AccountDoc::parse(ACCOUNTS_FILE_HEADER);
    let plan = AccountPlan {
        edits: accounts
            .iter()
            .map(|account| AccountEdit::Declare {
                name: account.name.clone(),
                tags: account.tags.clone(),
                note: String::new(),
            })
            .collect(),
    };
    let text = doc.apply(&plan)?;
    doc.verify(&plan, &text)?;
    Ok(text)
}

/// Parse both files together with the native parser, exactly as opening the
/// new `main.journal` will.
fn prove_readable(files: &LedgerFiles) -> Result<(), NewLedgerError> {
    let (main, accounts) = file_paths()?;
    let vault = crate::vault::MemVault::new();
    let overrides = HashMap::from([
        (main.clone(), files.main.clone()),
        (accounts, files.accounts.clone()),
    ]);
    parse_journal_in(&vault, &main, &overrides)
        .map(|_| ())
        .map_err(|error| NewLedgerError::Unreadable(error.to_string()))
}

fn file_paths() -> Result<(VaultPath, VaultPath), NewLedgerError> {
    let root = VaultPath::root();
    let child = |name: &str| {
        root.child(name)
            .map_err(|error| NewLedgerError::Unreadable(error.to_string()))
    };
    Ok((child(MAIN_FILE)?, child(ACCOUNTS_FILE)?))
}

/// Whether the vault's root holds nothing but dotfiles (`.DS_Store`, `.git`).
///
/// # Errors
/// [`NewLedgerError::Io`] when the folder cannot be listed.
pub fn folder_is_empty(vault: &dyn Vault) -> Result<bool, NewLedgerError> {
    Ok(vault
        .list_dir(&VaultPath::root(), Some(4096))?
        .iter()
        .all(|entry| entry.name().starts_with('.')))
}

/// Write `files` into the vault's root, which must hold nothing but dotfiles.
///
/// `accounts.journal` is created first and `main.journal` second, both with an
/// exclusive create that refuses anything already there (a file, a directory,
/// a symlink) — so nothing is ever overwritten, and a failure between the two
/// leaves at most an unused accounts file, which is removed again. Returns the
/// vault path of `main.journal`.
///
/// # Errors
/// [`NewLedgerError::FolderNotEmpty`] when the folder holds anything but
/// dotfiles; [`NewLedgerError::Io`] when a file cannot be created.
pub fn write_ledger(vault: &dyn Vault, files: &LedgerFiles) -> Result<VaultPath, NewLedgerError> {
    if !folder_is_empty(vault)? {
        return Err(NewLedgerError::FolderNotEmpty);
    }
    let (main, accounts) = file_paths()?;
    vault
        .create_new_shared(&accounts, files.accounts.as_bytes())
        .map_err(already_there)?;
    if let Err(error) = vault.create_new_shared(&main, files.main.as_bytes()) {
        // Ours, created a moment ago; leave the folder as we found it.
        let _ = vault.remove_file(&accounts);
        return Err(already_there(error));
    }
    Ok(main)
}

/// An exclusive create that lost to something already at the name means the
/// folder was not empty after all.
fn already_there(error: VaultError) -> NewLedgerError {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        NewLedgerError::FolderNotEmpty
    } else {
        NewLedgerError::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reports::account_types::{
        AccountType, account_decls, declared_types, resolve_account_type,
    };
    use crate::vault::MemVault;

    fn ledger(kind: LedgerKind) -> NewLedger {
        NewLedger {
            title: "Test books".to_string(),
            currency: Currency::usd(),
            fiscal_year_start: Month::JANUARY,
            accounts: chart(kind),
        }
    }

    fn parsed(files: &LedgerFiles) -> crate::model::Journal {
        let vault = MemVault::new()
            .with_file(MAIN_FILE, &files.main)
            .with_file(ACCOUNTS_FILE, &files.accounts);
        let main = VaultPath::root().child(MAIN_FILE).unwrap();
        parse_journal_in(&vault, &main, &HashMap::new())
            .expect("the new ledger parses")
            .journal
    }

    #[test]
    fn every_kind_renders_and_parses() {
        for kind in [
            LedgerKind::Individual,
            LedgerKind::Business,
            LedgerKind::Empty,
        ] {
            let files = render(&ledger(kind)).expect("renders");
            let journal = parsed(&files);
            assert_eq!(journal.accounts.len(), chart(kind).len(), "{kind:?}");
            assert_eq!(journal.leading_comment.as_deref(), Some("Test books"));
            assert_eq!(
                journal.default_commodity.as_ref().map(|c| c.0.as_str()),
                Some("$")
            );
            assert!(journal.transactions.is_empty());
        }
    }

    #[test]
    fn every_account_in_a_chart_resolves_to_a_type_by_declaration() {
        for kind in [LedgerKind::Individual, LedgerKind::Business] {
            let journal = parsed(&render(&ledger(kind)).unwrap());
            let declared = declared_types(&account_decls(&journal));
            for account in chart(kind) {
                // Declared on the account or an ancestor, never inferred from
                // the name: strip the name's own inference by checking the root.
                let root = account.name.split(':').next().unwrap();
                assert!(
                    declared.contains_key(root),
                    "{kind:?}: root {root} has no declared type"
                );
                assert!(
                    resolve_account_type(&account.name, &declared).is_some(),
                    "{kind:?}: {} has no type",
                    account.name
                );
            }
        }
    }

    #[test]
    fn charts_type_the_accounts_reports_depend_on() {
        let journal = parsed(&render(&ledger(LedgerKind::Business)).unwrap());
        let declared = declared_types(&account_decls(&journal));
        let ty = |name: &str| resolve_account_type(name, &declared);
        assert_eq!(ty("assets:bank:checking"), Some(AccountType::Cash));
        assert_eq!(ty("assets:petty-cash"), Some(AccountType::Cash));
        assert_eq!(ty("assets:receivable"), Some(AccountType::Asset));
        assert_eq!(ty("liabilities:payable"), Some(AccountType::Liability));
        assert_eq!(ty("equity:opening-balances"), Some(AccountType::Equity));
        assert_eq!(ty("revenue:sales"), Some(AccountType::Revenue));
        assert_eq!(ty("expenses:rent"), Some(AccountType::Expense));
        assert_eq!(ty("transfers:internal-bank"), Some(AccountType::Asset));

        let personal = parsed(&render(&ledger(LedgerKind::Individual)).unwrap());
        let declared = declared_types(&account_decls(&personal));
        assert_eq!(
            resolve_account_type("assets:bank:savings", &declared),
            Some(AccountType::Cash)
        );
        assert_eq!(
            resolve_account_type("income:salary", &declared),
            Some(AccountType::Revenue)
        );
    }

    #[test]
    fn the_transfers_account_is_a_clearing_account() {
        let journal = parsed(&render(&ledger(LedgerKind::Individual)).unwrap());
        let clearing = crate::matching::clearing_accounts(&journal);
        assert!(clearing.contains("transfers"), "{clearing:?}");
    }

    #[test]
    fn the_main_file_carries_title_currency_fiscal_year_and_the_include() {
        let files = render(&NewLedger {
            title: "  Acme Books  ".to_string(),
            currency: Currency {
                commodity: "€".to_string(),
                side: SymbolSide::Right,
                spaced: true,
                decimals: 2,
                decimal_mark: DecimalMark::Comma,
            },
            fiscal_year_start: Month::new(7).unwrap(),
            accounts: Vec::new(),
        })
        .unwrap();
        assert!(files.main.starts_with("; Acme Books\n"), "{}", files.main);
        assert!(files.main.contains("Fiscal year starts in July."));
        assert!(files.main.contains("decimal-mark ,\n"));
        assert!(files.main.contains("commodity 1.000,00 €\n"));
        assert!(files.main.contains("D 1.000,00 €\n"));
        assert!(files.main.contains("include accounts.journal\n"));
        assert_eq!(files.accounts, ACCOUNTS_FILE_HEADER);
        assert_eq!(
            parsed(&files).default_commodity.map(|c| c.0),
            Some("€".to_string())
        );
    }

    #[test]
    fn sample_amounts_follow_the_style() {
        let mut currency = Currency::usd();
        assert_eq!(currency.sample_amount(), "$1,000.00");
        currency.decimals = 0;
        assert_eq!(currency.sample_amount(), "$1,000.");
        currency.commodity = "CHF".to_string();
        currency.side = SymbolSide::Right;
        currency.spaced = true;
        currency.decimals = 2;
        assert_eq!(currency.sample_amount(), "1,000.00 CHF");
    }

    #[test]
    fn declarations_use_the_accounts_editor_format() {
        let files = render(&ledger(LedgerKind::Individual)).unwrap();
        assert!(files.accounts.starts_with(ACCOUNTS_FILE_HEADER));
        assert!(
            files
                .accounts
                .contains("account assets:bank  ; type: C, bsgroup: Cash and cash equivalents\n")
        );
        assert!(
            files
                .accounts
                .contains("account transfers  ; type: A, clearing:\n")
        );
        assert!(files.accounts.contains("account assets:bank:checking\n"));
    }

    #[test]
    fn bad_input_is_refused() {
        let mut bad = ledger(LedgerKind::Empty);
        bad.title = "two\nlines".to_string();
        assert!(matches!(render(&bad), Err(NewLedgerError::Title)));
        bad.title = "   ".to_string();
        assert!(matches!(render(&bad), Err(NewLedgerError::Title)));

        let mut bad = ledger(LedgerKind::Empty);
        for commodity in ["", "1USD", "U S", "$;", "\"$\"", "TOOLONGCODE", "-"] {
            bad.currency.commodity = commodity.to_string();
            assert!(
                matches!(render(&bad), Err(NewLedgerError::Commodity(_))),
                "{commodity:?}"
            );
        }
        let mut bad = ledger(LedgerKind::Empty);
        bad.currency.decimals = 5;
        assert!(matches!(render(&bad), Err(NewLedgerError::Decimals(5))));

        assert!(matches!(Month::new(0), Err(NewLedgerError::Month(0))));
        assert!(matches!(Month::new(13), Err(NewLedgerError::Month(13))));

        let mut bad = ledger(LedgerKind::Individual);
        bad.accounts.push(bad.accounts[0].clone());
        assert!(matches!(
            render(&bad),
            Err(NewLedgerError::DuplicateAccount(name)) if name == "assets"
        ));

        let mut bad = ledger(LedgerKind::Empty);
        bad.accounts.push(TemplateAccount {
            name: "assets;bank".to_string(),
            tags: Vec::new(),
        });
        assert!(matches!(render(&bad), Err(NewLedgerError::Account(_))));

        let mut bad = ledger(LedgerKind::Empty);
        bad.accounts.push(TemplateAccount {
            name: "assets".to_string(),
            tags: vec![("bsgroup".to_string(), "a, b".to_string())],
        });
        assert!(matches!(render(&bad), Err(NewLedgerError::Account(_))));
    }

    #[test]
    fn writing_needs_an_empty_folder_and_never_overwrites() {
        let files = render(&ledger(LedgerKind::Individual)).unwrap();

        let empty = MemVault::new().with_file(".DS_Store", "x");
        let main = write_ledger(&empty, &files).expect("an all-dotfile folder is empty");
        assert_eq!(main.as_str(), MAIN_FILE);
        assert_eq!(empty.read(&main).unwrap(), files.main.as_bytes());

        // A second write into the same folder is refused, and changes nothing.
        assert!(matches!(
            write_ledger(&empty, &files),
            Err(NewLedgerError::FolderNotEmpty)
        ));
        assert_eq!(empty.read(&main).unwrap(), files.main.as_bytes());

        let busy = MemVault::new().with_file("notes.txt", "mine");
        assert!(matches!(
            write_ledger(&busy, &files),
            Err(NewLedgerError::FolderNotEmpty)
        ));
        let names: Vec<String> = busy
            .list_dir(&VaultPath::root(), None)
            .unwrap()
            .iter()
            .map(|entry| entry.name().to_string())
            .collect();
        assert_eq!(names, vec!["notes.txt".to_string()]);

        let with_dir = MemVault::new().with_dir("old");
        assert!(matches!(
            write_ledger(&with_dir, &files),
            Err(NewLedgerError::FolderNotEmpty)
        ));
    }
}
