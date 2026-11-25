//! Receivables and payables from the books (invoicer L4, ARCHITECTURE §4.6):
//! each document's settlement, the open ones per settlement account, aging,
//! days sales outstanding and average days to pay.
//!
//! # What counts
//!
//! - A **document** is an invoice-side settlement posting: a receivable debit
//!   (an invoice) or a payable credit (a bill), real and written, with one
//!   cost-free amount, outside recognition entries — numbered by its
//!   transaction's `invoice:` tag or not (a hand-written invoice, an opening
//!   balance).
//! - It is **settled** only through `match:` links (plan 45 D-settle): every
//!   payment-side member of its link counts, whether the link is complete or
//!   still open ([`crate::matching::LinkIndex::is_open`]: one invoice, several
//!   payments — N:1). A link with several documents is settled oldest first.
//! - A settlement is a [`SettlementKind::Payment`] when its transaction has a
//!   cash leg, a [`SettlementKind::Void`] when it is a void reversal (header
//!   [`VOID_TAG`]), and a [`SettlementKind::Credit`] otherwise (a credit
//!   note).
//! - A payment-side posting in no link is **unapplied**: money received (or
//!   paid) and not yet matched to a document.
//!
//! Everything is exact [`Dec`] arithmetic; the two ratios (DSO and average
//! days to pay) are rounded once, half away from zero, at the end.
//!
//! The due date is the document's [`DUE_TAG`], else its date plus
//! [`ReceivablesOptions::default_terms_days`].

use super::schedule::Party;
use super::settle::party_of;
use super::{
    AccrualRole, AccrualRoles, DUE_TAG, FxHashMap, INVOICE_TAG, VOID_TAG, day_of, is_iso_date,
    iso_of, tag_value,
};
use crate::decimal::Dec;
use crate::matching::{self, MATCH_TAG, PostingRef};
use crate::model::{Journal, PostingType};
use crate::projections::is_generated;
use std::collections::BTreeMap;

/// Default net terms, in days, for a document with no `due:` tag.
pub const DEFAULT_TERMS_DAYS: u32 = 30;

/// How receivables are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceivablesOptions {
    /// Net terms for a document with no `due:` tag.
    pub default_terms_days: u32,
}

impl Default for ReceivablesOptions {
    fn default() -> Self {
        Self {
            default_terms_days: DEFAULT_TERMS_DAYS,
        }
    }
}

/// What settled (part of) a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettlementKind {
    /// Money: the settling transaction has a cash leg.
    Payment,
    /// The document's void reversal.
    Void,
    /// Anything else on the payment side: a credit note, a write-off.
    Credit,
}

/// One settlement applied to a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    /// The payment-side posting.
    pub at: PostingRef,
    /// Its date.
    pub date: String,
    /// The part of it applied to this document: a positive magnitude.
    pub amount: Dec,
    /// Payment, void or credit.
    pub kind: SettlementKind,
}

/// Where a payment stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaymentState {
    /// Nothing applied.
    Open,
    /// Some applied, some outstanding.
    Partial,
    /// Settled in full by payments or credits.
    Paid,
    /// Settled by its void reversal.
    Void,
}

/// Where a document's due date came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DueSource {
    /// Its `due:` tag.
    Tag,
    /// Its date plus the default terms.
    Terms,
}

/// One document and its settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentStatus {
    /// Its settlement posting.
    pub at: PostingRef,
    /// Invoice (receivable) or bill (payable).
    pub role: AccrualRole,
    /// The receivable or payable account.
    pub account: String,
    /// Its commodity.
    pub commodity: String,
    /// Its `invoice:` number, when numbered.
    pub number: Option<String>,
    /// Its customer or vendor.
    pub party: Option<Party>,
    /// Its date.
    pub date: String,
    /// When it falls due.
    pub due: String,
    /// Where `due` came from.
    pub due_source: DueSource,
    /// Its amount: a positive magnitude.
    pub amount: Dec,
    /// What its settlements cover: a positive magnitude, at most `amount`.
    pub settled: Dec,
    /// `amount - settled`.
    pub outstanding: Dec,
    /// The `match:` link it is in.
    pub link: Option<String>,
    /// Its settlements, oldest first.
    pub settlements: Vec<Settlement>,
    /// Open, partial, paid or void.
    pub state: PaymentState,
}

