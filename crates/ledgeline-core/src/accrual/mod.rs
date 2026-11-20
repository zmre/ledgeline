//! Accrual recognition: deferred revenue and prepaid expenses recognized over
//! a term, and invoices and bills matched to their payments
//! (`plans/45-accrual-recognition.md`).
//!
//! A cash-basis or household ledger never sees any of this: [`accrual_gate`]
//! decides whether the helpers apply at all (D7).
//!
//! # How a schedule is kept
//!
//! - The **source** is an ordinary transaction whose deferred-revenue credit
//!   (or prepaid debit) carries [`RECOGNITION_TAG`] plus the schedule's terms
//!   ([`SERVICE_START_TAG`], [`TERM_MONTHS_TAG`] or [`SERVICE_END_TAG`],
//!   [`PRORATION_TAG`]).
//! - The **rules** are plain hledger `~` rules in a recognition file the main
//!   journal does **not** include ([`book`]; D1), so they are never budget
//!   goals and never reach a projection (D2).
//! - Each period is **materialized** as a real transaction in the main
//!   journal ([`due`]), tagged with the schedule id and the period it fills
//!   ([`RECOGNIZED_TAG`]). Nothing is ever posted for a date after `today`.
//!
//! Everything that writes lives in [`ops`], so Ledgeline's server and any
//! other program built on this crate (an invoicing app) go through exactly the
//! same checks. Nothing in this module holds global state, and `today` is
//! always a parameter.
//!
//! # Example
//!
//! Build the schedule for a $12,000 annual prepayment and read its rules
//! back:
//!
//! ```
//! use ledgeline_core::accrual::schedule::{
//!     Party, PartyKind, Proration, ScheduleLine, ScheduleSpec, Term, build,
//! };
//! use ledgeline_core::parse::{JournalStyles, parse_journal};
//!
//! let journal = parse_journal("commodity $1,000.00\n", "books.journal").unwrap();
//! let total = parse_journal(
//!     "2026-11-17 x\n    liabilities:deferred-revenue  $-12000.00\n    assets:bank\n",
//!     "x.journal",
//! )
//! .unwrap()
//! .transactions[0]
//!     .postings[0]
//!     .amounts[0]
//!     .clone();
//! let spec = ScheduleSpec {
//!     id: "k7f3a2".into(),
//!     lines: vec![ScheduleLine {
//!         source_account: "liabilities:deferred-revenue".into(),
//!         target_account: "revenue:sales".into(),
//!         total,
//!         party: Some(Party { kind: PartyKind::Customer, name: "Customer X".into() }),
//!     }],
//!     start: "2026-11-17".into(),
//!     term: Term::Months(12),
//!     proration: Proration::Anniversary,
//!     description: "Customer X Recognized".into(),
//!     header_tags: vec![],
//! };
//! let schedule = build(&spec, &JournalStyles::of(&journal)).unwrap();
//! assert_eq!(schedule.periods.len(), 12);
//! assert_eq!(schedule.periods[0].date, "2026-11-17");
//! assert_eq!(schedule.periods[11].date, "2027-10-17");
//! assert_eq!(schedule.renewal_date, "2027-11-17");
//! assert!(schedule.rules[0].text.starts_with("~ monthly from 2026-11-17 to 2027-10-18"));
//! ```

pub mod book;
pub mod books_config;
pub mod documents;
pub mod due;
pub mod index;
pub mod invoice;
pub mod ops;
pub mod receivables;
pub mod restate;
pub mod schedule;
pub mod settle;

use crate::model::{AccountDeclaration, Journal, PeriodicTransaction, Transaction};
use crate::projections::is_generated;
use crate::reports::account_decls_from;
use crate::reports::account_types::{AccountType, AccountTypes, root_type};
use crate::reports::kpi::{
    KpiRole, declared_kpi_roles, has_declared_descendant, last_segment_contains,
    nearest_declared_role,
};
use crate::reports::periods::{civil_from_days, days_from_civil};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Tag vocabulary (D-vocab)
// ---------------------------------------------------------------------------

