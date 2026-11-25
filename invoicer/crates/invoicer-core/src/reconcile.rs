//! The reconciler (ARCHITECTURE §5.4): the books' payment state of every
//! sent invoice, mirrored into its `paid:` field, and every disagreement
//! between the books and the markdown (§4.3, §4.5).
//!
//! # Where the payment state comes from
//!
//! [`BookStates::build`] reads the journal once:
//!
//! - **Accrual books**: an invoice's receivable posting and its settlement
//!   links (`match:`), through Ledgeline's
//!   [`Receivables`](ledgeline_core::accrual::receivables): one invoice may
//!   be paid by several payments (N:1); a void reversal settles it as void.
//! - **Cash books**: the deposits Apply payment re-split and tagged
//!   `invoice:` (no `document:`, no `void:`); each one's received amount is
//!   what its `customer:`-tagged postings off the cash accounts hold.
//! - **A zero total** (a final invoice its deposit covers) is paid on its
//!   date, in either basis: nothing is owed.
//!
//! # The mirror
//!
//! [`BooksPaid`] is the `paid:` grammar (I7): `no`, `partial 2026-11-05
//! $1,000, 2026-11-20 $500` (payments on one day merged, amounts in the
//! table's money style, whole amounts without cents), or the date the last
//! payment completed it. A mirror that already says the same — the same
//! dates and amounts, however the amounts are written — is left alone, so
//! reconciling twice writes nothing. Only sent invoices are mirrored; a sent
//! invoice the books can't speak for (accrual, not booked) keeps its mirror
//! and gets a finding.
//!
//! # Findings
//!
//! [`audit`] is what `check` adds over the books, and what the reconciler
//! reports: an invoice number booked twice, a sent invoice with no
//! transaction, a transaction with no invoice file, a `document:` path that
//! no longer names its file, a draft already booked (a send whose markdown
//! write failed: run `send` to finish), a void on one side only, a stale
//! `paid:` mirror, and — comparing each booked invoice's markdown,
//! re-derived, with its transaction — books out of date (run `rebook`).
//!
//! Nothing is ever re-booked here: the only write is `paid:`, one key, the
//! rest of the file byte for byte, conditional on the revision read.

use crate::books::spec::{DOCUMENT_TAG, tag_text};
use crate::books::{Basis, BooksError, InvoiceBooks, StepCheck};
use crate::doc::{Paid, Status};
use crate::frontmatter::FieldEdit;
use crate::index::{DocumentIndex, IndexedDoc};
use crate::problem::{FixEdit, Problem, ProblemCode};
use crate::table::money::MoneyStyle;
use ledgeline_core::Dec;
use ledgeline_core::accrual::documents::document_posting;
use ledgeline_core::accrual::ops::Loaded;
use ledgeline_core::accrual::receivables::{
    PaymentState, Receivables, ReceivablesOptions, SettlementKind,
};
use ledgeline_core::accrual::{
    AccrualRole, AccrualRoles, CUSTOMER_TAG, INVOICE_TAG, VOID_TAG, is_recognition_entry, tag_value,
};
use ledgeline_core::model::{Journal, Tindex};
use ledgeline_core::projections::is_generated;
use ledgeline_core::reports::{AccountTypes, account_decls};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Where a transaction is in the journal, for findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Its position in `journal.transactions`.
    pub position: usize,
    /// Its id in this load.
    pub tindex: Tindex,
    /// Its date.
    pub date: String,
    /// The file it is in (the journal's key).
    pub file: PathBuf,
    /// Its first line.
    pub line: u32,
}

/// What an invoice's receivable posting holds (accrual books).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivableState {
    /// The receivable account.
    pub account: String,
    /// The commodity.
    pub commodity: String,
    /// The amount booked.
    pub amount: Dec,
    /// What is left to settle.
    pub outstanding: Dec,
    /// Open, partial, paid or void.
    pub state: PaymentState,
    /// Its payments and credits (not its void), oldest first: `(date,
    /// amount)`.
    pub payments: Vec<(String, Dec)>,
    /// The same payments and credits, with the transaction each is in.
    pub settled_by: Vec<SettledBy>,
}

/// One payment or credit applied to a receivable: where it is, for the
/// Books panel.
#[derive(Debug, Clone)]
pub struct SettledBy {
    /// The settling transaction's position in `journal.transactions`.
    pub position: usize,
    /// Its date (the posting's own, when it has one).
    pub date: String,
    /// The part applied to this invoice.
    pub amount: Dec,
    /// A credit (a credit note, a write-off), not money.
    pub credit: bool,
}