impl DocumentStatus {
    /// What was outstanding at the end of `date`: the amount less the
    /// settlements dated on or before it (the whole amount before the
    /// document's own date is still counted — callers filter by date).
    ///
    /// # Panics
    /// Never: amounts come from one parsed journal, far below `i128`.
    #[must_use]
    pub fn outstanding_on(&self, date: &str) -> Dec {
        self.settlements
            .iter()
            .filter(|s| s.date.as_str() <= date)
            .fold(self.amount, |left, s| left.sub(s.amount).unwrap_or(left))
    }

    /// The date it was settled in full (its last settlement), when it was.
    #[must_use]
    pub fn settled_on(&self) -> Option<&str> {
        match self.state {
            PaymentState::Paid | PaymentState::Void => {
                self.settlements.last().map(|s| s.date.as_str())
            }
            PaymentState::Open | PaymentState::Partial => None,
        }
    }

    /// Days past due on `today`: negative before the due date.
    #[must_use]
    pub fn days_past_due(&self, today: &str) -> i32 {
        day_of(today) - day_of(&self.due)
    }
}

/// A payment-side settlement posting in no settlement link: none, or a
/// broken one (so it is applied to nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unapplied {
    /// The posting.
    pub at: PostingRef,
    /// Receivable or payable.
    pub role: AccrualRole,
    /// The account.
    pub account: String,
    /// Its commodity.
    pub commodity: String,
    /// Its date.
    pub date: String,
    /// A positive magnitude.
    pub amount: Dec,
    /// Its transaction's `invoice:` number, if it names one.
    pub number: Option<String>,
    /// Its customer or vendor.
    pub party: Option<Party>,
}

/// An aging bucket: days past due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AgingBucket {
    /// Not yet due.
    Current,
    /// 1–30 days past due.
    Days1To30,
    /// 31–60.
    Days31To60,
    /// 61–90.
    Days61To90,
    /// 91 and more.
    Over90,
}

impl AgingBucket {
    /// Every bucket, youngest first.
    pub const ALL: [AgingBucket; 5] = [
        AgingBucket::Current,
        AgingBucket::Days1To30,
        AgingBucket::Days31To60,
        AgingBucket::Days61To90,
        AgingBucket::Over90,
    ];

    /// The bucket for `days` past due.
    #[must_use]
    pub fn of(days: i32) -> Self {
        match days {
            ..=0 => Self::Current,
            1..=30 => Self::Days1To30,
            31..=60 => Self::Days31To60,
            61..=90 => Self::Days61To90,
            _ => Self::Over90,
        }
    }

    /// Its column label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "Current",
            Self::Days1To30 => "1–30",
            Self::Days31To60 => "31–60",
            Self::Days61To90 => "61–90",
            Self::Over90 => "91+",
        }
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// One settlement account's aging in one commodity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgingRow {
    /// The receivable or payable account.
    pub account: String,
    /// The commodity.
    pub commodity: String,
    /// Outstanding per bucket, in [`AgingBucket::ALL`] order.
    pub buckets: [Dec; 5],
    /// Documents per bucket.
    pub counts: [usize; 5],
    /// The sum of the buckets.
    pub total: Dec,
    /// The oldest due date among the overdue documents.
    pub oldest_overdue: Option<String>,
}

/// Days sales outstanding over a window, in one commodity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dso {
    /// The commodity.
    pub commodity: String,
    /// Outstanding at the end of the window, from documents dated in it or
    /// before.
    pub outstanding: Dec,
    /// Documents dated in the window: the credit sales (or purchases).
    pub billed: Dec,
    /// The window's length.
    pub days: u32,
    /// `outstanding / billed × days`, to one decimal; `None` with nothing
    /// billed.
    pub value: Option<Dec>,
}

