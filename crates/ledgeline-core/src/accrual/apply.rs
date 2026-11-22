//! **Apply payment** for cash books (invoicer I17, PLAN Phase 2b): an
//! invoice is never booked in cash books, so when its payment arrives — a
//! deposit Ledgeline imported or synced — the deposit's income side is
//! re-split into the invoice's row accounts and tagged with the invoice.
//!
//! - The **income side** is every non-cash credit leg of the deposit (an
//!   imported `income:unknown`, say). Debit legs (a processor's fee) and the
//!   cash legs stay exactly as written — byte for byte, through
//!   [`EditOp::ReplacePostings`](crate::edit::EditOp::ReplacePostings).
//! - The income side's total is what was **received**. Equal to the invoice:
//!   each row account gets its share. Less: a **partial** payment, split pro
//!   rata over the rows with exact [`Dec`] shares that sum to what was
//!   received, the remainder on the largest row. More: refused (one deposit
//!   paying two invoices is M:N, later).
//! - A row may run against the invoice: a final invoice applying a deposit
//!   has a negative row on the deposit liability, which the payment debits
//!   (its share pro rata too, when partial).
//! - The header gains `invoice:` and each new posting `customer:`; nothing
//!   else changes. The transaction is never a document ([`super::documents`]
//!   needs a settlement posting), so `find_document` never finds it.
//! - [`ApplyUndo`] carries the deposit's original text; undoing writes it
//!   back exactly ([`EditOp::SetText`](crate::edit::EditOp::SetText)).
//!
//! [`candidates`] ranks deposits for an invoice with plan 45's settlement
//! signals ([`super::settle`]) less the receivable leg: the invoice number in
//! the description, then the customer (tagged, or learned from deposits
//! already applied), then the amount (exact, then short — a partial
//! payment), then the smallest date gap.

use super::ops::OpsError;
use super::schedule::Party;
use super::settle::{Fit, SettleSettings, gap_between, has_word, party_of};
use super::{AccrualRoles, CUSTOMER_TAG, INVOICE_TAG, day_of, tag_value};
use crate::decimal::{Dec, allocate};
use crate::edit::{EditOp, Fingerprint};
use crate::model::{Amount, AmountStyle, Journal, PostingType, Tindex, Transaction};
use crate::projections::is_generated;
use crate::reports::{AccountType, AccountTypes};
use crate::review::similarity_key;
use std::collections::{BTreeMap, HashMap};

/// What [`super::ops::Loaded::apply_payment`] is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPayment {
    /// The deposit transaction.
    pub tindex: Tindex,
    /// Its fingerprint as the caller saw it.
    pub fingerprint: Fingerprint,
    /// The invoice number (`invoice:` on the header).
    pub number: String,
    /// The customer (`customer:` on each new posting).
    pub party: Option<Party>,
    /// The invoice's split: each row account and its total on the invoice,
    /// in the deposit's commodity, signed as the invoice runs — positive for
    /// revenue, negative for a row against it (a deposit applied on a final
    /// invoice debits the deposit liability). An account named twice is
    /// merged; the split must sum to more than zero.
    pub split: Vec<(String, Dec)>,
}

/// What undoing an apply needs: the deposit as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyUndo {
    /// The invoice number it was applied to.
    pub number: String,
    /// The fingerprint of the deposit as the apply wrote it: the undo is
    /// refused if it changed since.
    pub fingerprint: Fingerprint,
    /// The deposit's source text before the apply.
    pub original_text: String,
}

/// What an apply wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedPayment {
    /// The deposit as written.
    pub transaction: Transaction,
    /// What the income side held: a positive magnitude.
    pub received: Dec,
    /// Each row account and what it received, in split order (negative for
    /// a row against the invoice, such as a deposit applied).
    pub applied: Vec<(String, Dec)>,
    /// Whether it pays less than the invoice.
    pub partial: bool,
    /// How to undo it.
    pub undo: ApplyUndo,
}

