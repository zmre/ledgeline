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
//!
//! A line may run against the direction (a **negative** amount, invoicer L2):
//! applying a deposit on a final invoice debits the deposit liability, and a
//! discount to a contra-revenue account debits it. The document's total —
//! what the settlement account carries — must still run with the direction.
//!
//! Extra tags (invoicer L1) ride along: [`DocumentSpec::tags`] on the header
//! (`document:`, `due:`, `reference:`) and [`DocumentLine::tags`] on a line's
//! posting (`item:`). Every value goes through
//! [`check_tag_value`](super::schedule::check_tag_value) — a comma would end
//! it — and the built transaction is checked to read every tag back as given.
//! [`tag_value_from`] turns free text into a value that passes.

use super::due::new_transaction;
use super::schedule::{
    Party, Proration, ScheduleError, ScheduleLine, ScheduleSpec, Term, check_description,
    check_tag_name, check_tag_value, is_valid_id,
};
use super::{
    CALENDAR_VALUE, CONTRACT_TAG, Contract, INVOICE_TAG, PRORATION_TAG, RECOGNITION_TAG,
    RECOGNIZED_TAG, RENEWAL_DATE_TAG, RESTATED_TAG, RESTATED_THROUGH_TAG, SERVICE_END_TAG,
    SERVICE_START_TAG, TERM_MONTHS_TAG, VOID_TAG, is_iso_date,
};
use crate::decimal::Dec;
use crate::matching::IdGenerator;
use crate::matching::{MATCH_TAG, NOMATCH_TAG};
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
    /// The amount. Positive runs with the [`Direction`] (credited on an
    /// invoice, debited on a bill); negative runs against it — a deposit
    /// applied, or a discount to a contra account (L2). Never zero; a
    /// deferred line is always positive.
    pub amount: Amount,
    /// `Some` for a line recognized over a term.
    pub deferral: Option<Deferral>,
    /// Extra tags on the line's posting, after the ones the builder writes
    /// (the party, the recognition terms) — `item:` (L1). A name the builder
    /// writes, or one of plan 45's or plan 35's, is refused.
    pub tags: Vec<(String, String)>,
}