/// Average days from document to full settlement by payment, in one role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaysToPay {
    /// How many paid documents it averages (voids are left out).
    pub documents: usize,
    /// The mean, to one decimal; `None` with no paid documents.
    pub average: Option<Dec>,
}

/// Every document's settlement in a journal.
#[derive(Debug, Clone, Default)]
pub struct Receivables {
    documents: Vec<DocumentStatus>,
    unapplied: Vec<Unapplied>,
    by_number: FxHashMap<(AccrualRole, String), Vec<usize>>,
}

/// A settlement-side candidate, as found in the scan.
struct Side {
    at: PostingRef,
    role: AccrualRole,
    quantity: Dec,
}

impl Receivables {
    /// One pass over `journal` for settlement postings, plus its links.
    #[must_use]
    pub fn build(journal: &Journal, roles: &AccrualRoles, options: &ReceivablesOptions) -> Self {
        let settlement: FxHashMap<&str, AccrualRole> = roles
            .iter()
            .filter(|(_, role, _)| role.is_settlement())
            .map(|(name, role, _)| (name, role))
            .collect();
        let types = crate::reports::AccountTypes::new(&crate::reports::account_decls(journal));
        let index = matching::links(journal);
        let mut out = Self::default();
        // Linked members by link id; then the documents and the unapplied.
        let mut linked: BTreeMap<&str, Vec<Side>> = BTreeMap::new();
        for (position, txn) in journal.transactions.iter().enumerate() {
            if is_generated(txn) || super::is_recognition_entry(txn) {
                continue;
            }
            for (posting_at, posting) in txn.postings.iter().enumerate() {
                let Some(&role) = settlement.get(posting.account.0.as_str()) else {
                    continue;
                };
                if posting.generated.is_some() || posting.ptype != PostingType::Regular {
                    continue;
                }
                let [amount] = posting.amounts.as_slice() else {
                    continue;
                };
                if amount.cost.is_some() || amount.quantity.is_zero() {
                    continue;
                }
                let at = PostingRef {
                    position,
                    txn: txn.index,
                    posting: posting_at,
                };
                let link = tag_value(&posting.tags, MATCH_TAG)
                    .filter(|id| !id.is_empty() && *id != matching::IGNORE_VALUE);
                if let Some(id) = link {
                    linked.entry(id).or_default().push(Side {
                        at,
                        role,
                        quantity: amount.quantity,
                    });
                }
                let invoice_side =
                    (amount.quantity.mantissa > 0) == (role == AccrualRole::Receivable);
                let magnitude = amount.quantity.abs().unwrap_or(amount.quantity);
                let number = tag_value(&txn.tags, INVOICE_TAG)
                    .filter(|n| !n.is_empty())
                    .map(str::to_string);
                let date = posting.date.clone().unwrap_or_else(|| txn.date.clone());
                if invoice_side && tag_value(&txn.tags, VOID_TAG).is_none() {
                    let (due, due_source) = match tag_value(&txn.tags, DUE_TAG) {
                        Some(due) if is_iso_date(due) => (due.to_string(), DueSource::Tag),
                        _ => (
                            iso_of(
                                day_of(&date)
                                    + i32::try_from(options.default_terms_days)
                                        .unwrap_or(i32::MAX / 2),
                            ),
                            DueSource::Terms,
                        ),
                    };
                    if let Some(number) = &number {
                        out.by_number
                            .entry((role, number.clone()))
                            .or_default()
                            .push(out.documents.len());
                    }
                    out.documents.push(DocumentStatus {
                        at,
                        role,
                        account: posting.account.0.clone(),
                        commodity: amount.commodity.0.clone(),
                        number,
                        party: party_of(txn, posting),
                        date,
                        due,
                        due_source,
                        amount: magnitude,
                        settled: Dec::zero(),
                        outstanding: magnitude,
                        link: link.map(str::to_string),
                        settlements: Vec::new(),
                        state: PaymentState::Open,
                    });
                } else if !invoice_side && !link.is_some_and(|id| index.is_settlement(id)) {
                    // No link, or one that settles nothing (broken: a lone
                    // member, an overpayment, mixed commodities): the
                    // payment is applied to nothing, so it is unapplied.
                    out.unapplied.push(Unapplied {
                        at,
                        role,
                        account: posting.account.0.clone(),
                        commodity: amount.commodity.0.clone(),
                        date,
                        amount: magnitude,
                        number,
                        party: party_of(txn, posting),
                    });
                }
            }
        }
        out.settle(journal, &types, &index, &linked);
        out
    }