/// The edit an apply makes, planned against one deposit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPlan {
    /// The deposit's ops, for [`crate::edit::JournalEditor::apply_batch`].
    pub ops: Vec<EditOp>,
    /// What the income side held.
    pub received: Dec,
    /// Each row account and its share.
    pub applied: Vec<(String, Dec)>,
    /// Whether it pays less than the invoice.
    pub partial: bool,
}

fn refused<T>(why: impl Into<String>) -> Result<T, OpsError> {
    Err(OpsError::Refused(why.into()))
}

/// The written, non-cash credit legs of `txn` — its income side — and their
/// common amount (the first one's), or why `txn` isn't a deposit.
fn income_side<'t>(
    txn: &'t Transaction,
    types: &AccountTypes,
    roles: &AccrualRoles,
) -> Result<(Vec<usize>, &'t Amount, Dec), String> {
    if is_generated(txn) || super::is_recognition_entry(txn) {
        return Err("it is not a transaction written in the journal".into());
    }
    let written: Vec<_> = txn
        .postings
        .iter()
        .enumerate()
        .filter(|(_, p)| p.generated.is_none())
        .collect();
    if !written.iter().any(|(_, p)| {
        types.is_cash(&p.account.0) && p.amounts.iter().any(|a| a.quantity.mantissa > 0)
    }) {
        return Err("it has no cash deposit leg".into());
    }
    let mut income = Vec::new();
    let mut total = Dec::zero();
    let mut first: Option<&Amount> = None;
    for (at, posting) in written {
        if types.is_cash(&posting.account.0) {
            continue;
        }
        let [amount] = posting.amounts.as_slice() else {
            return Err(format!(
                "its posting on {} has no single amount",
                posting.account.0
            ));
        };
        if amount.quantity.mantissa >= 0 {
            continue;
        }
        if roles
            .role(&posting.account.0)
            .is_some_and(|r| r.is_settlement())
        {
            return Err(format!(
                "it credits {}: a payment of a booked invoice, not a cash-books deposit",
                posting.account.0
            ));
        }
        if posting.ptype != PostingType::Regular
            || amount.cost.is_some()
            || posting.balance_assertion.is_some()
        {
            return Err(format!(
                "its posting on {} is virtual, priced or asserted",
                posting.account.0
            ));
        }
        if first.is_some_and(|f| f.commodity != amount.commodity) {
            return Err("its income side is in more than one commodity".into());
        }
        first.get_or_insert(amount);
        total = total
            .sub(amount.quantity)
            .map_err(|_| "the amounts are too large".to_string())?;
        income.push(at);
    }
    match first {
        Some(amount) => Ok((income, amount, total)),
        None => Err("it has no income side to re-split".into()),
    }
}

/// `received` split over `weights` pro rata, exactly, the remainder on the
/// largest weight (the first of equals). A negative weight (a deposit
/// applied) gets its negative share; the weights sum to more than zero.
fn pro_rata(received: Dec, weights: &[Dec]) -> Result<Vec<Dec>, OpsError> {
    let largest = weights.iter().enumerate().fold(
        0,
        |best, (at, w)| if *w > weights[best] { at } else { best },
    );
    // `allocate` puts the remainder on the LAST weight: move the largest there.
    let mut order: Vec<usize> = (0..weights.len()).filter(|&at| at != largest).collect();
    order.push(largest);
    let ordered: Vec<Dec> = order.iter().map(|&at| weights[at]).collect();
    let shares = allocate(received, &ordered)
        .map_err(|_| OpsError::Refused("the split can't be divided exactly".into()))?;
    let mut out = vec![Dec::zero(); weights.len()];
    for (at, share) in order.into_iter().zip(shares) {
        out[at] = share;
    }
    Ok(out)
}

