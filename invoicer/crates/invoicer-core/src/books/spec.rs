//! An invoice document to what the books get (PLAN 2b, ARCHITECTURE §4.2):
//! the [`booking`](crate::booking) split of its bill table, labelled with the
//! rows it came from, plus the header the journal transaction carries.
//! Pure: the caller reads the files and the journal.
//!
//! - **Lines.** One per resolved account and deferral, each item netting its
//!   own adjustments, subtotal adjustments spread pro rata over the revenue
//!   lines (the remainder on the largest), as [`crate::booking::split`]
//!   does. Each line's `item:` tag names the item rows it came from (joined
//!   with ` + `), or, for a line no item feeds (tax, a deposit applied, a
//!   discount account), the rows that did.
//! - **Header.** `invoice:` (the number), then `document:` (the file's path
//!   in the books folder), `due:` (`due_date`, else `date + terms`) and
//!   `reference:`. A comma would end a tag value, so commas and line breaks
//!   become spaces ([`tag_value_from`]).
//! - **Description.** `<invoicee_name> | Invoice <number>`.
//! - **Party.** The customer note's `ledger_customer` (else its title) on
//!   every posting as `customer:`.
//!
//! [`InvoiceBooking::document_spec`] makes the accrual transaction (the
//! receivable debited with the total); [`InvoiceBooking::cash_split`] is what
//! a cash-books payment is re-split into.

use super::BooksError;
use crate::booking::{self, AccountDefaults, Booking};
use crate::catalog::Catalog;
use crate::customers::Customer;
use crate::doc::{Invoice, Money, Status};
use crate::problem::{Problem, Severity};
use crate::settings::Settings;
use crate::table::books::{AccountName, BooksEntry};
use crate::table::money::plain;
use crate::table::{Evaluation, ItemCode, RowKind};
use ledgeline_core::Dec;
use ledgeline_core::accrual::AccrualRole;
use ledgeline_core::accrual::AccrualRoles;
use ledgeline_core::accrual::invoice::{
    Deferral, Direction, DocumentLine, DocumentSpec, money, tag_value_from,
};
use ledgeline_core::accrual::schedule::{Party, PartyKind};
use ledgeline_core::model::Journal;
use ledgeline_core::reports::account_types::{AccountType, account_decls};
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use std::collections::BTreeMap;

/// The header tag naming the markdown file.
pub const DOCUMENT_TAG: &str = "document";
/// The header tag carrying the due date.
pub const DUE_TAG: &str = "due";
/// The header tag carrying the customer's reference.
pub const REFERENCE_TAG: &str = "reference";
/// The posting tag naming the rows a line came from.
pub const ITEM_TAG: &str = "item";
/// The `kpi:` value that marks the revenue account (ARCHITECTURE §3.3,
/// step 5).
const KPI_REVENUE: &str = "revenue";
/// Where deposits book when nothing says otherwise (I11).
pub const DEFAULT_DEPOSIT_ACCOUNT: &str = "liabilities:deferred-revenue";

/// One line of an invoice's booking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BookLine {
    /// The account the invoice's transaction posts to (for a deferred line,
    /// the deferred-revenue account).
    pub account: String,
    /// For a deferred line: the account it is recognized into.
    pub recognize_into: Option<String>,
    /// The deferral, for a deferred line.
    #[serde(skip)]
    pub deferral: Option<Deferral>,
    /// The amount, signed as the table writes it (revenue positive, a
    /// deposit applied negative). The journal posting is its negation.
    #[serde(serialize_with = "super::dec_text")]
    pub amount: Dec,
    /// The `item:` tag value.
    pub item: Option<String>,
    /// The body rows (0-based) it came from.
    pub rows: Vec<usize>,
}

impl BookLine {
    /// Where a cash-books payment of this line goes: the account itself, or
    /// for a deferred line the account it recognizes into (cash books have no
    /// deferred revenue: revenue is recognized when paid).
    #[must_use]
    pub fn cash_account(&self) -> &str {
        self.recognize_into.as_deref().unwrap_or(&self.account)
    }
}

/// What an invoice books, as derived from its markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvoiceBooking {
    /// `invoice_num`.
    pub number: String,
    /// The file.
    pub path: VaultPath,
    /// `date`: the transaction date.
    pub date: String,
    /// The transaction description.
    pub description: String,
    /// The `customer:` tag value.
    pub customer: String,
    /// The commodity.
    pub commodity: String,
    /// The amount due (the receivable debit).
    #[serde(serialize_with = "super::dec_text")]
    pub total: Dec,
    /// The due date, when known.
    pub due: Option<String>,
    /// The header tags after `invoice:`, in order.
    pub tags: Vec<(String, String)>,
    /// The lines; they sum to [`total`](Self::total).
    pub lines: Vec<BookLine>,
}