    /// Apply each valid settlement link's payment side to its documents,
    /// oldest document first, oldest payment first.
    fn settle(
        &mut self,
        journal: &Journal,
        types: &crate::reports::AccountTypes,
        index: &matching::LinkIndex,
        linked: &BTreeMap<&str, Vec<Side>>,
    ) {
        let by_at: FxHashMap<PostingRef, usize> = self
            .documents
            .iter()
            .enumerate()
            .filter(|(_, d)| d.link.is_some())
            .map(|(i, d)| (d.at, i))
            .collect();
        for (id, members) in linked {
            if !index.is_settlement(id) {
                continue;
            }
            let mut documents: Vec<usize> = members
                .iter()
                .filter_map(|side| by_at.get(&side.at).copied())
                .collect();
            documents.sort_by(|a, b| {
                let (a, b) = (&self.documents[*a], &self.documents[*b]);
                (a.date.as_str(), a.at.position).cmp(&(b.date.as_str(), b.at.position))
            });
            let mut payments: Vec<&Side> = members
                .iter()
                .filter(|side| {
                    (side.quantity.mantissa > 0) != (side.role == AccrualRole::Receivable)
                })
                .collect();
            payments.sort_by_key(|side| {
                let txn = &journal.transactions[side.at.position];
                (txn.date.as_str(), side.at.position)
            });
            let mut queue = documents.into_iter().peekable();
            for side in payments {
                let txn = &journal.transactions[side.at.position];
                let kind = if tag_value(&txn.tags, VOID_TAG).is_some() {
                    SettlementKind::Void
                } else if txn.postings.iter().any(|p| types.is_cash(&p.account.0)) {
                    SettlementKind::Payment
                } else {
                    SettlementKind::Credit
                };
                let date = txn.postings[side.at.posting]
                    .date
                    .clone()
                    .unwrap_or_else(|| txn.date.clone());
                let mut left = side.quantity.abs().unwrap_or(side.quantity);
                while !left.is_zero() {
                    let Some(&doc) = queue.peek() else { break };
                    let document = &mut self.documents[doc];
                    let applied = if document.outstanding <= left {
                        document.outstanding
                    } else {
                        left
                    };
                    if !applied.is_zero() {
                        document.settlements.push(Settlement {
                            at: side.at,
                            date: date.clone(),
                            amount: applied,
                            kind,
                        });
                        document.settled =
                            document.settled.add(applied).unwrap_or(document.settled);
                        document.outstanding = document
                            .outstanding
                            .sub(applied)
                            .unwrap_or(document.outstanding);
                        left = left.sub(applied).unwrap_or(Dec::zero());
                    }
                    if document.outstanding.is_zero() {
                        queue.next();
                    }
                }
            }
        }
        for document in &mut self.documents {
            document.state = if document.outstanding.is_zero() {
                if document
                    .settlements
                    .iter()
                    .any(|s| s.kind == SettlementKind::Void)
                {
                    PaymentState::Void
                } else {
                    PaymentState::Paid
                }
            } else if document.settled.is_zero() {
                PaymentState::Open
            } else {
                PaymentState::Partial
            };
        }
    }

    /// Every document, in file order.
    #[must_use]
    pub fn documents(&self) -> &[DocumentStatus] {
        &self.documents
    }

    /// Every payment-side posting in no settlement link (none, or a broken
    /// one), in file order.
    #[must_use]
    pub fn unapplied(&self) -> &[Unapplied] {
        &self.unapplied
    }

