//! What the read commands show (ARCHITECTURE §6.4): `list`, `show`,
//! `check`, `customers list|show` and `config`, as serializable shapes the
//! CLI prints with `--json` and the HTTP API will return. The shapes are a
//! contract for scripts: fields are added, never renamed or removed.
//!
//! "Open" and "overdue" read the books when they can say
//! ([`BookStates::is_open`]: settlement links in accrual books, applied
//! payments in cash books, a zero total paid on its date), so a list is
//! right even before `reconcile` refreshes the `paid:` mirrors; an invoice
//! the books can't speak for (accrual, never booked) falls back to its
//! mirror. Overdue is open with `due_date` before `today`.

use crate::catalog;
use crate::customers::{self, BillingContact, ContactKind, ContractRef, Customer, CustomerLookup};
use crate::doc::{Header, Invoice, Paid, Status};
use crate::frontmatter::Fields;
use crate::index::{DocumentIndex, IndexedDoc, ListFilter};
use crate::problem::{Problem, ProblemCode, Severity};
use crate::reconcile::{self, BookStates};
use crate::session::Session;
use crate::settings::Settings;
use crate::table::books::BooksEntry;
use crate::table::columns::Role;
use crate::table::{BillTable, RowKind};
use ledgeline_core::accrual::schedule::{Proration, Term};
use ledgeline_core::vault::{VaultError, VaultPath};
use serde::Serialize;
use std::collections::HashMap;

pub mod account;

/// Whether a document is open by its `paid:` mirror alone: sent, and
/// `paid:` isn't a date.
#[must_use]
pub fn is_open(header: &Header) -> bool {
    header.status == Some(Status::Sent) && !matches!(header.paid, Some(Paid::Full { .. }))
}

/// Whether `doc` is open: by the books when given (and they can say), else
/// by its mirror.
#[must_use]
pub fn open_in(states: Option<&BookStates>, doc: &IndexedDoc) -> bool {
    states.map_or_else(|| is_open(&doc.header), |s| s.is_open(doc))
}

/// Whether `doc` is overdue on `today`: open (as [`open_in`]) and due
/// before it.
#[must_use]
pub fn overdue_in(states: Option<&BookStates>, doc: &IndexedDoc, today: &str) -> bool {
    open_in(states, doc)
        && doc
            .header
            .due_date
            .as_deref()
            .is_some_and(|due| due < today)
}

/// Whether a document is overdue on `today`: open, and due before it.
#[must_use]
pub fn is_overdue(header: &Header, today: &str) -> bool {
    is_open(header) && header.due_date.as_deref().is_some_and(|due| due < today)
}

/// Error and warning counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    /// Errors.
    pub errors: usize,
    /// Warnings.
    pub warnings: usize,
}

impl Counts {
    /// The counts of `problems`.
    pub fn of<'a>(problems: impl IntoIterator<Item = &'a Problem>) -> Self {
        problems
            .into_iter()
            .fold(Self::default(), |c, p| match p.severity {
                Severity::Error => Self {
                    errors: c.errors + 1,
                    ..c
                },
                Severity::Warning => Self {
                    warnings: c.warnings + 1,
                    ..c
                },
            })
    }
}

/// One row of `list`.
#[derive(Debug, Clone, Serialize)]
pub struct DocEntry<'a> {
    /// The file, vault-relative.
    pub path: &'a VaultPath,
    /// Its title.
    pub title: &'a str,
    /// The frontmatter fields the invoicer reads.
    #[serde(flatten)]
    pub header: &'a Header,
    /// The amount due, as the table writes it; `None` when the table doesn't
    /// read.
    pub total: Option<&'a str>,
    /// The commodity.
    pub commodity: Option<&'a str>,
    /// Sent and not paid in full (by the books; else the `paid:` mirror).
    pub open: bool,
    /// Open and past `due_date`.
    pub overdue: bool,
    /// What `check` finds in this file.
    pub problems: Counts,
}

/// `list`'s filters beyond [`ListFilter`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQuery {
    /// Type, status and customer.
    pub filter: ListFilter,
    /// Only open documents.
    pub open: bool,
    /// Only overdue documents.
    pub overdue: bool,
}

fn problem_counts(problems: &[Problem]) -> HashMap<&VaultPath, Counts> {
    let mut by_file: HashMap<&VaultPath, Vec<&Problem>> = HashMap::new();
    for problem in problems {
        if let Some(file) = &problem.file {
            by_file.entry(file).or_default().push(problem);
        }
    }
    by_file
        .into_iter()
        .map(|(file, found)| (file, Counts::of(found)))
        .collect()
}

fn entry<'a>(
    doc: &'a IndexedDoc,
    states: Option<&BookStates>,
    today: &str,
    counts: Counts,
) -> DocEntry<'a> {
    DocEntry {
        path: &doc.path,
        title: &doc.title,
        header: &doc.header,
        total: doc.summary.as_ref().map(|s| s.total_text.as_str()),
        commodity: doc.summary.as_ref().map(|s| s.commodity.as_str()),
        open: open_in(states, doc),
        overdue: overdue_in(states, doc, today),
        problems: counts,
    }
}

