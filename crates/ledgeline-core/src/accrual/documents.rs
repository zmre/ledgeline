//! Finding a document's transaction again (invoicer L3, ARCHITECTURE §4.3).
//!
//! `ledgeline-core` has no stable transaction id: a [`Tindex`] shifts on any
//! insert. A program that booked an invoice re-finds it after every load by
//! its number: the header tag [`INVOICE_TAG`], matched **exactly**, on a
//! transaction with an **invoice-side settlement posting** — a receivable
//! debit for an invoice, a payable credit for a bill.
//!
//! That rule leaves out, by construction, everything else that carries the
//! same number:
//!
//! - a payment tagged `invoice:` (its settlement posting runs the other way);
//! - a void reversal (it carries [`VOID_TAG`]);
//! - a cash-books **Apply payment** deposit (no settlement posting at all);
//! - a posted recognition entry or anything generated.
//!
//! Invoices and bills are separate namespaces ([`AccrualRole::Receivable`],
//! [`AccrualRole::Payable`]): a vendor's bill 105 is not your invoice 105.
//!
//! [`DocumentIndex::build`] is one pass over the journal, so a caller that
//! needs every invoice's transaction (a reconciler) pays for one scan, not one
//! per invoice.

use super::{AccrualRole, AccrualRoles, FxHashMap, INVOICE_TAG, VOID_TAG, tag_value};
use crate::edit::Fingerprint;
use crate::matching::PostingRef;
use crate::model::{Journal, PostingType, Tindex, Transaction};
use crate::projections::is_generated;

/// The invoice-side settlement posting of `txn`, and its role: the first real
/// posting with one cost-free, non-zero amount on a receivable account as a
/// debit, or on a payable account as a credit. `None` for a void reversal, a
/// recognition entry, or anything generated.
#[must_use]
pub fn document_posting(txn: &Transaction, roles: &AccrualRoles) -> Option<(usize, AccrualRole)> {
    if is_generated(txn)
        || super::is_recognition_entry(txn)
        || tag_value(&txn.tags, VOID_TAG).is_some()
    {
        return None;
    }
    txn.postings.iter().enumerate().find_map(|(at, posting)| {
        if posting.generated.is_some() || posting.ptype != PostingType::Regular {
            return None;
        }
        let role = roles.role(&posting.account.0)?;
        let [amount] = posting.amounts.as_slice() else {
            return None;
        };
        if amount.cost.is_some() || amount.quantity.is_zero() {
            return None;
        }
        let debit = amount.quantity.mantissa > 0;
        match (role, debit) {
            (AccrualRole::Receivable, true) | (AccrualRole::Payable, false) => Some((at, role)),
            _ => None,
        }
    })
}

/// Every numbered document in a journal, by role and number.
#[derive(Debug, Clone, Default)]
pub struct DocumentIndex {
    by_key: FxHashMap<(AccrualRole, String), Vec<PostingRef>>,
}

impl DocumentIndex {
    /// One pass over `journal`: every transaction with a non-empty
    /// [`INVOICE_TAG`] and an invoice-side settlement posting
    /// ([`document_posting`]).
    #[must_use]
    pub fn build(journal: &Journal, roles: &AccrualRoles) -> Self {
        let mut by_key: FxHashMap<(AccrualRole, String), Vec<PostingRef>> = FxHashMap::default();
        for (position, txn) in journal.transactions.iter().enumerate() {
            let Some(number) = tag_value(&txn.tags, INVOICE_TAG).filter(|n| !n.is_empty()) else {
                continue;
            };
            let Some((posting, role)) = document_posting(txn, roles) else {
                continue;
            };
            by_key
                .entry((role, number.to_string()))
                .or_default()
                .push(PostingRef {
                    position,
                    txn: txn.index,
                    posting,
                });
        }
        Self { by_key }
    }

