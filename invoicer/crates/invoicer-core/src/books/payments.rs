//! **Apply payment** in cash books (I17, PLAN 2b): an invoice books nothing
//! at send, so when its payment arrives — a deposit Ledgeline imported or
//! synced — the deposit's income side is re-split into the invoice's row
//! accounts and tagged `invoice:` (Ledgeline's
//! [`apply`](ledgeline_core::accrual::apply), through its transaction editor
//! and `ExternalChange` guard). The bank leg stays byte for byte. The
//! invoicer never writes a payment transaction of its own: that would
//! duplicate the bank import.
//!
//! - **Candidates** are ranked as plan 45 ranks settlements, less the
//!   receivable leg: the invoice number in the description, then the
//!   customer, then the amount (exact, then short), then the date gap. Each
//!   has a `reference`, the fingerprint of its text, which names it across
//!   loads (and refuses a deposit edited since it was listed).
//! - **Partial payments** split pro rata over the rows. A payment that
//!   settles the rest instead takes each row's remainder, so the rows end
//!   exactly at the invoice's amounts whatever the rounding before.
//! - **Undo** writes the deposit's original text back exactly. The
//!   [`PaymentUndo`] is plain data (serializable), so a later process can
//!   undo too, as long as the deposit hasn't changed since.

use super::deposits::names;
use super::lifecycle::{BookedTransaction, applied_payments};
use super::spec::{CashBooking, InvoiceBooking, scale};
use super::{BooksError, InvoiceBooks, check_generation};
use crate::books::lifecycle::Basis;
use crate::doc::Status;
use crate::index::DocumentIndex;
use crate::settings::PaymentAllocation;
use ledgeline_core::Dec;
use ledgeline_core::accrual::apply::{self, ApplyUndo, CashInvoice};
use ledgeline_core::accrual::invoice::money;
use ledgeline_core::accrual::ops::{Generation, Loaded};
use ledgeline_core::accrual::schedule::{Party, PartyKind};
use ledgeline_core::accrual::settle::{Fit, SettleSettings};
use ledgeline_core::accrual::{CUSTOMER_TAG, tag_value};
use ledgeline_core::model::Tindex;
use ledgeline_core::vault::VaultPath;
use serde::{Deserialize, Serialize};

/// A deposit that might pay an invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaymentCandidate {
    /// Names the deposit for [`ApplyPayment::reference`]: the fingerprint
    /// of its text.
    pub reference: String,
    /// Its date.
    pub date: String,
    /// Its description.
    pub description: String,
    /// What its income side holds.
    #[serde(serialize_with = "super::dec_text")]
    pub received: Dec,
    /// It pays the rest exactly (else it is a partial payment).
    pub exact: bool,
    /// Its description names the invoice number.
    pub by_number: bool,
    /// Its customer is the invoice's.
    pub by_customer: bool,
    /// Days between the invoice and the deposit.
    pub gap_days: u32,
    /// Its text in the journal.
    pub text: String,
}

/// What Apply payment can do for an invoice: what is due, and the deposits
/// that might pay it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaymentOptions {
    /// The invoice.
    pub number: String,
    /// The file.
    pub path: VaultPath,
    /// The commodity.
    pub commodity: String,
    /// Its total.
    #[serde(serialize_with = "super::dec_text")]
    pub total: Dec,
    /// What its applied payments have paid.
    #[serde(serialize_with = "super::dec_text")]
    pub paid: Dec,
    /// What is still due.
    #[serde(serialize_with = "super::dec_text")]
    pub remaining: Dec,
    /// The deposits that might pay it, best first (none once it is paid).
    pub candidates: Vec<PaymentCandidate>,
}

/// The shortest reference prefix [`ApplyPayment::reference`] accepts.
pub const MIN_REFERENCE_PREFIX: usize = 6;

/// **Apply payment**: re-split the deposit `reference` into invoice
/// `number`'s rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPayment {
    /// The invoice.
    pub number: String,
    /// The deposit: a [`PaymentCandidate::reference`], or a prefix of one
    /// at least [`MIN_REFERENCE_PREFIX`] long that names only it.
    pub reference: String,
    /// Today: candidates are looked for within the lookback before it.
    pub today: String,
    /// Refuse unless the books are still as listed.
    pub expected: Option<Generation>,
}