/// The schedule link: on a source posting, on every rule header, and on every
/// posted entry's and adjustment's header.
pub const RECOGNITION_TAG: &str = "recognition";
/// The [`RECOGNITION_TAG`] value on a source posting meaning "don't schedule
/// this, don't ask again".
pub const NONE_VALUE: &str = "none";
/// First day of service, `YYYY-MM-DD`, on a source posting. Defaults to the
/// transaction date.
pub const SERVICE_START_TAG: &str = "service-start";
/// Length of service in whole months, on a source posting.
pub const TERM_MONTHS_TAG: &str = "term-months";
/// Last day of service, inclusive, on a source posting; replaces
/// [`TERM_MONTHS_TAG`] for a term that isn't whole months (or ends early).
pub const SERVICE_END_TAG: &str = "service-end";
/// `calendar` on a source posting: calendar-month periods with day-prorated
/// stubs (D-anchor). Absent means anniversary periods.
pub const PRORATION_TAG: &str = "proration";
/// The [`PRORATION_TAG`] value for calendar months.
pub const CALENDAR_VALUE: &str = "calendar";
/// `new`, `renewal`, `expansion` or `reactivation`, on a source posting (Q5).
pub const CONTRACT_TAG: &str = "contract";
/// The customer a revenue line belongs to (plan 37's attribution).
pub const CUSTOMER_TAG: &str = "customer";
/// The vendor an expense line belongs to.
pub const VENDOR_TAG: &str = "vendor";
/// The invoice or bill number, on the invoice's and the payment's headers.
pub const INVOICE_TAG: &str = "invoice";
/// On a document's header: the date it falls due, `YYYY-MM-DD` (receivables
/// aging; invoicer ARCHITECTURE §4.2).
pub const DUE_TAG: &str = "due";
/// On a document's header: where its source lives (the invoicer's markdown
/// path).
pub const DOCUMENT_TAG: &str = "document";
/// On a document's header: the customer's reference (`PO #105`).
pub const REFERENCE_TAG: &str = "reference";
/// On a document's line: the row's description.
pub const ITEM_TAG: &str = "item";
/// On a void reversal's header: the date the document was voided. A
/// transaction carrying it is never itself a document; its settlement
/// posting settles the one it reverses ([`receivables`]).
pub const VOID_TAG: &str = "void";
/// Service start + term, on the main rule's header.
pub const RENEWAL_DATE_TAG: &str = "renewal-date";
/// The **scheduled** occurrence date a posted entry fills.
pub const RECOGNIZED_TAG: &str = "recognized";
/// On a restatement's catch-up adjustment: the date of the restatement.
pub const RESTATED_TAG: &str = "restated";
/// On a restatement's catch-up adjustment: the `closed-through` date in force
/// when it was written. Every occurrence of the schedule on or before it is
/// accounted for by the adjustment, so none of them is ever due again (D4's
/// "restated cut").
pub const RESTATED_THROUGH_TAG: &str = "restated-through";

/// The `contract:` vocabulary (Q5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Contract {
    /// A first contract.
    New,
    /// A contract following an earlier one with no gap.
    Renewal,
    /// The new line of an upgrade (the credit-back case).
    Expansion,
    /// A renewal after a gap.
    Reactivation,
}

impl Contract {
    /// Every value, in vocabulary order.
    pub const ALL: [Contract; 4] = [
        Contract::New,
        Contract::Renewal,
        Contract::Expansion,
        Contract::Reactivation,
    ];

    /// The tag value.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Renewal => "renewal",
            Self::Expansion => "expansion",
            Self::Reactivation => "reactivation",
        }
    }

    /// Parse a tag value, trimmed and ASCII case-insensitive.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|contract| contract.code().eq_ignore_ascii_case(value))
    }
}

/// The first value of tag `name` in `tags`, trimmed; `None` when absent.
#[must_use]
pub fn tag_value<'t>(tags: &'t [(String, String)], name: &str) -> Option<&'t str> {
    tags.iter()
        .find(|(tag, _)| tag == name)
        .map(|(_, value)| value.trim())
}

// ---------------------------------------------------------------------------
// Accrual roles (D-gate)
// ---------------------------------------------------------------------------

