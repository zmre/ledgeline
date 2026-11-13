//! A parsed QIF file ([`crate::qif`]) turned into the multi-posting
//! [`QbJournal`] shape the QuickBooks import pipeline already writes (plan 39
//! Phase 2). Everything downstream — alias mapping, `id:` dedupe,
//! `--update-changed`, review, the date-ambiguity warning — is that pipeline's,
//! unchanged.
//!
//! # Source names: what the mapping screen shows
//!
//! Every posting carries a *source* account name, which the user maps to a
//! journal account with a plain `alias` line:
//!
//! - **A register's own account**: the QIF account name, as written
//!   (`Checking`, `Visa`). A single-account file with no `!Account` record
//!   uses the account its `Opening Balance` record names (`L[Checking]`), or
//!   else the file name without its extension.
//! - **A transfer** `L[Savings]`: the target account's name, `Savings`, so one
//!   alias maps both the register and every transfer into it.
//! - **A category** `LFood:Groceries`: the category path as written. A
//!   mapping on `Food` covers `Food:Groceries` by hledger's own alias prefix
//!   rule. A category spelled exactly like one of the file's accounts is
//!   written `Name (category)` so the two can be mapped apart.
//! - **No category**: `Uncategorized`.
//! - **An opening balance** (a record whose transfer names its own account,
//!   which is how Quicken and Banktivity write `Opening Balance`):
//!   [`OPENING_BALANCES`].
//! - **Investments**: see [`investment`]. Holdings are
//!   `Account:Security`; income is `Investment Income:…`.
//!
//! A class (`LFood/Household`) becomes the posting's `class:` tag.
//!
//! # Status
//!
//! `C*` and `Cc` (cleared) and `CX`/`CR` (reconciled) are all written as
//! hledger's cleared `*`: hledger has no separate reconciled state. `C!` is
//! pending `!`. Anything else is unmarked.
//!
//! # Mirrored transfers
//!
//! A transfer between two accounts that are both in the file appears twice,
//! once in each register (Banktivity always writes both). One copy is dropped:
//! a plain transfer record in register `B` naming `[A]` is dropped when another
//! record on the same date, in register `A`, posts the same amount to `B` —
//! the other side of a plain transfer (the earlier of the two is kept) or a
//! split line `S[B]` (the split record is kept, since it says more). Each
//! posting is matched once, so two identical transfers on one day stay two.
//!
//! # Ids
//!
//! QIF has no transaction ids, so each one is made from what identifies it:
//! `qif-` and 16 hex digits of an FNV-1a hash over the date, the register
//! account, the amount, the payee, and the record's occurrence index among
//! identical keys in its register. A plain transfer hashes the two account
//! names in sorted order with the amount signed for the first and no payee,
//! so its two mirrored copies get the same id — which lets a re-import of
//! either side, even from separate files, dedupe against the other.
//!
//! Editing a transaction's date, amount or payee in the source app gives it a
//! new id, so a re-import writes it again rather than updating it.

use crate::decimal::Dec;
use crate::edit::render_dec;
use crate::qb_journal::{
    ImportSource, ImportStatus, QbJournal, QbPosting, QbTransaction, SourceAccountKind, Units,
};
use crate::qif::{self, QifError, QifFile, QifRecord, RegisterKind};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The source name an opening-balance record's other side is written to.
pub const OPENING_BALANCES: &str = "Equity:Opening Balances";

/// The source name a share trade's commission is written to.
pub const COMMISSIONS: &str = "Investment Expenses:Commissions";

/// The source name for a record or split with no category.
pub const UNCATEGORIZED: &str = "Uncategorized";

/// The source name a split's unexplained remainder is written to.
const SPLIT_REMAINDER: &str = UNCATEGORIZED;

/// The register account name for a single-account file, from its file name.
#[must_use]
pub fn fallback_account(file_name: &str) -> String {
    let stem = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_name)
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .trim();
    if stem.is_empty() {
        "QIF account".to_string()
    } else {
        stem.to_string()
    }
}

/// Parse `bytes` and import them. `name` is the upload's file name; it only
/// names the register of a single-account file.
///
/// # Errors
///
/// Any [`QifError`].
pub fn parse_and_import(name: &str, bytes: &[u8]) -> Result<QbJournal, QifError> {
    import(&qif::parse(bytes)?, &fallback_account(name))
}

/// Import a parsed file. `fallback` names the register account of a file
/// that has no `!Account` record.
///
/// # Errors
///
/// [`QifError::NoTransactions`], a date refusal from
/// [`qif::resolve_dates`], [`QifError::MissingDate`], or
/// [`QifError::AmountOverflow`].
pub fn import(file: &QifFile, fallback: &str) -> Result<QbJournal, QifError> {
    let records: Vec<(String, RegisterKind, &QifRecord)> = file
        .registers
        .iter()
        .flat_map(|register| {
            let account = register.account.as_ref().map_or_else(
                || opening_balance_account(register).unwrap_or_else(|| fallback.to_string()),
                |account| account.name.clone(),
            );
            register
                .records
                .iter()
                .map(move |record| (account.clone(), register.kind, record))
        })
        .collect();
    if records.is_empty() {
        return Err(QifError::NoTransactions);
    }

    let raw_dates: Vec<(&str, usize)> = records
        .iter()
        .map(|(_, _, record)| {
            record
                .date
                .as_deref()
                .map(|date| (date, record.line))
                .ok_or(QifError::MissingDate { line: record.line })
        })
        .collect::<Result<_, _>>()?;
    let dates = qif::resolve_dates(&raw_dates)?;

    let names = Names::new(file, records.iter().map(|(account, ..)| account.as_str()));
    let built: Vec<Built> = records
        .iter()
        .zip(dates.iso)
        .map(|((account, kind, record), date)| build(&names, account, *kind, record, date))
        .collect::<Result<_, _>>()?;

    let ids = ids(&built);
    let dropped = mirrored(&built);
    let transactions = built
        .into_iter()
        .zip(ids)
        .enumerate()
        .filter(|(at, _)| !dropped.contains(at))
        .map(|(_, (built, id))| built.into_transaction(id))
        .collect::<Vec<_>>();

    let transactions = unique_ids(transactions);
    let account_kinds = account_kinds(file, &names, &records, &transactions);
    let mut parsed = QbJournal {
        transactions,
        date_format: dates.guess,
        source: ImportSource::Qif,
        account_kinds,
    };
    crate::qb_import::classify_remaining_by_sign(&mut parsed);
    Ok(parsed)
}