/// How to undo an apply: the deposit as it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentUndo {
    /// The invoice.
    pub number: String,
    /// The fingerprint of the deposit as the apply wrote it.
    pub reference: String,
    /// The deposit's text before the apply.
    pub original_text: String,
}

/// What [`InvoiceBooks::apply_payment`] wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaymentApplied {
    /// The invoice.
    pub number: String,
    /// The deposit as written.
    pub transaction: BookedTransaction,
    /// What it paid.
    #[serde(serialize_with = "super::dec_text")]
    pub received: Dec,
    /// Each row account and its share.
    pub applied: Vec<(String, String)>,
    /// What is still due on the invoice after it.
    #[serde(serialize_with = "super::dec_text")]
    pub remaining: Dec,
    /// How to undo it.
    pub undo: PaymentUndo,
}

/// What is already paid on `booking`, per row account: the applied
/// deposits' postings on the split's accounts, tagged with the customer.
fn paid_so_far(
    loaded: &Loaded,
    booking: &InvoiceBooking,
    split: &[(String, Dec)],
) -> Result<Vec<Dec>, BooksError> {
    let mut paid = vec![Dec::zero(); split.len()];
    for txn in applied_payments(loaded, &booking.number) {
        for posting in &txn.postings {
            let Some(at) = split.iter().position(|(a, _)| *a == posting.account.0) else {
                continue;
            };
            if tag_value(&posting.tags, CUSTOMER_TAG) != Some(booking.customer.as_str()) {
                continue;
            }
            for amount in &posting.amounts {
                if amount.commodity.0 == booking.commodity {
                    paid[at] = paid[at]
                        .sub(amount.quantity)
                        .map_err(|_| BooksError::Overflow)?;
                }
            }
        }
    }
    Ok(paid)
}

/// Cash books, income on receipt: per account, what the deposits a final
/// invoice applies already booked as revenue. Each deposit row (`Less
/// deposit (Invoice 206)`) names its deposit invoice as a whole word; that
/// invoice's applied payments say where its money went, scaled to the row's
/// amount. A row naming no paid deposit invoice is put under `""`: an
/// account no row uses.
fn deposit_receipts(
    index: &DocumentIndex,
    loaded: &Loaded,
    booking: &InvoiceBooking,
) -> Result<Vec<(String, Dec)>, BooksError> {
    let mut out: Vec<(String, Dec)> = Vec::new();
    let mut add = |account: &str, amount: Dec| -> Result<(), BooksError> {
        match out.iter_mut().find(|(a, _)| a == account) {
            Some((_, sum)) => *sum = sum.add(amount).map_err(|_| BooksError::Overflow)?,
            None => out.push((account.to_string(), amount)),
        }
        Ok(())
    };
    for line in booking
        .lines
        .iter()
        .filter(|l| l.cash == CashBooking::Nothing)
    {
        let applied = line.amount.neg().map_err(|_| BooksError::Overflow)?;
        let label = line.item.as_deref().unwrap_or_default();
        let receipt = index
            .numbers()
            .filter(|n| *n != booking.number && names(label, n))
            .find_map(|n| {
                let by_account = received_by_account(loaded, n, booking).ok()?;
                (!by_account.is_empty()).then_some(by_account)
            });
        match receipt {
            Some(by_account) => {
                for (account, amount) in scale(applied, &by_account)? {
                    add(&account, amount)?;
                }
            }
            None => add("", applied)?,
        }
    }
    Ok(out)
}

/// What invoice `number`'s applied payments put in each account, for
/// `booking`'s customer and commodity.
fn received_by_account(
    loaded: &Loaded,
    number: &str,
    booking: &InvoiceBooking,
) -> Result<Vec<(String, Dec)>, BooksError> {
    let mut out: Vec<(String, Dec)> = Vec::new();
    for txn in applied_payments(loaded, number) {
        for posting in &txn.postings {
            if tag_value(&posting.tags, CUSTOMER_TAG) != Some(booking.customer.as_str()) {
                continue;
            }
            for amount in posting
                .amounts
                .iter()
                .filter(|a| a.commodity.0 == booking.commodity)
            {
                let credit = amount.quantity.neg().map_err(|_| BooksError::Overflow)?;
                match out.iter_mut().find(|(a, _)| *a == posting.account.0) {
                    Some((_, sum)) => *sum = sum.add(credit).map_err(|_| BooksError::Overflow)?,
                    None => out.push((posting.account.0.clone(), credit)),
                }
            }
        }
    }
    out.retain(|(_, d)| d.mantissa > 0);
    Ok(out)
}

