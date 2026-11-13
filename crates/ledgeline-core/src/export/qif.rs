//! Quicken Interchange Format (QIF) export (plan 39 Phase 1).
//!
//! QIF is single-entry: every transaction lives in one account's register,
//! and its other postings are categories or `[Account]` transfers. This
//! module maps a double-entry journal onto that shape. Every decision below
//! is deliberate and documented in `docs/exports/qif.md`.
//!
//! # Accounts
//!
//! Accounts are classified by their RESOLVED TYPE
//! ([`crate::reports::account_types`]), never by name, so a chart of accounts
//! in any language exports the same way:
//!
//! | hledger type             | QIF                                       |
//! |--------------------------|-------------------------------------------|
//! | Cash (`C`)               | `Bank` register                           |
//! | other Asset (`A`)        | `Oth A` register                          |
//! | Liability (`L`)          | `CCard` or `Oth L` register — see below   |
//! | Revenue (`R`, `G`)       | income category (`!Type:Cat` with `I`)    |
//! | Expense (`X`)            | expense category (`E`)                    |
//! | Equity (`E`, `V`)        | the [`HOLDING_ACCOUNT`], or an Opening Balance |
//! | no resolvable type       | expense category, listed in the report    |
//!
//! A liability is `CCard` when, in more than half of the transactions that
//! touch it (leaving out those with an equity leg, such as its opening
//! balance), it is the only register and the money went to an expense — a
//! card used for purchases. Anything else (a loan or mortgage, which is paid
//! from another account) is `Oth L`. An `account` declaration's `qif:` tag
//! (`qif: Cash`, `qif: CCard`, `qif: Oth L` …) overrides the classification.
//!
//! # Transactions
//!
//! Postings are split by commodity first: QIF has no currency field, so each
//! commodity gets its own file. A posting with a cost (`@`/`@@`) is written as
//! the cash it cost — shares bought into a brokerage account become the
//! dollars they cost — except on a bank (`Bank`/`Cash`) register, where a cost
//! is a currency exchange and the currency is kept. A commodity whose postings
//! do not balance on their own (that exchange, or a conversion with no written
//! cost) is balanced through the holding account in its file.
//!
//! Within one commodity, the transaction is written once, in its *home*
//! register — [`home_register`] — with a split line for every other posting:
//! a category, or `[Account]` for another register. The target app mirrors
//! the transfer into the other register itself. [`TransferPlacement`] is the
//! one switch that changes that.
//!
//! A transaction whose only non-register postings are equity is an opening
//! balance: each register in it that has not been seen before in that file
//! gets a QIF `Opening Balance` record (payee `Opening Balance`, category
//! `[itself]`), which is how Quicken-family apps set a starting balance.
//!
//! # Fields
//!
//! `D` is `MM/DD/YYYY`; `T`/`$` are plain numbers with a `.` decimal mark and
//! at least two places; `N` is the transaction code; `P` the description; `M`
//! the transaction comment without its tags; `E` a posting's comment without
//! its tags; `L`/`S` take `/Class` from a `class:` tag. Status maps
//! three-to-three ([`status_field`]).

use super::loss::{FileSummary, LossReport};
use super::{free_text, is_internal_tag, print_order};
use crate::auto::apply_journal;
use crate::decimal::{Dec, DecError};
use crate::edit::render_dec;
use crate::model::{Commodity, Journal, PostingType, Status, Transaction};
use crate::reports::account_types::{AccountType, AccountTypes, account_decls};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

/// The register that holds what QIF has no place for: equity legs, implicit
/// conversions, and transactions with no bank-like account at all. An
/// `Oth A` account, so its balance is the net of everything routed through it.
pub const HOLDING_ACCOUNT: &str = "Ledgeline Adjustments";

/// The `account` declaration tag that overrides an account's QIF type.
pub const QIF_TYPE_TAG: &str = "qif";

/// The tag exported as the QIF class (`L Category/Class`), for parity with
/// the QuickBooks import's classes.
pub const CLASS_TAG: &str = "class";

/// The payee QIF uses to mark an account's opening balance record.
const OPENING_BALANCE_PAYEE: &str = "Opening Balance";

/// Where a transfer between registers is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPlacement {
    /// Once, in the home register, as an `[Account]` split; the importing app
    /// creates the other side. This avoids the double-counted transfers
    /// Banktivity's own QIF re-export is known for.
    Once,
    /// In the home register AND, as a single `[Home]` record, in every other
    /// register the transaction touches — the way Quicken itself exports.
    /// For an app that does not create the mirror side on import.
    Mirrored,
}

/// The placement the export uses unless asked otherwise. Plan 39 Phase 3 (the
/// Banktivity round trip) settles whether this should flip.
pub const DEFAULT_TRANSFER_PLACEMENT: TransferPlacement = TransferPlacement::Once;

/// Options for [`export_qif`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QifOptions {
    /// Prefix for the file names (`{name}-{commodity}.qif`); usually the
    /// journal's file stem.
    pub name: String,
    /// Where transfers between registers are written.
    pub transfers: TransferPlacement,
}

impl Default for QifOptions {
    fn default() -> Self {
        Self {
            name: "ledgeline".to_string(),
            transfers: DEFAULT_TRANSFER_PLACEMENT,
        }
    }
}

/// One QIF file: every record in one commodity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QifFile {
    /// The commodity every amount in this file is in.
    pub commodity: Commodity,
    /// Suggested file name, `{name}-{slug}.qif`.
    pub file_name: String,
    /// The QIF text.
    pub text: String,
}

/// The result of [`export_qif`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QifExport {
    /// One file per commodity, the primary commodity first.
    pub files: Vec<QifFile>,
    /// What was dropped or reshaped.
    pub loss: LossReport,
}