/// What an account does in accrual books.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AccrualRole {
    /// A contract liability, recognized into revenue (`kpi: deferred-revenue`).
    DeferredRevenue,
    /// A prepaid expense, recognized into expense (`kpi: prepaid`).
    Prepaid,
    /// Trade receivables: invoices debit it, payments credit it.
    Receivable,
    /// Trade payables: bills credit it, payments debit it.
    Payable,
}

impl AccrualRole {
    /// The `kpi:` value that declares it.
    #[must_use]
    pub fn code(self) -> &'static str {
        self.kpi().code()
    }

    /// The `kpi:` role that declares it.
    #[must_use]
    pub fn kpi(self) -> KpiRole {
        match self {
            Self::DeferredRevenue => KpiRole::DeferredRevenue,
            Self::Prepaid => KpiRole::Prepaid,
            Self::Receivable => KpiRole::Receivable,
            Self::Payable => KpiRole::Payable,
        }
    }

    /// Whether a schedule recognizes out of this account.
    #[must_use]
    pub fn is_deferral(self) -> bool {
        matches!(self, Self::DeferredRevenue | Self::Prepaid)
    }

    /// Whether invoices or bills settle through this account (D-settle).
    #[must_use]
    pub fn is_settlement(self) -> bool {
        matches!(self, Self::Receivable | Self::Payable)
    }

    fn of_kpi(role: KpiRole) -> Option<Self> {
        match role {
            KpiRole::DeferredRevenue => Some(Self::DeferredRevenue),
            KpiRole::Prepaid => Some(Self::Prepaid),
            KpiRole::Receivable => Some(Self::Receivable),
            KpiRole::Payable => Some(Self::Payable),
            _ => None,
        }
    }
}

/// How an account came by its [`AccrualRole`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoleSource {
    /// A `kpi:` on the account or its nearest declaring ancestor.
    Declared,
    /// The account's last segment, read against its effective type.
    Inferred,
}

/// The name inference of D-gate step 2, for an account with no `kpi:` on it,
/// its ancestors or its descendants.
fn inferred_accrual_role(root: AccountType, account: &str) -> Option<AccrualRole> {
    match root {
        AccountType::Asset if last_segment_contains(account, "receivable") => {
            Some(AccrualRole::Receivable)
        }
        AccountType::Asset if last_segment_contains(account, "prepaid") => {
            Some(AccrualRole::Prepaid)
        }
        AccountType::Liability if last_segment_contains(account, "payable") => {
            Some(AccrualRole::Payable)
        }
        AccountType::Liability
            if last_segment_contains(account, "unearned")
                || (last_segment_contains(account, "deferred")
                    && (last_segment_contains(account, "revenue")
                        || last_segment_contains(account, "income"))) =>
        {
            Some(AccrualRole::DeferredRevenue)
        }
        _ => None,
    }
}

/// The accrual role resolver (D-gate): the nearest declared `kpi:` on the
/// account or an ancestor (a declared role that isn't an accrual one answers
/// `None`, never a name guess); else, with no `kpi:` in the subtree, the name
/// inference — receivable and payable as plan 37 infers them, plus `prepaid`
/// on an asset and `unearned` (or `deferred` with `revenue`/`income` in the
/// same segment) on a liability.
///
/// Insights keeps plan 37's narrower inference ([`crate::reports::kpi_resolver`]).
pub fn accrual_resolver<'a>(
    declared: &'a BTreeMap<String, KpiRole>,
    types: &'a AccountTypes,
) -> impl Fn(&str) -> Option<(AccrualRole, RoleSource)> + 'a {
    move |account: &str| {
        if let Some(role) = nearest_declared_role(declared, account) {
            return AccrualRole::of_kpi(role).map(|role| (role, RoleSource::Declared));
        }
        if has_declared_descendant(declared, account) {
            return None;
        }
        let root = types.resolve(account).map(root_type)?;
        inferred_accrual_role(root, account).map(|role| (role, RoleSource::Inferred))
    }
}

/// Every account a journal declares or posts to that has an [`AccrualRole`],
/// resolved once. Immutable and `Sync`, so one can be shared by every reader
/// of a snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccrualRoles {
    roles: BTreeMap<String, (AccrualRole, RoleSource)>,
}

impl AccrualRoles {
    /// Resolve every account `journal` declares or posts to.
    #[must_use]
    pub fn of(journal: &Journal) -> Self {
        Self::of_accounts(&journal.accounts, account_names(journal))
    }

