//! Building an invoice, bill or prepayment transaction from a typed spec, in
//! plan 45's tag vocabulary, together with the [`ScheduleSpec`]s of its
//! deferred lines — so a program (Ledgeline's editor, or an invoicing app)
//! writes the document and its schedules in one [`super::ops`] call.
//!
//! - An [`Direction::Revenue`] document (an invoice, or a prepayment received)
//!   credits each line and debits the settlement account (receivable for an
//!   invoice, the bank for a prepayment) with the total.
//! - An [`Direction::Expense`] document (a bill, or a prepayment made) debits
//!   each line and credits the settlement account (payable, or the bank).
//!
//! A line with [`Deferral`] terms is a deferred-revenue (or prepaid) line: its
//! posting carries `recognition:` and the terms. Lines with the same start,
//! term and proration share one schedule (D-sched), each recognizing into its
//! own target; lines with different terms get their own.

use super::due::new_transaction;
use super::schedule::{
    Party, Proration, ScheduleError, ScheduleLine, ScheduleSpec, Term, check_description,
    check_tag_value, is_valid_id,
};
use super::{
    CALENDAR_VALUE, CONTRACT_TAG, Contract, INVOICE_TAG, PRORATION_TAG, RECOGNITION_TAG,
    SERVICE_END_TAG, SERVICE_START_TAG, TERM_MONTHS_TAG, is_iso_date,
};
use crate::decimal::Dec;
use crate::matching::IdGenerator;
use crate::model::{
    AccountName, Amount, Commodity, Journal, Posting, PostingType, Status, Transaction,
};
use crate::parse::JournalStyles;
use crate::periodic::check_account;
use thiserror::Error;

/// Which way a document runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Lines are credited (revenue, deferred revenue); the settlement account
    /// is debited.
    Revenue,
    /// Lines are debited (expense, prepaid); the settlement account is
    /// credited.
    Expense,
}

/// A deferred line's recognition terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deferral {
    /// The revenue or expense account it recognizes into.
    pub target_account: String,
    /// First day of service; the document date when `None`.
    pub service_start: Option<String>,
    /// Length of service.
    pub term: Term,
    /// Anniversary or calendar periods.
    pub proration: Proration,
    /// New, renewal, expansion or reactivation.
    pub contract: Option<Contract>,
}

/// One line of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentLine {
    /// The account: revenue, expense, a fee, or — for a deferred line — the
    /// deferred-revenue or prepaid account.
    pub account: String,
    /// The amount, a positive magnitude; [`Direction`] gives it its sign.
    pub amount: Amount,
    /// `Some` for a line recognized over a term.
    pub deferral: Option<Deferral>,
}

/// An invoice, bill or prepayment to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSpec {
    /// Revenue or expense.
    pub direction: Direction,
    /// The transaction date.
    pub date: String,
    /// The description (payee).
    pub description: String,
    /// The invoice or bill number (`invoice:` on the header).
    pub number: Option<String>,
    /// The customer or vendor, tagged on every posting.
    pub party: Option<Party>,
    /// Receivable or payable for an invoice or bill; the bank or card for a
    /// prepayment.
    pub settlement_account: String,
    /// The lines.
    pub lines: Vec<DocumentLine>,
}

/// A built document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltDocument {
    /// The transaction, ready for [`crate::edit::JournalEditor::add_transactions`].
    pub transaction: Transaction,
    /// One schedule per distinct deferral terms, in line order.
    pub schedules: Vec<ScheduleSpec>,
}

/// Why a document can't be built.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DocumentError {
    /// An input that would not read back as written.
    #[error("{0}")]
    Invalid(String),
    /// The lines are in more than one commodity.
    #[error("the lines are in more than one commodity")]
    MixedCommodities,
    /// The arithmetic overflowed.
    #[error("the amounts are too large")]
    Overflow,
}

impl From<ScheduleError> for DocumentError {
    fn from(error: ScheduleError) -> Self {
        Self::Invalid(error.to_string())
    }
}

/// `quantity` of `commodity`, in the style `journal` writes it in (its
/// declared or first-seen style; `$`-left, two places, otherwise).
#[must_use]
pub fn money(journal: &Journal, commodity: &str, quantity: Dec) -> Amount {
    let commodity = Commodity(commodity.to_string());
    let style = JournalStyles::of(journal)
        .declared_style(&commodity)
        .cloned()
        .unwrap_or(crate::model::AmountStyle {
            side: crate::model::CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: None,
            precision: quantity.places,
        });
    super::schedule::amount_in(&commodity, &style, quantity)
}

fn invalid<T>(why: impl Into<String>) -> Result<T, DocumentError> {
    Err(DocumentError::Invalid(why.into()))
}