    /// The settlement postings of every `role` document numbered `number`, in
    /// file order: one, normally; none when it isn't booked; more when the
    /// number is used twice (a `check` finding).
    #[must_use]
    pub fn get(&self, role: AccrualRole, number: &str) -> &[PostingRef] {
        self.by_key
            .get(&(role, number.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// Every `(role, number)` used by more than one document, sorted.
    #[must_use]
    pub fn duplicates(&self) -> Vec<(AccrualRole, &str, &[PostingRef])> {
        let mut out: Vec<_> = self
            .by_key
            .iter()
            .filter(|(_, refs)| refs.len() > 1)
            .map(|((role, number), refs)| (*role, number.as_str(), refs.as_slice()))
            .collect();
        out.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        out
    }

    /// Every `(role, number, settlement postings)`, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (AccrualRole, &str, &[PostingRef])> {
        self.by_key
            .iter()
            .map(|((role, number), refs)| (*role, number.as_str(), refs.as_slice()))
    }

    /// How many distinct `(role, number)` keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Whether no document is numbered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

/// A booked document, as [`super::ops::Loaded::find_document`] finds it:
/// everything a re-book or a void needs to edit it safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundDocument {
    /// Its id in this load (it shifts on any insert).
    pub tindex: Tindex,
    /// Its position in `journal.transactions`.
    pub position: usize,
    /// The fingerprint of its source text: the optimistic-concurrency token
    /// for a [`super::ops::SourceChange::Replace`].
    pub fingerprint: Fingerprint,
    /// The transaction.
    pub transaction: Transaction,
    /// Its invoice-side settlement posting.
    pub settlement: PostingRef,
    /// Invoice (receivable) or bill (payable).
    pub role: AccrualRole,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;

    #[test]
    fn only_invoice_side_documents_are_indexed_by_role_and_number() {
        let journal = parse_journal(
            "account assets:ar  ; type: A, kpi: receivable\n\
             account liabilities:ap  ; type: L, kpi: payable\n\
             account assets:bank  ; type: C\n\
             2026-01-01 Invoice 105  ; invoice: 105\n    assets:ar  $100\n    revenue\n\
             2026-01-02 Bill 105  ; invoice: 105\n    expenses  $50\n    liabilities:ap\n\
             2026-01-03 Pay 105  ; invoice: 105\n    assets:bank  $100\n    assets:ar\n\
             2026-01-04 Void 105  ; invoice: 105, void: 2026-01-04\n    assets:ar  $-100\n    revenue\n\
             2026-01-05 Cash 105  ; invoice: 105\n    assets:bank  $100\n    revenue\n\
             2026-01-06 Invoice 106  ; invoice: 106\n    assets:ar  $10\n    revenue\n\
             2026-01-07 Copy 106  ; invoice: 106\n    assets:ar  $10\n    revenue\n\
             2026-01-08 Invoice 1050  ; invoice: 1050\n    assets:ar  $10\n    revenue\n",
            "inline.journal",
        )
        .expect("parses");
        let index = DocumentIndex::build(&journal, &AccrualRoles::of(&journal));
        let positions = |role, number| {
            index
                .get(role, number)
                .iter()
                .map(|at| at.position)
                .collect::<Vec<_>>()
        };
        assert_eq!(positions(AccrualRole::Receivable, "105"), vec![0]);
        assert_eq!(positions(AccrualRole::Payable, "105"), vec![1]);
        assert_eq!(positions(AccrualRole::Receivable, "106"), vec![5, 6]);
        assert_eq!(
            positions(AccrualRole::Receivable, "10"),
            Vec::<usize>::new()
        );
        assert_eq!(index.get(AccrualRole::Receivable, "105")[0].posting, 0);
        let duplicates: Vec<_> = index
            .duplicates()
            .into_iter()
            .map(|(role, number, refs)| (role, number, refs.len()))
            .collect();
        assert_eq!(duplicates, vec![(AccrualRole::Receivable, "106", 2)]);
        assert_eq!(index.len(), 4);
    }
}