/// `list`: matching documents, newest first, each with its `check` counts
/// (`problems`, from [`problems`]). Open and overdue read `states` (the
/// books) when given, else the `paid:` mirrors.
#[must_use]
pub fn list<'a>(
    index: &'a DocumentIndex,
    states: Option<&BookStates>,
    problems: &[Problem],
    query: &ListQuery,
    today: &str,
) -> Vec<DocEntry<'a>> {
    let counts = problem_counts(problems);
    index
        .list(&query.filter)
        .into_iter()
        .filter(|d| !query.open || open_in(states, d))
        .filter(|d| !query.overdue || overdue_in(states, d, today))
        .map(|d| {
            entry(
                d,
                states,
                today,
                counts.get(&d.path).copied().unwrap_or_default(),
            )
        })
        .collect()
}

/// A deferred row's recognition terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeferralView {
    /// The account it is recognized into.
    pub target_account: String,
    /// Whole months, for `N months`.
    pub months: Option<u32>,
    /// The last day, for `through D`.
    pub through: Option<String>,
    /// The first day of service, when written.
    pub service_start: Option<String>,
    /// Calendar-month proration.
    pub calendar: bool,
}

/// One bill-table row, as written and as computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowView {
    /// 1-based row number in the table.
    pub row: usize,
    /// 1-based line in the document.
    pub line: usize,
    /// `item`, `adjustment`, `fixed-adjustment`, `total` or `spacer`.
    pub kind: &'static str,
    /// The description cell.
    pub description: String,
    /// The code cell.
    pub code: Option<String>,
    /// The quantity cell.
    pub quantity: Option<String>,
    /// The unit cell.
    pub unit: Option<String>,
    /// The rate cell (money, or a percentage on an adjustment).
    pub rate: Option<String>,
    /// The amount the arithmetic gives, formatted (`None` for a spacer).
    pub amount: Option<String>,
    /// The amount cell as written.
    pub written: String,
    /// Whether the amount is computed rather than as written.
    pub computed: bool,
    /// For an adjustment: the 1-based row it applies to.
    pub applies_to: Option<usize>,
    /// The books cell, canonical (`a → b, 12 months`).
    pub books: Option<String>,
    /// The account the row posts to.
    pub account: Option<String>,
    /// A deferred row's terms.
    pub deferral: Option<DeferralView>,
}

/// A written amount the arithmetic disagrees with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscrepancyView {
    /// 1-based row.
    pub row: usize,
    /// 1-based line.
    pub line: usize,
    /// As written (empty when blank).
    pub written: String,
    /// What the arithmetic gives.
    pub expected: String,
}

/// The bill table, computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableView {
    /// The commodity.
    pub commodity: String,
    /// The amount due: the last total.
    pub total: String,
    /// Every body row.
    pub rows: Vec<RowView>,
    /// Written amounts that disagree (`recalc` fixes them).
    pub discrepancies: Vec<DiscrepancyView>,
}

fn kind_name(kind: RowKind) -> &'static str {
    match kind {
        RowKind::Spacer => "spacer",
        RowKind::Total => "total",
        RowKind::Adjustment(_) => "adjustment",
        RowKind::FixedAdjustment => "fixed-adjustment",
        RowKind::Item => "item",
    }
}

fn deferral_view(books: &BooksEntry) -> Option<DeferralView> {
    books.deferral().map(|d| DeferralView {
        target_account: d.target_account.clone(),
        months: match &d.term {
            Term::Months(n) => Some(*n),
            Term::Through(_) => None,
        },
        through: match &d.term {
            Term::Months(_) => None,
            Term::Through(end) => Some(end.clone()),
        },
        service_start: d.service_start.clone(),
        calendar: d.proration == Proration::Calendar,
    })
}