/// What the file says about each source account's kind, for the mapping
/// step's suggestions: register types, `!Account` types, transfer targets
/// (accounts of unstated kind), income categories, investment holdings and
/// the names this module invents. Categories the file leaves unflagged are
/// classified afterwards by the sign of their postings.
fn account_kinds(
    file: &QifFile,
    names: &Names,
    records: &[(String, RegisterKind, &QifRecord)],
    transactions: &[QbTransaction],
) -> BTreeMap<String, SourceAccountKind> {
    let transfers = records.iter().flat_map(|(register, _, record)| {
        std::iter::once(record.category.as_deref())
            .chain(record.splits.iter().map(|split| split.category.as_deref()))
            .filter_map(move |raw| match names.target(raw, register).0 {
                Target::Transfer(account) => Some(account),
                _ => None,
            })
    });
    let mut kinds: BTreeMap<String, SourceAccountKind> = transfers
        .map(|account| (account, SourceAccountKind::Tracked))
        .collect();
    // `Account:Security`: a holding inside an investment register.
    let investment_registers: Vec<String> = records
        .iter()
        .filter(|(_, kind, _)| *kind == RegisterKind::Investment)
        .map(|(register, ..)| format!("{register}:"))
        .collect();
    for posting in transactions.iter().flat_map(|txn| &txn.postings) {
        if investment_registers
            .iter()
            .any(|prefix| posting.account.starts_with(prefix.as_str()))
        {
            kinds.insert(posting.account.clone(), SourceAccountKind::Investment);
        }
    }
    for account in &file.accounts {
        if let Some(kind) = account.kind.as_deref().and_then(kind_of_type_line) {
            kinds.insert(account.name.clone(), kind);
        }
    }
    for (register, kind, _) in records {
        kinds.insert(register.clone(), register_kind(*kind));
    }
    for category in file.categories.iter().filter(|category| category.income) {
        let source = names.source(&Target::Category(category.name.clone()));
        kinds.insert(source, SourceAccountKind::Income);
    }
    kinds.insert(OPENING_BALANCES.to_string(), SourceAccountKind::Equity);
    kinds
}

fn register_kind(kind: RegisterKind) -> SourceAccountKind {
    match kind {
        RegisterKind::Bank => SourceAccountKind::Bank,
        RegisterKind::Cash => SourceAccountKind::Cash,
        RegisterKind::CreditCard => SourceAccountKind::CreditCard,
        RegisterKind::OtherAsset => SourceAccountKind::OtherAsset,
        RegisterKind::OtherLiability => SourceAccountKind::OtherLiability,
        RegisterKind::Investment => SourceAccountKind::Investment,
    }
}

/// An `!Account` record's `T` line (`Bank`, `CCard`, `Oth A`, `Invst` …).
fn kind_of_type_line(text: &str) -> Option<SourceAccountKind> {
    let key: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    Some(match key.as_str() {
        "bank" => SourceAccountKind::Bank,
        "cash" => SourceAccountKind::Cash,
        "ccard" => SourceAccountKind::CreditCard,
        "otha" => SourceAccountKind::OtherAsset,
        "othl" => SourceAccountKind::OtherLiability,
        "invst" | "port" | "401(k)" | "401k" | "mutual" => SourceAccountKind::Investment,
        _ => return None,
    })
}

/// The account a register with no `!Account` record belongs to, read off its
/// opening balance: Quicken starts a single-account export with
/// `POpening Balance` and `L[Checking]`, naming the account it is in.
fn opening_balance_account(register: &qif::QifRegister) -> Option<String> {
    let first = register.records.first()?;
    let payee = first.payee.as_deref()?.trim();
    if !payee.eq_ignore_ascii_case("opening balance") {
        return None;
    }
    let name = first
        .category
        .as_deref()?
        .split('/')
        .next()?
        .trim()
        .strip_prefix('[')?
        .strip_suffix(']')?
        .trim();
    non_empty(name)
}

// ---------------------------------------------------------------------------
// Source names
// ---------------------------------------------------------------------------

/// Every account name in the file, so a category spelled the same can be told
/// apart from it, and every security's ticker by the name `Y` uses.
struct Names {
    accounts: HashSet<String>,
    symbols: HashMap<String, String>,
}

/// Where a `L`/`S` value points.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Category(String),
    Transfer(String),
    Opening,
    Uncategorized,
}