/// The QIF register types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QifAccountType {
    /// `Bank`
    Bank,
    /// `Cash`
    Cash,
    /// `CCard`
    CreditCard,
    /// `Oth A`
    OtherAsset,
    /// `Oth L`
    OtherLiability,
}

impl QifAccountType {
    /// The spelling QIF uses in `!Account` (`T`) and `!Type:` headers.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Bank => "Bank",
            Self::Cash => "Cash",
            Self::CreditCard => "CCard",
            Self::OtherAsset => "Oth A",
            Self::OtherLiability => "Oth L",
        }
    }

    /// Read a `qif:` tag value, case- and space-insensitively.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let squashed: String = value
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        match squashed.as_str() {
            "bank" => Some(Self::Bank),
            "cash" => Some(Self::Cash),
            "ccard" | "creditcard" => Some(Self::CreditCard),
            "otha" | "otherasset" => Some(Self::OtherAsset),
            "othl" | "otherliability" => Some(Self::OtherLiability),
            _ => None,
        }
    }
}

/// hledger status → QIF `C` field, three states to three: unmarked has no
/// `C` line, pending (`!`) is cleared (`*`), cleared (`*`) is reconciled
/// (`X`). The mapping keeps every distinction, so an import maps it straight
/// back.
#[must_use]
pub fn status_field(status: Status) -> Option<&'static str> {
    match status {
        Status::Unmarked => None,
        Status::Pending => Some("*"),
        Status::Cleared => Some("X"),
    }
}

/// The register a transaction is written in, given the registers its postings
/// touch (QIF names): the first in name order, preferring any real account to
/// the [`HOLDING_ACCOUNT`]. Name order rather than posting order so that
/// reordering a transaction's postings never moves it to another register.
#[must_use]
pub fn home_register<'a>(registers: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    registers
        .into_iter()
        .min_by_key(|name| (*name == HOLDING_ACCOUNT, *name))
}

/// Export `journal` as QIF, one file per commodity.
///
/// # Errors
/// [`DecError`] only if summing a transaction's amounts overflows.
pub fn export_qif(journal: &Journal, options: &QifOptions) -> Result<QifExport, DecError> {
    let applied = apply_journal(journal);
    let classifier = Classifier::new(journal, &applied);
    let mut loss = LossReport {
        transactions: applied.transactions.len(),
        price_directives: journal.prices.len(),
        periodic_transactions: journal.periodic_transactions.len(),
        auto_posting_rules: journal.auto_rules.len(),
        auto_postings_applied: applied
            .transactions
            .iter()
            .flat_map(|txn| &txn.postings)
            .filter(|posting| posting.generated.is_some())
            .count(),
        account_tags: journal
            .accounts
            .iter()
            .filter(|decl| {
                decl.tags
                    .iter()
                    .any(|(name, _)| name != "type" && name != QIF_TYPE_TAG)
            })
            .count(),
        ..LossReport::default()
    };

    let mut files: BTreeMap<Commodity, FileBuilder> = BTreeMap::new();
    let mut names = Names::default();
    let mut untyped: BTreeSet<String> = BTreeSet::new();
    for txn in print_order(&applied) {
        let mut ctx = Context {
            classifier: &classifier,
            placement: options.transfers,
            names: &mut names,
            loss: &mut loss,
        };
        export_transaction(txn, &mut ctx, &mut files, &mut untyped)?;
    }

    let categories = classifier.categories(&applied, &mut names);
    let primary = primary_commodity(journal, &files);
    let mut ordered: Vec<(Commodity, FileBuilder)> = files.into_iter().collect();
    ordered.sort_by_key(|(commodity, _)| Some(commodity) != primary.as_ref());

    let used_slugs = &mut HashSet::new();
    let qif_files: Vec<QifFile> = ordered
        .into_iter()
        .map(|(commodity, builder)| {
            let file_name = format!(
                "{}-{}.qif",
                options.name,
                unique_slug(&commodity, used_slugs)
            );
            loss.files.push(FileSummary {
                file_name: file_name.clone(),
                commodity: commodity.0.clone(),
                records: builder.records,
            });
            loss.records += builder.records;
            QifFile {
                text: builder.render(&categories),
                commodity,
                file_name,
            }
        })
        .collect();

    loss.renamed_accounts = names.renamed.into_iter().collect();
    loss.untyped_accounts = untyped.into_iter().collect();
    loss.credit_cards = classifier.credit_cards();
    Ok(QifExport {
        files: qif_files,
        loss,
    })
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// What an account becomes in QIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// A register of this type.
    Register(QifAccountType),
    /// A category; `income` sets the `I` flag.
    Category { income: bool },
    /// Equity: routed to the holding account. `conversion` for a `type: V`
    /// account, which never makes an opening balance.
    Equity { conversion: bool },
}

struct Classifier {
    types: AccountTypes,
    overrides: HashMap<String, QifAccountType>,
    credit_cards: BTreeSet<String>,
}

impl Classifier {
    fn new(journal: &Journal, applied: &Journal) -> Self {
        let types = AccountTypes::new(&account_decls(journal));
        let overrides = journal
            .accounts
            .iter()
            .filter_map(|decl| {
                decl.tags
                    .iter()
                    .find(|(name, _)| name == QIF_TYPE_TAG)
                    .and_then(|(_, value)| QifAccountType::parse(value))
                    .map(|ty| (decl.name.0.clone(), ty))
            })
            .collect();
        let mut classifier = Self {
            types,
            overrides,
            credit_cards: BTreeSet::new(),
        };
        classifier.credit_cards = classifier.find_credit_cards(applied);
        classifier
    }