fn cell(table: &BillTable, row: usize, role: Role) -> Option<String> {
    let text = crate::table::money::plain(table.cell(row, role)).trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The bill table as [`TableView`].
///
/// # Errors
/// The table's [`crate::table::TableError`], as text.
pub fn table_view(
    table: &BillTable,
    words: &crate::table::words::Words,
    money: &crate::doc::Money,
) -> Result<TableView, crate::table::TableError> {
    let style = money.for_table(table);
    let evaluation = table.evaluate(words, &style)?;
    let discrepancies = table
        .check(words, &style)?
        .into_iter()
        .map(|d| DiscrepancyView {
            row: d.at.index + 1,
            line: d.at.line,
            written: d.written,
            expected: d.expected,
        })
        .collect();
    let rows = evaluation
        .rows
        .iter()
        .map(|r| {
            let i = r.at.index;
            RowView {
                row: i + 1,
                line: r.at.line,
                kind: kind_name(r.kind),
                description: crate::table::money::plain(&r.at.description)
                    .trim()
                    .to_string(),
                code: cell(table, i, Role::Item),
                quantity: cell(table, i, Role::Quantity),
                unit: cell(table, i, Role::Unit),
                rate: cell(table, i, Role::Rate),
                amount: r.amount.map(|a| style.format(a)),
                written: table.cell(i, Role::Amount).trim().to_string(),
                computed: r.computed,
                applies_to: r.applies_to.map(|t| t + 1),
                books: r.books.as_ref().map(ToString::to_string),
                account: r.books.as_ref().map(|b| b.account().as_str().to_string()),
                deferral: r.books.as_ref().and_then(deferral_view),
            }
        })
        .collect();
    Ok(TableView {
        commodity: style.commodity().to_string(),
        total: style.format(evaluation.total),
        rows,
        discrepancies,
    })
}

/// `show`: one document in full.
#[derive(Debug, Clone, Serialize)]
pub struct DocDetail<'a> {
    /// The file, vault-relative.
    pub path: &'a VaultPath,
    /// Its title.
    pub title: &'a str,
    /// The fields the invoicer reads.
    #[serde(flatten)]
    pub header: &'a Header,
    /// Sent and not paid in full.
    pub open: bool,
    /// Open and past due.
    pub overdue: bool,
    /// Every frontmatter field, lossless (custom fields included).
    pub frontmatter: Fields,
    /// The bill table, computed; `None` when there is none or it doesn't
    /// read (`problems` says why).
    pub table: Option<TableView>,
    /// Prose above the bill table, as written.
    pub prose_before: String,
    /// Prose below it (supporting tables included), as written.
    pub prose_after: String,
    /// What `check` finds in this file.
    pub problems: Vec<Problem>,
}

/// `show` for `doc`, reading its file once.
///
/// # Errors
/// When the file can't be read.
pub fn show<'a>(
    session: &'a Session,
    doc: &'a IndexedDoc,
    problems: &[Problem],
    today: &str,
) -> Result<DocDetail<'a>, VaultError> {
    let settings = session.settings();
    let money = session.index().money();
    let source = session.vault().read_to_string(&doc.path)?;
    let invoice = Invoice::parse(source, &settings.words);
    let table = invoice
        .bill_table()
        .ok()
        .flatten()
        .and_then(|t| table_view(t, &settings.words, money).ok());
    Ok(DocDetail {
        path: &doc.path,
        title: &doc.title,
        header: &doc.header,
        open: open_in(Some(session.book_states()), doc),
        overdue: overdue_in(Some(session.book_states()), doc, today),
        frontmatter: invoice.frontmatter().fields.clone(),
        table,
        prose_before: invoice.prose_before().trim().to_string(),
        prose_after: invoice.prose_after().trim().to_string(),
        problems: problems
            .iter()
            .filter(|p| p.file.as_ref() == Some(&doc.path))
            .cloned()
            .collect(),
    })
}

/// `check`'s report.
#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    /// Documents checked.
    pub documents: usize,
    /// Customers checked.
    pub customers: usize,
    /// Errors and warnings found.
    #[serde(flatten)]
    pub counts: Counts,
    /// Every problem, by file and line.
    pub problems: Vec<Problem>,
}

/// Every problem in the documents, customers and catalog, sorted by file and
/// line, without the books checks (`list` and `show` count these).
#[must_use]
pub fn document_problems(session: &Session) -> Vec<Problem> {
    let index = session.index();
    let mut out = index.check(session.vault());
    let (_, catalog) = catalog::load(
        session.vault(),
        index.site(),
        &index.settings().words,
        index.money(),
    );
    out.extend(catalog);
    sort(&mut out);
    out
}

fn sort(out: &mut [Problem]) {
    out.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.message.cmp(&b.message))
    });
}

/// The books checks ([`reconcile::audit`]): duplicate numbers, missing and
/// orphan transactions, moved `document:` paths, booked drafts, voids on
/// one side, books out of date and stale mirrors.
/// `only` limits them to those invoice numbers.
#[must_use]
pub fn books_problems(session: &Session, only: Option<&[String]>) -> Vec<Problem> {
    match session.loaded() {
        Ok(loaded) => reconcile::audit(
            &session.invoice_books(),
            session.index(),
            &loaded,
            session.book_states(),
            only,
        ),
        Err(error) => vec![
            Problem::error(
                ProblemCode::Books,
                format!("the books can't be read for the books checks: {error}"),
            )
            .in_file(session.main()),
        ],
    }
}

/// Every problem in the documents, customers, catalog and books, sorted by
/// file and line.
#[must_use]
pub fn problems(session: &Session) -> Vec<Problem> {
    let mut out = document_problems(session);
    out.extend(books_problems(session, None));
    sort(&mut out);
    out
}

/// `check`.
#[must_use]
pub fn check(session: &Session) -> CheckReport {
    let problems = problems(session);
    CheckReport {
        documents: session.index().docs().count(),
        customers: session.index().customers().len(),
        counts: Counts::of(&problems),
        problems,
    }
}