impl Names {
    fn new<'a>(file: &QifFile, registers: impl Iterator<Item = &'a str>) -> Self {
        Self {
            accounts: file
                .accounts
                .iter()
                .map(|account| account.name.clone())
                .chain(registers.map(str::to_string))
                .collect(),
            symbols: file
                .securities
                .iter()
                .filter_map(|security| {
                    let symbol = security.symbol.as_deref()?.trim();
                    (!symbol.is_empty()).then(|| (security.name.clone(), symbol.to_string()))
                })
                .collect(),
        }
    }

    /// The hledger commodity a security is written as: its ticker from the
    /// file's `!Type:Security` list, else the name `Y` gives it (written
    /// quoted when it holds spaces or digits). A `"` can't be quoted, so it
    /// is dropped.
    fn commodity(&self, security: &str) -> String {
        let security = security.trim();
        self.symbols
            .get(security)
            .map_or(security, String::as_str)
            .replace('"', "")
    }

    /// `raw` (`Food:Groceries/Class`, `[Savings]`, empty) from the point of
    /// view of `register`, plus its class.
    fn target(&self, raw: Option<&str>, register: &str) -> (Target, Option<String>) {
        let raw = raw.unwrap_or("").trim();
        let (category, class) = match raw.split_once('/') {
            Some((category, class)) => (category.trim(), non_empty(class.trim())),
            None => (raw, None),
        };
        let target = if let Some(account) = category
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .map(str::trim)
        {
            if account.is_empty() {
                Target::Uncategorized
            } else if account == register {
                Target::Opening
            } else {
                Target::Transfer(account.to_string())
            }
        } else if category.is_empty() {
            Target::Uncategorized
        } else {
            Target::Category(category.to_string())
        };
        (target, class)
    }

    /// The source name a target is written as.
    fn source(&self, target: &Target) -> String {
        match target {
            Target::Category(name) if self.accounts.contains(name) => format!("{name} (category)"),
            Target::Category(name) | Target::Transfer(name) => name.clone(),
            Target::Opening => OPENING_BALANCES.to_string(),
            Target::Uncategorized => UNCATEGORIZED.to_string(),
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

// ---------------------------------------------------------------------------
// One record → one transaction
// ---------------------------------------------------------------------------

/// One record, built but not yet given an id.
#[derive(Debug, Clone)]
struct Built {
    register: String,
    date: String,
    payee: Option<String>,
    number: Option<String>,
    kind: String,
    status: ImportStatus,
    postings: Vec<QbPosting>,
    /// The other account of a plain two-account transfer.
    transfer_to: Option<String>,
}

impl Built {
    fn into_transaction(self, id: String) -> QbTransaction {
        QbTransaction {
            id,
            date: self.date,
            transaction_type: self.kind,
            num: self.number,
            name: self.payee,
            postings: self.postings,
            status: self.status,
        }
    }

    /// The register posting's amount (always the first posting).
    fn amount(&self) -> Dec {
        self.postings.first().map_or_else(Dec::zero, |p| p.amount)
    }
}

fn posting(account: String, amount: Dec, memo: Option<String>, class: Option<String>) -> QbPosting {
    QbPosting {
        account,
        amount,
        memo,
        class,
        customer: None,
        vendor: None,
        name: None,
        units: None,
    }
}

fn status(mark: Option<char>) -> ImportStatus {
    match mark {
        Some('*' | 'c' | 'C' | 'x' | 'X' | 'r' | 'R') => ImportStatus::Cleared,
        Some('!') => ImportStatus::Pending,
        _ => ImportStatus::Unmarked,
    }
}

fn negate(amount: Dec, line: usize) -> Result<Dec, QifError> {
    amount.neg().map_err(|_| QifError::AmountOverflow { line })
}

fn build(
    names: &Names,
    register: &str,
    kind: RegisterKind,
    record: &QifRecord,
    date: String,
) -> Result<Built, QifError> {
    let base = Built {
        register: register.to_string(),
        date,
        payee: record.payee.clone(),
        number: record.number.clone(),
        kind: String::new(),
        status: status(record.status),
        postings: Vec::new(),
        transfer_to: None,
    };
    if kind == RegisterKind::Investment {
        return investment(names, register, record, base);
    }
    let line = record.line;

    if record.splits.is_empty() {
        let total = record.amount.unwrap_or_else(Dec::zero);
        let (target, class) = names.target(record.category.as_deref(), register);
        let transfer_to = match &target {
            Target::Transfer(account) => Some(account.clone()),
            _ => None,
        };
        return Ok(Built {
            postings: vec![
                posting(register.to_string(), total, record.memo.clone(), None),
                posting(names.source(&target), negate(total, line)?, None, class),
            ],
            transfer_to,
            ..base
        });
    }

    let split_sum = record
        .splits
        .iter()
        .filter_map(|split| split.amount)
        .try_fold(Dec::zero(), Dec::add)
        .map_err(|_| QifError::AmountOverflow { line })?;
    let total = record.amount.unwrap_or(split_sum);
    let mut postings = vec![posting(
        register.to_string(),
        total,
        record.memo.clone(),
        None,
    )];
    for split in &record.splits {
        let (target, class) = names.target(split.category.as_deref(), register);
        postings.push(posting(
            names.source(&target),
            negate(split.amount.unwrap_or_else(Dec::zero), line)?,
            split.memo.clone(),
            class,
        ));
    }
    let remainder = total
        .sub(split_sum)
        .map_err(|_| QifError::AmountOverflow { line })?;
    if !remainder.is_zero() {
        postings.push(posting(
            SPLIT_REMAINDER.to_string(),
            negate(remainder, line)?,
            Some("split remainder: the QIF splits did not add up to the total".to_string()),
            None,
        ));
    }
    Ok(Built { postings, ..base })
}

// ---------------------------------------------------------------------------
// Investments
// ---------------------------------------------------------------------------

/// What an investment action does to the account's cash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Buy,
    Sell,
    Income(&'static str),
    Reinvest(&'static str),
    ReturnOfCapital,
    SharesIn,
    SharesOut,
    TransferIn,
    TransferOut,
    Cash,
    Expense,
    /// Moves no money (`StkSplit`, `Reminder` …), or one this importer does
    /// not know.
    NoCash,
}

fn action_of(raw: &str) -> (Action, bool) {
    let lower = raw.trim().to_ascii_lowercase();
    let known = |name: &str| -> Option<Action> {
        Some(match name {
            "buy" | "cvrshrt" => Action::Buy,
            "sell" | "shtsell" => Action::Sell,
            "div" => Action::Income("Dividends"),
            "intinc" => Action::Income("Interest"),
            "cglong" | "cgmid" | "cgshort" => Action::Income("Capital Gains"),
            "miscinc" => Action::Income("Miscellaneous"),
            "reinvdiv" => Action::Reinvest("Dividends"),
            "reinvint" => Action::Reinvest("Interest"),
            "reinvlg" | "reinvmd" | "reinvsh" => Action::Reinvest("Capital Gains"),
            "rtrncap" => Action::ReturnOfCapital,
            "shrsin" => Action::SharesIn,
            "shrsout" => Action::SharesOut,
            "xin" => Action::TransferIn,
            "xout" => Action::TransferOut,
            "cash" => Action::Cash,
            "margint" | "miscexp" => Action::Expense,
            _ => return None,
        })
    };
    if let Some(action) = known(&lower) {
        return (action, false);
    }
    // A trailing `X` means the cash moves through the `L` account.
    match lower.strip_suffix('x').and_then(known) {
        Some(action) => (action, true),
        None => (Action::NoCash, false),
    }
}

/// One investment record as cash movements.
///
/// QbPosting carries a bare amount and no commodity, so shares cannot be
/// written. Each action becomes the money it moves, valued at the record's
/// total: a purchase moves cash into `Account:Security`, a sale moves it back
/// out (at the sale proceeds, with no cost basis or gain), income comes from
/// `Investment Income:…`, and `ShrsIn`/`ShrsOut` move value against
/// [`OPENING_BALANCES`]. The shares, price and commission are kept in the
/// first posting's memo. An action that moves no money, or that this importer
/// does not know, is written as a zero-amount transaction with the action in
/// its memo, so nothing in the file disappears silently.
fn investment(
    names: &Names,
    register: &str,
    record: &QifRecord,
    base: Built,
) -> Result<Built, QifError> {
    let line = record.line;
    let raw_action = record.action.clone().unwrap_or_default();
    let (action, via_link) = action_of(&raw_action);
    let total = match record.amount {
        Some(total) => Some(total),
        None => match (record.quantity, record.price) {
            (Some(quantity), Some(price)) => Some(
                quantity
                    .mul(price)
                    .map_err(|_| QifError::AmountOverflow { line })?,
            ),
            _ => record.transfer_amount,
        },
    };
    // `Cash` keeps its sign; every other action's total is written unsigned.
    let amount = match action {
        Action::Cash => total.unwrap_or_else(Dec::zero),
        _ => total
            .unwrap_or_else(Dec::zero)
            .abs()
            .map_err(|_| QifError::AmountOverflow { line })?,
    };

    let security = record
        .security
        .clone()
        .unwrap_or_else(|| "Securities".to_string());
    let holding = format!("{register}:{security}");
    let (link, class) = names.target(record.category.as_deref(), register);
    let cash = if via_link {
        names.source(&link)
    } else {
        register.to_string()
    };
    let linked = || names.source(&link);
    let income = |kind: &str| match (&link, via_link) {
        (Target::Category(_), false) => linked(),
        _ => format!("Investment Income:{kind}"),
    };

    let describe = describe(record);
    let memo = match (&record.memo, describe) {
        (Some(memo), Some(describe)) => Some(format!("{memo} — {describe}")),
        (memo, describe) => memo.clone().or(describe),
    };
    let neg = negate(amount, line)?;

    let reinvested_from = match action {
        Action::Reinvest(kind) => Some(income(kind)),
        _ => None,
    };
    let trade = Trade {
        holding: &holding,
        cash: &cash,
        reinvested_from,
        class: class.clone(),
    };
    if let Some(postings) = trade.postings(names, record, action)? {
        return Ok(Built {
            kind: raw_action,
            postings,
            transfer_to: None,
            ..base
        });
    }

    // (first, second): first carries the memo; the two always balance.
    let (first, second, transfer_to) = match action {
        Action::Buy => ((holding, amount), (cash, neg), None),
        Action::Sell => ((cash, amount), (holding, neg), None),
        Action::Income(kind) => ((cash, amount), (income(kind), neg), None),
        Action::Reinvest(kind) => ((holding, amount), (income(kind), neg), None),
        Action::ReturnOfCapital => ((cash, amount), (holding, neg), None),
        Action::SharesIn => ((holding, amount), (OPENING_BALANCES.to_string(), neg), None),
        Action::SharesOut => ((holding, neg), (OPENING_BALANCES.to_string(), amount), None),
        Action::TransferIn | Action::TransferOut | Action::Cash => {
            let signed = if action == Action::TransferOut {
                neg
            } else {
                amount
            };
            let to = match &link {
                Target::Transfer(account) => Some(account.clone()),
                _ => None,
            };
            (
                (register.to_string(), signed),
                (linked(), negate(signed, line)?),
                to,
            )
        }
        Action::Expense => {
            let expense = match &link {
                Target::Category(_) if !via_link => linked(),
                _ => "Investment Expenses".to_string(),
            };
            ((expense, amount), (cash, neg), None)
        }
        Action::NoCash => (
            (register.to_string(), Dec::zero()),
            (holding, Dec::zero()),
            None,
        ),
    };
    let memo = if action == Action::NoCash && !amount.is_zero() {
        let note = format!(
            "QIF action {raw_action:?} not imported: amount {}",
            render_dec(amount, '.')
        );
        Some(memo.map_or_else(|| note.clone(), |memo| format!("{memo} ({note})")))
    } else {
        memo
    };
    Ok(Built {
        kind: raw_action,
        postings: vec![
            posting(first.0, first.1, memo, None),
            posting(second.0, second.1, None, class),
        ],
        transfer_to,
        ..base
    })
}

/// Where a share trade's postings go.
struct Trade<'a> {
    holding: &'a str,
    cash: &'a str,
    /// The income account a reinvested dividend or interest is paid from.
    reinvested_from: Option<String>,
    class: Option<String>,
}

impl Trade<'_> {
    /// A record that moves shares (`Buy`, `Sell`, a reinvestment, `ShrsIn`,
    /// `ShrsOut`, and their `X` forms) as units of the security, so the
    /// holding carries shares and a cost the way a hand-written hledger
    /// purchase does (plan 39):
    ///
    /// ```text
    /// Buy   Brokerage:ACME   40 ACME @ $50.00     Sell  Checking        $545.05
    ///       Investment Expenses:Commissions $4.95       Investment Expenses:Commissions $4.95
    ///       Brokerage       $-2004.95                   Brokerage:ACME  -10 ACME @ $55.00
    /// ```
    ///
    /// A sale is written at its sale price with no realized-gain posting, the
    /// same convention as Ledgeline's own sample journal; the holdings report
    /// works out the gain from average cost. The shares are worth the record's
    /// total less (for a buy) or plus (for a sale) its commission, and the
    /// price is kept when quantity × price is exactly that. Shares moved in or
    /// out with no total and no price carry no cost (`@@ $0`).
    ///
    /// `None` when the record names no security or no quantity, or the action
    /// moves no shares: it is then written as the cash it moved.
    fn postings(
        &self,
        names: &Names,
        record: &QifRecord,
        action: Action,
    ) -> Result<Option<Vec<QbPosting>>, QifError> {
        let moves_shares = matches!(
            action,
            Action::Buy | Action::Sell | Action::Reinvest(_) | Action::SharesIn | Action::SharesOut
        );
        let (Some(security), Some(quantity), true) =
            (record.security.as_deref(), record.quantity, moves_shares)
        else {
            return Ok(None);
        };
        let line = record.line;
        let overflow = |_| QifError::AmountOverflow { line };
        let quantity = quantity.abs().map_err(overflow)?;
        let commission = record
            .commission
            .unwrap_or_else(Dec::zero)
            .abs()
            .map_err(overflow)?;
        let total = record.amount.map(Dec::abs).transpose().map_err(overflow)?;
        let at_price = record
            .price
            .map(|price| price.mul(quantity))
            .transpose()
            .map_err(overflow)?;
        let value = match (action, total) {
            (Action::Buy, Some(total)) => total.sub(commission).map_err(overflow)?,
            (Action::Sell, Some(total)) => total.add(commission).map_err(overflow)?,
            (_, Some(total)) => total,
            (_, None) => at_price.unwrap_or_else(Dec::zero),
        };

        let shares = |inward: bool| -> Result<QbPosting, QifError> {
            let (amount, quantity) = if inward {
                (value, quantity)
            } else {
                (
                    value.neg().map_err(overflow)?,
                    quantity.neg().map_err(overflow)?,
                )
            };
            Ok(QbPosting {
                units: Some(Units {
                    commodity: names.commodity(security),
                    quantity,
                    price: record.price.map(Dec::abs).transpose().map_err(overflow)?,
                }),
                ..posting(self.holding.to_string(), amount, record.memo.clone(), None)
            })
        };
        let money = |account: &str, amount: Dec| {
            posting(account.to_string(), amount, None, self.class.clone())
        };
        let fee = (!commission.is_zero())
            .then(|| posting(COMMISSIONS.to_string(), commission, None, None));

        let postings = match action {
            Action::Buy => {
                let paid = value.add(commission).map_err(overflow)?;
                std::iter::once(shares(true)?)
                    .chain(fee)
                    .chain([money(self.cash, paid.neg().map_err(overflow)?)])
                    .collect()
            }
            Action::Sell => {
                let proceeds = value.sub(commission).map_err(overflow)?;
                std::iter::once(money(self.cash, proceeds))
                    .chain(fee)
                    .chain([shares(false)?])
                    .collect()
            }
            Action::Reinvest(_) => {
                let income = self
                    .reinvested_from
                    .as_deref()
                    .unwrap_or("Investment Income");
                vec![shares(true)?, money(income, value.neg().map_err(overflow)?)]
            }
            Action::SharesIn => vec![
                shares(true)?,
                money(OPENING_BALANCES, value.neg().map_err(overflow)?),
            ],
            Action::SharesOut => vec![shares(false)?, money(OPENING_BALANCES, value)],
            _ => return Ok(None),
        };
        Ok(Some(postings))
    }
}

/// `10 ACME @ 15.00, commission 4.95`, from whichever of those fields the
/// record carries. The action itself is the transaction type, which the
/// description already leads with (`Buy: 10 ACME …`).
fn describe(record: &QifRecord) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(quantity) = record.quantity {
        parts.push(render_dec(quantity, '.'));
    }
    if let Some(security) = &record.security {
        parts.push(security.clone());
    }
    if let Some(price) = record.price {
        parts.push(format!("@ {}", render_dec(price, '.')));
    }
    let mut text = parts.join(" ");
    if let Some(commission) = record.commission.filter(|c| !c.is_zero()) {
        text.push_str(&format!(", commission {}", render_dec(commission, '.')));
    }
    (!text.is_empty()).then_some(text)
}