    /// Resolve `names` under the declarations in `accounts`.
    #[must_use]
    pub fn of_accounts<'n>(
        accounts: &[AccountDeclaration],
        names: impl IntoIterator<Item = &'n str>,
    ) -> Self {
        let declared = declared_kpi_roles(accounts);
        let types = AccountTypes::new(&account_decls_from(accounts));
        let resolve = accrual_resolver(&declared, &types);
        Self {
            roles: names
                .into_iter()
                .filter_map(|name| resolve(name).map(|role| (name.to_string(), role)))
                .collect(),
        }
    }

    /// The role of `account`, when it has one.
    #[must_use]
    pub fn role(&self, account: &str) -> Option<AccrualRole> {
        self.roles.get(account).map(|(role, _)| *role)
    }

    /// The role and how it was decided.
    #[must_use]
    pub fn get(&self, account: &str) -> Option<(AccrualRole, RoleSource)> {
        self.roles.get(account).copied()
    }

    /// Every account with a role, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, AccrualRole, RoleSource)> {
        self.roles
            .iter()
            .map(|(name, (role, source))| (name.as_str(), *role, *source))
    }

    /// The receivable and payable accounts: where invoices settle (D-settle).
    #[must_use]
    pub fn settlement_accounts(&self) -> BTreeSet<String> {
        self.iter()
            .filter(|(_, role, _)| role.is_settlement())
            .map(|(name, _, _)| name.to_string())
            .collect()
    }

    /// Whether no account has a role.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty()
    }
}

/// Every account `journal` declares or posts to, deduplicated. Hashed rather
/// than sorted: a 200k journal has half a million postings and a few hundred
/// accounts, and sorting the postings' names was most of an index build.
fn account_names(journal: &Journal) -> FxHashSet<&str> {
    journal
        .accounts
        .iter()
        .map(|decl| decl.name.0.as_str())
        .chain(journal.transactions.iter().flat_map(|txn| {
            txn.postings
                .iter()
                .map(|posting| posting.account.0.as_str())
        }))
        .collect()
}

/// How many cases a property test in this module runs: `default`, or
/// `PROPTEST_CASES` when set (proptest's own variable, which an explicit
/// `with_cases` would otherwise ignore). Keeps the development loop quick and
/// a deeper run one variable away.
#[cfg(test)]
pub(crate) fn test_cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
        .unwrap_or(default)
}

/// `fixtures/accrual`, parsed once per test binary and shared: the journal,
/// its recognition book, and their index.
#[cfg(test)]
pub(crate) fn test_fixture() -> &'static (Journal, book::RecognitionBook, index::RecognitionIndex) {
    static FIXTURE: std::sync::OnceLock<(Journal, book::RecognitionBook, index::RecognitionIndex)> =
        std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| {
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/accrual");
        let main = dir.join("books.journal");
        let journal = crate::parse::parse_journal(
            &std::fs::read_to_string(&main).expect("fixture"),
            &main.to_string_lossy(),
        )
        .expect("parses");
        let rules = dir.join("recognition.journal");
        let book = book::RecognitionBook::parse(
            &std::fs::read_to_string(&rules).expect("fixture"),
            &rules,
            &journal.commodity_styles,
        )
        .expect("parses");
        let index = index::RecognitionIndex::build(&journal, &book);
        (journal, book, index)
    })
}

// ---------------------------------------------------------------------------
// A fast hasher for internal keys
// ---------------------------------------------------------------------------

/// The Fx hash (rustc's): a multiply and a rotate per word. For keys this
/// module builds from a journal it has already parsed — ids, day numbers,
/// account names — where SipHash's flooding resistance buys nothing and
/// costs most of a lookup.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FxHasher(u64);

const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
    }
}