/// One row of `customers list`.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerEntry {
    /// The note.
    pub path: VaultPath,
    /// Its title.
    pub title: String,
    /// `person` or `organization`.
    pub kind: ContactKind,
    /// The `customer:` tag value in the books.
    pub ledger_customer: String,
    /// Net days.
    pub terms: Option<u32>,
    /// Who invoices go to.
    pub billing: BillingContact,
    /// The billing portal's name, when invoices go through one.
    pub portal: Option<String>,
    /// Documents billed to it.
    pub invoices: usize,
    /// Of those, the open ones.
    pub open: usize,
}

fn billed_to<'a>(index: &'a DocumentIndex, customer: &Customer) -> Vec<&'a IndexedDoc> {
    let site = index.site();
    index
        .list(&ListFilter::default())
        .into_iter()
        .filter(|d| {
            d.header.invoicee.link.as_deref().is_some_and(|link| {
                site.resolve_wikilink(&d.path, link)
                    .is_some_and(|n| n.path == customer.contact.path)
            })
        })
        .collect()
}

fn customer_entry(
    index: &DocumentIndex,
    states: Option<&BookStates>,
    customer: &Customer,
) -> CustomerEntry {
    let docs = billed_to(index, customer);
    CustomerEntry {
        path: customer.contact.path.clone(),
        title: customer.contact.title.clone(),
        kind: customer.contact.kind,
        ledger_customer: customer.ledger_customer.clone(),
        terms: customer.terms,
        billing: customer.billing_contact(index.site(), &index.settings().words),
        portal: customer.billing_portal.as_ref().map(|p| {
            p.name
                .clone()
                .or_else(|| p.url.clone())
                .unwrap_or_else(|| "portal".to_string())
        }),
        invoices: docs.len(),
        open: docs.iter().filter(|d| open_in(states, d)).count(),
    }
}

/// `customers list`, by title. Open counts read `states` (the books) when
/// given.
#[must_use]
pub fn customers(index: &DocumentIndex, states: Option<&BookStates>) -> Vec<CustomerEntry> {
    let mut all: Vec<CustomerEntry> = index
        .customers()
        .iter()
        .map(|c| customer_entry(index, states, c))
        .collect();
    all.sort_by(|a, b| {
        crate::notes::normalize_name(&a.title)
            .cmp(&crate::notes::normalize_name(&b.title))
            .then_with(|| a.path.cmp(&b.path))
    });
    all
}

/// A `contracts:` entry and where it is.
#[derive(Debug, Clone, Serialize)]
pub struct ContractView {
    /// As written.
    #[serde(flatten)]
    pub entry: ContractRef,
    /// The note or file it names; `None` when it doesn't exist.
    pub found: Option<VaultPath>,
    /// What to call it: the note's title, else the link's target, or the
    /// file's name. Never the raw `[[…]]` or path.
    pub name: String,
}

/// A contract's name from its entry alone: a link's target (`[[MSA|the
/// MSA]]` → `MSA`), or a path's file name (`/c/SOW 3.pdf` → `SOW 3.pdf`).
#[must_use]
pub fn contract_name(contract: &ContractRef) -> String {
    match contract {
        ContractRef::Note { link } => crate::notes::wikilink_target(link)
            .unwrap_or(link.as_str())
            .to_string(),
        ContractRef::File { path } => {
            let trimmed = path.trim().trim_end_matches('/');
            trimmed
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(trimmed)
                .to_string()
        }
    }
}

/// `customers show`.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerDetail<'a> {
    /// The list row.
    #[serde(flatten)]
    pub entry: CustomerEntry,
    /// The contact fields: emails, addresses, company.
    pub contact: &'a crate::customers::Contact,
    /// `revenue_account`.
    pub revenue_account: Option<&'a str>,
    /// `invoice_template`.
    pub invoice_template: Option<&'a str>,
    /// `contracts:`, resolved.
    pub contracts: Vec<ContractView>,
    /// `billing_portal:`.
    pub billing_portal: Option<&'a crate::customers::BillingPortal>,
    /// `open_pos:`.
    pub open_pos: &'a [crate::customers::OpenPo],
    /// `snippets:` (default snippets, as written).
    pub snippets: &'a [String],
    /// Its documents, newest first.
    pub documents: Vec<DocEntry<'a>>,
    /// What `check` finds in the customer note.
    pub problems: Vec<Problem>,
}

/// The customer `name` names: a title, alias, file stem or `[[wikilink]]`.
///
/// # Errors
/// [`CustomerLookup`].
pub fn find_customer(index: &DocumentIndex, name: &str) -> Result<Customer, CustomerLookup> {
    customers::find_customer(
        index.site(),
        &VaultPath::root(),
        name,
        &index.settings().words,
    )
}