/// `payment-allocation = "match-rows"`: the split of what is due so that,
/// once paid, each row account holds the invoice's row for it, counting
/// what `receipts` (the deposits applied) already put there.
///
/// - Each row's target is its amount less the deposit already in its
///   account.
/// - A deposit larger than its row's amount leaves that row at zero (a
///   revenue row is never split negative); the excess comes off the other
///   rows, pro rata.
/// - A deposit in an account no row uses stays there; the rows give up the
///   same amount, pro rata.
/// - Contra rows (negative, a discount account) keep their amount.
///
/// The result sums to what is due.
fn match_rows(
    booking: &InvoiceBooking,
    receipts: &[(String, Dec)],
) -> Result<Vec<(String, Dec)>, BooksError> {
    let overflow = |_| BooksError::Overflow;
    let mut targets: Vec<(String, Dec)> = Vec::new();
    for (account, row) in booking.cash_rows()? {
        let deposit = receipts
            .iter()
            .find(|(a, _)| *a == account)
            .map_or(Dec::zero(), |(_, d)| *d);
        let target = row.sub(deposit).map_err(overflow)?;
        let target = if row.mantissa > 0 && target.mantissa < 0 {
            Dec::zero()
        } else {
            target
        };
        targets.push((account, target));
    }
    let sum = sum(targets.iter().map(|(_, d)| *d))?;
    let over = sum.sub(booking.total).map_err(overflow)?;
    if !over.is_zero() {
        let positive: Vec<(String, Dec)> = targets
            .iter()
            .filter(|(_, d)| d.mantissa > 0)
            .cloned()
            .collect();
        if positive.is_empty() {
            return booking.cash_split();
        }
        for (account, cut) in scale(over, &positive)? {
            if let Some((_, target)) = targets.iter_mut().find(|(a, _)| *a == account) {
                *target = target.sub(cut).map_err(overflow)?;
            }
        }
    }
    targets.retain(|(_, d)| !d.is_zero());
    Ok(targets)
}

fn sum(values: impl IntoIterator<Item = Dec>) -> Result<Dec, BooksError> {
    values
        .into_iter()
        .try_fold(Dec::zero(), |acc, d| acc.add(d))
        .map_err(|_| BooksError::Overflow)
}

/// An invoice's cash position: its split, what each row has received, and
/// the rest.
struct Owed {
    path: VaultPath,
    booking: InvoiceBooking,
    split: Vec<(String, Dec)>,
    paid: Vec<Dec>,
    remaining: Dec,
}

impl InvoiceBooks {
    fn owed(
        &self,
        index: &DocumentIndex,
        number: &str,
        loaded: &Loaded,
    ) -> Result<Owed, BooksError> {
        let document = self.document(index, number)?;
        match document.invoice.header().status {
            Some(Status::Sent) => {}
            _ => {
                return Err(BooksError::Status {
                    number: number.to_string(),
                    status: document.status_text(),
                    hint: "only a sent invoice takes a payment".into(),
                });
            }
        }
        if Basis::of(loaded) == Basis::Accrual {
            return Err(BooksError::Refused(format!(
                "these are accrual books: invoice {number}'s payment settles its receivable \
                 (link it in Ledgeline); Apply payment is for cash books"
            )));
        }
        let booking = self.booking(index, &document, loaded)?;
        let split = match index.settings().payment_allocation {
            PaymentAllocation::MatchRows if booking.applies_a_received_deposit() => {
                match_rows(&booking, &deposit_receipts(index, loaded, &booking)?)?
            }
            _ => booking.cash_split()?,
        };
        let paid = paid_so_far(loaded, &booking, &split)?;
        let remaining = booking
            .total
            .sub(sum(paid.iter().copied())?)
            .map_err(|_| BooksError::Overflow)?;
        Ok(Owed {
            path: document.path,
            booking,
            split,
            paid,
            remaining,
        })
    }