impl InvoiceBooking {
    /// The accrual transaction: `receivable` debited with the total, each
    /// line credited (a negative line debited). With a zero total (a final
    /// invoice its deposit covers) it is built with
    /// [`build_settled_document`](ledgeline_core::accrual::invoice::build_settled_document):
    /// no receivable posting.
    #[must_use]
    pub fn document_spec(&self, journal: &Journal, receivable: &str) -> DocumentSpec {
        self.spec_with(journal, receivable, false)
    }

    /// The cash-books transaction of a zero-total invoice: each line into
    /// its [`cash_account`](BookLine::cash_account), with no deferral (cash
    /// books recognize revenue when it is paid, and a deposit paid it).
    /// `settlement` is never written (the total is zero) but must name an
    /// account.
    #[must_use]
    pub fn cash_document_spec(&self, journal: &Journal, settlement: &str) -> DocumentSpec {
        self.spec_with(journal, settlement, true)
    }

    fn spec_with(&self, journal: &Journal, settlement: &str, cash: bool) -> DocumentSpec {
        DocumentSpec {
            direction: Direction::Revenue,
            date: self.date.clone(),
            description: self.description.clone(),
            number: Some(self.number.clone()),
            party: Some(Party {
                kind: PartyKind::Customer,
                name: self.customer.clone(),
            }),
            settlement_account: settlement.to_string(),
            lines: self
                .lines
                .iter()
                .map(|line| DocumentLine {
                    account: if cash {
                        line.cash_account().to_string()
                    } else {
                        line.account.clone()
                    },
                    amount: money(journal, &self.commodity, line.amount),
                    deferral: if cash { None } else { line.deferral.clone() },
                    tags: line
                        .item
                        .iter()
                        .map(|item| (ITEM_TAG.to_string(), item.clone()))
                        .collect(),
                })
                .collect(),
            tags: self.tags.clone(),
        }
    }

    /// What a cash-books payment is re-split into: each line's
    /// [`cash_account`](BookLine::cash_account) and amount, merged by
    /// account in first-appearance order, zero lines dropped.
    ///
    /// # Errors
    /// [`BooksError::Overflow`].
    pub fn cash_split(&self) -> Result<Vec<(String, Dec)>, BooksError> {
        let mut out: Vec<(String, Dec)> = Vec::new();
        for line in &self.lines {
            let account = line.cash_account();
            match out.iter_mut().find(|(a, _)| a == account) {
                Some((_, sum)) => *sum = sum.add(line.amount).map_err(|_| BooksError::Overflow)?,
                None => out.push((account.to_string(), line.amount)),
            }
        }
        out.retain(|(_, amount)| !amount.is_zero());
        Ok(out)
    }
}

/// What [`booking_of`] reads besides the document: the books folder's
/// settings and money styles, the journal (for account inference), the
/// customer and the catalogs.
#[derive(Debug, Clone, Copy)]
pub struct SpecContext<'a> {
    /// `[invoicer]`.
    pub settings: &'a Settings,
    /// The journal's money styles.
    pub money: &'a Money,
    /// The journal.
    pub journal: &'a Journal,
    /// The customer the invoice's `customer:` names.
    pub customer: &'a Customer,
    /// Every catalog.
    pub catalogs: &'a [Catalog],
}

/// The single account declared with `kpi: revenue`, else the single one
/// declared with type `R` (ARCHITECTURE §3.3, step 5).
#[must_use]
pub fn inferred_revenue_account(journal: &Journal) -> Option<String> {
    let single = |names: Vec<String>| match names.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    };
    let kpi: Vec<String> = journal
        .accounts
        .iter()
        .filter(|decl| {
            decl.tags
                .iter()
                .any(|(name, value)| name == "kpi" && value.trim() == KPI_REVENUE)
        })
        .map(|decl| decl.name.0.clone())
        .collect();
    if !kpi.is_empty() {
        return single(kpi);
    }
    single(
        account_decls(journal)
            .into_iter()
            .filter(|decl| decl.account_type == Some(AccountType::Revenue))
            .map(|decl| decl.name)
            .collect(),
    )
}