// ---------------------------------------------------------------------------
// Mirrored transfers
// ---------------------------------------------------------------------------

/// The indices of records that are the mirror copy of a transfer another
/// record already carries. See the module docs.
fn mirrored(built: &[Built]) -> HashSet<usize> {
    let mut by_register: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    for (at, row) in built.iter().enumerate() {
        by_register
            .entry((row.register.as_str(), row.date.as_str()))
            .or_default()
            .push(at);
    }
    let mut dropped: HashSet<usize> = HashSet::new();
    let mut consumed: HashSet<(usize, usize)> = HashSet::new();
    for (at, row) in built.iter().enumerate() {
        let Some(other) = &row.transfer_to else {
            continue;
        };
        let amount = row.amount();
        let Some(candidates) = by_register.get(&(other.as_str(), row.date.as_str())) else {
            continue;
        };
        let partner = candidates.iter().find_map(|&candidate| {
            let side = &built[candidate];
            if candidate == at
                || dropped.contains(&candidate)
                || (side.transfer_to.is_some() && candidate > at)
            {
                return None;
            }
            side.postings
                .iter()
                .enumerate()
                .find(|(leg, posting)| {
                    posting.account == row.register
                        && posting.amount == amount
                        && !consumed.contains(&(candidate, *leg))
                })
                .map(|(leg, _)| (candidate, leg))
        });
        if let Some(leg) = partner {
            consumed.insert(leg);
            dropped.insert(at);
        }
    }
    dropped
}