impl DocumentLine {
    /// A line with no deferral and no extra tags.
    #[must_use]
    pub fn new(account: impl Into<String>, amount: Amount) -> Self {
        Self {
            account: account.into(),
            amount,
            deferral: None,
            tags: Vec::new(),
        }
    }
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
    /// Extra header tags, after `invoice:` — `document:`, `due:`,
    /// `reference:` (L1). `invoice` (use [`number`](Self::number)), `void`
    /// and plan 45's and plan 35's names are refused.
    pub tags: Vec<(String, String)>,
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

/// Tag names with a meaning to plan 45 (recognition) or plan 35 (links): an
/// extra tag may never set one, on a posting or a header.
const RESERVED_TAGS: [&str; 12] = [
    RECOGNITION_TAG,
    SERVICE_START_TAG,
    TERM_MONTHS_TAG,
    SERVICE_END_TAG,
    PRORATION_TAG,
    CONTRACT_TAG,
    RECOGNIZED_TAG,
    RESTATED_TAG,
    RESTATED_THROUGH_TAG,
    RENEWAL_DATE_TAG,
    MATCH_TAG,
    NOMATCH_TAG,
];

/// `text` as a tag value that reads back as itself, or `None` when nothing
/// is left: commas and control characters become spaces, runs of spaces
/// collapse to one, the ends are trimmed, and anything past 256 bytes is cut
/// at a character boundary. For free text a program writes as a tag value —
/// a `reference:` or a `document:` path is the usual comma carrier.
#[must_use]
pub fn tag_value_from(text: &str) -> Option<String> {
    let mut value = String::with_capacity(text.len());
    for word in text
        .split(|c: char| c == ',' || c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
    {
        let extra = usize::from(!value.is_empty()) + word.len();
        if value.len() + extra > 256 {
            if value.is_empty() {
                let mut end = 256;
                while !word.is_char_boundary(end) {
                    end -= 1;
                }
                value.push_str(&word[..end]);
            }
            break;
        }
        if !value.is_empty() {
            value.push(' ');
        }
        value.push_str(word);
    }
    (!value.is_empty()).then_some(value)
}

/// Check extra tags: usable names and values, none reserved (`also`: the
/// names this place may not take besides [`RESERVED_TAGS`]), no name twice.
fn check_extra_tags(
    what: &str,
    tags: &[(String, String)],
    also: &[&str],
) -> Result<(), DocumentError> {
    for (at, (name, value)) in tags.iter().enumerate() {
        check_tag_name(name)?;
        check_tag_value(&format!("the `{name}` tag on {what}"), value)?;
        if RESERVED_TAGS.contains(&name.as_str()) || also.contains(&name.as_str()) {
            return invalid(format!(
                "`{name}` can't be an extra tag on {what}: the document writes it itself, or it \
                 means something to recognition or matching"
            ));
        }
        if tags[..at].iter().any(|(earlier, _)| earlier == name) {
            return invalid(format!("the `{name}` tag is given twice on {what}"));
        }
    }
    Ok(())
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
    check_extra_tags("the header", &spec.tags, &[INVOICE_TAG, VOID_TAG])?;
    let party_name = spec.party.as_ref().map(|party| party.kind.tag());
    let mut total = Dec::zero();
    for line in &spec.lines {
        if line.amount.quantity.is_zero() {
            return invalid(format!("the line on {} has no amount", line.account));
        }
        if line.deferral.is_some() && line.amount.quantity.mantissa < 0 {
            return invalid(format!(
                "the deferred line on {} must run with the document: a negative line can't be \
                 recognized over a term",
                line.account
            ));
        }
        total = total
            .add(line.amount.quantity)
            .map_err(|_| DocumentError::Overflow)?;
        check_extra_tags(
            &format!("the line on {}", line.account),
            &line.tags,
            party_name.as_slice(),
        )?;
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
    if total.mantissa <= 0 {
        return invalid(
            "the lines total zero or less: a document's total must run with its direction \
             (a credit note is a document of its own)",
        );
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
    build_document_with(spec, &mut || ids.next_id())
}

/// [`build_document`], with each new schedule's id from `next_id`, in line
/// order — a re-book hands back the document's existing ids first, so its
/// schedules are restated rather than started again.
pub(crate) fn build_document_with(
    spec: &DocumentSpec,
    next_id: &mut dyn FnMut() -> String,
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
                    let id = next_id();
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
        tags.extend(
            line.tags
                .iter()
                .map(|(name, value)| (name.as_str(), value.clone())),
        );
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
        .map(|number| (INVOICE_TAG, number))
        .into_iter()
        .chain(
            spec.tags
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .collect();
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
    read_back(spec, &transaction)?;
    Ok(BuiltDocument {
        transaction,
        schedules,
    })
}

/// Every extra tag reads back from the built transaction exactly as given —
/// the guard against a value the tag parser would split or trim.
fn read_back(spec: &DocumentSpec, transaction: &Transaction) -> Result<(), DocumentError> {
    let has = |tags: &[(String, String)], (name, value): &(String, String)| {
        tags.iter().any(|(n, v)| n == name && v == value)
    };
    if let Some(tag) = spec.tags.iter().find(|tag| !has(&transaction.tags, tag)) {
        return invalid(format!("the header tag `{}` would not read back", tag.0));
    }
    for (line, posting) in spec.lines.iter().zip(&transaction.postings[1..]) {
        if let Some(tag) = line.tags.iter().find(|tag| !has(&posting.tags, tag)) {
            return invalid(format!(
                "the tag `{}` on the line on {} would not read back",
                tag.0, line.account
            ));
        }
    }
    Ok(())
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
                    tags: vec![],
                },
                DocumentLine {
                    account: "liabilities:deferred-revenue".into(),
                    amount: usd(240_000),
                    deferral: deferral("revenue:support", 12),
                    tags: vec![],
                },
                DocumentLine {
                    account: "liabilities:deferred-revenue".into(),
                    amount: usd(300_000),
                    deferral: deferral("revenue:onboarding", 3),
                    tags: vec![],
                },
                DocumentLine::new("revenue:setup", usd(50_000)),
            ],
            tags: vec![],
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

    fn customer_x() -> Option<Party> {
        Some(Party {
            kind: PartyKind::Customer,
            name: "Customer X".into(),
        })
    }

    fn tag(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    /// ARCHITECTURE §4.2's invoice 105: two revenue rows with `item:`, the
    /// deposit applied as a negative line, and the header tags.
    fn invoice_105(journal: &Journal) -> DocumentSpec {
        let usd = |cents: i128| money(journal, "$", Dec::new(cents, 2));
        DocumentSpec {
            direction: Direction::Revenue,
            date: "2026-11-17".into(),
            description: "Customer X | Invoice 105".into(),
            number: Some("105".into()),
            party: customer_x(),
            settlement_account: "assets:accounts receivable".into(),
            lines: vec![
                DocumentLine {
                    tags: vec![tag("item", "Senior architect hours")],
                    ..DocumentLine::new("revenue:consulting", usd(720_000))
                },
                DocumentLine {
                    tags: vec![tag("item", "Junior engineer")],
                    ..DocumentLine::new("revenue:consulting:delivery", usd(950_000))
                },
                DocumentLine {
                    tags: vec![tag("item", "Less deposit (Invoice 104)")],
                    ..DocumentLine::new("liabilities:customer-deposits", usd(-835_000))
                },
            ],
            tags: vec![
                tag("document", "invoices/2026-11-17 Customer X Invoice 105.md"),
                tag("due", "2026-12-17"),
                tag("reference", "PO #PY75334"),
            ],
        }
    }

    #[test]
    fn extra_tags_and_a_negative_deposit_line_build_section_4_2() {
        let journal = parse_journal("commodity $1,000.00\n", "books.journal").expect("parses");
        let built = build_document(
            &invoice_105(&journal),
            &mut IdGenerator::avoiding(Vec::new()),
        )
        .expect("builds");
        assert!(built.schedules.is_empty());
        let text = crate::edit::format_transaction(&built.transaction);
        assert_eq!(
            text,
            "2026-11-17 Customer X | Invoice 105  ; invoice: 105, document: invoices/2026-11-17 \
             Customer X Invoice 105.md, due: 2026-12-17, reference: PO #PY75334\n    \
             assets:accounts receivable     $8350.00  ; customer: Customer X\n    \
             revenue:consulting             $-7200.00  ; customer: Customer X, item: Senior architect hours\n    \
             revenue:consulting:delivery    $-9500.00  ; customer: Customer X, item: Junior engineer\n    \
             liabilities:customer-deposits  $8350.00  ; customer: Customer X, item: Less deposit (Invoice 104)\n"
        );
        let reparsed = parse_journal(&format!("commodity $1,000.00\n\n{text}"), "x.journal")
            .expect("reads back");
        let txn = &reparsed.transactions[0];
        assert_eq!(
            txn.tags,
            vec![
                tag("invoice", "105"),
                tag("document", "invoices/2026-11-17 Customer X Invoice 105.md"),
                tag("due", "2026-12-17"),
                tag("reference", "PO #PY75334"),
            ]
        );
        assert_eq!(
            txn.postings[3].tags,
            vec![
                tag("customer", "Customer X"),
                tag("item", "Less deposit (Invoice 104)")
            ]
        );
        let sum = txn
            .postings
            .iter()
            .try_fold(Dec::zero(), |sum, p| sum.add(p.amounts[0].quantity))
            .expect("sums");
        assert!(sum.is_zero());
    }

    #[test]
    fn a_contra_revenue_discount_on_a_bill_runs_against_the_direction() {
        let journal = parse_journal("commodity $1,000.00\n", "books.journal").expect("parses");
        let usd = |cents: i128| money(&journal, "$", Dec::new(cents, 2));
        let spec = DocumentSpec {
            direction: Direction::Expense,
            date: "2026-11-17".into(),
            description: "Northwind".into(),
            number: Some("B-9".into()),
            party: None,
            settlement_account: "liabilities:accounts payable".into(),
            lines: vec![
                DocumentLine::new("expenses:software", usd(100_000)),
                DocumentLine::new("expenses:software:discounts", usd(-10_000)),
            ],
            tags: vec![],
        };
        let built = build_document(&spec, &mut IdGenerator::avoiding(Vec::new())).expect("builds");
        let amounts: Vec<Dec> = built
            .transaction
            .postings
            .iter()
            .map(|p| p.amounts[0].quantity)
            .collect();
        assert_eq!(
            amounts,
            vec![
                Dec::new(-90_000, 2),
                Dec::new(100_000, 2),
                Dec::new(-10_000, 2)
            ]
        );
    }

    #[test]
    fn unusable_extra_tags_and_lines_are_refused() {
        let journal = parse_journal("commodity $1,000.00\n", "books.journal").expect("parses");
        let usd = |cents: i128| money(&journal, "$", Dec::new(cents, 2));
        let refused = |edit: &dyn Fn(&mut DocumentSpec)| {
            let mut spec = invoice_105(&journal);
            edit(&mut spec);
            build_document(&spec, &mut IdGenerator::avoiding(Vec::new())).err()
        };
        let cases: [(&str, &dyn Fn(&mut DocumentSpec)); 10] = [
            ("comma", &|s| s.tags[2].1 = "PO 1, rev 2".into()),
            ("padded", &|s| s.tags[2].1 = " PO 1".into()),
            ("invoice on header", &|s| s.tags.push(tag("invoice", "9"))),
            ("void on header", &|s| {
                s.tags.push(tag("void", "2026-11-18"))
            }),
            ("match on a line", &|s| {
                s.lines[0].tags.push(tag("match", "abc"))
            }),
            ("recognition on a line", &|s| {
                s.lines[0].tags.push(tag("recognition", "abc"));
            }),
            ("party twice", &|s| {
                s.lines[0].tags.push(tag("customer", "Y"))
            }),
            ("name twice", &|s| s.tags.push(tag("due", "2026-12-18"))),
            ("bad name", &|s| s.tags.push(tag("due date", "x"))),
            ("zero line", &|s| s.lines[0].amount = usd(0)),
        ];
        for (case, edit) in cases {
            assert!(
                matches!(refused(edit), Some(DocumentError::Invalid(_))),
                "{case}"
            );
        }
        // The total must still run with the direction.
        assert!(matches!(
            refused(&|s| s.lines[2].amount = usd(-1_670_000)),
            Some(DocumentError::Invalid(_))
        ));
        // A deferred line can't be negative.
        assert!(matches!(
            refused(&|s| {
                s.lines[2].deferral = Some(Deferral {
                    target_account: "revenue:consulting".into(),
                    service_start: None,
                    term: Term::Months(3),
                    proration: Proration::Anniversary,
                    contract: None,
                });
            }),
            Some(DocumentError::Invalid(_))
        ));
        // With no party, a line may name its own customer.
        let mut spec = invoice_105(&journal);
        spec.party = None;
        spec.lines[0].tags.push(tag("customer", "Customer X"));
        assert!(build_document(&spec, &mut IdGenerator::avoiding(Vec::new())).is_ok());
    }

    #[test]
    fn tag_value_from_makes_free_text_safe() {
        assert_eq!(
            tag_value_from("  PO #105,  rev\t2\n").as_deref(),
            Some("PO #105 rev 2")
        );
        assert_eq!(tag_value_from(" , \n").as_deref(), None);
        let long = "é".repeat(200);
        let cut = tag_value_from(&long).expect("value");
        assert!(cut.len() <= 256 && long.starts_with(&cut));
        let words = "word ".repeat(100);
        let cut = tag_value_from(&words).expect("value");
        assert!(cut.len() <= 256 && !cut.ends_with(' '));
        for value in ["PO #105 rev 2", cut.as_str()] {
            assert!(check_tag_value("x", value).is_ok());
        }
    }
}