impl std::hash::Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_i32(&mut self, value: i32) {
        self.add(u64::from(value.cast_unsigned()));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// [`FxHasher`] as a `BuildHasher`.
pub(crate) type FxBuild = std::hash::BuildHasherDefault<FxHasher>;
/// A `HashMap` keyed through [`FxHasher`].
pub(crate) type FxHashMap<K, V> = std::collections::HashMap<K, V, FxBuild>;
/// A `HashSet` keyed through [`FxHasher`].
pub(crate) type FxHashSet<K> = std::collections::HashSet<K, FxBuild>;

/// `ledgeline.toml`'s `[books] basis`: an override of the gate, both ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Basis {
    /// Never show the helpers.
    Cash,
    /// Always show them, even before any account is declared.
    Accrual,
}

impl Basis {
    /// The setting's value.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Cash => "cash",
            Self::Accrual => "accrual",
        }
    }

    /// Parse a setting value, trimmed and ASCII case-insensitive.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Cash, Self::Accrual]
            .into_iter()
            .find(|basis| basis.code().eq_ignore_ascii_case(value.trim()))
    }
}

/// Why the gate is open or shut — what the UI shows the first time ("because
/// `liabilities:taxes payable` looks like a payable").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateReason {
    /// `[books] basis` says so.
    Basis(Basis),
    /// An account declares an accrual `kpi:` (the first, by name).
    Declared {
        /// The account.
        account: String,
        /// Its role.
        role: AccrualRole,
    },
    /// An account's name looks like one (the first, by name).
    Inferred {
        /// The account.
        account: String,
        /// The role it looks like.
        role: AccrualRole,
    },
    /// No account has an accrual role.
    NoAccrualAccounts,
}

/// The gate's verdict (D-gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccrualGate {
    /// Whether the helpers apply to this ledger.
    pub enabled: bool,
    /// Why.
    pub reason: GateReason,
}

/// D-gate: the ledger is accrual when any account resolves to an
/// [`AccrualRole`] (a declared one is preferred as the reason), unless
/// `basis` overrides it either way.
#[must_use]
pub fn accrual_gate(roles: &AccrualRoles, basis: Option<Basis>) -> AccrualGate {
    if let Some(basis) = basis {
        return AccrualGate {
            enabled: basis == Basis::Accrual,
            reason: GateReason::Basis(basis),
        };
    }
    let declared = roles
        .iter()
        .find(|(_, _, source)| *source == RoleSource::Declared);
    let reason = match declared.or_else(|| roles.iter().next()) {
        Some((account, role, RoleSource::Declared)) => GateReason::Declared {
            account: account.to_string(),
            role,
        },
        Some((account, role, RoleSource::Inferred)) => GateReason::Inferred {
            account: account.to_string(),
            role,
        },
        None => GateReason::NoAccrualAccounts,
    };
    AccrualGate {
        enabled: reason != GateReason::NoAccrualAccounts,
        reason,
    }
}

/// Whether the accrual helpers apply to `journal` under `basis` (D-gate).
#[must_use]
pub fn is_accrual_ledger(journal: &Journal, basis: Option<Basis>) -> bool {
    accrual_gate(&AccrualRoles::of(journal), basis).enabled
}

// ---------------------------------------------------------------------------
// Which transactions and rules belong to recognition
// ---------------------------------------------------------------------------

/// A posted recognition entry: a real transaction carrying both
/// [`RECOGNITION_TAG`] and [`RECOGNIZED_TAG`] on its header.
#[must_use]
pub fn is_posted_entry(txn: &Transaction) -> bool {
    !is_generated(txn)
        && tag_value(&txn.tags, RECOGNITION_TAG).is_some()
        && tag_value(&txn.tags, RECOGNIZED_TAG).is_some()
}

/// A restatement's catch-up adjustment: [`RECOGNITION_TAG`] and
/// [`RESTATED_TAG`] on the header.
#[must_use]
pub fn is_adjustment(txn: &Transaction) -> bool {
    !is_generated(txn)
        && tag_value(&txn.tags, RECOGNITION_TAG).is_some()
        && tag_value(&txn.tags, RESTATED_TAG).is_some()
}

/// Anything Ledgeline wrote for a schedule: a posted entry or an adjustment.
/// Review queues and subscription detection skip these (Q6; "Other readers").
#[must_use]
pub fn is_recognition_entry(txn: &Transaction) -> bool {
    is_posted_entry(txn) || is_adjustment(txn)
}