/// Plan applying `request` to the deposit `txn` of `journal`.
///
/// # Errors
/// [`OpsError::Refused`] when `txn` isn't a cash-books deposit (no cash
/// debit, no income side, a settlement leg, mixed commodities), is already
/// tagged `invoice:`, the split is empty or sums to zero or less, or the deposit is
/// more than the invoice; [`OpsError::Document`] for an unusable account or
/// tag value.
pub fn plan(
    journal: &Journal,
    txn: &Transaction,
    request: &ApplyPayment,
) -> Result<ApplyPlan, OpsError> {
    use super::invoice::DocumentError;
    super::schedule::check_tag_value("the invoice number", &request.number)
        .map_err(|e| OpsError::Document(DocumentError::Invalid(e.to_string())))?;
    if let Some(party) = &request.party {
        super::schedule::check_tag_value(party.kind.tag(), &party.name)
            .map_err(|e| OpsError::Document(DocumentError::Invalid(e.to_string())))?;
    }
    if let Some(existing) = tag_value(&txn.tags, INVOICE_TAG) {
        return refused(format!(
            "this deposit is already applied to invoice {existing}; undo that first"
        ));
    }
    let types = AccountTypes::new(&crate::reports::account_decls(journal));
    let roles = AccrualRoles::of(journal);
    let (income, like, received) = income_side(txn, &types, &roles).map_err(|why| {
        OpsError::Refused(format!("this transaction can't take a payment: {why}"))
    })?;
    // The split, merged by account in first-seen order.
    let mut split: Vec<(String, Dec)> = Vec::new();
    for (account, amount) in &request.split {
        crate::periodic::check_account(account)
            .map_err(|e| OpsError::Document(DocumentError::Invalid(e.to_string())))?;
        if amount.is_zero() {
            return refused(format!("the split's amount on {account} is zero"));
        }
        match split.iter_mut().find(|(a, _)| a == account) {
            Some((_, total)) => {
                *total = total
                    .add(*amount)
                    .map_err(|_| OpsError::Document(DocumentError::Overflow))?;
            }
            None => split.push((account.clone(), *amount)),
        }
    }
    split.retain(|(_, amount)| !amount.is_zero());
    if split.is_empty() {
        return refused("the invoice's split is empty");
    }
    let invoiced = split
        .iter()
        .try_fold(Dec::zero(), |sum, (_, amount)| sum.add(*amount))
        .map_err(|_| OpsError::Document(DocumentError::Overflow))?;
    if invoiced.mantissa <= 0 {
        return refused("the invoice's split must sum to more than zero");
    }
    let partial = match received.cmp(&invoiced) {
        std::cmp::Ordering::Greater => {
            return refused(format!(
                "the deposit ({}) is more than invoice {} ({}); one deposit for several \
                 invoices isn't supported yet",
                crate::edit::render_dec(received, '.'),
                request.number,
                crate::edit::render_dec(invoiced, '.')
            ));
        }
        std::cmp::Ordering::Equal => false,
        std::cmp::Ordering::Less => true,
    };
    let shares: Vec<Dec> = if partial {
        let weights: Vec<Dec> = split.iter().map(|(_, amount)| *amount).collect();
        pro_rata(received, &weights)?
    } else {
        split.iter().map(|(_, amount)| *amount).collect()
    };
    let party_tag: Vec<(&str, String)> = request
        .party
        .iter()
        .map(|party| (party.kind.tag(), party.name.clone()))
        .collect();
    let applied: Vec<(String, Dec)> = split
        .iter()
        .zip(&shares)
        .filter(|(_, share)| !share.is_zero())
        .map(|((account, _), share)| (account.clone(), *share))
        .collect();
    let with = applied
        .iter()
        .map(|(account, share)| {
            let quantity = share
                .neg()
                .map_err(|_| OpsError::Document(DocumentError::Overflow))?;
            // In the journal's own style for the commodity (an elided leg's
            // inferred amount carries no grouping of its own).
            let styled = super::invoice::money(journal, &like.commodity.0, quantity);
            let amount = Amount {
                style: AmountStyle {
                    precision: styled.style.precision.max(quantity.places),
                    ..styled.style.clone()
                },
                ..styled
            };
            Ok(super::invoice::posting(account, amount, &party_tag))
        })
        .collect::<Result<Vec<_>, OpsError>>()?;
    Ok(ApplyPlan {
        ops: vec![
            EditOp::ReplacePostings {
                postings: income,
                with,
            },
            EditOp::SetTxnTag {
                name: INVOICE_TAG.to_string(),
                value: Some(request.number.clone()),
            },
        ],
        received,
        applied,
        partial,
    })
}