    /// The `qif:` override on `account` or its nearest declared ancestor.
    fn override_for(&self, account: &str) -> Option<QifAccountType> {
        crate::model::AccountName(account.to_string())
            .self_and_ancestors()
            .iter()
            .find_map(|name| self.overrides.get(name).copied())
    }

    /// Whether `account` is a balance-sheet register (before deciding which
    /// kind of liability it is).
    fn is_register(&self, account: &str) -> bool {
        self.override_for(account).is_some()
            || self.types.is_type(account, AccountType::Asset)
            || self.types.is_type(account, AccountType::Liability)
    }

    /// The liabilities used like a credit card: in more than half of the
    /// transactions touching one, it is the only register and the other side
    /// includes an expense. Transactions with an equity leg (opening balances,
    /// adjustments) do not vote. Overridden liabilities are left to their tag.
    fn find_credit_cards(&self, applied: &Journal) -> BTreeSet<String> {
        let mut touched: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for txn in &applied.transactions {
            let accounts: BTreeSet<&str> = txn
                .postings
                .iter()
                .filter(|p| p.ptype != PostingType::Virtual)
                .map(|p| p.account.0.as_str())
                .collect();
            if accounts
                .iter()
                .any(|a| self.types.is_type(a, AccountType::Equity))
            {
                continue;
            }
            let registers: Vec<&str> = accounts
                .iter()
                .copied()
                .filter(|a| self.is_register(a))
                .collect();
            let spends = accounts
                .iter()
                .any(|a| self.types.is_type(a, AccountType::Expense));
            for account in accounts.iter().copied().filter(|a| {
                self.override_for(a).is_none() && self.types.is_type(a, AccountType::Liability)
            }) {
                let entry = touched.entry(account).or_default();
                entry.0 += 1;
                if spends && registers == [account] {
                    entry.1 += 1;
                }
            }
        }
        touched
            .into_iter()
            .filter(|(_, (total, purchases))| purchases * 2 > *total)
            .map(|(account, _)| account.to_string())
            .collect()
    }

    fn role(&self, account: &str) -> Role {
        if let Some(ty) = self.override_for(account) {
            return Role::Register(ty);
        }
        match self.types.resolve(account) {
            Some(AccountType::Cash) => Role::Register(QifAccountType::Bank),
            Some(AccountType::Asset) => Role::Register(QifAccountType::OtherAsset),
            Some(AccountType::Liability) => {
                Role::Register(if self.credit_cards.contains(account) {
                    QifAccountType::CreditCard
                } else {
                    QifAccountType::OtherLiability
                })
            }
            Some(AccountType::Equity) => Role::Equity { conversion: false },
            Some(AccountType::Conversion) => Role::Equity { conversion: true },
            Some(AccountType::Revenue | AccountType::Gain) => Role::Category { income: true },
            Some(AccountType::Expense) => Role::Category { income: false },
            None => Role::Category { income: false },
        }
    }

    /// Whether `account` has no type at all (and no `qif:` override) — it is
    /// exported as an expense category, and the report lists it.
    fn is_untyped(&self, account: &str) -> bool {
        self.override_for(account).is_none() && self.types.resolve(account).is_none()
    }

    /// Every category the export knows: declared category accounts and every
    /// category account a transaction posts to, by QIF name.
    fn categories(&self, applied: &Journal, names: &mut Names) -> BTreeMap<String, bool> {
        applied
            .accounts
            .iter()
            .map(|decl| decl.name.0.as_str())
            .chain(
                applied
                    .transactions
                    .iter()
                    .flat_map(|txn| &txn.postings)
                    .filter(|p| p.ptype != PostingType::Virtual)
                    .map(|p| p.account.0.as_str()),
            )
            .filter_map(|account| match self.role(account) {
                Role::Category { income } => Some((names.qif(account), income)),
                _ => None,
            })
            .collect()
    }

    fn credit_cards(&self) -> Vec<String> {
        self.credit_cards.iter().cloned().collect()
    }
}

/// hledger account names → QIF names. `/` separates a class in an `L` field
/// and `[`/`]` mark a transfer, so they are replaced; every rename is reported.
#[derive(Default)]
struct Names {
    renamed: BTreeSet<(String, String)>,
}

impl Names {
    fn qif(&mut self, account: &str) -> String {
        let name = sanitize(account);
        if name != account {
            self.renamed.insert((account.to_string(), name.clone()));
        }
        name
    }
}

/// Replace the characters QIF gives a meaning to in a category, transfer or
/// class name, and fold line breaks to spaces.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' => '-',
            '[' => '(',
            ']' => ')',
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

/// A single-line QIF field value.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

// ---------------------------------------------------------------------------
// Transactions → records
// ---------------------------------------------------------------------------

/// One posting amount, in the commodity it is exported in.
#[derive(Debug, Clone)]
struct Leg {
    /// The hledger account.
    account: String,
    role: Role,
    quantity: Dec,
    status: Status,
    memo: String,
    class: Option<String>,
    /// A synthetic leg balancing an implicit conversion.
    conversion: bool,
}

/// A QIF split line (or the single `L` of an unsplit record).
#[derive(Debug, Clone)]
struct Split {
    target: Target,
    class: Option<String>,
    memo: String,
    /// The memo is (or includes) a posting's own comment.
    from_comment: bool,
    amount: Dec,
}

#[derive(Debug, Clone)]
enum Target {
    Category(String),
    Transfer(String),
}

/// One `^`-terminated QIF transaction.
#[derive(Debug, Clone)]
struct Record {
    date: String,
    status: Status,
    code: String,
    payee: String,
    memo: String,
    total: Dec,
    splits: Vec<Split>,
}