/// A `~` rule whose header carries [`RECOGNITION_TAG`]: a schedule, never a
/// budget goal (D-guards).
#[must_use]
pub fn is_recognition_rule(rule: &PeriodicTransaction) -> bool {
    tag_value(&rule.tags, RECOGNITION_TAG).is_some()
}

/// `rules` without the recognition rules — borrowed, with no copy, in the
/// usual case of there being none. The defensive filter every budget-goal
/// reader applies (D-guards).
#[must_use]
pub fn without_recognition_rules(rules: &[PeriodicTransaction]) -> Cow<'_, [PeriodicTransaction]> {
    if rules.iter().any(is_recognition_rule) {
        Cow::Owned(
            rules
                .iter()
                .filter(|rule| !is_recognition_rule(rule))
                .cloned()
                .collect(),
        )
    } else {
        Cow::Borrowed(rules)
    }
}

// ---------------------------------------------------------------------------
// Day numbers
// ---------------------------------------------------------------------------

/// Days since 1970-01-01 of an ISO `YYYY-MM-DD` date.
#[must_use]
pub(crate) fn day_of(date: &str) -> i32 {
    // The common case, `YYYY-MM-DD` in ASCII digits, read byte by byte: this
    // runs several times per posted entry.
    let bytes = date.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        bytes.get(range).and_then(|slice| {
            slice.iter().try_fold(0_i64, |value, byte| {
                byte.is_ascii_digit()
                    .then(|| value * 10 + i64::from(byte - b'0'))
            })
        })
    };
    let fast = (bytes.len() >= 10).then(|| Some((digits(0..4)?, digits(5..7)?, digits(8..10)?)));
    let (year, month, day) = match fast.flatten() {
        Some(parts) => parts,
        None => {
            let field = |range: std::ops::Range<usize>| -> i64 {
                date.get(range).and_then(|s| s.parse().ok()).unwrap_or(0)
            };
            (field(0..4), field(5..7), field(8..10))
        }
    };
    i32::try_from(days_from_civil(year, month, day)).unwrap_or(i32::MAX)
}

/// The ISO date of a day number.
#[must_use]
pub(crate) fn iso_of(day: i32) -> String {
    let (year, month, day) = civil_from_days(i64::from(day));
    let narrow = |value: i64| i32::try_from(value).unwrap_or(i32::MAX);
    crate::parse::iso_ymd((narrow(year), narrow(month), narrow(day)))
}