    /// The document numbered `number` (the first, if the number is used
    /// twice — [`super::documents::DocumentIndex::duplicates`] lists those).
    #[must_use]
    pub fn by_number(&self, role: AccrualRole, number: &str) -> Option<&DocumentStatus> {
        self.by_number
            .get(&(role, number.to_string()))
            .and_then(|at| at.first())
            .map(|&at| &self.documents[at])
    }

    /// The documents of `role` still outstanding (open or partial).
    pub fn open(&self, role: AccrualRole) -> impl Iterator<Item = &DocumentStatus> {
        self.documents
            .iter()
            .filter(move |d| d.role == role && !d.outstanding.is_zero())
    }

    /// [`open`](Self::open), per settlement account, each oldest due first.
    #[must_use]
    pub fn open_by_account(&self, role: AccrualRole) -> BTreeMap<&str, Vec<&DocumentStatus>> {
        let mut out: BTreeMap<&str, Vec<&DocumentStatus>> = BTreeMap::new();
        for document in self.open(role) {
            out.entry(document.account.as_str())
                .or_default()
                .push(document);
        }
        for documents in out.values_mut() {
            documents.sort_by(|a, b| (&a.due, a.at.position).cmp(&(&b.due, b.at.position)));
        }
        out
    }

    /// The aging of `role` as of `today`, per account and commodity: each
    /// document dated on or before `today`, outstanding at the end of it,
    /// bucketed by days past due.
    #[must_use]
    pub fn aging(&self, role: AccrualRole, today: &str) -> Vec<AgingRow> {
        let mut rows: BTreeMap<(&str, &str), AgingRow> = BTreeMap::new();
        for document in self
            .documents
            .iter()
            .filter(|d| d.role == role && d.date.as_str() <= today)
        {
            let outstanding = document.outstanding_on(today);
            if outstanding.is_zero() {
                continue;
            }
            let row = rows
                .entry((document.account.as_str(), document.commodity.as_str()))
                .or_insert_with(|| AgingRow {
                    account: document.account.clone(),
                    commodity: document.commodity.clone(),
                    buckets: [Dec::zero(); 5],
                    counts: [0; 5],
                    total: Dec::zero(),
                    oldest_overdue: None,
                });
            let days = document.days_past_due(today);
            let slot = AgingBucket::of(days).slot();
            row.buckets[slot] = row.buckets[slot]
                .add(outstanding)
                .unwrap_or(row.buckets[slot]);
            row.counts[slot] += 1;
            row.total = row.total.add(outstanding).unwrap_or(row.total);
            if days > 0
                && row
                    .oldest_overdue
                    .as_deref()
                    .is_none_or(|oldest| document.due.as_str() < oldest)
            {
                row.oldest_overdue = Some(document.due.clone());
            }
        }
        rows.into_values().collect()
    }

    /// Days sales outstanding for `role` over the `days` ending `today`, per
    /// commodity: outstanding at the end of `today` (documents dated on or
    /// before it) over what was billed in the window, times `days`. A voided
    /// document was never a sale and counts in neither.
    #[must_use]
    pub fn dso(&self, role: AccrualRole, today: &str, days: u32) -> Vec<Dso> {
        let start = day_of(today) - i32::try_from(days).unwrap_or(i32::MAX / 2);
        let mut sums: BTreeMap<&str, (Dec, Dec)> = BTreeMap::new();
        for document in self
            .documents
            .iter()
            .filter(|d| d.role == role && d.date.as_str() <= today && d.state != PaymentState::Void)
        {
            let (outstanding, billed) = sums
                .entry(document.commodity.as_str())
                .or_insert((Dec::zero(), Dec::zero()));
            *outstanding = outstanding
                .add(document.outstanding_on(today))
                .unwrap_or(*outstanding);
            if day_of(&document.date) > start {
                *billed = billed.add(document.amount).unwrap_or(*billed);
            }
        }
        sums.into_iter()
            .map(|(commodity, (outstanding, billed))| Dso {
                commodity: commodity.to_string(),
                outstanding,
                billed,
                days,
                value: outstanding
                    .mul(Dec::new(i128::from(days), 0))
                    .ok()
                    .and_then(|scaled| ratio(scaled, billed, 1)),
            })
            .collect()
    }