fn posting(account: &str, amount: Amount, tags: &[(&str, String)]) -> Posting {
    let comment = if tags.is_empty() {
        String::new()
    } else {
        format!(
            "{}\n",
            tags.iter()
                .map(|(name, value)| format!("{name}: {value}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    Posting {
        status: Status::Unmarked,
        ptype: PostingType::Regular,
        account: AccountName(account.to_string()),
        amounts: vec![amount],
        balance_assertion: None,
        date: None,
        date2: None,
        tags: crate::parse::parse_tags(&comment),
        comment,
        generated: None,
        elided: false,
    }
}

fn signed(amount: &Amount, negative: bool) -> Result<Amount, DocumentError> {
    let quantity = if negative {
        amount.quantity.neg().map_err(|_| DocumentError::Overflow)?
    } else {
        amount.quantity
    };
    Ok(Amount {
        quantity,
        ..amount.clone()
    })
}

fn check(spec: &DocumentSpec) -> Result<(), DocumentError> {
    if !is_iso_date(&spec.date) {
        return invalid(format!("`{}` is not a date", spec.date));
    }
    check_description(&spec.description)?;
    if let Some(number) = &spec.number {
        check_tag_value("the invoice number", number)?;
    }
    if let Some(party) = &spec.party {
        check_tag_value(party.kind.tag(), &party.name)?;
    }
    if spec.lines.is_empty() {
        return invalid("a document needs at least one line");
    }
    let accounts =
        std::iter::once(&spec.settlement_account).chain(spec.lines.iter().flat_map(|line| {
            std::iter::once(&line.account).chain(line.deferral.as_ref().map(|d| &d.target_account))
        }));
    for account in accounts {
        check_account(account).map_err(|error| DocumentError::Invalid(error.to_string()))?;
    }
    for line in &spec.lines {
        if line.amount.quantity.mantissa <= 0 {
            return invalid(format!(
                "the line on {} must be a positive amount; the direction gives it its sign",
                line.account
            ));
        }
        if line.amount.cost.is_some() {
            return invalid(format!("the line on {} has a cost", line.account));
        }
        if let Some(deferral) = &line.deferral
            && let Some(start) = &deferral.service_start
            && !is_iso_date(start)
        {
            return invalid(format!("`{start}` is not a date"));
        }
    }
    let commodity = &spec.lines[0].amount.commodity;
    if spec
        .lines
        .iter()
        .any(|line| &line.amount.commodity != commodity)
    {
        return Err(DocumentError::MixedCommodities);
    }
    Ok(())
}

/// The terms deferred lines share a schedule on.
fn terms_key(spec: &DocumentSpec, deferral: &Deferral) -> (String, Term, Proration) {
    (
        deferral
            .service_start
            .clone()
            .unwrap_or_else(|| spec.date.clone()),
        deferral.term.clone(),
        deferral.proration,
    )
}

/// Build the document and its schedules. New `recognition:` ids come from
/// `ids` (see [`super::ops::Loaded::id_generator`]).
///
/// # Errors
/// [`DocumentError`] for an input that would not read back as written, mixed
/// commodities, or overflow.
pub fn build_document(
    spec: &DocumentSpec,
    ids: &mut IdGenerator,
) -> Result<BuiltDocument, DocumentError> {
    check(spec)?;
    let revenue = spec.direction == Direction::Revenue;
    let party_tag = spec
        .party
        .as_ref()
        .map(|party| (party.kind.tag(), party.name.clone()));
    // One id per distinct terms, in line order.
    let mut groups: Vec<((String, Term, Proration), String)> = Vec::new();
    let mut postings = Vec::with_capacity(spec.lines.len() + 1);
    let mut total = Dec::zero();
    for line in &spec.lines {
        total = total
            .add(line.amount.quantity)
            .map_err(|_| DocumentError::Overflow)?;
        let mut tags: Vec<(&str, String)> = party_tag.iter().cloned().collect();
        if let Some(deferral) = &line.deferral {
            let key = terms_key(spec, deferral);
            let id = match groups.iter().find(|(k, _)| *k == key) {
                Some((_, id)) => id.clone(),
                None => {
                    let id = ids.next_id();
                    groups.push((key, id.clone()));
                    id
                }
            };
            tags.push((RECOGNITION_TAG, id));
            if let Some(start) = deferral.service_start.as_ref().filter(|s| **s != spec.date) {
                tags.push((SERVICE_START_TAG, start.clone()));
            }
            match &deferral.term {
                Term::Months(n) => tags.push((TERM_MONTHS_TAG, n.to_string())),
                Term::Through(end) => tags.push((SERVICE_END_TAG, end.clone())),
            }
            if deferral.proration == Proration::Calendar {
                tags.push((PRORATION_TAG, CALENDAR_VALUE.to_string()));
            }
            if let Some(contract) = deferral.contract {
                tags.push((CONTRACT_TAG, contract.code().to_string()));
            }
        }
        postings.push(posting(
            &line.account,
            signed(&line.amount, revenue)?,
            &tags,
        ));
    }
    let settlement = Amount {
        quantity: total,
        style: crate::model::AmountStyle {
            precision: spec.lines[0].amount.style.precision.max(total.places),
            ..spec.lines[0].amount.style.clone()
        },
        ..spec.lines[0].amount.clone()
    };
    let party_only: Vec<(&str, String)> = party_tag.iter().cloned().collect();
    postings.insert(
        0,
        posting(
            &spec.settlement_account,
            signed(&settlement, !revenue)?,
            &party_only,
        ),
    );
    let header: Vec<(&str, &str)> = spec
        .number
        .as_deref()
        .map(|number| vec![(INVOICE_TAG, number)])
        .unwrap_or_default();
    let transaction = new_transaction(
        &spec.date,
        &spec.description,
        super::due::comment_with("", &header)
            .trim_start_matches('\n')
            .to_string(),
        postings,
    );
    let description = spec.party.as_ref().map_or_else(
        || format!("{} Recognized", spec.description),
        |party| format!("{} Recognized", party.name),
    );
    let schedules = groups
        .into_iter()
        .map(|((start, term, proration), id)| {
            debug_assert!(is_valid_id(&id));
            let lines = spec
                .lines
                .iter()
                .filter(|line| {
                    line.deferral.as_ref().is_some_and(|d| {
                        terms_key(spec, d) == (start.clone(), term.clone(), proration)
                    })
                })
                .map(|line| {
                    Ok(ScheduleLine {
                        source_account: line.account.clone(),
                        target_account: line
                            .deferral
                            .as_ref()
                            .map(|d| d.target_account.clone())
                            .unwrap_or_default(),
                        total: signed(&line.amount, revenue)?,
                        party: spec.party.clone(),
                    })
                })
                .collect::<Result<Vec<_>, DocumentError>>()?;
            Ok(ScheduleSpec {
                id,
                lines,
                start,
                term,
                proration,
                description: description.clone(),
                header_tags: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, DocumentError>>()?;
    Ok(BuiltDocument {
        transaction,
        schedules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accrual::schedule::PartyKind;
    use crate::parse::parse_journal;

    #[test]
    fn a_multi_line_invoice_shares_a_schedule_per_terms() {
        let journal = parse_journal("commodity $1,000.00\n", "books.journal").expect("parses");
        let usd = |cents: i128| money(&journal, "$", Dec::new(cents, 2));
        let deferral = |target: &str, months| {
            Some(Deferral {
                target_account: target.into(),
                service_start: None,
                term: Term::Months(months),
                proration: Proration::Anniversary,
                contract: Some(Contract::New),
            })
        };
        let spec = DocumentSpec {
            direction: Direction::Revenue,
            date: "2026-11-17".into(),
            description: "Acme Corp".into(),
            number: Some("1042".into()),
            party: Some(Party {
                kind: PartyKind::Customer,
                name: "Acme".into(),
            }),
            settlement_account: "assets:accounts receivable".into(),
            lines: vec![
                DocumentLine {
                    account: "liabilities:deferred-revenue".into(),
                    amount: usd(1_200_000),
                    deferral: deferral("revenue:subscriptions", 12),
                },
                DocumentLine {
                    account: "liabilities:deferred-revenue".into(),
                    amount: usd(240_000),
                    deferral: deferral("revenue:support", 12),
                },
                DocumentLine {
                    account: "liabilities:deferred-revenue".into(),
                    amount: usd(300_000),
                    deferral: deferral("revenue:onboarding", 3),
                },
                DocumentLine {
                    account: "revenue:setup".into(),
                    amount: usd(50_000),
                    deferral: None,
                },
            ],
        };
        let mut ids = IdGenerator::avoiding(Vec::new());
        let built = build_document(&spec, &mut ids).expect("builds");
        assert_eq!(built.schedules.len(), 2);
        assert_eq!(built.schedules[0].lines.len(), 2);
        assert_eq!(built.schedules[1].lines.len(), 1);
        let text = crate::edit::format_transaction(&built.transaction);
        let reparsed = parse_journal(&format!("commodity $1,000.00\n\n{text}"), "x.journal")
            .expect("reads back");
        let txn = &reparsed.transactions[0];
        assert_eq!(txn.tags, vec![("invoice".to_string(), "1042".to_string())]);
        assert_eq!(txn.postings[0].amounts[0].quantity, Dec::new(1_790_000, 2));
        assert!(
            txn.postings[1]
                .tags
                .iter()
                .any(|(k, v)| k == "recognition" && *v == built.schedules[0].id)
        );
        assert!(
            txn.postings[1]
                .tags
                .iter()
                .any(|(k, v)| k == "term-months" && v == "12")
        );
        assert!(
            txn.postings[3]
                .tags
                .iter()
                .any(|(k, v)| k == "contract" && v == "new")
        );
        assert!(!txn.postings[4].tags.iter().any(|(k, _)| k == "recognition"));
        assert_eq!(
            built.schedules[0].lines[0].total.quantity,
            Dec::new(-1_200_000, 2)
        );
    }
}