/// `customers show` for `customer`.
#[must_use]
pub fn customer_detail<'a>(
    session: &'a Session,
    customer: &'a Customer,
    problems: &[Problem],
    today: &str,
) -> CustomerDetail<'a> {
    let index = session.index();
    let counts = problem_counts(problems);
    let contracts = customer
        .contracts
        .iter()
        .map(|c| {
            let target = customer.resolve_contract(c, index.site(), session.vault());
            let name = match (&target, c) {
                (Some(customers::ContractTarget::Note(p)), _) => index
                    .site()
                    .note(p)
                    .map_or_else(|| contract_name(c), |n| n.title.clone()),
                _ => contract_name(c),
            };
            ContractView {
                entry: c.clone(),
                found: target.map(|t| match t {
                    customers::ContractTarget::Note(p) | customers::ContractTarget::File(p) => p,
                }),
                name,
            }
        })
        .collect();
    let states = Some(session.book_states());
    CustomerDetail {
        entry: customer_entry(index, states, customer),
        contact: &customer.contact,
        revenue_account: customer.revenue_account.as_deref(),
        invoice_template: customer.invoice_template.as_deref(),
        contracts,
        billing_portal: customer.billing_portal.as_ref(),
        open_pos: &customer.open_pos,
        snippets: &customer.snippets,
        documents: billed_to(index, customer)
            .into_iter()
            .map(|d| {
                entry(
                    d,
                    states,
                    today,
                    counts.get(&d.path).copied().unwrap_or_default(),
                )
            })
            .collect(),
        problems: problems
            .iter()
            .filter(|p| p.file.as_ref() == Some(&customer.contact.path))
            .cloned()
            .collect(),
    }
}

/// One posting of a transaction, as the books panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PostingView {
    /// The account.
    pub account: String,
    /// Its amounts, each in the journal's style (`-$8,350.00`).
    pub amounts: Vec<String>,
}

/// One transaction the books hold under an invoice's number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryView {
    /// Its date.
    pub date: String,
    /// Its description.
    pub description: String,
    /// The journal file it is in, relative to the books folder (or its file
    /// name, when it is outside the folder): never a full local path.
    pub file: String,
    /// Its first line in that file.
    pub line: u32,
    /// Its postings, in file order.
    pub postings: Vec<PostingView>,
}

/// One payment or credit against an invoice's receivable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceivedView {
    /// When.
    pub date: String,
    /// How much, in the journal's style.
    pub amount: String,
    /// `payment` (money came in) or `credit` (a credit note, a write-off).
    pub kind: &'static str,
    /// The settling transaction's description.
    pub description: String,
    /// Where it came from: the transaction's other accounts (the bank, a
    /// credit's account), not the receivable.
    pub from: Vec<String>,
    /// The journal file it is in (as [`EntryView::file`]).
    pub file: String,
    /// Its first line in that file.
    pub line: u32,
}

/// Where an invoice stands with the books, in one line: what the Books
/// panel says first ("Paid in full on …", "Partly paid: $X of $Y, $Z due").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettlementView {
    /// `paid`, `partial`, `unpaid` or `void`.
    pub state: &'static str,
    /// What it is for, in the journal's style.
    pub total: String,
    /// What has been paid or credited.
    pub paid: String,
    /// What is left to pay.
    pub outstanding: String,
    /// When it was paid in full.
    pub paid_on: Option<String>,
    /// When it is due (the file's `due_date`).
    pub due_date: Option<String>,
}

/// An invoice's receivable posting (accrual books).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceivableView {
    /// The receivable account.
    pub account: String,
    /// What was booked.
    pub amount: String,
    /// What is left to settle.
    pub outstanding: String,
    /// `open`, `partial`, `paid` or `void`.
    pub state: &'static str,
    /// Its payments and credits, oldest first.
    pub payments: Vec<ReceivedView>,
}

/// A cash-books payment applied to an invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaymentView {
    /// The payment transaction.
    #[serde(flatten)]
    pub entry: EntryView,
    /// What it paid toward the invoice.
    pub amount: String,
}

/// What the books say about one invoice: the Books panel (ARCHITECTURE
/// §6.3), read-only. `GET /api/invoices/{num}/books`.
#[derive(Debug, Clone, Serialize)]
pub struct InvoiceBooksView {
    /// The books' basis.
    pub basis: crate::books::Basis,
    /// `invoice_num`, when the document has one.
    pub number: Option<String>,
    /// `status`.
    pub status: Option<Status>,
    /// The transactions it is booked as (one, normally).
    pub bookings: Vec<EntryView>,
    /// Its receivable (accrual books, once booked).
    pub receivable: Option<ReceivableView>,
    /// The reversals that void it.
    pub voids: Vec<EntryView>,
    /// Cash books: the payments applied to it.
    pub payments: Vec<PaymentView>,
    /// What the books say `paid:` should be, when they can say.
    pub books_paid: Option<reconcile::BooksPaid>,
    /// The `paid:` mirror the books say (`no`, a date, `partial …`).
    pub books_paid_text: Option<String>,
    /// The `paid:` mirror as the file writes it.
    pub paid: Option<Paid>,
    /// Whether the file's mirror says what the books say (`None` when the
    /// books can't say).
    pub in_step: Option<bool>,
    /// Where it stands, in one line (a sent or void invoice the books can
    /// say something about).
    pub settlement: Option<SettlementView>,
    /// When the file's `paid:` mirror disagrees with the books: that, in
    /// plain words.
    pub mirror_problem: Option<String>,
    /// What `check` finds in the books for this invoice.
    pub problems: Vec<Problem>,
}

