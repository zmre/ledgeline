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

use super::lifecycle::{BookedTransaction, applied_payments};
use super::spec::InvoiceBooking;
use super::{BooksError, InvoiceBooks, check_generation};
use crate::books::lifecycle::Basis;
use crate::doc::Status;
use crate::index::DocumentIndex;
use ledgeline_core::Dec;
use ledgeline_core::accrual::apply::{self, ApplyUndo, CashInvoice};
use ledgeline_core::accrual::invoice::money;
use ledgeline_core::accrual::ops::{Generation, Loaded};
use ledgeline_core::accrual::schedule::{Party, PartyKind};
use ledgeline_core::accrual::settle::{Fit, SettleSettings};
use ledgeline_core::accrual::{CUSTOMER_TAG, tag_value};
use ledgeline_core::model::Tindex;
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

/// **Apply payment**: re-split the deposit `reference` into invoice
/// `number`'s rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPayment {
    /// The invoice.
    pub number: String,
    /// The deposit: a [`PaymentCandidate::reference`].
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

fn sum(values: impl IntoIterator<Item = Dec>) -> Result<Dec, BooksError> {
    values
        .into_iter()
        .try_fold(Dec::zero(), |acc, d| acc.add(d))
        .map_err(|_| BooksError::Overflow)
}

/// An invoice's cash position: its split, what each row has received, and
/// the rest.
struct Owed {
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
        let split = booking.cash_split()?;
        let paid = paid_so_far(loaded, &booking, &split)?;
        let remaining = booking
            .total
            .sub(sum(paid.iter().copied())?)
            .map_err(|_| BooksError::Overflow)?;
        Ok(Owed {
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
        let loaded = self.load()?;
        let owed = self.owed(index, number, &loaded)?;
        Ok(Self::candidates_of(&loaded, &owed, today)
            .into_iter()
            .map(|(_, c)| c)
            .collect())
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
        let (tindex, candidate) = Self::candidates_of(&loaded, &owed, &request.today)
            .into_iter()
            .find(|(_, c)| c.reference == request.reference)
            .ok_or_else(|| {
                BooksError::Refused(format!(
                    "{} is not a deposit that can pay invoice {} (it changed since it was \
                     listed, is already applied, or is more than the {} still due)",
                    request.reference,
                    request.number,
                    ledgeline_core::edit::render_dec(owed.remaining, '.')
                ))
            })?;
        // Paying the rest: each row's remainder, exactly. Paying part: the
        // whole split as the weights, pro rata.
        let split: Vec<(String, Dec)> = if candidate.received == owed.remaining {
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