    fn candidates_of(loaded: &Loaded, owed: &Owed, today: &str) -> Vec<(Tindex, PaymentCandidate)> {
        if owed.remaining.mantissa <= 0 {
            return Vec::new();
        }
        let invoice = CashInvoice {
            number: owed.booking.number.clone(),
            party: Some(Party {
                kind: PartyKind::Customer,
                name: owed.booking.customer.clone(),
            }),
            date: owed.booking.date.clone(),
            due: money(loaded.journal(), &owed.booking.commodity, owed.remaining),
        };
        let found = apply::candidates(
            loaded.journal(),
            &invoice,
            &SettleSettings::default(),
            today,
        );
        let tindices: Vec<Tindex> = found.iter().map(|c| c.tindex).collect();
        found
            .into_iter()
            .zip(loaded.editor().transaction_fingerprints(&tindices))
            .filter_map(|(c, fingerprint)| {
                let txn = loaded.journal().transactions.get(c.position)?;
                Some((
                    c.tindex,
                    PaymentCandidate {
                        reference: fingerprint?.token(),
                        date: txn.date.clone(),
                        description: txn.description.clone(),
                        received: c.received,
                        exact: c.fit == Fit::Exact,
                        by_number: c.by_number,
                        by_customer: c.by_party,
                        gap_days: c.gap_days,
                        text: loaded.editor().transaction_source(c.tindex)?,
                    },
                ))
            })
            .collect()
    }

    /// The deposits that might pay sent invoice `number` (cash books), best
    /// first; none once it is paid.
    ///
    /// # Errors
    /// [`BooksError::Status`] for an invoice not sent;
    /// [`BooksError::Refused`] in accrual books; as
    /// [`InvoiceBooks::booking`].
    pub fn payment_candidates(
        &self,
        index: &DocumentIndex,
        number: &str,
        today: &str,
    ) -> Result<Vec<PaymentCandidate>, BooksError> {
        Ok(self.payment_options(index, number, today)?.candidates)
    }

    /// What is due on sent invoice `number` (cash books) and the deposits
    /// that might pay it, best first.
    ///
    /// # Errors
    /// As [`InvoiceBooks::payment_candidates`].
    pub fn payment_options(
        &self,
        index: &DocumentIndex,
        number: &str,
        today: &str,
    ) -> Result<PaymentOptions, BooksError> {
        let loaded = self.load()?;
        let owed = self.owed(index, number, &loaded)?;
        let candidates = Self::candidates_of(&loaded, &owed, today)
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        Ok(PaymentOptions {
            number: number.to_string(),
            path: owed.path.clone(),
            commodity: owed.booking.commodity.clone(),
            total: owed.booking.total,
            paid: sum(owed.paid.iter().copied())?,
            remaining: owed.remaining,
            candidates,
        })
    }