fn payment_state(state: ledgeline_core::accrual::receivables::PaymentState) -> &'static str {
    use ledgeline_core::accrual::receivables::PaymentState;
    match state {
        PaymentState::Open => "open",
        PaymentState::Partial => "partial",
        PaymentState::Paid => "paid",
        PaymentState::Void => "void",
    }
}

/// A journal file as the panel names it: by the books folder, else by its
/// file name (never a full local path).
fn journal_file(session: &Session, file: &std::path::Path) -> String {
    session.vault().vault_path(file).map_or_else(
        |_| {
            file.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        },
        |p| p.to_string(),
    )
}

/// A payment or credit on `receivable`, as the panel shows it.
fn received_view(
    session: &Session,
    receivable: &reconcile::ReceivableState,
    settled: &reconcile::SettledBy,
    style: &crate::table::money::MoneyStyle,
) -> ReceivedView {
    let txn = session.journal().transactions.get(settled.position);
    let mut from: Vec<String> = Vec::new();
    for posting in txn.map(|t| t.postings.as_slice()).unwrap_or_default() {
        let account = &posting.account.0;
        if account != &receivable.account && !from.contains(account) {
            from.push(account.clone());
        }
    }
    ReceivedView {
        date: settled.date.clone(),
        amount: style.format(settled.amount),
        kind: if settled.credit { "credit" } else { "payment" },
        description: txn.map(|t| t.description.clone()).unwrap_or_default(),
        from,
        file: txn
            .map(|t| journal_file(session, &t.source_file))
            .unwrap_or_default(),
        line: txn.map_or(0, |t| t.source_span.0.line),
    }
}