    /// The mean days from document date to full settlement over the `role`
    /// documents paid in full (voids left out).
    #[must_use]
    pub fn average_days_to_pay(&self, role: AccrualRole) -> DaysToPay {
        let (count, total) = self
            .documents
            .iter()
            .filter(|d| d.role == role && d.state == PaymentState::Paid)
            .filter_map(|d| d.settled_on().map(|paid| day_of(paid) - day_of(&d.date)))
            .fold((0u32, 0i128), |(n, sum), days| {
                (n + 1, sum + i128::from(days))
            });
        DaysToPay {
            documents: count as usize,
            average: ratio(Dec::new(total, 0), Dec::new(i128::from(count), 0), 1),
        }
    }

    /// The numbers a [`super::documents::DocumentIndex`] would find, for a consistency check:
    /// every `(role, number)` here is one there.
    #[must_use]
    pub fn numbers(&self) -> Vec<(AccrualRole, &str)> {
        let mut out: Vec<(AccrualRole, &str)> = self
            .by_number
            .keys()
            .map(|(role, number)| (*role, number.as_str()))
            .collect();
        out.sort_unstable();
        out
    }
}

/// `numerator / denominator` to `places`, rounded half away from zero, exact
/// to that point; `None` for a zero denominator or overflow.
fn ratio(numerator: Dec, denominator: Dec, places: u32) -> Option<Dec> {
    if denominator.is_zero() {
        return None;
    }
    // n / d = (n.m / 10^n.p) / (d.m / 10^d.p) = n.m × 10^(d.p + places)
    //   / (d.m × 10^n.p), in units of 10^-places.
    let pow = |e: u32| 10_i128.checked_pow(e);
    let top = numerator
        .mantissa
        .checked_mul(pow(denominator.places.checked_add(places)?)?)?;
    let bottom = denominator.mantissa.checked_mul(pow(numerator.places)?)?;
    let quotient = top / bottom;
    let remainder = top % bottom;
    let negative = (top < 0) != (bottom < 0);
    let rounded = if remainder.unsigned_abs() * 2 >= bottom.unsigned_abs() {
        if negative { quotient - 1 } else { quotient + 1 }
    } else {
        quotient
    };
    Some(Dec::new(rounded, places))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;

    const BOOKS: &str = "\
account assets:ar  ; type: A, kpi: receivable
account liabilities:ap  ; type: L, kpi: payable
account assets:bank  ; type: C
account revenue  ; type: R
account expenses  ; type: X

2026-01-01 Invoice 1  ; invoice: 1, due: 2026-01-31
    assets:ar  $1000.00  ; customer: Acme, match: p1
    revenue
2026-01-10 Payment part 1  ; invoice: 1
    assets:bank  $400.00
    assets:ar  $-400.00  ; customer: Acme, match: p1
2026-02-15 Invoice 2  ; invoice: 2
    assets:ar  $500.00  ; customer: Beta, match: v2
    revenue
2026-02-20 Invoice 2 (void)  ; invoice: 2, void: 2026-02-20
    assets:ar  $-500.00  ; customer: Beta, match: v2
    revenue
2026-03-01 Invoice 3  ; invoice: 3, due: 2026-03-15
    assets:ar  $300.00  ; match: full
    revenue
2026-03-05 Payment 3a
    assets:bank  $100.00
    assets:ar  $-100.00  ; match: full
2026-03-20 Payment 3b
    assets:bank  $200.00
    assets:ar  $-200.00  ; match: full
2026-03-25 Stray payment
    assets:bank  $75.00
    assets:ar  $-75.00  ; customer: Gamma
2026-03-26 Bill B-1  ; invoice: 1
    expenses  $60.00
    liabilities:ap  $-60.00  ; vendor: Northwind
";

    fn receivables() -> (Journal, Receivables) {
        let journal = parse_journal(BOOKS, "inline.journal").expect("parses");
        let roles = AccrualRoles::of(&journal);
        let r = Receivables::build(&journal, &roles, &ReceivablesOptions::default());
        (journal, r)
    }

    #[test]
    fn partial_void_n_to_one_and_unapplied_settle_as_linked() {
        let (journal, r) = receivables();
        let links = matching::links(&journal);
        assert!(links.is_open("p1") && links.is_settlement("p1"));
        assert!(!links.is_open("full") && links.is_settlement("full"));
        assert!(links.broken().is_empty(), "{:?}", links.broken());

        let one = r.by_number(AccrualRole::Receivable, "1").expect("1");
        assert_eq!(one.state, PaymentState::Partial);
        assert_eq!(one.outstanding, Dec::new(60_000, 2));
        assert_eq!(one.due_source, DueSource::Tag);
        assert_eq!(one.party.as_ref().map(|p| p.name.as_str()), Some("Acme"));
        let two = r.by_number(AccrualRole::Receivable, "2").expect("2");
        assert_eq!(two.state, PaymentState::Void);
        assert_eq!(two.settlements[0].kind, SettlementKind::Void);
        assert_eq!(two.due, "2026-03-17", "date plus 30 days");
        let three = r.by_number(AccrualRole::Receivable, "3").expect("3");
        assert_eq!(three.state, PaymentState::Paid);
        assert_eq!(three.settlements.len(), 2);
        assert_eq!(three.settled_on(), Some("2026-03-20"));
        let bill = r.by_number(AccrualRole::Payable, "1").expect("bill 1");
        assert_eq!(bill.state, PaymentState::Open);
        assert_eq!(bill.amount, Dec::new(6_000, 2));

        assert_eq!(r.unapplied().len(), 1);
        assert_eq!(r.unapplied()[0].amount, Dec::new(7_500, 2));
        let open: Vec<_> = r
            .open(AccrualRole::Receivable)
            .map(|d| d.number.as_deref())
            .collect();
        assert_eq!(open, vec![Some("1")]);
        assert_eq!(
            r.numbers(),
            vec![
                (AccrualRole::Receivable, "1"),
                (AccrualRole::Receivable, "2"),
                (AccrualRole::Receivable, "3"),
                (AccrualRole::Payable, "1"),
            ]
        );
    }

    #[test]
    fn aging_dso_and_days_to_pay_are_exact() {
        let (_, r) = receivables();
        // On 2026-01-05 the invoice is current and unpaid.
        let early = r.aging(AccrualRole::Receivable, "2026-01-05");
        assert_eq!(early.len(), 1);
        assert_eq!(early[0].buckets[0], Dec::new(100_000, 2));
        // On 2026-04-01 the $600 left of invoice 1 is 60 days past due.
        let late = r.aging(AccrualRole::Receivable, "2026-04-01");
        assert_eq!(
            late[0].buckets[AgingBucket::Days31To60.slot()],
            Dec::new(60_000, 2)
        );
        assert_eq!(late[0].total, Dec::new(60_000, 2));
        assert_eq!(late[0].oldest_overdue.as_deref(), Some("2026-01-31"));
        // As of 2026-03-10, invoice 3 had $200 outstanding (3b not yet in).
        let mid = r.aging(AccrualRole::Receivable, "2026-03-10");
        assert_eq!(mid[0].total, Dec::new(80_000, 2));

        // DSO over the 90 days to 2026-04-01: invoice 1 (2026-01-01) is the
        // window's open start and 2 was voided, so billed is 3's $300;
        // outstanding is the $600 left of 1: 600 / 300 × 90 = 180.0.
        let dso = r.dso(AccrualRole::Receivable, "2026-04-01", 90);
        assert_eq!(dso.len(), 1);
        assert_eq!(dso[0].outstanding, Dec::new(60_000, 2));
        assert_eq!(dso[0].billed, Dec::new(30_000, 2));
        assert_eq!(dso[0].value, Some(Dec::new(1800, 1)));

        // Only invoice 3 is paid (the void doesn't count): 19 days.
        let paid = r.average_days_to_pay(AccrualRole::Receivable);
        assert_eq!(paid.documents, 1);
        assert_eq!(paid.average, Some(Dec::new(190, 1)));
    }

    #[test]
    fn an_over_settled_link_is_still_broken() {
        let text = BOOKS
            .replace(
                "$-400.00  ; customer: Acme, match: p1",
                "$-1100.00  ; customer: Acme, match: p1",
            )
            .replace("assets:bank  $400.00", "assets:bank  $1100.00");
        let journal = parse_journal(&text, "inline.journal").expect("parses");
        let links = matching::links(&journal);
        assert!(links.broken().iter().any(|b| b.id == "p1"));
        let r = Receivables::build(
            &journal,
            &AccrualRoles::of(&journal),
            &ReceivablesOptions::default(),
        );
        assert_eq!(
            r.by_number(AccrualRole::Receivable, "1").map(|d| d.state),
            Some(PaymentState::Open)
        );
        // The overpayment settles nothing, so it is unapplied (RA-1).
        let amounts: Vec<Dec> = r.unapplied().iter().map(|u| u.amount).collect();
        assert_eq!(amounts, [Dec::new(110_000, 2), Dec::new(7_500, 2)]);
    }

    /// RA-1: a payment whose `match:` id names no settlement (a typo, so a
    /// lone member) is unapplied, so receivables still add up to the
    /// receivable account's balance.
    #[test]
    fn a_payment_in_a_broken_link_is_unapplied() {
        let text = BOOKS.replace(
            "$-400.00  ; customer: Acme, match: p1",
            "$-400.00  ; customer: Acme, match: p11",
        );
        let journal = parse_journal(&text, "inline.journal").expect("parses");
        let links = matching::links(&journal);
        assert!(links.broken().iter().any(|b| b.id == "p11"));
        let r = Receivables::build(
            &journal,
            &AccrualRoles::of(&journal),
            &ReceivablesOptions::default(),
        );
        let one = r.by_number(AccrualRole::Receivable, "1").expect("1");
        assert_eq!(one.outstanding, Dec::new(100_000, 2));
        let unapplied: Vec<(Option<&str>, Dec)> = r
            .unapplied()
            .iter()
            .map(|u| (u.number.as_deref(), u.amount))
            .collect();
        assert_eq!(
            unapplied,
            [(Some("1"), Dec::new(40_000, 2)), (None, Dec::new(7_500, 2))]
        );
        // Open less unapplied is the AR balance: 1000 + 0 + 0 − 400 − 75 on
        // the receivable side (2 is void, 3 is paid).
        let open = r
            .open(AccrualRole::Receivable)
            .fold(Dec::zero(), |sum, d| sum.add(d.outstanding).expect("adds"));
        let unapplied = r
            .unapplied()
            .iter()
            .filter(|u| u.role == AccrualRole::Receivable)
            .fold(Dec::zero(), |sum, u| sum.add(u.amount).expect("adds"));
        assert_eq!(open.sub(unapplied).expect("subs"), Dec::new(52_500, 2));
    }

    #[test]
    fn ratio_rounds_half_away_from_zero() {
        assert_eq!(
            ratio(Dec::new(1, 0), Dec::new(3, 0), 1),
            Some(Dec::new(3, 1))
        );
        assert_eq!(
            ratio(Dec::new(2, 0), Dec::new(3, 0), 1),
            Some(Dec::new(7, 1))
        );
        assert_eq!(
            ratio(Dec::new(5, 2), Dec::new(1, 1), 1),
            Some(Dec::new(5, 1))
        );
        assert_eq!(
            ratio(Dec::new(-1, 0), Dec::new(4, 0), 1),
            Some(Dec::new(-3, 1))
        );
        assert_eq!(ratio(Dec::new(1, 0), Dec::zero(), 1), None);
    }
}
