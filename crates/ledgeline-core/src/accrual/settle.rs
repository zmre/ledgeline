//! Invoices, bills and their payments (plan 45 D-settle, D5): 1:1 matching
//! through a receivable or payable account, with a short payment's
//! difference offered as a fee.
//!
//! Only the tag, the ids and the link machinery are plan 35's
//! ([`crate::matching`]); the candidates and the ranking are this module's:
//!
//! - **Invoice side:** a receivable debit (or payable credit) with no `match`.
//! - **Payment side:** a receivable credit (or payable debit) with no `match`,
//!   in a transaction with a cash leg.
//! - **Pairing:** same account and commodity, the payment on or after the
//!   invoice, both inside the lookback; ranked by invoice number, then
//!   customer/vendor, then amount (exact, then short by at most the
//!   tolerance), then the smallest gap; assigned one-to-one in that order.
//!
//! A pair is **clear** when it is unambiguous, has an invoice-number or
//! customer signal, and its amounts are exact. A short payment links through
//! [`link_ops`] with the fee edit; an overpayment is shown but never linked
//! (customer credits come later).

use super::schedule::{Party, PartyKind};
use super::{AccrualRole, AccrualRoles, CUSTOMER_TAG, INVOICE_TAG, VENDOR_TAG, day_of, tag_value};
use crate::decimal::Dec;
use crate::edit::EditOp;
use crate::matching::{self, MATCH_TAG, PostingRef, TxnOps};
use crate::model::{Amount, Journal, Posting, PostingType, Transaction};
use crate::projections::is_generated;
use crate::review::similarity_key;
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

/// Default lookback, in days.
pub const DEFAULT_LOOKBACK_DAYS: u32 = 180;
/// Default fee tolerance: 3% of the invoice, no cap (Q3), in basis points.
pub const DEFAULT_TOLERANCE_BPS: u32 = 300;

/// What settlement matching is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettleSettings {
    /// Only postings dated within this many days before `today`; `0` is no
    /// limit.
    pub lookback_days: u32,
    /// The largest short payment, in basis points of the invoice.
    pub tolerance_bps: u32,
}

impl Default for SettleSettings {
    fn default() -> Self {
        Self {
            lookback_days: DEFAULT_LOOKBACK_DAYS,
            tolerance_bps: DEFAULT_TOLERANCE_BPS,
        }
    }
}

/// Which side of a settlement a posting is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The invoice or bill.
    Invoice,
    /// The payment.
    Payment,
}

/// One candidate posting.
#[derive(Debug, Clone)]
pub struct SettleCandidate<'j> {
    /// Where it is.
    pub at: PostingRef,
    /// Invoice or payment.
    pub side: Side,
    /// The receivable or payable account.
    pub account: &'j str,
    /// Receivable or payable.
    pub role: AccrualRole,
    /// The effective date.
    pub date: &'j str,
    /// Its one amount.
    pub amount: &'j Amount,
    /// The transaction's `invoice:` tag.
    pub invoice: Option<&'j str>,
    /// The `customer:`/`vendor:` it carries.
    pub party: Option<Party>,
}

/// How a payment's amount fits its invoice's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Equal.
    Exact,
    /// Short by `fee`, within the tolerance.
    Short {
        /// The difference, a positive magnitude.
        fee: Dec,
    },
    /// The payment is larger: shown, never linked.
    Over {
        /// The excess, a positive magnitude.
        excess: Dec,
    },
}

/// A proposed invoice ↔ payment pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleProposal {
    /// The invoice's settlement posting.
    pub invoice: PostingRef,
    /// The payment's settlement posting.
    pub payment: PostingRef,
    /// How the amounts fit.
    pub fit: Fit,
    /// Matched on the invoice number.
    pub by_number: bool,
    /// Matched on the customer or vendor.
    pub by_party: bool,
    /// Days from the invoice to the payment.
    pub gap_days: u32,
    /// Another pair shared a side with the same rank.
    pub ambiguous: bool,
    /// Eligible for "Link all clear".
    pub clear: bool,
}