/// Everything written to one commodity's file.
#[derive(Default)]
struct FileBuilder {
    /// Registers by QIF name: their type and records, in print order.
    registers: BTreeMap<String, (QifAccountType, Vec<Record>)>,
    /// Registers already touched by a transaction in this file — the test for
    /// whether an opening balance can still be an `Opening Balance` record.
    touched: HashSet<String>,
    records: usize,
}

impl FileBuilder {
    fn register(&mut self, name: &str, ty: QifAccountType) -> &mut Vec<Record> {
        &mut self
            .registers
            .entry(name.to_string())
            .or_insert_with(|| (ty, Vec::new()))
            .1
    }

    fn push(&mut self, name: &str, ty: QifAccountType, record: Record) {
        self.register(name, ty).push(record);
        self.records += 1;
    }

    fn render(&self, categories: &BTreeMap<String, bool>) -> String {
        let mut out = String::new();
        let classes: BTreeSet<&str> = self
            .registers
            .values()
            .flat_map(|(_, records)| records)
            .flat_map(|record| &record.splits)
            .filter_map(|split| split.class.as_deref())
            .collect();
        if !classes.is_empty() {
            out.push_str("!Type:Class\n");
            for class in classes {
                let _ = write!(out, "N{class}\n^\n");
            }
        }
        if !categories.is_empty() {
            out.push_str("!Type:Cat\n");
            for (name, income) in categories {
                let _ = write!(out, "N{name}\n{}\n^\n", if *income { "I" } else { "E" });
            }
        }
        out.push_str("!Option:AutoSwitch\n!Account\n");
        for (name, (ty, _)) in &self.registers {
            let _ = write!(out, "N{name}\nT{}\n^\n", ty.code());
        }
        out.push_str("!Clear:AutoSwitch\n");
        for (name, (ty, records)) in self.registers.iter().filter(|(_, (_, r))| !r.is_empty()) {
            let _ = write!(
                out,
                "!Account\nN{name}\nT{code}\n^\n!Type:{code}\n",
                code = ty.code()
            );
            for record in records {
                render_record(&mut out, record);
            }
        }
        out
    }
}

fn render_record(out: &mut String, record: &Record) {
    let _ = writeln!(out, "D{}", qif_date(&record.date));
    let _ = writeln!(out, "T{}", qif_amount(record.total));
    if let Some(mark) = status_field(record.status) {
        let _ = writeln!(out, "C{mark}");
    }
    if !record.code.is_empty() {
        let _ = writeln!(out, "N{}", one_line(&record.code));
    }
    if !record.payee.is_empty() {
        let _ = writeln!(out, "P{}", one_line(&record.payee));
    }
    if !record.memo.is_empty() {
        let _ = writeln!(out, "M{}", one_line(&record.memo));
    }
    // Quicken's own convention: an `L` line naming the (first) split's target
    // even on a split record, then the `S`/`E`/`$` triples.
    if let Some(first) = record.splits.first() {
        let _ = writeln!(out, "L{}", split_target(first));
    }
    if record.splits.len() > 1 {
        for split in &record.splits {
            let _ = writeln!(out, "S{}", split_target(split));
            if !split.memo.is_empty() {
                let _ = writeln!(out, "E{}", one_line(&split.memo));
            }
            let _ = writeln!(out, "${}", qif_amount(split.amount));
        }
    }
    out.push_str("^\n");
}

fn split_target(split: &Split) -> String {
    let target = match &split.target {
        Target::Category(name) => name.clone(),
        Target::Transfer(name) => format!("[{name}]"),
    };
    match &split.class {
        Some(class) => format!("{target}/{class}"),
        None => target,
    }
}

/// `2024-07-01` → `07/01/2024`.
fn qif_date(iso: &str) -> String {
    match (iso.get(0..4), iso.get(5..7), iso.get(8..10)) {
        (Some(y), Some(m), Some(d)) => format!("{m}/{d}/{y}"),
        _ => iso.to_string(),
    }
}

/// A plain number, `.` decimal mark, no grouping, at least two places.
fn qif_amount(value: Dec) -> String {
    let widened = if value.places < 2 {
        value.rounded(2).unwrap_or(value)
    } else {
        value
    };
    render_dec(widened, '.')
}

fn sum(legs: &[&Leg]) -> Result<Dec, DecError> {
    legs.iter()
        .try_fold(Dec::zero(), |total, leg| total.add(leg.quantity))
}

/// The value of a posting's own `class:` tag, else its transaction's.
fn class_of(own: &[(String, String)], txn: &Transaction) -> Option<String> {
    own.iter()
        .chain(&txn.tags)
        .find(|(name, value)| name == CLASS_TAG && !value.is_empty())
        .map(|(_, value)| sanitize(value))
}

/// Whether a tag list carries anything QIF drops: tags other than the class,
/// the posting `date:`/`date2:` tags (counted on their own) and the engine's.
fn has_dropped_tags(tags: &[(String, String)]) -> bool {
    tags.iter().any(|(name, _)| {
        name != CLASS_TAG && name != "date" && name != "date2" && !is_internal_tag(name)
    })
}

/// What every transaction's export reads and updates.
struct Context<'a> {
    classifier: &'a Classifier,
    placement: TransferPlacement,
    names: &'a mut Names,
    loss: &'a mut LossReport,
}