/// The transaction at `source`, as the panel shows it.
fn entry_view(session: &Session, source: &reconcile::Source) -> EntryView {
    let money = session.index().money();
    let txn = session.journal().transactions.get(source.position);
    let file = journal_file(session, &source.file);
    EntryView {
        date: source.date.clone(),
        description: txn.map(|t| t.description.clone()).unwrap_or_default(),
        file,
        line: source.line,
        postings: txn
            .map(|t| {
                t.postings
                    .iter()
                    .map(|p| PostingView {
                        account: p.account.0.clone(),
                        amounts: p
                            .amounts
                            .iter()
                            .map(|a| money.style(Some(&a.commodity.0)).format(a.quantity))
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The Books panel for `doc`: what the journal holds under its number.
#[must_use]
pub fn invoice_books(session: &Session, doc: &IndexedDoc) -> InvoiceBooksView {
    let states = session.book_states();
    let money = session.index().money();
    let commodity = doc.summary.as_ref().map(|s| s.commodity.as_str());
    let style = money.style(commodity);
    let number = doc.number().map(str::to_string);
    let standing = number.as_deref().and_then(|n| states.standing(n));
    let entries = |sources: &mut dyn Iterator<Item = &reconcile::Source>| -> Vec<EntryView> {
        sources.map(|s| entry_view(session, s)).collect()
    };
    let bookings = standing.map_or_else(Vec::new, |s| {
        entries(&mut s.bookings.iter().map(|b| &b.source))
    });
    let voids = standing.map_or_else(Vec::new, |s| entries(&mut s.voids.iter()));
    let receivable = standing
        .and_then(|s| s.bookings.iter().find_map(|b| b.receivable.as_ref()))
        .map(|r| {
            let style = money.style(Some(&r.commodity));
            ReceivableView {
                account: r.account.clone(),
                amount: style.format(r.amount),
                outstanding: style.format(r.outstanding),
                state: payment_state(r.state),
                payments: r
                    .settled_by
                    .iter()
                    .map(|settled| received_view(session, r, settled, &style))
                    .collect(),
            }
        });
    let payments = standing.map_or_else(Vec::new, |s| {
        s.payments
            .iter()
            .map(|p| PaymentView {
                entry: entry_view(session, &p.source),
                amount: money.style(Some(&p.commodity)).format(p.amount),
            })
            .collect()
    });
    let books_paid = states.paid(doc);
    let in_step = books_paid
        .as_ref()
        .map(|b| b.agrees_with(doc.header.paid.as_ref(), &style));
    let problems = number.as_ref().map_or_else(Vec::new, |n| {
        books_problems(session, Some(std::slice::from_ref(n)))
    });
    let settlement = settlement(session, doc, standing, books_paid.as_ref());
    let mirror_problem = match (&books_paid, in_step) {
        (Some(books), Some(false)) => Some(mirror_problem(doc.header.paid.as_ref(), books, &style)),
        _ => None,
    };
    InvoiceBooksView {
        basis: session.basis(),
        number,
        status: doc.header.status,
        bookings,
        receivable,
        voids,
        payments,
        books_paid_text: books_paid.as_ref().map(|b| b.text(&style)),
        books_paid,
        paid: doc.header.paid.clone(),
        in_step,
        settlement,
        mirror_problem,
        problems,
    }
}

/// Where `doc` stands with the books, for the panel's first line: from its
/// receivable (accrual books), else from the payments applied to it (cash
/// books), else from what the books say its `paid:` should be. `None` for a
/// draft, or a sent invoice the books know nothing of (a problem says so).
fn settlement(
    session: &Session,
    doc: &IndexedDoc,
    standing: Option<&reconcile::Standing>,
    books_paid: Option<&reconcile::BooksPaid>,
) -> Option<SettlementView> {
    use ledgeline_core::accrual::receivables::PaymentState;
    let money = session.index().money();
    let due_date = doc.header.due_date.clone();
    let paid_on = match books_paid {
        Some(reconcile::BooksPaid::Full { date }) => Some(date.clone()),
        _ => None,
    };
    if let Some(r) = standing.and_then(|s| s.bookings.iter().find_map(|b| b.receivable.as_ref())) {
        let style = money.style(Some(&r.commodity));
        let paid = if r.state == PaymentState::Void {
            ledgeline_core::Dec::zero()
        } else {
            r.amount.sub(r.outstanding).unwrap_or(r.amount)
        };
        let state = match r.state {
            PaymentState::Open => "unpaid",
            PaymentState::Partial => "partial",
            PaymentState::Paid => "paid",
            PaymentState::Void => "void",
        };
        return Some(SettlementView {
            state,
            total: style.format(r.amount),
            paid: style.format(paid),
            outstanding: style.format(r.outstanding),
            paid_on,
            due_date,
        });
    }
    if doc.header.status == Some(Status::Void) {
        let summary = doc.summary.as_ref();
        let style = money.style(summary.map(|s| s.commodity.as_str()));
        let total = summary.map_or_else(ledgeline_core::Dec::zero, |s| s.total);
        return Some(SettlementView {
            state: "void",
            total: style.format(total),
            paid: style.format(ledgeline_core::Dec::zero()),
            outstanding: style.format(ledgeline_core::Dec::zero()),
            paid_on: None,
            due_date,
        });
    }
    if doc.header.status != Some(Status::Sent) {
        return None;
    }
    let summary = doc.summary.as_ref()?;
    let style = money.style(Some(&summary.commodity));
    let applied = standing.map_or_else(ledgeline_core::Dec::zero, |s| {
        s.payments
            .iter()
            .filter(|p| p.commodity == summary.commodity)
            .fold(ledgeline_core::Dec::zero(), |sum, p| {
                sum.add(p.amount).unwrap_or(sum)
            })
    });
    let paid = if paid_on.is_some() {
        summary.total
    } else {
        applied
    };
    let outstanding = summary.total.sub(paid).unwrap_or(summary.total);
    let state = if paid_on.is_some() || (!summary.total.is_zero() && outstanding.mantissa <= 0) {
        "paid"
    } else if !paid.is_zero() {
        "partial"
    } else if session.basis() == crate::books::Basis::Accrual && books_paid.is_none() {
        // Accrual books, not booked: the missing transaction is a problem.
        return None;
    } else {
        "unpaid"
    };
    Some(SettlementView {
        state,
        total: summary.total_text.clone(),
        paid: style.format(paid),
        outstanding: style.format(if outstanding.mantissa < 0 {
            ledgeline_core::Dec::zero()
        } else {
            outstanding
        }),
        paid_on,
        due_date,
    })
}

/// The file's `paid:` mirror against the books', in plain words.
fn mirror_problem(
    written: Option<&Paid>,
    books: &reconcile::BooksPaid,
    style: &crate::table::money::MoneyStyle,
) -> String {
    let file_says = match written {
        None => "has no `paid:` line".to_string(),
        Some(Paid::No) => "says it is unpaid (`paid: no`)".to_string(),
        Some(Paid::Full { date }) => format!("says it was paid on {date}"),
        Some(Paid::Partial { parts }) => format!(
            "says it was partly paid ({})",
            parts
                .iter()
                .map(|p| format!("{} on {}", p.amount, p.date))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let books_say = match books {
        reconcile::BooksPaid::No => "nothing has been paid".to_string(),
        reconcile::BooksPaid::Full { date } => format!("it was paid in full on {date}"),
        reconcile::BooksPaid::Partial { parts } => format!(
            "it was partly paid ({})",
            parts
                .iter()
                .map(|p| format!("{} on {}", style.format_compact(p.amount), p.date))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    format!(
        "The invoice file {file_says}, but the books say {books_say}. The books are right: \
         the file's `paid:` line is rewritten from them when they change, or now with \
         `ledgeline-invoicer reconcile`."
    )
}

/// `[books]`, as `config` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct BooksView {
    /// `basis`, when set.
    pub basis: Option<&'static str>,
    /// `closed-through`, when set.
    pub closed_through: Option<String>,
    /// `recognition-file`, when set.
    pub recognition_file: Option<String>,
}

/// `config`: where the books are and the effective settings.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigView<'a> {
    /// The main journal, as a local path.
    pub journal: String,
    /// The books folder (the vault root).
    pub books_folder: String,
    /// `ledgeline.toml [books]` (shared with Ledgeline).
    pub books: BooksView,
    /// `ledgeline.toml [invoicer]`, defaults filled in, words included.
    pub invoicer: &'a Settings,
    /// What `[books]` ignored, and why.
    pub warnings: &'a [String],
}

/// `config`.
#[must_use]
pub fn config(session: &Session) -> ConfigView<'_> {
    let books = session.books_config();
    ConfigView {
        journal: session.local_path(session.main()).display().to_string(),
        books_folder: session.vault().root().display().to_string(),
        books: BooksView {
            basis: books.basis.map(|b| b.code()),
            closed_through: books.closed_through.clone(),
            recognition_file: books.recognition_file.clone(),
        },
        invoicer: session.settings(),
        warnings: &books.warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn session(name: &str) -> Session {
        let journal = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
            .join("books.journal");
        Session::open_local(&journal).expect("opens")
    }

    #[test]
    fn open_and_overdue_read_the_paid_mirror() {
        let session = session("accrual");
        let problems = problems(&session);
        let numbers = |query: ListQuery| -> Vec<String> {
            list(
                session.index(),
                Some(session.book_states()),
                &problems,
                &query,
                "2026-11-17",
            )
            .iter()
            .map(|d| {
                d.header
                    .number
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            })
            .collect()
        };
        assert_eq!(
            numbers(ListQuery {
                open: true,
                ..ListQuery::default()
            }),
            ["105", "107", "106"]
        );
        let overdue = numbers(ListQuery {
            overdue: true,
            ..ListQuery::default()
        });
        assert!(overdue.contains(&"106".to_string()), "{overdue:?}");
        assert!(!overdue.contains(&"105".to_string()), "{overdue:?}");
    }

    #[test]
    fn show_computes_the_table() {
        let session = session("accrual");
        let problems = problems(&session);
        let doc = session.find("105").expect("105");
        let detail = show(&session, doc, &problems, "2026-11-17").expect("reads");
        let table = detail.table.expect("table");
        assert_eq!(table.total, "$8,350.00");
        let kinds: Vec<&str> = table.rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [
                "item",
                "adjustment",
                "item",
                "adjustment",
                "total",
                "fixed-adjustment",
                "total"
            ]
        );
        assert_eq!(table.rows[1].applies_to, Some(1));
        assert!(table.discrepancies.is_empty());
        assert!(detail.prose_after.contains("## Timesheet"));
        let draft = session.find("108").expect("108");
        let detail = show(&session, draft, &problems, "2026-11-17").expect("reads");
        let deferred = detail.table.expect("table").rows[0]
            .deferral
            .clone()
            .expect("deferred");
        assert_eq!(deferred.target_account, "revenue:saas");
        assert_eq!(deferred.months, Some(12));
        assert_eq!(detail.frontmatter["support_tier"], "gold");
    }

    #[test]
    fn customers_count_their_documents() {
        let session = session("accrual");
        let all = customers(session.index(), Some(session.book_states()));
        let x = all
            .iter()
            .find(|c| c.title == "Customer X")
            .expect("Customer X");
        assert_eq!(x.portal.as_deref(), Some("Coupa"));
        assert_eq!(x.invoices, 3);
        let customer = find_customer(session.index(), "Customer X").expect("found");
        let detail = customer_detail(&session, &customer, &problems(&session), "2026-11-17");
        assert!(detail.contracts.iter().all(|c| c.found.is_some()));
        assert_eq!(detail.documents.len(), 3);
    }

    #[test]
    fn the_fixtures_check_clean_but_for_plan_45s_orphans() {
        let cash = check(&session("cash"));
        assert_eq!(cash.counts, Counts::default(), "{:#?}", cash.problems);
        assert!(cash.documents > 0 && cash.customers > 0);
        // Plan 45's invoices 2001–2004 are booked with no files
        // (fixtures/README.md): warnings, nothing else.
        let accrual = check(&session("accrual"));
        let found: Vec<(ProblemCode, String)> = accrual
            .problems
            .iter()
            .map(|p| (p.code, p.to_string()))
            .collect();
        assert_eq!(
            found,
            ["2001", "2002", "2003", "2004"]
                .iter()
                .zip([136, 277, 329, 347])
                .map(|(n, line)| (
                    ProblemCode::OrphanTransaction,
                    format!(
                        "books.journal:{line}: warning: the books have a booking for invoice {n}, \
                         which has no file (restore or renumber its file, or leave it if another \
                         app booked it)"
                    )
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(accrual.counts.errors, 0);
    }

    #[test]
    fn config_shows_books_and_invoicer() {
        let session = session("cash");
        let view = config(&session);
        assert_eq!(view.books.basis, Some("cash"));
        assert_eq!(view.invoicer.first_number(), "201");
        let json = serde_json::to_value(&view).expect("json");
        assert_eq!(json["invoicer"]["number-start"], "201");
        assert_eq!(json["invoicer"]["words"]["total"][0], "Total due");
    }
}