/// Why a settlement can't be linked.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SettleError {
    /// The postings aren't a legal settlement pair.
    #[error("{0}")]
    NotAPair(String),
    /// A short payment needs a fee account.
    #[error("a fee account is needed for the {0} difference")]
    FeeAccountNeeded(String),
    /// The payment is larger than the invoice.
    #[error("the payment is larger than the invoice; customer credits are not supported yet")]
    Overpayment,
    /// The link ops refused.
    #[error(transparent)]
    Match(#[from] matching::MatchError),
}

fn party_tag(tags: &[(String, String)]) -> Option<Party> {
    [
        (CUSTOMER_TAG, PartyKind::Customer),
        (VENDOR_TAG, PartyKind::Vendor),
    ]
    .into_iter()
    .find_map(|(tag, kind)| {
        tag_value(tags, tag)
            .filter(|name| !name.is_empty())
            .map(|name| Party {
                kind,
                name: name.to_string(),
            })
    })
}

fn party_of(txn: &Transaction, posting: &Posting) -> Option<Party> {
    party_tag(&posting.tags).or_else(|| party_tag(&txn.tags))
}

/// Every candidate (D-settle), in file order.
#[must_use]
pub fn candidates<'j>(
    journal: &'j Journal,
    roles: &AccrualRoles,
    settings: &SettleSettings,
    today: &str,
) -> Vec<SettleCandidate<'j>> {
    let types = crate::reports::AccountTypes::new(&crate::reports::account_decls(journal));
    let cutoff = (settings.lookback_days > 0)
        .then(|| day_of(today) - i32::try_from(settings.lookback_days).unwrap_or(i32::MAX));
    journal
        .transactions
        .iter()
        .enumerate()
        .filter(|(_, txn)| !is_generated(txn) && !super::is_recognition_entry(txn))
        .flat_map(|(position, txn)| {
            let cash = txn.postings.iter().any(|p| types.is_cash(&p.account.0));
            txn.postings
                .iter()
                .enumerate()
                .filter_map(move |(at, posting)| {
                    let role = roles
                        .role(&posting.account.0)
                        .filter(|r| r.is_settlement())?;
                    if posting.generated.is_some()
                        || posting.ptype != PostingType::Regular
                        || posting.balance_assertion.is_some()
                        || posting.tags.iter().any(|(name, _)| name == MATCH_TAG)
                    {
                        return None;
                    }
                    let [amount] = posting.amounts.as_slice() else {
                        return None;
                    };
                    if amount.cost.is_some() || amount.quantity.is_zero() {
                        return None;
                    }
                    let debit = amount.quantity.mantissa > 0;
                    let side = match (role, debit) {
                        (AccrualRole::Receivable, true) | (AccrualRole::Payable, false) => {
                            Side::Invoice
                        }
                        _ if cash => Side::Payment,
                        _ => return None,
                    };
                    Some(SettleCandidate {
                        at: PostingRef {
                            position,
                            txn: txn.index,
                            posting: at,
                        },
                        side,
                        account: &posting.account.0,
                        role,
                        date: posting.date.as_deref().unwrap_or(&txn.date),
                        amount,
                        invoice: tag_value(&txn.tags, INVOICE_TAG).filter(|n| !n.is_empty()),
                        party: party_of(txn, posting),
                    })
                })
        })
        .filter(|candidate| cutoff.is_none_or(|cutoff| day_of(candidate.date) >= cutoff))
        .collect()
}