/// Where a payment is in the journal is not its state: an insertion above
/// it moves it without changing what it paid ([`BookStates::changed_since`]).
impl PartialEq for SettledBy {
    fn eq(&self, other: &Self) -> bool {
        self.date == other.date && self.amount == other.amount && self.credit == other.credit
    }
}

impl Eq for SettledBy {}

/// One transaction an invoice is booked as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Booking {
    /// Where it is.
    pub source: Source,
    /// Its `document:` tag.
    pub document: Option<String>,
    /// Its receivable posting's state; `None` for a booking with no
    /// receivable (a zero total, or a cash-books `$0` marker).
    pub receivable: Option<ReceivableState>,
}

/// A cash-books payment applied to an invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedPayment {
    /// Where it is.
    pub source: Source,
    /// What it paid: its `customer:`-tagged postings off the cash accounts.
    pub amount: Dec,
    /// Their commodity.
    pub commodity: String,
}

/// Everything the books hold under one invoice number.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Standing {
    /// Its bookings: one, normally; more is a finding.
    pub bookings: Vec<Booking>,
    /// Its void reversals.
    pub voids: Vec<Source>,
    /// Cash books: the payments applied to it.
    pub payments: Vec<AppliedPayment>,
}

impl Standing {
    /// Its booking with a receivable, else its first.
    #[must_use]
    pub fn booking(&self) -> Option<&Booking> {
        self.bookings
            .iter()
            .find(|b| b.receivable.is_some())
            .or_else(|| self.bookings.first())
    }

    /// Whether anything at all is booked under the number.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bookings.is_empty() && self.voids.is_empty() && self.payments.is_empty()
    }

    /// Whether `other` holds the same, wherever in the journal it is.
    fn same_as(&self, other: &Self) -> bool {
        let place = |a: &Source, b: &Source| a.date == b.date && a.file == b.file;
        self.bookings.len() == other.bookings.len()
            && self.voids.len() == other.voids.len()
            && self.payments.len() == other.payments.len()
            && self.bookings.iter().zip(&other.bookings).all(|(a, b)| {
                place(&a.source, &b.source)
                    && a.document == b.document
                    && a.receivable == b.receivable
            })
            && self
                .voids
                .iter()
                .zip(&other.voids)
                .all(|(a, b)| place(a, b))
            && self.payments.iter().zip(&other.payments).all(|(a, b)| {
                place(&a.source, &b.source) && a.amount == b.amount && a.commodity == b.commodity
            })
    }
}

/// One payment in the books' `paid:` mirror.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaidAmount {
    /// When.
    pub date: String,
    /// How much.
    #[serde(serialize_with = "crate::books::dec_text")]
    pub amount: Dec,
}

/// What the books say `paid:` should be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum BooksPaid {
    /// Nothing paid.
    No,
    /// Part paid, by these payments (one per day).
    Partial {
        /// The payments.
        parts: Vec<PaidAmount>,
    },
    /// Paid in full on this date.
    Full {
        /// The date.
        date: String,
    },
}