/// An unpaid invoice in cash books, as [`candidates`] looks for its deposit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashInvoice {
    /// Its number.
    pub number: String,
    /// Its customer.
    pub party: Option<Party>,
    /// Its date.
    pub date: String,
    /// What is still due on it, a positive amount.
    pub due: Amount,
}

/// A deposit that might pay a [`CashInvoice`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyCandidate {
    /// The deposit.
    pub tindex: Tindex,
    /// Its position in `journal.transactions`.
    pub position: usize,
    /// What its income side holds.
    pub received: Dec,
    /// [`Fit::Exact`], or [`Fit::Short`] for a partial payment.
    pub fit: Fit,
    /// Its description names the invoice number.
    pub by_number: bool,
    /// Its customer is the invoice's (tagged, or learned).
    pub by_party: bool,
    /// Days between the invoice and the deposit.
    pub gap_days: u32,
}

/// The payee similarity key → customer of the deposits already applied (a
/// header `invoice:` and a posting `customer:`, no settlement leg).
fn learned_customers(journal: &Journal, roles: &AccrualRoles) -> HashMap<String, String> {
    let mut counts: HashMap<String, BTreeMap<String, usize>> = HashMap::new();
    for txn in &journal.transactions {
        if tag_value(&txn.tags, INVOICE_TAG).is_none()
            || txn
                .postings
                .iter()
                .any(|p| roles.role(&p.account.0).is_some_and(|r| r.is_settlement()))
        {
            continue;
        }
        let customer = txn
            .postings
            .iter()
            .find_map(|p| tag_value(&p.tags, CUSTOMER_TAG).filter(|c| !c.is_empty()));
        if let (Some(customer), Some(key)) = (customer, similarity_key(&txn.description)) {
            *counts
                .entry(key)
                .or_default()
                .entry(customer.to_string())
                .or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter_map(|(key, names)| {
            names
                .into_iter()
                .max_by_key(|(_, n)| *n)
                .map(|(name, _)| (key, name))
        })
        .collect()
}

/// The deposits that might pay `invoice`, best first: dated no more than
/// `settings.lead_days` before it and inside the lookback before `today`, in
/// its commodity, not already applied, and holding no more than is due.
/// Ranked as plan 45 ranks settlements, less the receivable leg.
#[must_use]
pub fn candidates(
    journal: &Journal,
    invoice: &CashInvoice,
    settings: &SettleSettings,
    today: &str,
) -> Vec<ApplyCandidate> {
    let types = AccountTypes::new(&crate::reports::account_decls(journal));
    let roles = AccrualRoles::of(journal);
    let learned = learned_customers(journal, &roles);
    let earliest =
        day_of(&invoice.date) - i32::try_from(settings.lead_days).unwrap_or(i32::MAX / 2);
    let cutoff = (settings.lookback_days > 0)
        .then(|| day_of(today) - i32::try_from(settings.lookback_days).unwrap_or(i32::MAX / 2));
    let due = invoice.due.quantity;
    let mut out: Vec<ApplyCandidate> = journal
        .transactions
        .iter()
        .enumerate()
        .filter(|(_, txn)| {
            let day = day_of(&txn.date);
            day >= earliest
                && cutoff.is_none_or(|cutoff| day >= cutoff)
                && tag_value(&txn.tags, INVOICE_TAG).is_none()
        })
        .filter_map(|(position, txn)| {
            let (income, amount, received) = income_side(txn, &types, &roles).ok()?;
            if amount.commodity != invoice.due.commodity || received > due {
                return None;
            }
            // Only revenue-like income: a loan or an owner's contribution
            // is never an invoice's payment.
            let revenue_like = income.iter().all(|&at| {
                matches!(
                    types
                        .resolve(&txn.postings[at].account.0)
                        .map(crate::reports::account_types::root_type),
                    Some(AccountType::Revenue) | None
                )
            });
            if !revenue_like {
                return None;
            }
            let fit = if received == due {
                Fit::Exact
            } else {
                Fit::Short {
                    fee: due.sub(received).ok()?,
                }
            };
            let party = income
                .iter()
                .find_map(|&at| party_of(txn, &txn.postings[at]))
                .map(|p| p.name)
                .or_else(|| {
                    similarity_key(&txn.description).and_then(|key| learned.get(&key).cloned())
                });
            let by_party = invoice
                .party
                .as_ref()
                .is_some_and(|p| party.as_deref() == Some(p.name.as_str()));
            Some(ApplyCandidate {
                tindex: txn.index,
                position,
                received,
                fit,
                by_number: has_word(&txn.description, &invoice.number),
                by_party,
                gap_days: gap_between(&invoice.date, &txn.date).0,
            })
        })
        .collect();
    out.sort_by_key(|c| {
        (
            !c.by_number,
            !c.by_party,
            u8::from(c.fit != Fit::Exact),
            c.gap_days,
            c.position,
        )
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_journal;

    const BOOKS: &str = "\
account assets:bank  ; type: C
account income:unknown  ; type: R
account revenue:consulting  ; type: R
account revenue:delivery  ; type: R
account expenses:fees  ; type: X
account liabilities:loan  ; type: L

2026-03-01 ACME CORP ACH
    assets:bank  $1000.00
    income:unknown

2026-03-02 Stripe payout
    assets:bank  $970.00
    expenses:fees  $30.00
    income:unknown  $-1000.00

2026-03-03 Bank loan
    assets:bank  $5000.00
    liabilities:loan

2026-03-04 Wire INV 105
    assets:bank  $250.00
    income:unknown
";

    fn request(journal: &Journal, position: usize, split: &[(&str, i128)]) -> ApplyPayment {
        let _ = journal;
        ApplyPayment {
            tindex: Tindex(u32::try_from(position + 1).expect("small")),
            fingerprint: Fingerprint::of_bytes(b""),
            number: "105".into(),
            party: Some(Party {
                kind: super::super::schedule::PartyKind::Customer,
                name: "Acme".into(),
            }),
            split: split
                .iter()
                .map(|(a, cents)| ((*a).to_string(), Dec::new(*cents, 2)))
                .collect(),
        }
    }

    #[test]
    fn an_exact_payment_takes_the_split_and_a_partial_one_splits_pro_rata() {
        let journal = parse_journal(BOOKS, "inline.journal").expect("parses");
        let exact = plan(
            &journal,
            &journal.transactions[0],
            &request(
                &journal,
                0,
                &[("revenue:consulting", 60_000), ("revenue:delivery", 40_000)],
            ),
        )
        .expect("plans");
        assert!(!exact.partial);
        assert_eq!(exact.received, Dec::new(100_000, 2));
        assert_eq!(
            exact.applied,
            vec![
                ("revenue:consulting".to_string(), Dec::new(60_000, 2)),
                ("revenue:delivery".to_string(), Dec::new(40_000, 2))
            ]
        );
        // $250 of a $1,000 invoice split 333.33 / 333.33 / 333.34: shares
        // 83.33 + 83.33 + 83.34, the cent on the largest row.
        let partial = plan(
            &journal,
            &journal.transactions[3],
            &request(
                &journal,
                3,
                &[
                    ("revenue:consulting", 33_333),
                    ("revenue:delivery", 33_334),
                    ("revenue:consulting:extra", 33_333),
                ],
            ),
        )
        .expect("plans");
        assert!(partial.partial);
        let shares: Vec<Dec> = partial.applied.iter().map(|(_, d)| *d).collect();
        assert_eq!(
            shares,
            vec![Dec::new(8_333, 2), Dec::new(8_334, 2), Dec::new(8_333, 2)]
        );
        let sum = shares
            .iter()
            .try_fold(Dec::zero(), |s, d| s.add(*d))
            .expect("sums");
        assert_eq!(sum, Dec::new(25_000, 2));
    }

    #[test]
    fn fees_stay_and_non_deposits_and_overpayments_are_refused() {
        let journal = parse_journal(BOOKS, "inline.journal").expect("parses");
        let stripe = plan(
            &journal,
            &journal.transactions[1],
            &request(&journal, 1, &[("revenue:consulting", 100_000)]),
        )
        .expect("plans");
        let EditOp::ReplacePostings { postings, .. } = &stripe.ops[0] else {
            panic!("a posting replacement");
        };
        assert_eq!(postings, &vec![2], "the fee leg stays");
        assert!(matches!(
            plan(
                &journal,
                &journal.transactions[0],
                &request(&journal, 0, &[("revenue:consulting", 90_000)])
            ),
            Err(OpsError::Refused(_))
        ));
        assert!(matches!(
            plan(
                &journal,
                &journal.transactions[0],
                &request(&journal, 0, &[])
            ),
            Err(OpsError::Refused(_))
        ));
    }

    #[test]
    fn a_row_against_the_invoice_is_debited_with_its_share() {
        // A final invoice of $1,000: $1,200 of work less a $200 deposit.
        let journal = parse_journal(BOOKS, "inline.journal").expect("parses");
        let split = [
            ("revenue:consulting", 120_000),
            ("liabilities:customer-deposits", -20_000),
        ];
        let exact = plan(
            &journal,
            &journal.transactions[0],
            &request(&journal, 0, &split),
        )
        .expect("plans");
        assert!(!exact.partial);
        assert_eq!(
            exact.applied,
            vec![
                ("revenue:consulting".to_string(), Dec::new(120_000, 2)),
                (
                    "liabilities:customer-deposits".to_string(),
                    Dec::new(-20_000, 2)
                )
            ]
        );
        let EditOp::ReplacePostings { with, .. } = &exact.ops[0] else {
            panic!("a posting replacement");
        };
        let amounts: Vec<Dec> = with.iter().map(|p| p.amounts[0].quantity).collect();
        assert_eq!(amounts, vec![Dec::new(-120_000, 2), Dec::new(20_000, 2)]);
        // $250 of it: 300 / -50, pro rata.
        let partial = plan(
            &journal,
            &journal.transactions[3],
            &request(&journal, 3, &split),
        )
        .expect("plans");
        assert!(partial.partial);
        assert_eq!(
            partial.applied.iter().map(|(_, d)| *d).collect::<Vec<_>>(),
            vec![Dec::new(30_000, 2), Dec::new(-5_000, 2)]
        );
        // A split summing to nothing (or less) is refused.
        assert!(matches!(
            plan(
                &journal,
                &journal.transactions[0],
                &request(
                    &journal,
                    0,
                    &[("revenue:consulting", 20_000), ("liabilities:x", -20_000)]
                ),
            ),
            Err(OpsError::Refused(_))
        ));
    }

    #[test]
    fn candidates_rank_number_then_amount_and_skip_loans() {
        let journal = parse_journal(BOOKS, "inline.journal").expect("parses");
        let invoice = CashInvoice {
            number: "105".into(),
            party: None,
            date: "2026-03-01".into(),
            due: crate::accrual::invoice::money(&journal, "$", Dec::new(100_000, 2)),
        };
        let found = candidates(&journal, &invoice, &SettleSettings::default(), "2026-03-31");
        let positions: Vec<usize> = found.iter().map(|c| c.position).collect();
        // The wire names INV 105 (partial); then the exact ACH; then Stripe.
        assert_eq!(positions, vec![3, 0, 1]);
        assert!(found[0].by_number && matches!(found[0].fit, Fit::Short { .. }));
        assert_eq!(found[1].fit, Fit::Exact);
    }
}