    /// **Apply payment** (I17): re-split the deposit `request.reference`
    /// into invoice `request.number`'s rows — pro rata when it pays part,
    /// each row's remainder when it pays the rest — tag it `invoice:` and
    /// its new postings `customer:`, and keep every other line byte for byte.
    /// The markdown is not written: the reconciler mirrors `paid:` from the
    /// books.
    ///
    /// # Errors
    /// [`BooksError::Refused`] for a reference that is no longer a candidate
    /// (edited, applied elsewhere, or more than is due); as
    /// [`InvoiceBooks::payment_candidates`]; [`BooksError::Ops`] —
    /// `ExternalChange` when the journal changed since it was read; nothing
    /// written.
    pub fn apply_payment(
        &self,
        index: &DocumentIndex,
        request: &ApplyPayment,
    ) -> Result<PaymentApplied, BooksError> {
        let mut loaded = self.load()?;
        check_generation(&loaded, request.expected.as_ref())?;
        let owed = self.owed(index, &request.number, &loaded)?;
        let candidates = Self::candidates_of(&loaded, &owed, &request.today);
        let wanted = request.reference.trim();
        let exact = candidates.iter().position(|(_, c)| c.reference == wanted);
        let by_prefix: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, (_, c))| {
                wanted.len() >= MIN_REFERENCE_PREFIX && c.reference.starts_with(wanted)
            })
            .map(|(at, _)| at)
            .collect();
        let chosen = match (exact, by_prefix.as_slice()) {
            (Some(at), _) | (None, &[at]) => Some(at),
            (None, []) => None,
            (None, _) => {
                return Err(BooksError::Refused(format!(
                    "{wanted} names {} deposits; give more of its reference",
                    by_prefix.len()
                )));
            }
        };
        let (tindex, candidate) = chosen
            .and_then(|at| candidates.into_iter().nth(at))
            .ok_or_else(|| {
                BooksError::Refused(format!(
                    "{} is not a deposit that can pay invoice {} (it changed since it was \
                     listed, is already applied, or is more than the {} still due)",
                    request.reference,
                    request.number,
                    ledgeline_core::edit::render_dec(owed.remaining, '.')
                ))
            })?;
        // Paying the rest: each row's remainder, exactly. Paying part: pro
        // rata over what is left on each row (match-rows), or over the whole
        // split (pro-rata).
        let by_remainder = candidate.received == owed.remaining
            || index.settings().payment_allocation == PaymentAllocation::MatchRows;
        let split: Vec<(String, Dec)> = if by_remainder {
            owed.split
                .iter()
                .zip(&owed.paid)
                .map(|((account, amount), paid)| {
                    amount
                        .sub(*paid)
                        .map(|rest| (account.clone(), rest))
                        .map_err(|_| BooksError::Overflow)
                })
                .collect::<Result<_, _>>()?
        } else {
            owed.split.clone()
        };
        let fingerprint = loaded
            .editor()
            .transaction_fingerprint(tindex)
            .ok_or(ledgeline_core::accrual::ops::OpsError::Stale)?;
        let applied = loaded.apply_payment(
            &apply::ApplyPayment {
                tindex,
                fingerprint,
                number: request.number.clone(),
                party: Some(Party {
                    kind: PartyKind::Customer,
                    name: owed.booking.customer.clone(),
                }),
                split,
            },
            None,
        )?;
        let transaction = BookedTransaction {
            date: applied.transaction.date.clone(),
            description: applied.transaction.description.clone(),
            text: loaded
                .editor()
                .transaction_source(tindex)
                .unwrap_or_default(),
        };
        let remaining = owed
            .remaining
            .sub(applied.received)
            .map_err(|_| BooksError::Overflow)?;
        Ok(PaymentApplied {
            number: request.number.clone(),
            transaction,
            received: applied.received,
            applied: applied
                .applied
                .iter()
                .map(|(account, amount)| {
                    (
                        account.clone(),
                        ledgeline_core::edit::render_dec(*amount, '.'),
                    )
                })
                .collect(),
            remaining,
            undo: PaymentUndo {
                number: request.number.clone(),
                reference: applied.undo.fingerprint.token(),
                original_text: applied.undo.original_text,
            },
        })
    }

    /// Undo an [`apply_payment`](Self::apply_payment): the deposit's
    /// original text, written back exactly.
    ///
    /// # Errors
    /// [`BooksError::Ops`] — `Stale` when no deposit tagged with the number
    /// still has the text the apply wrote (it was edited since), or
    /// `ExternalChange`; nothing written.
    pub fn undo_payment(
        &self,
        undo: &PaymentUndo,
        expected: Option<&Generation>,
    ) -> Result<(), BooksError> {
        let mut loaded = self.load()?;
        check_generation(&loaded, expected)?;
        let tagged: Vec<Tindex> = applied_payments(&loaded, &undo.number)
            .iter()
            .map(|txn| txn.index)
            .collect();
        let fingerprint = loaded
            .editor()
            .transaction_fingerprints(&tagged)
            .into_iter()
            .flatten()
            .find(|f| f.token() == undo.reference)
            .ok_or(ledgeline_core::accrual::ops::OpsError::Stale)?;
        loaded.undo_apply_payment(
            &ApplyUndo {
                number: undo.number.clone(),
                fingerprint,
                original_text: undo.original_text.clone(),
            },
            None,
        )?;
        Ok(())
    }
}