/// Whether `text` contains `word` as a whole word (ASCII case-insensitive).
fn has_word(text: &str, word: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let word = word.to_ascii_lowercase();
    text.match_indices(&word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The payee similarity key → party of the invoices already linked to
/// payments with that key: "the customer most often seen with the payment's
/// payee in earlier matched payments".
fn learned_parties(journal: &Journal) -> HashMap<String, Party> {
    let links = matching::links(journal);
    let mut counts: HashMap<String, BTreeMap<(u8, String), usize>> = HashMap::new();
    for (id, (a, b)) in links.pairs() {
        if !links.is_settlement(id) {
            continue;
        }
        let side = |at: PostingRef| {
            let txn = &journal.transactions[at.position];
            (txn, &txn.postings[at.posting])
        };
        let ((ta, pa), (tb, pb)) = (side(*a), side(*b));
        // The invoice is the one with a party; the payment the other.
        let (party, payment) = match (party_of(ta, pa), party_of(tb, pb)) {
            (Some(party), None) => (party, tb),
            (None, Some(party)) => (party, ta),
            _ => continue,
        };
        if let Some(key) = similarity_key(&payment.description) {
            let kind = u8::from(party.kind == PartyKind::Vendor);
            *counts
                .entry(key)
                .or_default()
                .entry((kind, party.name))
                .or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter_map(|(key, counts)| {
            let ((kind, name), _) = counts.into_iter().max_by_key(|(_, n)| *n)?;
            let kind = if kind == 1 {
                PartyKind::Vendor
            } else {
                PartyKind::Customer
            };
            Some((key, Party { kind, name }))
        })
        .collect()
}

/// The fit of `payment` against `invoice`, or `None` beyond the tolerance.
fn fit(invoice: &Amount, payment: &Amount, tolerance_bps: u32) -> Option<Fit> {
    let inv = invoice.quantity.abs().ok()?;
    let pay = payment.quantity.abs().ok()?;
    let diff = inv.sub(pay).ok()?;
    if diff.is_zero() {
        return Some(Fit::Exact);
    }
    if diff.mantissa < 0 {
        return Some(Fit::Over {
            excess: diff.neg().ok()?,
        });
    }
    // diff ≤ inv × bps / 10000, exactly.
    let limit = inv.mul(Dec::new(i128::from(tolerance_bps), 4)).ok()?;
    (limit.sub(diff).ok()?.mantissa >= 0).then_some(Fit::Short { fee: diff })
}

/// Propose 1:1 pairs among the candidates (D-settle), in invoice date order.
#[must_use]
pub fn propose(
    journal: &Journal,
    roles: &AccrualRoles,
    settings: &SettleSettings,
    today: &str,
) -> Vec<SettleProposal> {
    let all = candidates(journal, roles, settings, today);
    let learned = learned_parties(journal);
    let (invoices, payments): (Vec<&SettleCandidate<'_>>, Vec<&SettleCandidate<'_>>) =
        all.iter().partition(|c| c.side == Side::Invoice);
    let payment_party = |c: &SettleCandidate<'_>| {
        c.party.clone().or_else(|| {
            similarity_key(&journal.transactions[c.at.position].description)
                .and_then(|key| learned.get(&key).cloned())
        })
    };
    // Every legal pair with its rank.
    let mut pairs: Vec<(usize, usize, SettleProposal)> = Vec::new();
    for (i, invoice) in invoices.iter().enumerate() {
        for (j, payment) in payments.iter().enumerate() {
            if invoice.account != payment.account
                || invoice.amount.commodity != payment.amount.commodity
                || payment.date < invoice.date
                || invoice.at.position == payment.at.position
            {
                continue;
            }
            let Some(fit) = fit(invoice.amount, payment.amount, settings.tolerance_bps) else {
                continue;
            };
            let description = &journal.transactions[payment.at.position].description;
            let by_number = invoice.invoice.is_some_and(|number| {
                payment.invoice == Some(number) || has_word(description, number)
            });
            let by_party = invoice
                .party
                .as_ref()
                .is_some_and(|party| payment_party(payment).as_ref() == Some(party));
            let gap =
                u32::try_from(day_of(payment.date) - day_of(invoice.date)).unwrap_or(u32::MAX);
            pairs.push((
                i,
                j,
                SettleProposal {
                    invoice: invoice.at,
                    payment: payment.at,
                    fit,
                    by_number,
                    by_party,
                    gap_days: gap,
                    ambiguous: false,
                    clear: false,
                },
            ));
        }
    }
    let rank = |p: &SettleProposal| {
        (
            !p.by_number,
            !p.by_party,
            match p.fit {
                Fit::Exact => 0,
                Fit::Short { .. } => 1,
                Fit::Over { .. } => 2,
            },
            p.gap_days,
        )
    };
    pairs.sort_by_key(|(i, j, p)| (rank(p), invoices[*i].at.position, payments[*j].at.position));
    let mut taken_invoice = vec![false; invoices.len()];
    let mut taken_payment = vec![false; payments.len()];
    let mut chosen: Vec<SettleProposal> = Vec::new();
    for (k, (i, j, proposal)) in pairs.iter().enumerate() {
        if taken_invoice[*i] || taken_payment[*j] {
            continue;
        }
        taken_invoice[*i] = true;
        taken_payment[*j] = true;
        let ambiguous = pairs.iter().enumerate().any(|(other, (oi, oj, op))| {
            other != k && (oi == i || oj == j) && rank(op) == rank(proposal)
        });
        let clear =
            !ambiguous && (proposal.by_number || proposal.by_party) && proposal.fit == Fit::Exact;
        chosen.push(SettleProposal {
            ambiguous,
            clear,
            ..proposal.clone()
        });
    }
    chosen.sort_by_key(|p| {
        (
            journal.transactions[p.invoice.position].date.clone(),
            p.invoice.position,
        )
    });
    chosen
}

/// The proposal for one chosen pair, scored as [`propose`] scores it — for
/// linking a pair by hand, or re-checking a proposal against a fresh journal.
/// `None` when the two aren't a legal pair (not an invoice and a payment
/// candidate on one account and commodity, the payment first, or beyond the
/// tolerance).
#[must_use]
pub fn pair(
    journal: &Journal,
    roles: &AccrualRoles,
    invoice: PostingRef,
    payment: PostingRef,
    tolerance_bps: u32,
) -> Option<SettleProposal> {
    let settings = SettleSettings {
        lookback_days: 0,
        tolerance_bps,
    };
    let all = candidates(journal, roles, &settings, "9999-12-31");
    let find = |at: PostingRef, side| all.iter().find(|c| c.at == at && c.side == side);
    let (inv, pay) = (find(invoice, Side::Invoice)?, find(payment, Side::Payment)?);
    if inv.account != pay.account
        || inv.amount.commodity != pay.amount.commodity
        || pay.date < inv.date
        || inv.at.position == pay.at.position
    {
        return None;
    }
    let fit = fit(inv.amount, pay.amount, tolerance_bps)?;
    let description = &journal.transactions[pay.at.position].description;
    let by_number = inv
        .invoice
        .is_some_and(|number| pay.invoice == Some(number) || has_word(description, number));
    let by_party = inv.party.is_some() && inv.party == pay.party;
    Some(SettleProposal {
        invoice,
        payment,
        fit,
        by_number,
        by_party,
        gap_days: u32::try_from(day_of(pay.date) - day_of(inv.date)).unwrap_or(u32::MAX),
        ambiguous: false,
        clear: by_number || by_party,
    })
}

/// The account most recently used for fees in payments already linked on
/// `account`: the expense posting of the latest linked payment that has one.
#[must_use]
pub fn fee_account_for(journal: &Journal, account: &str) -> Option<String> {
    let links = matching::links(journal);
    let types = crate::reports::AccountTypes::new(&crate::reports::account_decls(journal));
    links
        .pairs()
        .iter()
        .filter(|(id, _)| links.is_settlement(id))
        .flat_map(|(_, (a, b))| [*a, *b])
        .filter(|at| {
            journal.transactions[at.position].postings[at.posting]
                .account
                .0
                == account
        })
        .map(|at| &journal.transactions[at.position])
        .filter_map(|txn| {
            txn.postings
                .iter()
                .find(|p| types.is_type(&p.account.0, crate::reports::AccountType::Expense))
                .map(|p| (txn.date.clone(), p.account.0.clone()))
        })
        .max()
        .map(|(_, account)| account)
}

fn posting_at(journal: &Journal, at: PostingRef) -> Result<(&Transaction, &Posting), SettleError> {
    journal
        .transactions
        .get(at.position)
        .filter(|txn| txn.index == at.txn)
        .and_then(|txn| txn.postings.get(at.posting).map(|p| (txn, p)))
        .ok_or_else(|| SettleError::NotAPair("a posting is no longer where it was".into()))
}

/// The fee edit (D-settle "Fees"): the payment's settlement posting set to
/// exactly the invoice's negation (made explicit if it was elided), and a
/// `fee_account` posting for the difference. Every other leg is kept as
/// written — an elided one stays elided. One [`EditOp::Replace`].
///
/// # Errors
/// [`SettleError::NotAPair`] when a posting moved or the fit isn't short.
pub fn fee_ops(
    journal: &Journal,
    proposal: &SettleProposal,
    fee_account: &str,
) -> Result<Vec<EditOp>, SettleError> {
    let (_, invoice) = posting_at(journal, proposal.invoice)?;
    let (txn, payment) = posting_at(journal, proposal.payment)?;
    let (Some(invoice_amount), Some(old)) = (invoice.amounts.first(), payment.amounts.first())
    else {
        return Err(SettleError::NotAPair("a posting has no amount".into()));
    };
    crate::periodic::check_account(fee_account)
        .map_err(|e| SettleError::NotAPair(e.to_string()))?;
    let wanted = invoice_amount
        .quantity
        .neg()
        .map_err(|_| SettleError::NotAPair("overflow".into()))?;
    let delta = wanted
        .sub(old.quantity)
        .map_err(|_| SettleError::NotAPair("overflow".into()))?;
    let fee = delta
        .neg()
        .map_err(|_| SettleError::NotAPair("overflow".into()))?;
    let explicit = |amount: &Amount, quantity: Dec| Amount {
        quantity,
        style: crate::model::AmountStyle {
            precision: amount.style.precision.max(quantity.places),
            ..amount.style.clone()
        },
        ..amount.clone()
    };
    let mut postings: Vec<Posting> = txn
        .postings
        .iter()
        .filter(|p| p.generated.is_none())
        .enumerate()
        .map(|(at, p)| {
            if at == proposal.payment.posting {
                Posting {
                    amounts: vec![explicit(old, wanted)],
                    elided: false,
                    ..p.clone()
                }
            } else if p.elided {
                Posting {
                    amounts: Vec::new(),
                    ..p.clone()
                }
            } else {
                p.clone()
            }
        })
        .collect();
    postings.push(Posting {
        status: crate::model::Status::Unmarked,
        ptype: PostingType::Regular,
        account: crate::model::AccountName(fee_account.to_string()),
        amounts: vec![explicit(old, fee)],
        balance_assertion: None,
        date: None,
        date2: None,
        comment: String::new(),
        tags: Vec::new(),
        generated: None,
        elided: false,
    });
    Ok(vec![EditOp::Replace(Box::new(Transaction {
        postings,
        ..txn.clone()
    }))])
}

/// Link `proposal` with `match:id` on both settlement postings, and, for a
/// short payment, the fee edit first (one batch).
///
/// # Errors
/// [`SettleError::FeeAccountNeeded`] for a short payment with no
/// `fee_account`; [`SettleError::Overpayment`]; [`SettleError::Match`] when
/// the exact pair isn't a legal link.
pub fn link_ops(
    journal: &Journal,
    proposal: &SettleProposal,
    fee_account: Option<&str>,
    id: &str,
) -> Result<TxnOps, SettleError> {
    match proposal.fit {
        Fit::Exact => Ok(matching::link_ops(
            journal,
            proposal.invoice,
            proposal.payment,
            id,
        )?),
        Fit::Over { .. } => Err(SettleError::Overpayment),
        Fit::Short { fee } => {
            let fee_account = fee_account
                .filter(|a| !a.trim().is_empty())
                .ok_or_else(|| SettleError::FeeAccountNeeded(crate::edit::render_dec(fee, '.')))?;
            if !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || id.is_empty()
                || id == matching::IGNORE_VALUE
            {
                return Err(SettleError::Match(matching::MatchError::BadId(
                    id.to_string(),
                )));
            }
            let tag = |at: PostingRef| EditOp::SetPostingTag {
                posting: at.posting,
                name: MATCH_TAG.to_string(),
                value: Some(id.to_string()),
            };
            let mut payment_ops = fee_ops(journal, proposal, fee_account)?;
            payment_ops.push(tag(proposal.payment));
            let mut ops = vec![
                (
                    proposal.invoice.position,
                    proposal.invoice.txn,
                    vec![tag(proposal.invoice)],
                ),
                (proposal.payment.position, proposal.payment.txn, payment_ops),
            ];
            ops.sort_by_key(|(position, _, _)| *position);
            Ok(ops.into_iter().map(|(_, txn, ops)| (txn, ops)).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::JournalEditor;
    use crate::parse::parse_journal;

    const TEXT: &str = "\
account assets:accounts receivable  ; type: A, kpi: receivable
account assets:bank  ; type: C
account revenue:services  ; type: R
account expenses:fees  ; type: X

2026-10-01 Invoice 1042  ; invoice: 1042
    assets:accounts receivable  $1000.00  ; customer: Acme
    revenue:services

2026-10-02 Invoice 1043  ; invoice: 1043
    assets:accounts receivable  $1000.00  ; customer: Beta
    revenue:services

2026-10-20 Acme payment INV 1042
    assets:bank  $970.00
    assets:accounts receivable

2026-10-21 Stripe payout
    assets:bank  $1000.00
    assets:accounts receivable

2026-10-22 Big payment
    assets:bank  $5000.00
    assets:accounts receivable
";

    fn setup(text: &str) -> (Journal, AccrualRoles) {
        let journal = parse_journal(text, "inline.journal").expect("parses");
        let roles = AccrualRoles::of(&journal);
        (journal, roles)
    }

    #[test]
    fn an_invoice_number_beats_an_amount_only_match() {
        let (journal, roles) = setup(TEXT);
        let proposals = propose(&journal, &roles, &SettleSettings::default(), "2026-11-17");
        let p1042 = proposals
            .iter()
            .find(|p| p.invoice.position == 0)
            .expect("1042 proposed");
        assert_eq!(p1042.payment.position, 2, "{proposals:#?}");
        assert!(p1042.by_number);
        assert_eq!(
            p1042.fit,
            Fit::Short {
                fee: Dec::new(3000, 2)
            }
        );
        assert!(!p1042.clear, "short is check");
        let p1043 = proposals
            .iter()
            .find(|p| p.invoice.position == 1)
            .expect("1043 proposed");
        assert_eq!((p1043.payment.position, p1043.fit), (3, Fit::Exact));
        assert!(!p1043.clear, "amount only is check");
        // $5000 for a $1000 invoice is never proposed: no invoice is left.
        assert!(proposals.iter().all(|p| p.payment.position != 4));
    }

    #[test]
    fn beyond_tolerance_is_no_proposal_and_overpayment_is_check() {
        let text = TEXT.replace("$970.00", "$969.00");
        let (journal, roles) = setup(&text);
        let proposals = propose(&journal, &roles, &SettleSettings::default(), "2026-11-17");
        assert!(
            proposals.iter().all(|p| p.payment.position != 2),
            "{proposals:#?}"
        );
        let over = TEXT.replace("$970.00", "$1100.00");
        let (journal, roles) = setup(&over);
        let proposals = propose(&journal, &roles, &SettleSettings::default(), "2026-11-17");
        let p = proposals
            .iter()
            .find(|p| p.invoice.position == 0)
            .expect("proposed");
        assert!(matches!(p.fit, Fit::Over { .. }) && !p.clear);
        assert!(matches!(
            link_ops(&journal, p, None, "abc123"),
            Err(SettleError::Overpayment)
        ));
    }

    #[test]
    fn a_fee_link_balances_and_the_legs_cancel_into_a_valid_unbroken_link() {
        let (journal, roles) = setup(TEXT);
        let proposals = propose(&journal, &roles, &SettleSettings::default(), "2026-11-17");
        let p = proposals
            .iter()
            .find(|p| p.invoice.position == 0)
            .expect("proposed");
        assert!(matches!(
            link_ops(&journal, p, None, "abc123"),
            Err(SettleError::FeeAccountNeeded(_))
        ));
        let ops = link_ops(&journal, p, Some("expenses:fees"), "abc123").expect("ops");
        let dir = std::env::temp_dir().join(format!("ll-settle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("books.journal");
        std::fs::write(&path, TEXT).expect("write");
        let mut editor = JournalEditor::open(&path).expect("opens");
        let edits: Vec<_> = ops
            .into_iter()
            .map(|(tindex, ops)| {
                (
                    tindex,
                    editor.transaction_fingerprint(tindex).expect("fp"),
                    ops,
                )
            })
            .collect();
        editor.apply_batch(&edits).expect("applies");
        let after = editor.journal().clone();
        let links = matching::links(&after);
        assert!(links.broken().is_empty(), "{:?}", links.broken());
        assert!(links.is_settlement("abc123"));
        let payment = &after.transactions[2];
        assert!(crate::edit::format_transaction(payment).contains("expenses:fees"));
        // Settlement links never collapse into transfer rows.
        let transfers = crate::view::rows::Transfers::of(&after);
        assert!(transfers.is_empty() && !transfers.is_linked(0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