// ---------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------

/// FNV-1a, 64-bit: fixed forever, unlike `std`'s hasher, so an id made today
/// matches the one a re-import makes next year.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// What identifies a record, before its occurrence index.
fn key(row: &Built) -> String {
    let amount = row.amount();
    let render = |value: Dec| render_dec(value.normalized(), '.');
    match &row.transfer_to {
        Some(other) => {
            let (low, high, signed) = if row.register <= *other {
                (row.register.as_str(), other.as_str(), amount)
            } else {
                (
                    other.as_str(),
                    row.register.as_str(),
                    amount.neg().unwrap_or(amount),
                )
            };
            ["transfer", &row.date, low, high, &render(signed)].join("\u{1f}")
        }
        None => [
            "record",
            &row.date,
            &row.register,
            &render(amount),
            row.payee.as_deref().unwrap_or(""),
        ]
        .join("\u{1f}"),
    }
}

/// One id per record, in order. The occurrence index counts identical keys
/// within the same register, so a transfer's two mirrored copies — each first
/// in its own register — hash alike.
fn ids(built: &[Built]) -> Vec<String> {
    let mut seen: HashMap<(String, &str), usize> = HashMap::new();
    built
        .iter()
        .map(|row| {
            let key = key(row);
            let count = seen
                .entry((key.clone(), row.register.as_str()))
                .or_default();
            let occurrence = *count;
            *count += 1;
            format!("qif-{:016x}", fnv1a(&format!("{key}\u{1f}{occurrence}")))
        })
        .collect()
}