/// The receivable account: `[invoicer] receivable-account`, else the one
/// account the journal declares (or names) as a receivable.
///
/// # Errors
/// [`BooksError::Setup`] when there is none, or more than one.
pub fn receivable_account(settings: &Settings, journal: &Journal) -> Result<String, BooksError> {
    if let Some(account) = &settings.receivable_account {
        return Ok(account.clone());
    }
    let roles = AccrualRoles::of(journal);
    let found: Vec<&str> = roles
        .iter()
        .filter(|(_, role, _)| *role == AccrualRole::Receivable)
        .map(|(account, _, _)| account)
        .collect();
    match found.as_slice() {
        [one] => Ok((*one).to_string()),
        [] => Err(BooksError::Setup(
            "the books have no receivable account: declare one with `kpi: receivable`, or set \
             [invoicer] receivable-account in ledgeline.toml"
                .into(),
        )),
        many => Err(BooksError::Setup(format!(
            "the books have {} receivable accounts ({}); set [invoicer] receivable-account in \
             ledgeline.toml",
            many.len(),
            many.join(", ")
        ))),
    }
}

/// The deposit account (I11): `[invoicer] deposit-account`, else the one
/// account declared `kpi: deferred-revenue`, else
/// [`DEFAULT_DEPOSIT_ACCOUNT`].
#[must_use]
pub fn deposit_account(settings: &Settings, journal: &Journal) -> String {
    if let Some(account) = &settings.deposit_account {
        return account.clone();
    }
    let roles = AccrualRoles::of(journal);
    let declared: Vec<&str> = roles
        .iter()
        .filter(|(_, role, source)| {
            *role == AccrualRole::DeferredRevenue
                && *source == ledgeline_core::accrual::RoleSource::Declared
        })
        .map(|(account, _, _)| account)
        .collect();
    match declared.as_slice() {
        [one] => (*one).to_string(),
        _ => DEFAULT_DEPOSIT_ACCOUNT.to_string(),
    }
}

/// The defaults a blank books cell falls back through, for `customer`.
#[must_use]
pub fn account_defaults(
    settings: &Settings,
    journal: &Journal,
    customer: &Customer,
    catalogs: &[Catalog],
) -> AccountDefaults {
    let account = |name: &str| AccountName::new(name).ok();
    let mut catalog: BTreeMap<ItemCode, BooksEntry> = BTreeMap::new();
    for item in catalogs.iter().flat_map(|c| &c.items) {
        if let Some(books) = &item.books {
            catalog
                .entry(ItemCode(item.code.clone()))
                .or_insert_with(|| books.clone());
        }
    }
    AccountDefaults {
        catalog,
        customer: customer
            .revenue_account
            .as_deref()
            .and_then(|text| BooksEntry::parse(text).ok().flatten()),
        settings: settings.revenue_account.as_deref().and_then(account),
        inferred: inferred_revenue_account(journal)
            .as_deref()
            .and_then(account),
        discount: settings.discount_account.as_deref().and_then(account),
    }
}

/// A free-text value as a tag value. A comma would end it, so a thousands
/// separator (a comma between digits: `$16,700`) is dropped, and every
/// other comma and line break becomes a space ([`tag_value_from`]).
#[must_use]
pub fn tag_text(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let joined: String = chars
        .iter()
        .enumerate()
        .filter(|&(at, c)| {
            !(*c == ','
                && at > 0
                && chars[at - 1].is_ascii_digit()
                && chars.get(at + 1).is_some_and(char::is_ascii_digit))
        })
        .map(|(_, c)| *c)
        .collect();
    tag_value_from(&joined)
}

/// A table cell as plain text: bold and `&nbsp;` stripped, escaped pipes
/// unescaped.
fn cell_text(text: &str) -> String {
    plain(text).replace("\\|", "|")
}

/// The `item:` value of a line: the item rows it came from, else every row.
fn item_of(rows: &[usize], evaluation: &Evaluation) -> Option<String> {
    let described = |kinds: &dyn Fn(RowKind) -> bool| -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for &row in rows {
            let row = &evaluation.rows[row];
            let name = cell_text(&row.at.description);
            if kinds(row.kind) && !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
        names
    };
    let items = described(&|kind| kind == RowKind::Item);
    let names = if items.is_empty() {
        described(&|_| true)
    } else {
        items
    };
    tag_text(&names.join(" + "))
}

/// The errors that stop a document from being booked: any error-severity
/// problem of the file on its own (fields, table, arithmetic).
fn blocking(invoice: &Invoice, settings: &Settings, money: &Money) -> Vec<Problem> {
    invoice
        .problems(&settings.words, money)
        .into_iter()
        .filter(|p| p.severity == Severity::Error)
        .collect()
}