fn export_transaction(
    txn: &Transaction,
    ctx: &mut Context<'_>,
    files: &mut BTreeMap<Commodity, FileBuilder>,
    untyped: &mut BTreeSet<String>,
) -> Result<(), DecError> {
    if txn.date2.is_some() {
        ctx.loss.secondary_dates += 1;
    }
    if has_dropped_tags(&txn.tags) {
        ctx.loss.transaction_tags += 1;
    }

    // Legs, grouped by the commodity they are exported in.
    let mut groups: BTreeMap<Commodity, Vec<Leg>> = BTreeMap::new();
    for posting in &txn.postings {
        if posting.ptype == PostingType::Virtual {
            ctx.loss.virtual_postings += 1;
            continue;
        }
        if posting.balance_assertion.is_some() {
            ctx.loss.balance_assertions += 1;
        }
        // A generated posting inherits the matched posting's date; count the
        // date once, where it was written.
        if posting.generated.is_none() && (posting.date.is_some() || posting.date2.is_some()) {
            ctx.loss.posting_dates += 1;
        }
        if has_dropped_tags(&posting.tags) {
            ctx.loss.posting_tags += 1;
        }
        let account = posting.account.0.as_str();
        let role = ctx.classifier.role(account);
        if ctx.classifier.is_untyped(account) {
            untyped.insert(account.to_string());
        }
        // A cost on a bank account is a currency exchange: keep the currency,
        // and let the residual below balance each side. Anywhere else
        // (shares, a foreign-currency expense) the posting is the cash it cost.
        let exchange = matches!(
            role,
            Role::Register(QifAccountType::Bank | QifAccountType::Cash)
        );
        if !exchange && posting.amounts.iter().any(|amount| amount.cost.is_some()) {
            ctx.loss.cost_postings += 1;
        }
        for amount in &posting.amounts {
            let (commodity, quantity) = if exchange {
                (&amount.commodity, amount.quantity)
            } else {
                amount.at_cost()?
            };
            groups.entry(commodity.clone()).or_default().push(Leg {
                account: account.to_string(),
                role,
                quantity,
                status: posting.status,
                memo: free_text(&posting.comment),
                class: class_of(&posting.tags, txn),
                conversion: false,
            });
        }
    }

    // A commodity whose legs do not sum to zero is one side of a conversion
    // (an exchange, or two commodities with no written cost): balance it
    // through the holding account.
    let mut converted = false;
    for legs in groups.values_mut() {
        let residual = sum(&legs.iter().collect::<Vec<_>>())?;
        if !residual.is_zero() {
            converted = true;
            legs.push(Leg {
                account: HOLDING_ACCOUNT.to_string(),
                role: Role::Equity { conversion: true },
                quantity: residual.neg()?,
                status: txn.status,
                memo: "commodity conversion".to_string(),
                class: class_of(&[], txn),
                conversion: true,
            });
        }
    }
    if converted {
        ctx.loss.conversions += 1;
    }

    let mut held = false;
    for (commodity, legs) in groups {
        let file = files.entry(commodity).or_default();
        held |= write_group(txn, &legs, ctx, file)?;
    }
    if held {
        ctx.loss.holding_account_transactions += 1;
    }
    Ok(())
}

/// Write one commodity's share of a transaction. Returns whether anything went
/// through the holding account.
fn write_group(
    txn: &Transaction,
    legs: &[Leg],
    ctx: &mut Context<'_>,
    file: &mut FileBuilder,
) -> Result<bool, DecError> {
    let registers: Vec<&Leg> = legs
        .iter()
        .filter(|leg| matches!(leg.role, Role::Register(_)))
        .collect();
    let is_opening = !registers.is_empty()
        && legs.len() > registers.len()
        && legs.iter().all(|leg| {
            matches!(
                leg.role,
                Role::Register(_) | Role::Equity { conversion: false }
            )
        });
    if !is_opening {
        return write_record(txn, legs, ctx, file);
    }

    // Opening balances: each register not yet seen in this file gets its own
    // `Opening Balance` record. What is left (registers that already have
    // history) is written normally, against one holding leg that balances it.
    let mut opened: Vec<(String, QifAccountType)> = Vec::new();
    let mut remaining: Vec<Leg> = Vec::new();
    let mut equity_names: Vec<&str> = Vec::new();
    for leg in legs {
        match leg.role {
            Role::Register(ty) => {
                let name = ctx.names.qif(&leg.account);
                if opened.iter().any(|(opened, _)| *opened == name) {
                    continue;
                }
                if file.touched.contains(&name) {
                    remaining.push(leg.clone());
                } else {
                    opened.push((name, ty));
                }
            }
            _ => {
                if !equity_names.contains(&leg.account.as_str()) {
                    equity_names.push(&leg.account);
                }
            }
        }
    }
    for (name, ty) in &opened {
        let own: Vec<&Leg> = registers
            .iter()
            .copied()
            .filter(|leg| sanitize(&leg.account) == *name)
            .collect();
        let total = sum(&own)?;
        let record = Record {
            date: txn.date.clone(),
            status: leg_status(&own, txn),
            code: txn.code.clone(),
            payee: OPENING_BALANCE_PAYEE.to_string(),
            memo: memo_of(txn, &own),
            total,
            splits: vec![Split {
                target: Target::Transfer(name.clone()),
                class: own.first().and_then(|leg| leg.class.clone()),
                memo: String::new(),
                from_comment: false,
                amount: total,
            }],
        };
        file.push(name, *ty, record);
        file.touched.insert(name.clone());
        ctx.loss.opening_balances += 1;
    }
    if remaining.is_empty() {
        return Ok(false);
    }
    let rest = sum(&remaining.iter().collect::<Vec<_>>())?;
    remaining.push(Leg {
        account: equity_names.join(", "),
        role: Role::Equity { conversion: false },
        quantity: rest.neg()?,
        status: txn.status,
        memo: String::new(),
        class: class_of(&[], txn),
        conversion: false,
    });
    write_record(txn, &remaining, ctx, file)
}