/// Make every id unique: a later repeat (two unmatched one-sided copies of
/// the same transfer, say) gets `-2`, `-3` …
fn unique_ids(transactions: Vec<QbTransaction>) -> Vec<QbTransaction> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    transactions
        .into_iter()
        .map(|mut txn| {
            let count = seen.entry(txn.id.clone()).or_default();
            *count += 1;
            if *count > 1 {
                txn.id = format!("{}-{count}", txn.id);
            }
            txn
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Dec {
        Dec::parse(text, '.').expect("test literal parses")
    }

    fn import_text(text: &str) -> QbJournal {
        parse_and_import("Checking.qif", text.as_bytes()).expect("imports")
    }

    /// `(account, amount)` per posting.
    fn legs(txn: &QbTransaction) -> Vec<(String, Dec)> {
        txn.postings
            .iter()
            .map(|p| (p.account.clone(), p.amount))
            .collect()
    }

    fn balanced(journal: &QbJournal) {
        for txn in &journal.transactions {
            let sum = txn
                .postings
                .iter()
                .try_fold(Dec::zero(), |sum, p| sum.add(p.amount))
                .expect("sums");
            assert!(sum.is_zero(), "{} does not balance: {txn:?}", txn.id);
        }
    }

    #[test]
    fn a_simple_record_becomes_two_postings() {
        let journal = import_text(
            "!Type:Bank\nD01/05/2026\nT-42.50\nPCorner Grocer\nMWeekly\nN1001\nC*\nLFood:Groceries/Household\n^\n",
        );
        assert_eq!(journal.source, ImportSource::Qif);
        let txn = &journal.transactions[0];
        assert_eq!(txn.date, "2026-01-05");
        assert_eq!(txn.name.as_deref(), Some("Corner Grocer"));
        assert_eq!(txn.num.as_deref(), Some("1001"));
        assert_eq!(txn.status, ImportStatus::Cleared);
        assert!(
            txn.id.starts_with("qif-") && txn.id.len() == 20,
            "{}",
            txn.id
        );
        assert_eq!(
            legs(txn),
            vec![
                ("Checking".to_string(), dec("-42.50")),
                ("Food:Groceries".to_string(), dec("42.50")),
            ]
        );
        assert_eq!(txn.postings[0].memo.as_deref(), Some("Weekly"));
        assert_eq!(txn.postings[1].class.as_deref(), Some("Household"));
    }

    #[test]
    fn splits_become_one_posting_each_and_a_remainder_is_kept() {
        let journal = import_text(
            "!Type:Bank\nD1/5/26\nT-100.00\nSFood\nEbread\n$-60.00\nSHome/Rental\n$-30.00\n^\n",
        );
        balanced(&journal);
        let txn = &journal.transactions[0];
        assert_eq!(
            legs(txn),
            vec![
                ("Checking".to_string(), dec("-100")),
                ("Food".to_string(), dec("60")),
                ("Home".to_string(), dec("30")),
                (UNCATEGORIZED.to_string(), dec("10")),
            ]
        );
        assert_eq!(txn.postings[1].memo.as_deref(), Some("bread"));
        assert_eq!(txn.postings[2].class.as_deref(), Some("Rental"));
    }

    #[test]
    fn a_missing_total_is_the_sum_of_the_splits() {
        let journal = import_text("!Type:Bank\nD1/5/26\nSFood\n$-6\nSHome\n$-4\n^\n");
        assert_eq!(journal.transactions[0].postings[0].amount, dec("-10"));
        assert_eq!(journal.transactions[0].postings.len(), 3);
    }

    #[test]
    fn opening_balance_goes_to_equity() {
        let journal = import_text(
            "!Account\nNChecking\nTBank\n^\n!Type:Bank\nD1/1/26\nT1000\nCX\nPOpening Balance\nL[Checking]\n^\n",
        );
        let txn = &journal.transactions[0];
        assert_eq!(
            legs(txn),
            vec![
                ("Checking".to_string(), dec("1000")),
                (OPENING_BALANCES.to_string(), dec("-1000")),
            ]
        );
        assert_eq!(txn.status, ImportStatus::Cleared, "CX is reconciled");
    }

    #[test]
    fn no_category_is_uncategorized_and_a_clashing_category_is_marked() {
        let journal = import_text(
            "!Account\nNSavings\nTBank\n^\n!Account\nNChecking\nTBank\n^\n!Type:Bank\nD1/5/26\nT-1\n^\nD1/6/26\nT-2\nLSavings\n^\n",
        );
        assert_eq!(journal.transactions[0].postings[1].account, UNCATEGORIZED);
        assert_eq!(
            journal.transactions[1].postings[1].account,
            "Savings (category)"
        );
    }

    #[test]
    fn a_single_account_file_is_named_after_the_file() {
        let journal = parse_and_import(
            "downloads/Visa Card.QIF",
            b"!Type:CCard\nD1/5/26\nT-5\nLFees\n^\n",
        )
        .expect("imports");
        assert_eq!(journal.transactions[0].postings[0].account, "Visa Card");
        assert_eq!(fallback_account(".qif"), "QIF account");
    }

    const MIRRORED: &str = "!Account\nNChecking\nTBank\n^\n!Type:Bank\n\
        D1/5/26\nT-200\nPTo savings\nL[Savings]\n^\n\
        D1/5/26\nT-200\nPTo savings again\nL[Savings]\n^\n\
        !Account\nNSavings\nTBank\n^\n!Type:Bank\n\
        D1/5/26\nT200\nPFrom checking\nL[Checking]\n^\n\
        D1/5/26\nT200\nL[Checking]\n^\n";

    #[test]
    fn mirrored_transfers_collapse_one_for_one() {
        let journal = import_text(MIRRORED);
        assert_eq!(journal.transactions.len(), 2, "{:#?}", journal.transactions);
        for txn in &journal.transactions {
            assert_eq!(
                txn.postings[0].account, "Checking",
                "the earlier side is kept"
            );
        }
        let ids: HashSet<_> = journal.transactions.iter().map(|t| &t.id).collect();
        assert_eq!(ids.len(), 2, "two identical transfers keep two ids");
    }

    #[test]
    fn a_one_sided_transfer_file_gets_the_same_ids_as_the_other_side() {
        let both = import_text(MIRRORED);
        let savings_only = import_text(
            "!Account\nNSavings\nTBank\n^\n!Type:Bank\nD1/5/26\nT200\nL[Checking]\n^\nD1/5/26\nT200\nL[Checking]\n^\n",
        );
        let left: Vec<_> = both.transactions.iter().map(|t| t.id.clone()).collect();
        let right: Vec<_> = savings_only
            .transactions
            .iter()
            .map(|t| t.id.clone())
            .collect();
        assert_eq!(left, right);
    }

    #[test]
    fn a_transfer_without_a_mirror_is_kept() {
        let journal = import_text(
            "!Account\nNChecking\nTBank\n^\n!Type:Bank\nD1/5/26\nT-200\nL[Savings]\n^\n!Account\nNSavings\nTBank\n^\n!Type:Bank\nD1/6/26\nT200\nL[Checking]\n^\n",
        );
        assert_eq!(
            journal.transactions.len(),
            2,
            "different dates are not mirrors"
        );
    }

    #[test]
    fn a_split_transfer_absorbs_its_plain_mirror() {
        let journal = import_text(
            "!Account\nNSavings\nTBank\n^\n!Type:Bank\nD1/5/26\nT60\nL[Checking]\n^\n\
             !Account\nNChecking\nTBank\n^\n!Type:Bank\nD1/5/26\nT-100\nS[Savings]\n$-60\nSFood\n$-40\n^\n",
        );
        assert_eq!(journal.transactions.len(), 1);
        assert_eq!(journal.transactions[0].postings.len(), 3);
        balanced(&journal);
    }

    #[test]
    fn identical_records_get_distinct_stable_ids() {
        let text = "!Type:Bank\nD1/5/26\nT-3\nPCoffee\n^\nD1/5/26\nT-3\nPCoffee\n^\n";
        let first = import_text(text);
        let again = import_text(text);
        assert_ne!(first.transactions[0].id, first.transactions[1].id);
        assert_eq!(first.transactions[0].id, again.transactions[0].id);
        assert_eq!(first.transactions[1].id, again.transactions[1].id);
    }

    #[test]
    fn investment_actions_carry_shares_and_cash() {
        let journal = import_text(
            "!Type:Security\nNACME Index Fund\nSACME\nTMutual Fund\n^\n\
             !Account\nNBrokerage\nTInvst\n^\n!Type:Invst\n\
             D1/5/26\nNBuy\nYACME Index Fund\nI15.00\nQ10\nO4.95\nT154.95\n^\n\
             D1/6/26\nNDiv\nYACME Index Fund\nT12.00\n^\n\
             D1/7/26\nNSellX\nYACME Index Fund\nQ5\nI20\nT100\nL[Checking]\n$100\n^\n\
             D1/8/26\nNReinvDiv\nYACME Index Fund\nI15\nQ0.2\nT3\n^\n\
             D1/9/26\nNStkSplit\nYACME Index Fund\nQ2\n^\n\
             D1/10/26\nNXIn\nT500\nL[Checking]\n^\n\
             D1/11/26\nNShrsIn\nYNO LIST CO\nQ3\n^\n",
        );
        balanced(&journal);
        let all: Vec<_> = journal.transactions.iter().map(legs).collect();
        let units = |txn: usize, posting: usize| {
            journal.transactions[txn].postings[posting]
                .units
                .clone()
                .map(|units| (units.commodity, units.quantity, units.price))
        };

        // A buy: shares at the price, the commission, and the cash paid.
        assert_eq!(
            all[0],
            vec![
                ("Brokerage:ACME Index Fund".to_string(), dec("150")),
                (COMMISSIONS.to_string(), dec("4.95")),
                ("Brokerage".to_string(), dec("-154.95")),
            ]
        );
        assert_eq!(
            units(0, 0),
            Some(("ACME".to_string(), dec("10"), Some(dec("15.00"))))
        );
        assert_eq!(journal.transactions[0].transaction_type, "Buy");
        // A dividend is cash.
        assert_eq!(
            all[1],
            vec![
                ("Brokerage".to_string(), dec("12")),
                ("Investment Income:Dividends".to_string(), dec("-12")),
            ]
        );
        assert_eq!(units(1, 0), None);
        // A sale through another account: proceeds there, shares out at the sale price.
        assert_eq!(
            all[2],
            vec![
                ("Checking".to_string(), dec("100")),
                ("Brokerage:ACME Index Fund".to_string(), dec("-100")),
            ]
        );
        assert_eq!(
            units(2, 1),
            Some(("ACME".to_string(), dec("-5"), Some(dec("20"))))
        );
        // A reinvested dividend: shares bought with the income.
        assert_eq!(
            all[3],
            vec![
                ("Brokerage:ACME Index Fund".to_string(), dec("3")),
                ("Investment Income:Dividends".to_string(), dec("-3")),
            ]
        );
        assert_eq!(
            units(3, 0),
            Some(("ACME".to_string(), dec("0.2"), Some(dec("15"))))
        );
        // A split moves no money and no units here.
        assert!(all[4].iter().all(|(_, amount)| amount.is_zero()));
        assert_eq!(
            all[5],
            vec![
                ("Brokerage".to_string(), dec("500")),
                ("Checking".to_string(), dec("-500")),
            ]
        );
        // Shares moved in with no price: units at no cost, and a security
        // with no list entry keeps its own name.
        assert_eq!(
            all[6],
            vec![
                ("Brokerage:NO LIST CO".to_string(), dec("0")),
                (OPENING_BALANCES.to_string(), dec("0")),
            ]
        );
        assert_eq!(
            units(6, 0),
            Some(("NO LIST CO".to_string(), dec("3"), None))
        );
    }

    #[test]
    fn an_investment_transfer_absorbs_the_bank_side() {
        let journal = import_text(
            "!Account\nNBrokerage\nTInvst\n^\n!Type:Invst\nD1/10/26\nNXIn\nT500\nL[Checking]\n^\n\
             !Account\nNChecking\nTBank\n^\n!Type:Bank\nD1/10/26\nT-500\nL[Brokerage]\n^\n",
        );
        assert_eq!(journal.transactions.len(), 1);
    }

    #[test]
    fn an_unknown_action_keeps_its_amount_in_the_memo() {
        let journal = import_text("!Type:Invst\nD1/5/26\nNFrobnicate\nT7\n^\n");
        let txn = &journal.transactions[0];
        assert!(txn.postings.iter().all(|p| p.amount.is_zero()));
        let memo = txn.postings[0].memo.as_deref().unwrap_or("");
        assert!(memo.contains("Frobnicate") && memo.contains('7'), "{memo}");
    }

    #[test]
    fn refusals() {
        assert_eq!(
            parse_and_import("x.qif", b"!Type:Cat\nNFood\n^\n"),
            Err(QifError::NoTransactions)
        );
        assert_eq!(
            parse_and_import("x.qif", b"!Type:Bank\nT1\n^\n"),
            Err(QifError::MissingDate { line: 2 })
        );
    }

    #[test]
    fn the_ambiguity_flag_reaches_the_journal() {
        let journal = import_text("!Type:Bank\nD01/02/2026\nT1\n^\nD02/03/2026\nT1\n^\n");
        assert!(journal.date_format.ambiguous);
        assert_eq!(journal.transactions[0].date, "2026-01-02");
    }

    proptest::proptest! {
        /// Whatever records a register holds, every imported transaction
        /// balances and every id is unique.
        #[test]
        fn imports_always_balance_with_unique_ids(
            lines in proptest::collection::vec(
                proptest::prop_oneof![
                    proptest::strategy::Just("D1/5/26".to_string()),
                    "T-?[0-9]{1,5}(\\.[0-9]{2})?",
                    "\\$-?[0-9]{1,4}",
                    "L(\\[(Checking|Savings)\\]|Food|Home/Kids)?",
                    "S(\\[(Checking|Savings)\\]|Food)?",
                    "N(Buy|SellX|Div|XIn|Cash|Bogus)",
                    "PCoffee",
                    proptest::strategy::Just("^".to_string()),
                ],
                0..80,
            )
        ) {
            let text = format!(
                "!Account\nNChecking\nTBank\n^\n!Type:Bank\n{body}\n^\n!Account\nNSavings\nTInvst\n^\n!Type:Invst\n{body}\n^\n",
                body = lines.join("\n")
            );
            if let Ok(journal) = parse_and_import("x.qif", text.as_bytes()) {
                let ids: HashSet<_> = journal.transactions.iter().map(|t| t.id.clone()).collect();
                proptest::prop_assert_eq!(ids.len(), journal.transactions.len());
                for txn in &journal.transactions {
                    let sum = txn.postings.iter().try_fold(Dec::zero(), |sum, p| sum.add(p.amount));
                    proptest::prop_assert!(sum.is_ok_and(|sum| sum.is_zero()));
                }
            }
        }
    }
}