/// Derive what `invoice` (at `path`) books.
///
/// # Errors
/// [`BooksError::Invalid`] for a document with errors (a missing field, a
/// table that doesn't read or add up — run `recalc`); [`BooksError::Booking`]
/// for a row whose account doesn't resolve; [`BooksError::Refused`] for a
/// negative total (a credit note, later) or a number, customer or
/// reference that can't be a tag value.
pub fn booking_of(
    invoice: &Invoice,
    path: &VaultPath,
    cx: &SpecContext<'_>,
) -> Result<InvoiceBooking, BooksError> {
    let problems = blocking(invoice, cx.settings, cx.money);
    if !problems.is_empty() {
        return Err(BooksError::Invalid {
            path: path.clone(),
            problems: problems.into_iter().map(|p| p.in_file(path)).collect(),
        });
    }
    let header = invoice.header();
    let invalid = |message: &str| BooksError::Invalid {
        path: path.clone(),
        problems: vec![
            Problem::error(crate::problem::ProblemCode::MissingField, message).in_file(path),
        ],
    };
    let number = header
        .number
        .as_ref()
        .map(|n| n.as_str().to_string())
        .ok_or_else(|| invalid("`invoice_num` is missing"))?;
    let date = header
        .date
        .clone()
        .ok_or_else(|| invalid("`date` is missing"))?;
    let evaluation = invoice
        .evaluate(&cx.settings.words, cx.money)
        .map_err(|e| BooksError::Refused(format!("{path}: {e}")))?
        .ok_or_else(|| invalid("there is no bill table"))?;
    let table = invoice
        .bill_table()
        .ok()
        .flatten()
        .ok_or_else(|| invalid("there is no bill table"))?;
    let style = cx.money.for_table(table);
    let defaults = account_defaults(cx.settings, cx.journal, cx.customer, cx.catalogs);
    let Booking { total, lines } =
        booking::split(&evaluation, &defaults, &style).map_err(|source| BooksError::Booking {
            path: path.clone(),
            source,
        })?;
    if total.mantissa < 0 {
        return Err(BooksError::Refused(format!(
            "invoice {number}'s total is {}; a negative total can't be booked (a credit note is \
             a later document type)",
            style.format(total)
        )));
    }
    // A hand-written `$8,350` books as `$8350.00`: at least the commodity's
    // precision, never rounded.
    let precision = style.precision();
    let padded = |d: Dec| {
        if d.places < precision {
            d.rounded(precision).map_err(|_| BooksError::Overflow)
        } else {
            Ok(d)
        }
    };
    let total = padded(total)?;
    let lines: Vec<BookLine> = lines
        .into_iter()
        .map(|line| {
            Ok(BookLine {
                account: line.books.account().as_str().to_string(),
                recognize_into: line.books.deferral().map(|d| d.target_account.clone()),
                deferral: line.books.deferral().cloned(),
                amount: padded(line.amount)?,
                item: item_of(&line.rows, &evaluation),
                rows: line.rows,
            })
        })
        .collect::<Result<_, BooksError>>()?;
    let due = header.due_date.clone().or_else(|| {
        header
            .terms
            .filter(|t| *t >= 0)
            .map(|t| ledgeline_core::reports::add_days(&date, t))
    });
    let document = tag_text(path.as_str()).ok_or_else(|| invalid("the file has no name"))?;
    let mut tags = vec![(super::spec::DOCUMENT_TAG.to_string(), document)];
    if let Some(due) = &due {
        tags.push((DUE_TAG.to_string(), due.clone()));
    }
    if let Some(reference) = header.reference.as_deref().and_then(tag_text) {
        tags.push((REFERENCE_TAG.to_string(), reference));
    }
    let customer = tag_text(&cx.customer.ledger_customer).ok_or_else(|| {
        BooksError::Refused(format!(
            "{}: the customer has no name to tag the books with",
            cx.customer.contact.path
        ))
    })?;
    let payee = header
        .invoicee
        .name
        .as_deref()
        .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| customer.clone());
    Ok(InvoiceBooking {
        description: format!("{payee} | Invoice {number}"),
        number,
        path: path.clone(),
        date,
        customer,
        commodity: style.commodity().to_string(),
        total,
        due,
        tags,
        lines,
    })
}

/// Whether `status` is one a document of this kind may be booked from.
#[must_use]
pub fn is_bookable(status: Option<Status>) -> bool {
    matches!(status, Some(Status::Draft | Status::Sent))
}