/// Whether `date` is a well-formed ISO date that names a real day.
#[must_use]
pub(crate) fn is_iso_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
        && iso_of(day_of(date)) == date
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;

    fn roles(text: &str) -> AccrualRoles {
        AccrualRoles::of(&parse_journal(text, "inline.journal").expect("parses"))
    }

    #[test]
    fn declared_roles_win_and_a_declared_non_accrual_role_is_never_inferred() {
        let r = roles(
            "account assets:prepaid  ; type: A, kpi: prepaid\n\
             account liabilities:deferred revenue  ; type: L, kpi: recurring\n\
             account liabilities:contract  ; type: L, kpi: deferred-revenue\n\
             account liabilities:contract:annual\n",
        );
        assert_eq!(
            r.get("assets:prepaid"),
            Some((AccrualRole::Prepaid, RoleSource::Declared))
        );
        assert_eq!(r.role("liabilities:deferred revenue"), None);
        assert_eq!(
            r.get("liabilities:contract:annual"),
            Some((AccrualRole::DeferredRevenue, RoleSource::Declared))
        );
    }

    #[test]
    fn names_infer_only_without_kpi_in_the_subtree_and_on_type() {
        let r = roles(
            "account assets:accounts receivable  ; type: A\n\
             account assets:prepaid expenses  ; type: A\n\
             account liabilities:unearned income  ; type: L\n\
             account liabilities:deferred-revenue  ; type: L\n\
             account liabilities:deferred-tax  ; type: L\n\
             account liabilities:deferred:revenue  ; type: L\n\
             account expenses:prepaid  ; type: X\n\
             account liabilities:accounts payable  ; type: L\n\
             account assets:prepaid:tagged  ; kpi: ga\n",
        );
        assert_eq!(
            r.role("assets:accounts receivable"),
            Some(AccrualRole::Receivable)
        );
        assert_eq!(
            r.role("assets:prepaid expenses"),
            Some(AccrualRole::Prepaid)
        );
        assert_eq!(
            r.role("liabilities:unearned income"),
            Some(AccrualRole::DeferredRevenue)
        );
        assert_eq!(
            r.role("liabilities:deferred-revenue"),
            Some(AccrualRole::DeferredRevenue)
        );
        assert_eq!(r.role("liabilities:deferred-tax"), None);
        assert_eq!(r.role("liabilities:deferred:revenue"), None);
        assert_eq!(r.role("expenses:prepaid"), None);
        assert_eq!(
            r.role("liabilities:accounts payable"),
            Some(AccrualRole::Payable)
        );
        // A declared descendant switches the parent's inference off.
        assert_eq!(r.role("assets:prepaid"), None);
        assert!(
            r.get("assets:accounts receivable")
                .is_some_and(|(_, s)| s == RoleSource::Inferred)
        );
    }

    #[test]
    fn basis_overrides_both_ways() {
        let household = roles("2026-01-01 x\n    expenses:food  $5\n    assets:cash\n");
        assert!(!accrual_gate(&household, None).enabled);
        assert_eq!(
            accrual_gate(&household, None).reason,
            GateReason::NoAccrualAccounts
        );
        assert!(accrual_gate(&household, Some(Basis::Accrual)).enabled);
        let business = roles("account assets:ar  ; type: A, kpi: receivable\n");
        assert!(accrual_gate(&business, None).enabled);
        assert!(!accrual_gate(&business, Some(Basis::Cash)).enabled);
        let inferred = roles("account liabilities:taxes payable  ; type: L\n");
        assert_eq!(
            accrual_gate(&inferred, None).reason,
            GateReason::Inferred {
                account: "liabilities:taxes payable".into(),
                role: AccrualRole::Payable
            }
        );
    }

    #[test]
    fn contract_and_basis_parse() {
        for contract in Contract::ALL {
            assert_eq!(
                Contract::parse(&contract.code().to_uppercase()),
                Some(contract)
            );
        }
        assert_eq!(Contract::parse("upsell"), None);
        assert_eq!(Basis::parse(" Accrual "), Some(Basis::Accrual));
        assert_eq!(Basis::parse("modified"), None);
    }

    #[test]
    fn day_numbers_round_trip_and_reject_impossible_dates() {
        for date in ["1970-01-01", "2024-02-29", "2026-11-17", "2099-12-31"] {
            assert_eq!(iso_of(day_of(date)), date);
            assert!(is_iso_date(date));
        }
        for bad in ["2026-02-30", "2026-13-01", "2026/01/01", "26-01-01", ""] {
            assert!(!is_iso_date(bad), "{bad}");
        }
    }

    #[test]
    fn recognition_predicates() {
        let j = parse_journal(
            "2026-01-01 a  ; recognition: abc123, recognized: 2026-01-01\n    x  $1\n    y\n\
             2026-01-02 b  ; recognition: abc123, restated: 2026-01-02\n    x  $1\n    y\n\
             2026-01-03 c  ; recognition: abc123\n    x  $1\n    y\n",
            "inline.journal",
        )
        .expect("parses");
        let flags: Vec<(bool, bool, bool)> = j
            .transactions
            .iter()
            .map(|t| {
                (
                    is_posted_entry(t),
                    is_adjustment(t),
                    is_recognition_entry(t),
                )
            })
            .collect();
        assert_eq!(
            flags,
            vec![
                (true, false, true),
                (false, true, true),
                (false, false, false)
            ]
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(test_cases(512)))]

        /// The byte-level `day_of` reads every string as the field parse did.
        #[test]
        fn day_of_matches_the_field_parse(date in "[0-9+ -]{0,12}|[0-9]{4}-[0-9]{2}-[0-9]{2}[0-9T:]{0,3}") {
            let field = |range: std::ops::Range<usize>| -> i64 {
                date.get(range).and_then(|s| s.parse().ok()).unwrap_or(0)
            };
            let reference = i32::try_from(days_from_civil(field(0..4), field(5..7), field(8..10))).unwrap_or(i32::MAX);
            proptest::prop_assert_eq!(day_of(&date), reference);
        }
    }
}