impl BooksPaid {
    /// Whether it is paid in full.
    #[must_use]
    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full { .. })
    }

    /// What was paid in all, of an invoice for `total`: all of it when
    /// paid in full.
    #[must_use]
    pub fn paid(&self, total: Dec) -> Dec {
        match self {
            Self::No => Dec::zero(),
            Self::Partial { parts } => parts
                .iter()
                .fold(Dec::zero(), |sum, p| sum.add(p.amount).unwrap_or(sum)),
            Self::Full { .. } => total,
        }
    }

    /// The mirror's text: `no`, a date, or `partial <date> <amount>, …`
    /// with amounts in `style`.
    #[must_use]
    pub fn text(&self, style: &MoneyStyle) -> String {
        match self {
            Self::No => "no".to_string(),
            Self::Full { date } => date.clone(),
            Self::Partial { parts } => format!(
                "partial {}",
                parts
                    .iter()
                    .map(|p| format!("{} {}", p.date, style.format_compact(p.amount)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// Whether the written mirror says the same: the same state, dates and
    /// amounts (however the amounts are written).
    #[must_use]
    pub fn agrees_with(&self, written: Option<&Paid>, style: &MoneyStyle) -> bool {
        match (self, written) {
            (Self::No, Some(Paid::No)) => true,
            (Self::Full { date }, Some(Paid::Full { date: written })) => date == written,
            (Self::Partial { parts }, Some(Paid::Partial { parts: written })) => {
                parts.len() == written.len()
                    && parts.iter().zip(written).all(|(p, w)| {
                        p.date == w.date
                            && style.parse(&w.amount).is_ok_and(|m| m.value == p.amount)
                    })
            }
            _ => false,
        }
    }
}

/// `(date, amount)` pairs, oldest first, merged by day.
fn by_day(payments: impl IntoIterator<Item = (String, Dec)>) -> Vec<PaidAmount> {
    let mut days: BTreeMap<String, Dec> = BTreeMap::new();
    for (date, amount) in payments {
        let sum = days.entry(date).or_insert_with(Dec::zero);
        *sum = sum.add(amount).unwrap_or(*sum);
    }
    days.into_iter()
        .map(|(date, amount)| PaidAmount { date, amount })
        .collect()
}

/// The books' standing of every invoice number, from one pass over a
/// journal.
#[derive(Debug, Clone)]
pub struct BookStates {
    basis: Basis,
    roles: AccrualRoles,
    numbers: HashMap<String, Standing>,
    receivables: Receivables,
}

impl BookStates {
    /// Read `journal` (whose basis is `basis`).
    #[must_use]
    pub fn build(journal: &Journal, basis: Basis) -> Self {
        let roles = AccrualRoles::of(journal);
        let types = AccountTypes::new(&account_decls(journal));
        let receivables = Receivables::build(journal, &roles, &ReceivablesOptions::default());
        let by_position: HashMap<usize, usize> = receivables
            .documents()
            .iter()
            .enumerate()
            .filter(|(_, d)| d.role == AccrualRole::Receivable)
            .map(|(at, d)| (d.at.position, at))
            .collect();
        let mut numbers: HashMap<String, Standing> = HashMap::new();
        for (position, txn) in journal.transactions.iter().enumerate() {
            if is_generated(txn) || is_recognition_entry(txn) {
                continue;
            }
            let Some(number) = tag_value(&txn.tags, INVOICE_TAG).filter(|n| !n.is_empty()) else {
                continue;
            };
            let source = Source {
                position,
                tindex: txn.index,
                date: txn.date.clone(),
                file: txn.source_file.clone(),
                line: txn.source_span.0.line,
            };
            let settlement_role = |role: AccrualRole| {
                txn.postings
                    .iter()
                    .any(|p| roles.role(&p.account.0) == Some(role))
            };
            if settlement_role(AccrualRole::Payable) {
                // A vendor's bill (or its payment): its own namespace.
                continue;
            }
            if tag_value(&txn.tags, VOID_TAG).is_some() {
                numbers
                    .entry(number.to_string())
                    .or_default()
                    .voids
                    .push(source);
                continue;
            }
            let document = tag_value(&txn.tags, DOCUMENT_TAG).map(str::to_string);
            match document_posting(txn, &roles) {
                Some((_, AccrualRole::Receivable)) => {
                    let receivable = by_position
                        .get(&position)
                        .map(|&at| &receivables.documents()[at])
                        .map(|d| ReceivableState {
                            account: d.account.clone(),
                            commodity: d.commodity.clone(),
                            amount: d.amount,
                            outstanding: d.outstanding,
                            state: d.state,
                            payments: d
                                .settlements
                                .iter()
                                .filter(|s| s.kind != SettlementKind::Void)
                                .map(|s| (s.date.clone(), s.amount))
                                .collect(),
                            settled_by: d
                                .settlements
                                .iter()
                                .filter(|s| s.kind != SettlementKind::Void)
                                .map(|s| SettledBy {
                                    position: s.at.position,
                                    date: s.date.clone(),
                                    amount: s.amount,
                                    credit: s.kind == SettlementKind::Credit,
                                })
                                .collect(),
                        });
                    numbers
                        .entry(number.to_string())
                        .or_default()
                        .bookings
                        .push(Booking {
                            source,
                            document,
                            receivable,
                        });
                }
                Some((_, AccrualRole::Payable)) => {}
                Some(_) | None => {
                    if settlement_role(AccrualRole::Receivable) {
                        // A payment of a booked invoice: its links count.
                        continue;
                    }
                    if document.is_some() {
                        numbers
                            .entry(number.to_string())
                            .or_default()
                            .bookings
                            .push(Booking {
                                source,
                                document,
                                receivable: None,
                            });
                        continue;
                    }
                    let mut amount = Dec::zero();
                    let mut commodity: Option<String> = None;
                    for posting in &txn.postings {
                        if posting.generated.is_some()
                            || types.is_cash(&posting.account.0)
                            || tag_value(&posting.tags, CUSTOMER_TAG).is_none()
                        {
                            continue;
                        }
                        for a in &posting.amounts {
                            if commodity.as_deref().is_none_or(|c| c == a.commodity.0) {
                                commodity = Some(a.commodity.0.clone());
                                amount = amount.sub(a.quantity).unwrap_or(amount);
                            }
                        }
                    }
                    if let Some(commodity) = commodity {
                        numbers
                            .entry(number.to_string())
                            .or_default()
                            .payments
                            .push(AppliedPayment {
                                source,
                                amount,
                                commodity,
                            });
                    }
                }
            }
        }
        Self {
            basis,
            roles,
            numbers,
            receivables,
        }
    }

    /// The journal's accrual roles, as inferred for these states.
    #[must_use]
    pub fn roles(&self) -> &AccrualRoles {
        &self.roles
    }

    /// The books' basis.
    #[must_use]
    pub fn basis(&self) -> Basis {
        self.basis
    }

    /// Every settlement in the books (accrual: Ledgeline's receivables).
    #[must_use]
    pub fn receivables(&self) -> &Receivables {
        &self.receivables
    }

    /// What the books hold under `number`.
    #[must_use]
    pub fn standing(&self, number: &str) -> Option<&Standing> {
        self.numbers.get(number)
    }

    /// Every number the books hold, sorted.
    #[must_use]
    pub fn numbers(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.numbers.keys().map(String::as_str).collect();
        out.sort_unstable_by(|a, b| crate::numbering::natural_cmp(a, b));
        out
    }

    /// The numbers whose standing differs between `before` and `self`: what
    /// a journal change touched, so the reconciler can re-run for those
    /// alone (ARCHITECTURE §5.6). Positions are ignored, so an insert
    /// elsewhere in the journal touches nothing.
    #[must_use]
    pub fn changed_since(&self, before: &Self) -> Vec<String> {
        let mut out: Vec<String> = self
            .numbers
            .iter()
            .filter(|(n, s)| before.numbers.get(*n).is_none_or(|old| !old.same_as(s)))
            .map(|(n, _)| n.clone())
            .chain(
                before
                    .numbers
                    .keys()
                    .filter(|n| !self.numbers.contains_key(*n))
                    .cloned(),
            )
            .collect();
        out.sort_unstable_by(|a, b| crate::numbering::natural_cmp(a, b));
        out
    }

    /// What the books say `doc`'s `paid:` is, or `None` when they can't say
    /// (no number, a table that doesn't read, accrual books with no
    /// receivable booked, a void in the books) or it has none (a void
    /// invoice: voiding removes `paid:`).
    #[must_use]
    pub fn paid(&self, doc: &IndexedDoc) -> Option<BooksPaid> {
        if doc.header.status == Some(Status::Void) {
            return None;
        }
        let number = doc.number()?;
        let summary = doc.summary.as_ref()?;
        if summary.total.is_zero() {
            return doc.header.date.clone().map(|date| BooksPaid::Full { date });
        }
        let standing = self.numbers.get(number);
        match self.basis {
            Basis::Accrual => {
                let receivable = standing?
                    .bookings
                    .iter()
                    .find_map(|b| b.receivable.as_ref())?;
                match receivable.state {
                    PaymentState::Open => Some(BooksPaid::No),
                    PaymentState::Partial => Some(BooksPaid::Partial {
                        parts: by_day(receivable.payments.iter().cloned()),
                    }),
                    PaymentState::Paid => receivable
                        .payments
                        .last()
                        .map(|(date, _)| BooksPaid::Full { date: date.clone() }),
                    PaymentState::Void => None,
                }
            }
            Basis::Cash => {
                let mut payments: Vec<(String, Dec)> = standing
                    .map(|s| {
                        s.payments
                            .iter()
                            .filter(|p| p.commodity == summary.commodity)
                            .map(|p| (p.source.date.clone(), p.amount))
                            .collect()
                    })
                    .unwrap_or_default();
                payments.sort_by(|a, b| a.0.cmp(&b.0));
                let paid = payments
                    .iter()
                    .fold(Dec::zero(), |sum, (_, a)| sum.add(*a).unwrap_or(sum));
                if paid.mantissa <= 0 {
                    Some(BooksPaid::No)
                } else if paid >= summary.total {
                    payments
                        .last()
                        .map(|(date, _)| BooksPaid::Full { date: date.clone() })
                } else {
                    Some(BooksPaid::Partial {
                        parts: by_day(payments),
                    })
                }
            }
        }
    }

    /// The money the books record as received for `doc`: `(date, amount)`,
    /// oldest first, in its table's commodity. Payments only: a credit (a
    /// credit note, a write-off) and a void are not money in. `None` when
    /// the books can't say (no number or table; accrual books with no
    /// receivable booked); a zero total is paid without money, so empty.
    #[must_use]
    pub fn money_received(&self, doc: &IndexedDoc) -> Option<Vec<(String, Dec)>> {
        let number = doc.number()?;
        let summary = doc.summary.as_ref()?;
        let standing = self.numbers.get(number);
        let mut out: Vec<(String, Dec)> = match self.basis {
            Basis::Accrual => standing?
                .bookings
                .iter()
                .find_map(|b| b.receivable.as_ref())?
                .settled_by
                .iter()
                .filter(|s| !s.credit)
                .map(|s| (s.date.clone(), s.amount))
                .collect(),
            Basis::Cash => standing
                .map(|s| {
                    s.payments
                        .iter()
                        .filter(|p| p.commodity == summary.commodity)
                        .map(|p| (p.source.date.clone(), p.amount))
                        .collect()
                })
                .unwrap_or_default(),
        };
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Some(out)
    }

    /// Whether `doc` is open: sent, and not paid in full — by the books
    /// when they can say, else by its `paid:` mirror.
    #[must_use]
    pub fn is_open(&self, doc: &IndexedDoc) -> bool {
        doc.header.status == Some(Status::Sent)
            && match self.paid(doc) {
                Some(paid) => !paid.is_full(),
                None => crate::views::is_open(&doc.header),
            }
    }
}

/// The money style `doc`'s table is written in.
fn style_of(index: &DocumentIndex, doc: &IndexedDoc) -> MoneyStyle {
    index
        .money()
        .style(doc.summary.as_ref().map(|s| s.commodity.as_str()))
}

/// A sent invoice's mirror, as the reconciler changes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MirrorChange {
    /// The file.
    pub path: VaultPath,
    /// The number.
    pub number: String,
    /// `paid:` as written (`None` when absent).
    pub before: Option<String>,
    /// `paid:` from the books.
    pub after: String,
    /// The books' state.
    pub paid: BooksPaid,
    /// Whether it was written (false with `dry_run`, or when it failed).
    pub written: bool,
}

/// A mirror the reconciler couldn't write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MirrorFailure {
    /// The file.
    pub path: VaultPath,
    /// The number.
    pub number: String,
    /// Why (a [`BooksError::code`]).
    pub code: &'static str,
    /// Why, in words.
    pub error: String,
}

/// What the reconciler does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileOptions {
    /// Only these numbers (`None`: every sent invoice).
    pub only: Option<Vec<String>>,
    /// Report what would change, write nothing.
    pub dry_run: bool,
}

/// What [`reconcile`] did.
#[derive(Debug, Clone, Serialize)]
pub struct Reconciled {
    /// The books' basis.
    pub basis: Basis,
    /// Sent invoices looked at.
    pub checked: usize,
    /// Of those, the ones whose mirror already agreed.
    pub unchanged: usize,
    /// The mirrors changed (or, dry, to change).
    pub changed: Vec<MirrorChange>,
    /// The mirrors that couldn't be written.
    pub failed: Vec<MirrorFailure>,
    /// Errors and warnings among `problems`.
    #[serde(flatten)]
    pub counts: crate::views::Counts,
    /// Book/markdown mismatches: [`audit`]'s findings, less the stale
    /// mirrors (`changed` lists those).
    pub problems: Vec<Problem>,
}

/// The mirrors [`reconcile`] would change: each sent invoice (of `only`)
/// whose `paid:` disagrees with the books.
#[must_use]
pub fn stale_mirrors(
    index: &DocumentIndex,
    states: &BookStates,
    only: Option<&[String]>,
) -> (usize, Vec<MirrorChange>) {
    let mut checked = 0;
    let mut out = Vec::new();
    for doc in index.docs() {
        let Some(number) = doc.number() else { continue };
        if doc.header.status != Some(Status::Sent)
            || only.is_some_and(|only| !only.iter().any(|n| n == number))
        {
            continue;
        }
        checked += 1;
        let Some(paid) = states.paid(doc) else {
            continue;
        };
        let style = style_of(index, doc);
        if paid.agrees_with(doc.header.paid.as_ref(), &style) {
            continue;
        }
        out.push(MirrorChange {
            path: doc.path.clone(),
            number: number.to_string(),
            before: doc.header.paid.as_ref().map(ToString::to_string),
            after: paid.text(&style),
            paid,
            written: false,
        });
    }
    out.sort_by(|a, b| crate::numbering::natural_cmp(&a.number, &b.number));
    (checked, out)
}

/// Write one mirror: the file read afresh, still sent and still
/// disagreeing, `paid:` set, conditional on the revision read.
fn write_mirror(
    books: &InvoiceBooks,
    index: &mut DocumentIndex,
    change: &MirrorChange,
) -> Result<bool, BooksError> {
    let document = books.document_at(index, &change.path)?;
    let header = document.invoice.header();
    if document.number() != change.number || header.status != Some(Status::Sent) {
        return Ok(false);
    }
    let style = index.money().style(
        index
            .doc(&change.path)
            .and_then(|d| d.summary.as_ref())
            .map(|s| s.commodity.as_str()),
    );
    if change.paid.agrees_with(header.paid.as_ref(), &style) {
        return Ok(false);
    }
    books.write_fields(
        index,
        &document,
        &[FieldEdit::Set(
            "paid".to_string(),
            Value::String(change.after.clone()),
        )],
    )?;
    Ok(true)
}

/// **Reconcile** (ARCHITECTURE §5.4): mirror the books' payment state into
/// each sent invoice's `paid:` where it differs, and report every
/// book/markdown mismatch. `loaded` is the books as read; the files are
/// re-read before each write.
#[must_use]
pub fn reconcile(
    books: &InvoiceBooks,
    index: &mut DocumentIndex,
    loaded: &Loaded,
    options: &ReconcileOptions,
) -> Reconciled {
    let states = BookStates::build(loaded.journal(), Basis::of(loaded));
    reconcile_with(books, index, loaded, &states, options)
}

/// [`reconcile`] with `states` already read from `loaded` (a caller that
/// diffed them against the last ones with [`BookStates::changed_since`]
/// and reconciles only those numbers: ARCHITECTURE §5.6).
#[must_use]
pub fn reconcile_with(
    books: &InvoiceBooks,
    index: &mut DocumentIndex,
    loaded: &Loaded,
    states: &BookStates,
    options: &ReconcileOptions,
) -> Reconciled {
    let only = options.only.as_deref();
    let (checked, mut changed) = stale_mirrors(index, states, only);
    let mut failed = Vec::new();
    if !options.dry_run {
        for change in &mut changed {
            match write_mirror(books, index, change) {
                Ok(written) => change.written = written,
                Err(error) => failed.push(MirrorFailure {
                    path: change.path.clone(),
                    number: change.number.clone(),
                    code: error.code(),
                    error: error.to_string(),
                }),
            }
        }
    }
    let problems: Vec<Problem> = audit(books, index, loaded, states, only)
        .into_iter()
        .filter(|p| p.code != ProblemCode::MissingMirror)
        .collect();
    Reconciled {
        basis: states.basis(),
        checked,
        unchanged: checked.saturating_sub(changed.len()),
        changed,
        failed,
        counts: crate::views::Counts::of(&problems),
        problems,
    }
}

fn command(text: String) -> Option<FixEdit> {
    Some(FixEdit::Command { command: text })
}

/// A transaction's place as a problem's file and line.
fn at_source(vault: &dyn Vault, source: &Source, problem: Problem) -> Problem {
    match vault.vault_path(&source.file) {
        Ok(path) => problem.in_file(&path).at_line(Some(source.line as usize)),
        Err(_) => problem,
    }
}

/// The books checks (ARCHITECTURE §4.3, §4.5) over `states` (read from
/// `loaded`), for every document (or the numbers in `only`). Sorted by file
/// and line.
#[must_use]
pub fn audit(
    books: &InvoiceBooks,
    index: &DocumentIndex,
    loaded: &Loaded,
    states: &BookStates,
    only: Option<&[String]>,
) -> Vec<Problem> {
    let vault = books.vault();
    let basis = states.basis();
    let closed_through = loaded.config().closed_through.as_deref();
    let mut step = StepCheck::new(books, index, loaded).with_roles(states.roles());
    let mut out: Vec<Problem> = Vec::new();
    let line =
        |doc: &IndexedDoc, key: &str| index.site().note(&doc.path).and_then(|n| n.line_of(key));
    // Files by number: what has no file is an orphan.
    let mut files: HashMap<&str, Vec<&IndexedDoc>> = HashMap::new();
    for doc in index.docs() {
        if let Some(number) = doc.number() {
            files.entry(number).or_default().push(doc);
        }
    }
    let numbers: Vec<&str> = match only {
        Some(only) => only.iter().map(String::as_str).collect(),
        None => states.numbers(),
    };
    for number in numbers {
        let Some(standing) = states.standing(number) else {
            continue;
        };
        if standing.bookings.len() > 1 {
            out.push(at_source(
                vault,
                &standing.bookings[1].source,
                Problem::error(
                    ProblemCode::DuplicateNumber,
                    format!(
                        "invoice {number} is booked {} times (first on {}); remove the extra \
                         transaction",
                        standing.bookings.len(),
                        standing.bookings[0].source.date
                    ),
                ),
            ));
        }
        if !files.contains_key(number) {
            let (what, source) = match (
                standing.booking(),
                standing.payments.first(),
                standing.voids.first(),
            ) {
                (Some(b), _, _) => ("a booking", &b.source),
                (None, Some(p), _) => ("a payment applied", &p.source),
                (None, None, Some(v)) => ("a void", v),
                (None, None, None) => continue,
            };
            let tag = standing
                .booking()
                .and_then(|b| b.document.as_deref())
                .map(|d| format!(" (its `document:` says {d})"))
                .unwrap_or_default();
            out.push(at_source(
                vault,
                source,
                Problem::warning(
                    ProblemCode::OrphanTransaction,
                    format!("the books have {what} for invoice {number}, which has no file{tag}"),
                )
                .with_fix(
                    "restore or renumber its file, or leave it if another app booked it",
                    None,
                ),
            ));
        }
    }
    let docs: Vec<&IndexedDoc> = match only {
        Some(only) => only.iter().flat_map(|n| index.by_number(n)).collect(),
        None => index.docs().collect(),
    };
    for doc in docs {
        let Some(number) = doc.number() else { continue };
        let standing = states.standing(number);
        let status = doc.header.status;
        let zero = doc.summary.as_ref().is_some_and(|s| s.total.is_zero());
        let booking = standing.and_then(Standing::booking);
        let voided_in_books = standing.and_then(|s| s.voids.first());
        let at = |p: Problem, key: &str| p.in_file(&doc.path).at_line(line(doc, key));
        match status {
            Some(Status::Draft) => {
                if let Some(booking) = booking {
                    let same = books
                        .document_at(index, &doc.path)
                        .ok()
                        .and_then(|d| {
                            let txn = loaded.journal().transactions.get(booking.source.position)?;
                            step.in_step(&d, txn).ok()
                        })
                        .unwrap_or(false);
                    out.push(at(
                        if same {
                            Problem::error(
                                ProblemCode::BookedDraft,
                                format!(
                                    "draft {number} is already booked ({}): a send stopped \
                                     after writing the books",
                                    booking.source.date
                                ),
                            )
                            .with_fix(
                                format!("finish it: ledgeline-invoicer send {number}"),
                                command(format!("send {number}")),
                            )
                        } else {
                            Problem::error(
                                ProblemCode::BookedDraft,
                                format!(
                                    "draft {number} is booked ({}), and not as its file says",
                                    booking.source.date
                                ),
                            )
                            .with_fix(
                                "set `status: sent` and re-book it, or remove the transaction",
                                None,
                            )
                        },
                        "status",
                    ));
                }
                continue;
            }
            Some(Status::Void) => {
                let reversed = voided_in_books.is_some();
                if let Some(booking) = booking
                    && !reversed
                    && booking
                        .receivable
                        .as_ref()
                        .is_none_or(|r| r.state != PaymentState::Void)
                {
                    out.push(at(
                        Problem::error(
                            ProblemCode::VoidMismatch,
                            format!(
                                "invoice {number} is void, but its booking ({}) isn't reversed",
                                booking.source.date
                            ),
                        )
                        .with_fix(
                            format!(
                                "set `status: sent`, then ledgeline-invoicer void {number}{}",
                                doc.header
                                    .void_date
                                    .as_deref()
                                    .map(|d| format!(" --date {d}"))
                                    .unwrap_or_default()
                            ),
                            None,
                        ),
                        "status",
                    ));
                }
                if basis == Basis::Cash && standing.is_some_and(|s| !s.payments.is_empty()) {
                    out.push(at(
                        Problem::warning(
                            ProblemCode::VoidMismatch,
                            format!("invoice {number} is void, but payments are applied to it"),
                        )
                        .with_fix(
                            format!("undo them: ledgeline-invoicer apply-payment {number} --undo"),
                            command(format!("apply-payment {number} --undo")),
                        ),
                        "status",
                    ));
                }
                continue;
            }
            Some(Status::Sent) => {}
            _ => continue,
        }
        // Sent.
        if let Some(void) = voided_in_books {
            out.push(at(
                Problem::error(
                    ProblemCode::VoidMismatch,
                    format!(
                        "invoice {number} is reversed in the books ({}), but its file says sent",
                        void.date
                    ),
                )
                .with_fix(
                    format!(
                        "finish the void: ledgeline-invoicer void {number} --date {}",
                        void.date
                    ),
                    command(format!("void {number} --date {}", void.date)),
                ),
                "status",
            ));
            continue;
        }
        let books_it = basis == Basis::Accrual || zero;
        let Some(booking) = booking else {
            if books_it && doc.summary.is_some() {
                out.push(at(
                    Problem::error(
                        ProblemCode::MissingTransaction,
                        format!(
                            "invoice {number} is sent, but the books have no transaction for it"
                        ),
                    )
                    .with_fix(
                        format!("book it: ledgeline-invoicer send {number}"),
                        command(format!("send {number}")),
                    ),
                    "status",
                ));
            }
            continue;
        };
        // Re-booking is refused on or before `closed-through` (§4.5).
        let closed = closed_through.filter(|through| booking.source.date.as_str() <= *through);
        let mut moved = false;
        if let Some(tag) = booking.document.as_deref()
            && tag != doc.path.as_str()
        {
            // What booking writes for this path: commas and runs of spaces
            // can't survive in a tag value.
            let sanitized = tag_text(doc.path.as_str());
            moved = sanitized.as_deref() != Some(tag);
            let problem = if moved {
                let problem = Problem::warning(
                    ProblemCode::DocumentPath,
                    format!("the books' `document:` for invoice {number} is {tag}, not this file"),
                );
                match closed {
                    Some(through) => problem.with_fix(
                        format!(
                            "its booking is in the closed period (through {through}): set the \
                             tag to this file by hand"
                        ),
                        None,
                    ),
                    None => problem.with_fix(
                        format!("repair it: ledgeline-invoicer rebook {number}"),
                        command(format!("rebook {number}")),
                    ),
                }
            } else {
                Problem::warning(
                    ProblemCode::DocumentPath,
                    format!(
                        "the file name can't be a tag value exactly (a comma, or a run of \
                         spaces): the books' `document:` reads {tag}"
                    ),
                )
                .with_fix("rename the file without them, then re-book it", None)
            };
            out.push(at(problem, "invoice_num"));
        }
        if !moved
            && books_it
            && let Ok(document) = books.document_at(index, &doc.path)
            && let Some(txn) = loaded.journal().transactions.get(booking.source.position)
        {
            match step.in_step(&document, txn) {
                Ok(true) => {}
                Ok(false) => {
                    let problem = Problem::warning(
                        ProblemCode::BooksOutOfDate,
                        format!(
                            "invoice {number} is booked ({}) differently from what its file now \
                             says",
                            booking.source.date
                        ),
                    );
                    out.push(at(
                        match closed {
                            Some(through) => problem.with_fix(
                                format!(
                                    "its booking is in the closed period (through {through}): \
                                     undo the edit, or void it and reissue it"
                                ),
                                None,
                            ),
                            None => problem.with_fix(
                                format!(
                                    "re-book it (ledgeline-invoicer rebook {number}), or keep the \
                                     books and undo the edit"
                                ),
                                command(format!("rebook {number}")),
                            ),
                        },
                        "status",
                    ));
                }
                Err(BooksError::Booking { source, .. }) => out.push(at(
                    Problem::error(ProblemCode::Booking, source.to_string()),
                    "status",
                )),
                // Field, table and customer problems are the document
                // checks' to report.
                Err(_) => {}
            }
        }
        if let Some(paid) = states.paid(doc) {
            let style = style_of(index, doc);
            if !paid.agrees_with(doc.header.paid.as_ref(), &style) {
                let text = paid.text(&style);
                out.push(at(
                    Problem::warning(
                        ProblemCode::MissingMirror,
                        format!(
                            "`paid: {}` disagrees with the books: {text}",
                            doc.header
                                .paid
                                .as_ref()
                                .map_or_else(|| "(none)".to_string(), ToString::to_string)
                        ),
                    )
                    .with_fix(
                        "mirror the books: ledgeline-invoicer reconcile",
                        Some(FixEdit::SetField {
                            key: "paid".to_string(),
                            value: Value::String(text),
                        }),
                    ),
                    "paid",
                ));
            }
        }
    }
    out.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.message.cmp(&b.message))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paid_counts_a_full_payment_as_the_total() {
        let total = Dec::new(100_000, 2);
        assert_eq!(BooksPaid::No.paid(total), Dec::zero());
        let full = BooksPaid::Full {
            date: "2026-11-20".into(),
        };
        assert_eq!(full.paid(total), total);
        let partial = BooksPaid::Partial {
            parts: vec![
                PaidAmount {
                    date: "2026-11-20".into(),
                    amount: Dec::new(25_000, 2),
                },
                PaidAmount {
                    date: "2026-11-21".into(),
                    amount: Dec::new(5_000, 2),
                },
            ],
        };
        assert_eq!(partial.paid(total), Dec::new(30_000, 2));
    }
}