/// Write `legs` (one commodity, balanced) as a record in their home register,
/// plus mirror records under [`TransferPlacement::Mirrored`]. Returns whether
/// the holding account was used.
fn write_record(
    txn: &Transaction,
    legs: &[Leg],
    ctx: &mut Context<'_>,
    file: &mut FileBuilder,
) -> Result<bool, DecError> {
    // Each leg's register (QIF name and type) — the holding account for
    // equity — or `None` for a category.
    let placed: Vec<(Option<(String, QifAccountType)>, &Leg)> = legs
        .iter()
        .map(|leg| {
            let register = match leg.role {
                Role::Register(ty) => Some((ctx.names.qif(&leg.account), ty)),
                Role::Equity { .. } => {
                    Some((HOLDING_ACCOUNT.to_string(), QifAccountType::OtherAsset))
                }
                Role::Category { .. } => None,
            };
            (register, leg)
        })
        .collect();
    let registers: Vec<&(String, QifAccountType)> = placed
        .iter()
        .filter_map(|(register, _)| register.as_ref())
        .collect();
    let (home, home_type) = home_register(registers.iter().map(|(name, _)| name.as_str()))
        .and_then(|home| registers.iter().find(|(name, _)| name == home))
        .map_or_else(
            || (HOLDING_ACCOUNT.to_string(), QifAccountType::OtherAsset),
            |(name, ty)| (name.clone(), *ty),
        );
    let held = home == HOLDING_ACCOUNT || registers.iter().any(|(name, _)| name == HOLDING_ACCOUNT);

    let mut home_legs: Vec<&Leg> = Vec::new();
    let mut others: BTreeMap<String, (QifAccountType, Vec<&Leg>)> = BTreeMap::new();
    let mut splits: Vec<Split> = Vec::new();
    for (register, leg) in &placed {
        let target = match register {
            Some((name, _)) if *name == home => {
                home_legs.push(leg);
                continue;
            }
            Some((name, ty)) => {
                others
                    .entry(name.clone())
                    .or_insert_with(|| (*ty, Vec::new()))
                    .1
                    .push(leg);
                Target::Transfer(name.clone())
            }
            None => Target::Category(ctx.names.qif(&leg.account)),
        };
        let memo = match leg.role {
            // Name the equity account the money really came from.
            Role::Equity { .. } if !leg.conversion => {
                join_memo([leg.account.as_str(), leg.memo.as_str()])
            }
            _ => leg.memo.clone(),
        };
        splits.push(Split {
            target,
            class: leg.class.clone(),
            memo,
            from_comment: !leg.conversion && !leg.memo.is_empty(),
            amount: leg.quantity.neg()?,
        });
    }

    let mut memo = memo_of(txn, &home_legs);
    if let [only] = splits.as_slice()
        && !only.memo.is_empty()
    {
        if only.from_comment {
            ctx.loss.posting_comments_merged += 1;
        }
        memo = join_memo([memo.as_str(), only.memo.as_str()]);
    }
    let record = Record {
        date: txn.date.clone(),
        status: leg_status(&home_legs, txn),
        code: txn.code.clone(),
        payee: txn.description.clone(),
        memo,
        total: sum(&home_legs)?,
        splits,
    };
    file.push(&home, home_type, record);
    file.touched.insert(home.clone());

    for (name, (ty, own)) in others {
        file.touched.insert(name.clone());
        if ctx.placement == TransferPlacement::Mirrored {
            let total = sum(&own)?;
            let record = Record {
                date: txn.date.clone(),
                status: leg_status(&own, txn),
                code: txn.code.clone(),
                payee: txn.description.clone(),
                memo: memo_of(txn, &own),
                total,
                splits: vec![Split {
                    target: Target::Transfer(home.clone()),
                    class: own.first().and_then(|leg| leg.class.clone()),
                    memo: String::new(),
                    from_comment: false,
                    amount: total,
                }],
            };
            file.push(&name, ty, record);
        } else {
            // Listed in `!Account` so the transfer target exists on import.
            file.register(&name, ty);
        }
    }
    Ok(held)
}

/// A record's status: the first marked posting on its own register, else the
/// transaction's.
fn leg_status(own: &[&Leg], txn: &Transaction) -> Status {
    own.iter()
        .map(|leg| leg.status)
        .find(|status| *status != Status::Unmarked)
        .unwrap_or(txn.status)
}

/// A record's memo: the transaction comment's text, then its own postings'.
fn memo_of(txn: &Transaction, own: &[&Leg]) -> String {
    let txn_memo = free_text(&txn.comment);
    join_memo(std::iter::once(txn_memo.as_str()).chain(own.iter().map(|leg| leg.memo.as_str())))
}

fn join_memo<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for part in parts {
        if !part.is_empty() && !seen.contains(&part) {
            seen.push(part);
        }
    }
    seen.join("; ")
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// The commodity whose file comes first: the journal's `D` default commodity
/// when it has records, else the one with the most records (ties by name).
fn primary_commodity(
    journal: &Journal,
    files: &BTreeMap<Commodity, FileBuilder>,
) -> Option<Commodity> {
    journal
        .default_commodity
        .as_ref()
        .filter(|commodity| files.contains_key(*commodity))
        .cloned()
        .or_else(|| {
            files
                .iter()
                .max_by(|(a_name, a), (b_name, b)| {
                    a.records.cmp(&b.records).then_with(|| b_name.cmp(a_name))
                })
                .map(|(commodity, _)| commodity.clone())
        })
}

/// A file-name-safe spelling of a commodity symbol: ASCII letters and digits
/// kept, the common currency signs spelled out, anything else as its code
/// point. Made unique across one export.
fn unique_slug(commodity: &Commodity, used: &mut HashSet<String>) -> String {
    let base = commodity_slug(&commodity.0);
    let mut slug = base.clone();
    let mut n = 2;
    while !used.insert(slug.clone()) {
        slug = format!("{base}-{n}");
        n += 1;
    }
    slug
}

fn commodity_slug(symbol: &str) -> String {
    if symbol.is_empty() {
        return "no-commodity".to_string();
    }
    let slug: String = symbol
        .chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() => c.to_string(),
            '$' => "dollar".to_string(),
            '€' => "euro".to_string(),
            '£' => "pound".to_string(),
            '¥' => "yen".to_string(),
            '₹' => "rupee".to_string(),
            '-' | '_' | '.' => c.to_string(),
            other => format!("u{:04X}", u32::from(other)),
        })
        .collect();
    slug
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;

    fn export(text: &str) -> QifExport {
        export_with(text, DEFAULT_TRANSFER_PLACEMENT)
    }

    fn export_with(text: &str, transfers: TransferPlacement) -> QifExport {
        let journal = parse_journal(text, "test.journal").expect("parses");
        export_qif(
            &journal,
            &QifOptions {
                name: "t".into(),
                transfers,
            },
        )
        .expect("exports")
    }

    const DECLS: &str = "account assets:bank  ; type: C\n\
                         account assets:savings  ; type: C\n\
                         account liabilities:visa  ; type: L\n\
                         account liabilities:loan  ; type: L\n\
                         account expenses  ; type: X\n\
                         account income  ; type: R\n\
                         account equity  ; type: E\n\n";

    #[test]
    fn dates_amounts_and_status_are_formatted_for_qif() {
        assert_eq!(qif_date("2024-07-01"), "07/01/2024");
        assert_eq!(qif_amount(Dec::new(5, 0)), "5.00");
        assert_eq!(qif_amount(Dec::new(-1234567, 3)), "-1234.567");
        assert_eq!(status_field(Status::Unmarked), None);
        assert_eq!(status_field(Status::Pending), Some("*"));
        assert_eq!(status_field(Status::Cleared), Some("X"));
    }

    #[test]
    fn home_register_prefers_real_accounts_in_name_order() {
        assert_eq!(home_register(["b", "a"]), Some("a"));
        assert_eq!(home_register([HOLDING_ACCOUNT, "z"]), Some("z"));
        assert_eq!(home_register([HOLDING_ACCOUNT]), Some(HOLDING_ACCOUNT));
        assert_eq!(home_register([]), None);
    }

    #[test]
    fn a_split_purchase_is_one_record_with_s_lines() {
        let out = export(&format!(
            "{DECLS}2024-01-02 * (12) Shop  ; note, trip:x\n    expenses:food  $10.00  ; milk\n    expenses:home  $5\n    assets:bank\n"
        ));
        let text = &out.files[0].text;
        assert!(
            text.contains(
                "!Type:Bank\nD01/02/2024\nT-15.00\nCX\nN12\nPShop\nMnote\nLexpenses:food\n\
                 Sexpenses:food\nEmilk\n$-10.00\nSexpenses:home\n$-5.00\n^\n"
            ),
            "{text}"
        );
        assert_eq!(out.loss.transaction_tags, 1);
    }

    #[test]
    fn a_transfer_is_written_once_in_the_first_register() {
        let out = export(&format!(
            "{DECLS}2024-01-03 Move\n    assets:savings  $100\n    assets:bank\n"
        ));
        let text = &out.files[0].text;
        assert_eq!(text.matches("\nPMove\n").count(), 1, "{text}");
        assert!(
            text.contains("!Type:Bank\nD01/03/2024\nT-100.00\nPMove\nL[assets:savings]\n^\n"),
            "{text}"
        );
        assert_eq!(out.loss.records, 1);
    }

    #[test]
    fn mirrored_placement_writes_the_other_side_too() {
        let out = export_with(
            &format!("{DECLS}2024-01-03 Move\n    assets:savings  $100\n    assets:bank\n"),
            TransferPlacement::Mirrored,
        );
        let text = &out.files[0].text;
        assert!(
            text.contains("!Type:Bank\nD01/03/2024\nT100.00\nPMove\nL[assets:bank]\n^\n"),
            "{text}"
        );
        assert_eq!(out.loss.records, 2);
    }

    #[test]
    fn equity_against_new_registers_becomes_opening_balances() {
        let out = export(&format!(
            "{DECLS}2024-01-01 * Opening\n    assets:bank  $500\n    liabilities:visa  $-50\n    equity:opening\n\n\
             2024-02-01 Opening again\n    assets:bank  $5\n    equity:opening\n"
        ));
        let text = &out.files[0].text;
        assert!(
            text.contains("D01/01/2024\nT500.00\nCX\nPOpening Balance\nL[assets:bank]\n^\n"),
            "{text}"
        );
        assert!(
            text.contains("D01/01/2024\nT-50.00\nCX\nPOpening Balance\nL[liabilities:visa]\n^\n"),
            "{text}"
        );
        // A second equity entry on a register with history goes through the
        // holding account, naming the equity account.
        assert!(
            text.contains(
                "D02/01/2024\nT5.00\nPOpening again\nMequity:opening\nL[Ledgeline Adjustments]\n^\n"
            ),
            "{text}"
        );
        assert_eq!(out.loss.opening_balances, 2);
        assert_eq!(out.loss.holding_account_transactions, 1);
    }

    #[test]
    fn a_transaction_with_no_register_goes_to_the_holding_account() {
        let out = export(&format!(
            "{DECLS}2024-03-01 Reclass\n    expenses:a  $7\n    expenses:b  $-7\n"
        ));
        let text = &out.files[0].text;
        assert!(
            text.contains(
                "!Account\nNLedgeline Adjustments\nTOth A\n^\n!Type:Oth A\nD03/01/2024\nT0.00\n\
                 PReclass\nLexpenses:a\nSexpenses:a\n$-7.00\nSexpenses:b\n$7.00\n^\n"
            ),
            "{text}"
        );
        assert_eq!(out.loss.holding_account_transactions, 1);
    }

    #[test]
    fn each_commodity_gets_its_own_file_and_costs_become_cash() {
        let out = export(&format!(
            "{DECLS}D $1,000.00\naccount assets:broker  ; type: A\n\n\
             2024-01-02 Buy\n    assets:broker  10 AAPL @ $150\n    assets:bank  $-1500\n\n\
             2024-01-03 Trip\n    expenses:travel  20 EUR\n    liabilities:visa  -20 EUR\n\n\
             2024-01-04 Lunch\n    expenses:food  $3\n    liabilities:visa\n"
        ));
        let names: Vec<&str> = out.files.iter().map(|f| f.file_name.as_str()).collect();
        assert_eq!(names, ["t-dollar.qif", "t-EUR.qif"]);
        assert!(
            out.files[0].text.contains("L[assets:broker]\n"),
            "{}",
            out.files[0].text
        );
        assert!(!out.files[0].text.contains("AAPL"));
        assert_eq!(out.loss.cost_postings, 1);
        assert_eq!(out.loss.extra_commodity_files(), 1);
    }

    #[test]
    fn an_implicit_conversion_is_balanced_in_each_file() {
        let out = export(&format!(
            "{DECLS}2024-01-02 Swap\n    assets:bank  10 EUR\n    assets:savings  $-11\n"
        ));
        assert_eq!(out.loss.conversions, 1);
        for file in &out.files {
            assert!(
                file.text
                    .contains("Mcommodity conversion\nL[Ledgeline Adjustments]\n"),
                "{}",
                file.text
            );
        }
    }

    #[test]
    fn credit_cards_are_liabilities_used_for_purchases() {
        let out = export(&format!(
            "{DECLS}2024-01-02 A\n    expenses:food  $1\n    liabilities:visa\n\n\
             2024-01-03 B\n    expenses:food  $1\n    liabilities:visa\n\n\
             2024-01-04 Pay\n    liabilities:visa  $2\n    assets:bank\n\n\
             2024-01-05 Loan payment\n    liabilities:loan  $90\n    expenses:interest  $10\n    assets:bank\n"
        ));
        let text = &out.files[0].text;
        assert!(text.contains("Nliabilities:visa\nTCCard\n"), "{text}");
        assert!(text.contains("Nliabilities:loan\nTOth L\n"), "{text}");
        assert_eq!(out.loss.credit_cards, ["liabilities:visa"]);
    }

    #[test]
    fn a_qif_tag_overrides_the_type() {
        let out = export(
            "account assets:wallet  ; type: C, qif: Cash\naccount expenses  ; type: X\n\n\
             2024-01-02 Coffee\n    expenses:coffee  $3\n    assets:wallet\n",
        );
        assert!(
            out.files[0].text.contains("!Type:Cash\n"),
            "{}",
            out.files[0].text
        );
        // `qif:` is ours, not an account tag the export loses.
        assert_eq!(out.loss.account_tags, 0);
    }

    #[test]
    fn the_class_tag_becomes_the_qif_class() {
        let out = export(&format!(
            "{DECLS}2024-01-02 Sale  ; class: Retail\n    assets:bank  $10\n    income:sales\n"
        ));
        let text = &out.files[0].text;
        assert!(text.starts_with("!Type:Class\nNRetail\n^\n"), "{text}");
        assert!(text.contains("Lincome:sales/Retail\n"), "{text}");
        assert!(text.contains("Nincome:sales\nI\n^\n"), "{text}");
        assert_eq!(out.loss.transaction_tags, 0);
    }

    #[test]
    fn untyped_accounts_and_renames_are_reported() {
        let out = export(&format!(
            "{DECLS}2024-01-02 Odd\n    misc/stuff  $1\n    assets:bank\n"
        ));
        assert_eq!(out.loss.untyped_accounts, ["misc/stuff"]);
        assert_eq!(
            out.loss.renamed_accounts,
            [("misc/stuff".to_string(), "misc-stuff".to_string())]
        );
        assert!(out.files[0].text.contains("Lmisc-stuff\n"));
    }

    #[test]
    fn dropped_features_are_counted() {
        let out = export(&format!(
            "{DECLS}P 2024-01-01 AAPL $100\n~ monthly\n    expenses:food  $100\n    assets:bank\n\n\
             2024-01-02=2024-01-03 Thing\n    expenses:food  $1  ; date: 2024-01-05\n    (budget)  $1\n    assets:bank  = $-1\n"
        ));
        let loss = &out.loss;
        assert_eq!(loss.price_directives, 1);
        assert_eq!(loss.periodic_transactions, 1);
        assert_eq!(loss.secondary_dates, 1);
        assert_eq!(loss.posting_dates, 1);
        assert_eq!(loss.virtual_postings, 1);
        assert_eq!(loss.balance_assertions, 1);
        assert_eq!(loss.posting_tags, 0);
    }

    #[test]
    fn commodity_slugs_are_file_safe_and_unique() {
        let used = &mut HashSet::new();
        assert_eq!(unique_slug(&Commodity("$".into()), used), "dollar");
        assert_eq!(unique_slug(&Commodity("€".into()), used), "euro");
        assert_eq!(
            unique_slug(&Commodity("\"ABC 1\"".into()), used),
            "u0022ABCu00201u0022"
        );
        assert_eq!(unique_slug(&Commodity(String::new()), used), "no-commodity");
        assert_eq!(unique_slug(&Commodity("dollar".into()), used), "dollar-2");
    }
}
