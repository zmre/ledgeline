//! Preview, send, re-book and void (ARCHITECTURE §4.1, §4.5).

use super::spec::{DOCUMENT_TAG, InvoiceBooking, receivable_account};
use super::{BooksError, Document, InvoiceBooks, check_generation};
use crate::catalog::Catalog;
use crate::doc::{Invoice, Status};
use crate::frontmatter::FieldEdit;
use crate::index::DocumentIndex;
use crate::settings::CashDeposits;
use ledgeline_core::Dec;
use ledgeline_core::accrual::invoice::{DocumentSpec, build_settled_document, money};
use ledgeline_core::accrual::ops::{Generation, Loaded, OpsError, SaveRequest, SourceChange};
use ledgeline_core::accrual::{
    AccrualRole, AccrualRoles, CUSTOMER_TAG, INVOICE_TAG, RECOGNITION_TAG, VOID_TAG, is_iso_date,
    is_posted_entry, tag_value,
};
use ledgeline_core::edit::{Fingerprint, format_transaction, render_amount};
use ledgeline_core::matching::{IdGenerator, MATCH_TAG, NOMATCH_TAG};
use ledgeline_core::model::{Amount, Posting, Tindex, Transaction};
use ledgeline_core::reports::{AccountTypes, account_decls};
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use serde_json::Value;

/// How the books recognize revenue: plan 45's gate under `[books] basis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Basis {
    /// An invoice books a receivable when sent.
    Accrual,
    /// An invoice books nothing until it is paid (Apply payment).
    Cash,
}

impl Basis {
    /// The basis of `loaded`.
    #[must_use]
    pub fn of(loaded: &Loaded) -> Self {
        if loaded.gate().enabled {
            Self::Accrual
        } else {
            Self::Cash
        }
    }
}

/// A transaction in the books, as its source text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BookedTransaction {
    /// Its date.
    pub date: String,
    /// Its description.
    pub description: String,
    /// Its text in the journal, exactly.
    pub text: String,
}

impl BookedTransaction {
    fn of(loaded: &Loaded, tindex: Tindex) -> Option<Self> {
        let txn = loaded
            .journal()
            .transactions
            .iter()
            .find(|txn| txn.index == tindex)?;
        Some(Self {
            date: txn.date.clone(),
            description: txn.description.clone(),
            text: loaded.editor().transaction_source(tindex)?,
        })
    }
}

fn generation_token<S: serde::Serializer>(g: &Generation, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(g.token())
}

/// What an invoice books and has booked: the books panel, and the
/// reconciler's "books out of date" comparison.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    /// The number.
    pub number: String,
    /// The file.
    pub path: VaultPath,
    /// `status`, as written.
    pub status: String,
    /// The books' basis.
    pub basis: Basis,
    /// What the markdown says to book.
    pub booking: InvoiceBooking,
    /// Accrual books: the receivable account.
    pub receivable: Option<String>,
    /// Accrual books: the transaction sending (or re-booking) would write.
    pub would_book: Option<String>,
    /// Accrual books: the transaction booked under the number, if any.
    pub booked: Option<BookedTransaction>,
    /// Whether [`booked`](Self::booked) is what the markdown says to book
    /// (`None` when nothing is booked).
    pub in_step: Option<bool>,
    /// Accrual books: whether payments are linked to the booked invoice.
    pub linked: bool,
    /// The books as read: hand it to the write to refuse a stale one.
    #[serde(serialize_with = "generation_token")]
    pub generation: Generation,
}

/// **Send** an invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Send {
    /// The number.
    pub number: String,
    /// Today: `sent_date`.
    pub today: String,
    /// How it was delivered (`sent_via`), when known.
    pub sent_via: Option<String>,
    /// Refuse unless the books are still as previewed.
    pub expected: Option<Generation>,
}

/// What [`InvoiceBooks::send`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sent {
    /// The number.
    pub number: String,
    /// The file.
    pub path: VaultPath,
    /// The books' basis.
    pub basis: Basis,
    /// Accrual books: the invoice's transaction.
    pub booked: Option<BookedTransaction>,
    /// Whether the journal was written now (false in cash books, and when an
    /// earlier send had already booked it).
    pub journal_written: bool,
    /// Whether the markdown was written now.
    pub markdown_written: bool,
}

/// **Re-book** a sent invoice after an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebook {
    /// The number.
    pub number: String,
    /// Today: the date of any restatement adjustment.
    pub today: String,
    /// Refuse unless the books are still as previewed.
    pub expected: Option<Generation>,
}

/// What [`InvoiceBooks::rebook`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rebooked {
    /// The number.
    pub number: String,
    /// The file.
    pub path: VaultPath,
    /// The books' basis.
    pub basis: Basis,
    /// Whether the books changed (false when they already agreed, and in
    /// cash books).
    pub changed: bool,
    /// The transaction before.
    pub previous: Option<BookedTransaction>,
    /// The transaction now.
    pub booked: Option<BookedTransaction>,
    /// What the caller should tell the user.
    pub warnings: Vec<String>,
}

/// **Void** an invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Void {
    /// The number.
    pub number: String,
    /// The void date: the reversal's date and `void_date`.
    pub date: String,
    /// Today: a void is never dated after it.
    pub today: String,
    /// Refuse unless the books are still as previewed.
    pub expected: Option<Generation>,
}

/// What [`InvoiceBooks::void`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Voided {
    /// The number.
    pub number: String,
    /// The file.
    pub path: VaultPath,
    /// The books' basis.
    pub basis: Basis,
    /// Accrual books: the reversing transaction.
    pub reversal: Option<BookedTransaction>,
    /// Accrual books: the `match:` id joining it to the invoice.
    pub link: Option<String>,
    /// Whether the journal was written now.
    pub journal_written: bool,
    /// Whether the markdown was written now.
    pub markdown_written: bool,
}

/// A transaction reduced to what booking decides: date, description, header
/// tags, and each posting's account, amounts and tags — less links
/// (`match:`, `nomatch:`), which the books add after booking, and with
/// `recognition:` ids blanked, since ids are assigned at write.
type Shape = (
    String,
    String,
    Vec<(String, String)>,
    Vec<(String, Vec<(String, String)>, Vec<(String, String)>)>,
);

fn shape(txn: &Transaction) -> Shape {
    let tags = |tags: &[(String, String)]| -> Vec<(String, String)> {
        tags.iter()
            .filter(|(name, _)| name != MATCH_TAG && name != NOMATCH_TAG)
            .map(|(name, value)| {
                let value = if name == RECOGNITION_TAG {
                    String::new()
                } else {
                    value.trim().to_string()
                };
                (name.clone(), value)
            })
            .collect()
    };
    (
        txn.date.clone(),
        txn.description.clone(),
        tags(&txn.tags),
        txn.postings
            .iter()
            .filter(|p| p.generated.is_none())
            .map(|p| {
                (
                    p.account.0.clone(),
                    p.amounts
                        .iter()
                        .map(|a| {
                            (
                                a.commodity.0.clone(),
                                ledgeline_core::edit::render_dec(a.quantity.normalized(), '.'),
                            )
                        })
                        .collect(),
                    tags(&p.tags),
                )
            })
            .collect(),
    )
}

/// The transaction `spec` builds (with placeholder schedule ids). A zero
/// total builds with no settlement posting, as it is written.
fn built(loaded: &Loaded, spec: &DocumentSpec) -> Result<Transaction, BooksError> {
    built_with(spec, &mut loaded.id_generator())
}

/// [`built`] with ids from `ids` (their values never matter: [`shape`]
/// blanks them).
fn built_with(spec: &DocumentSpec, ids: &mut IdGenerator) -> Result<Transaction, BooksError> {
    Ok(build_settled_document(spec, &mut || ids.next_id())
        .map_err(OpsError::from)?
        .transaction)
}

/// An invoice's transaction in the books, found by its number.
struct Booked {
    tindex: Tindex,
    fingerprint: Fingerprint,
    transaction: Transaction,
    /// Its receivable posting, when it has one. A final invoice its deposit
    /// covers (a zero total) is booked with none: there is nothing to
    /// receive, so Ledgeline's documents, receivables and matching never see
    /// it.
    settlement: Option<usize>,
}

impl Booked {
    fn linked(&self) -> bool {
        self.settlement
            .is_some_and(|at| tag_value(&self.transaction.postings[at].tags, MATCH_TAG).is_some())
    }
}

/// Whether `txn` is a zero-total invoice's own booking: `invoice:` and
/// `document:` on the header, no settlement leg, not a void. (A cash-books
/// payment applied carries `invoice:` but no `document:`.)
fn is_settled_booking(txn: &Transaction, number: &str, roles: &AccrualRoles) -> bool {
    tag_value(&txn.tags, INVOICE_TAG) == Some(number)
        && tag_value(&txn.tags, DOCUMENT_TAG).is_some()
        && tag_value(&txn.tags, VOID_TAG).is_none()
        && !is_posted_entry(txn)
        && !txn
            .postings
            .iter()
            .any(|p| roles.role(&p.account.0).is_some_and(|r| r.is_settlement()))
}

/// The one transaction booked under `number`, if any: Ledgeline's document
/// (a receivable debit), else a zero-total booking. Fingerprinting every
/// document (which `find_document` does on first use) is left for when there
/// is one.
fn booked(loaded: &Loaded, number: &str) -> Result<Option<Booked>, BooksError> {
    let twice = || {
        BooksError::Refused(format!(
            "invoice {number} is booked more than once; remove the extra transaction by hand"
        ))
    };
    match loaded
        .documents()
        .get(AccrualRole::Receivable, number)
        .len()
    {
        0 => {}
        1 => {
            return Ok(loaded.find_document(number).map(|found| Booked {
                tindex: found.tindex,
                fingerprint: found.fingerprint,
                transaction: found.transaction,
                settlement: Some(found.settlement.posting),
            }));
        }
        _ => return Err(twice()),
    }
    let roles = loaded.roles();
    let settled: Vec<&Transaction> = loaded
        .journal()
        .transactions
        .iter()
        .filter(|txn| is_settled_booking(txn, number, &roles))
        .collect();
    match settled.as_slice() {
        [] => Ok(None),
        [txn] => Ok(Some(Booked {
            tindex: txn.index,
            fingerprint: loaded
                .editor()
                .transaction_fingerprint(txn.index)
                .ok_or(OpsError::Stale)?,
            transaction: (*txn).clone(),
            settlement: None,
        })),
        _ => Err(twice()),
    }
}

/// The transaction booked under `number`, as written (after a write).
fn booked_text(loaded: &Loaded, number: &str) -> Option<BookedTransaction> {
    let found = booked(loaded, number).ok()??;
    BookedTransaction::of(loaded, found.tindex)
}

/// The void reversal booked for `number`, if any.
fn void_reversal(loaded: &Loaded, number: &str) -> Option<Tindex> {
    loaded
        .journal()
        .transactions
        .iter()
        .find(|txn| {
            tag_value(&txn.tags, INVOICE_TAG) == Some(number)
                && tag_value(&txn.tags, VOID_TAG).is_some()
        })
        .map(|txn| txn.index)
}

/// Cash books: the transactions a payment of `number` was applied to (its
/// own zero-total booking, which carries `document:`, aside).
pub(super) fn applied_payments<'j>(loaded: &'j Loaded, number: &str) -> Vec<&'j Transaction> {
    loaded
        .journal()
        .transactions
        .iter()
        .filter(|txn| {
            tag_value(&txn.tags, INVOICE_TAG) == Some(number)
                && tag_value(&txn.tags, VOID_TAG).is_none()
                && tag_value(&txn.tags, DOCUMENT_TAG).is_none()
        })
        .collect()
}

/// The postings of a payment applied to an invoice that say where its money
/// went: tagged `customer:` (`customer`'s, when given), not generated, and
/// not on a cash account. A bank leg an importer also tagged `customer:` is
/// the money arriving, not where it went, so it never counts. The one rule
/// for "what this invoice received" that Apply payment, open deposits and
/// the reconciler's paid mirror share (RC-12).
pub(crate) fn receipt_postings<'t>(
    txn: &'t Transaction,
    types: &'t AccountTypes,
    customer: Option<&'t str>,
) -> impl Iterator<Item = &'t Posting> + 't {
    txn.postings.iter().filter(move |posting| {
        posting.generated.is_none()
            && !types.is_cash(&posting.account.0)
            && tag_value(&posting.tags, CUSTOMER_TAG)
                .is_some_and(|tagged| customer.is_none_or(|c| c == tagged))
    })
}

/// Cash books: what the payments applied to invoice `number` put in each
/// account ([`receipt_postings`] for `customer`, in `commodity`), as
/// credits: positive where money came in. In the order first seen.
pub(super) fn received_by_account(
    loaded: &Loaded,
    number: &str,
    customer: &str,
    commodity: &str,
) -> Result<Vec<(String, Dec)>, BooksError> {
    let types = AccountTypes::new(&account_decls(loaded.journal()));
    let mut out: Vec<(String, Dec)> = Vec::new();
    for txn in applied_payments(loaded, number) {
        for posting in receipt_postings(txn, &types, Some(customer)) {
            for amount in posting
                .amounts
                .iter()
                .filter(|a| a.commodity.0 == commodity)
            {
                let credit = amount.quantity.neg().map_err(|_| BooksError::Overflow)?;
                match out.iter_mut().find(|(a, _)| *a == posting.account.0) {
                    Some((_, sum)) => *sum = sum.add(credit).map_err(|_| BooksError::Overflow)?,
                    None => out.push((posting.account.0.clone(), credit)),
                }
            }
        }
    }
    Ok(out)
}

/// Refuse `date` on or before `[books] closed-through`.
fn check_open(loaded: &Loaded, number: &str, date: &str) -> Result<(), BooksError> {
    match loaded.config().closed_through.as_deref() {
        Some(closed) if date <= closed => Err(BooksError::Refused(format!(
            "invoice {number} is dated {date}, on or before closed-through {closed}; date it in \
             an open period"
        ))),
        _ => Ok(()),
    }
}

/// The `recognition:` ids on `txn`'s postings, in order, once each.
fn schedule_ids(txn: &Transaction) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for posting in &txn.postings {
        if let Some(id) = tag_value(&posting.tags, RECOGNITION_TAG).map(str::trim)
            && !id.is_empty()
            && !ids.iter().any(|old| old == id)
        {
            ids.push(id.to_string());
        }
    }
    ids
}

/// A zero-total invoice's reversal: every posting negated, its tags kept,
/// dated `date`, tagged `invoice:` and `void:`. (No `match:` link: there is
/// no receivable to net.)
fn settled_reversal(
    original: &Transaction,
    number: &str,
    date: &str,
) -> Result<Transaction, BooksError> {
    let mut text = format!(
        "{date} {} (void)  ; {INVOICE_TAG}: {number}, {VOID_TAG}: {date}\n",
        original.description
    );
    for posting in original.postings.iter().filter(|p| p.generated.is_none()) {
        let [amount] = posting.amounts.as_slice() else {
            return Err(BooksError::Refused(format!(
                "invoice {number}'s posting on {} has no single amount to reverse",
                posting.account.0
            )));
        };
        let reversed = Amount {
            quantity: amount.quantity.neg().map_err(|_| BooksError::Overflow)?,
            ..amount.clone()
        };
        let comment = posting.comment.trim();
        text.push_str(&format!(
            "    {}  {}{}\n",
            posting.account.0,
            render_amount(&reversed),
            if comment.is_empty() {
                String::new()
            } else {
                format!("  ; {comment}")
            }
        ));
    }
    let parsed = ledgeline_core::parse_journal(&text, "void.journal")
        .map_err(|e| BooksError::Refused(format!("the void of invoice {number}: {e}")))?;
    parsed.transactions.into_iter().next().ok_or_else(|| {
        BooksError::Refused(format!("the void of invoice {number} didn't read back"))
    })
}

fn set(key: &str, value: &str) -> FieldEdit {
    FieldEdit::Set(key.to_string(), Value::String(value.to_string()))
}

fn check_date(what: &str, date: &str) -> Result<(), BooksError> {
    if is_iso_date(date) {
        Ok(())
    } else {
        Err(BooksError::Refused(format!(
            "{what} `{date}` is not a date (YYYY-MM-DD)"
        )))
    }
}

fn status_error(document: &Document, hint: &str) -> BooksError {
    BooksError::Status {
        number: document.number().to_string(),
        status: document.status_text(),
        hint: hint.to_string(),
    }
}

/// Whether the books get a transaction for `booking`: always in accrual
/// books; in cash books only for a zero total (a final invoice its deposit
/// covers), as a `$0` marker: no payment will ever arrive to apply, and the
/// marker is how the books (and `check`, and the reconciler) know it was
/// sent.
fn books_it(basis: Basis, booking: &InvoiceBooking) -> bool {
    basis == Basis::Accrual || booking.total.is_zero()
}

/// What sending writes for an invoice.
enum Plan {
    /// Accrual books: Ledgeline's document (with no receivable posting for
    /// a zero total).
    Document(DocumentSpec),
    /// Cash books, a zero total: a `$0` marker.
    Marker(Transaction),
}

impl Plan {
    /// The transaction as it would be written (placeholder schedule ids).
    fn transaction(&self, loaded: &Loaded) -> Result<Transaction, BooksError> {
        match self {
            Self::Document(spec) => built(loaded, spec),
            Self::Marker(txn) => Ok(txn.clone()),
        }
    }
}

/// Compares many documents with their bookings in one pass over loaded
/// books (`check` and the reconciler, ARCHITECTURE §4.5 "Books out of
/// date"): the catalogs are read once, the receivable account looked up
/// once, and one id generator serves every comparison.
pub struct StepCheck<'a> {
    books: &'a InvoiceBooks,
    index: &'a DocumentIndex,
    loaded: &'a Loaded,
    basis: Basis,
    /// Read on first use (a check of nothing reads nothing).
    catalogs: Option<Vec<Catalog>>,
    accounts: Option<super::spec::JournalAccounts>,
    receivable: Option<Result<String, String>>,
    ids: IdGenerator,
    roles: Option<&'a AccrualRoles>,
}

impl<'a> StepCheck<'a> {
    /// A comparer over `loaded`.
    #[must_use]
    pub fn new(books: &'a InvoiceBooks, index: &'a DocumentIndex, loaded: &'a Loaded) -> Self {
        Self {
            books,
            index,
            loaded,
            basis: Basis::of(loaded),
            catalogs: None,
            accounts: None,
            receivable: None,
            // Ids are blanked in the comparison, so none need avoiding.
            ids: IdGenerator::avoiding(Vec::new()),
            roles: None,
        }
    }

    /// Use `roles` (the journal's, already inferred) rather than inferring
    /// them again: each inference scans every posting.
    #[must_use]
    pub fn with_roles(mut self, roles: &'a AccrualRoles) -> Self {
        self.roles = Some(roles);
        self
    }

    /// The books' basis.
    #[must_use]
    pub fn basis(&self) -> Basis {
        self.basis
    }

    /// What `document` books, derived from its markdown.
    ///
    /// # Errors
    /// As [`InvoiceBooks::booking`].
    pub fn booking(&mut self, document: &Document) -> Result<InvoiceBooking, BooksError> {
        let customer = self.books.customer(self.index, document)?;
        let (books, index, journal) = (self.books, self.index, self.loaded.journal());
        let catalogs = self.catalogs.get_or_insert_with(|| books.catalogs(index));
        let roles = self.roles;
        let accounts = self.accounts.get_or_insert_with(|| match roles {
            Some(roles) => super::spec::JournalAccounts::of_roles(index.settings(), journal, roles),
            None => super::spec::JournalAccounts::of(index.settings(), journal),
        });
        super::spec::booking_of(
            &document.invoice,
            &document.path,
            &super::spec::SpecContext {
                settings: self.index.settings(),
                money: self.index.money(),
                journal: self.loaded.journal(),
                customer: &customer,
                catalogs,
                accounts: Some(accounts),
            },
        )
    }

    /// Whether `booked` (the transaction booked under `document`'s number)
    /// is what `document` says to book.
    ///
    /// # Errors
    /// When `document`'s booking can't be derived (as
    /// [`InvoiceBooks::booking`]), or the receivable account can't be found.
    pub fn in_step(
        &mut self,
        document: &Document,
        booked: &Transaction,
    ) -> Result<bool, BooksError> {
        let booking = self.booking(document)?;
        if !books_it(self.basis, &booking) {
            // Cash books booked a zero total that isn't zero any more.
            return Ok(false);
        }
        let would = match self.basis {
            Basis::Accrual => {
                let (settings, journal, roles) =
                    (self.index.settings(), self.loaded.journal(), self.roles);
                let receivable = match self.receivable.get_or_insert_with(|| {
                    match roles {
                        Some(roles) => super::spec::receivable_account_in(settings, roles),
                        None => receivable_account(settings, journal),
                    }
                    .map_err(|e| e.to_string())
                }) {
                    Ok(account) => account.clone(),
                    Err(why) => return Err(BooksError::Setup(why.clone())),
                };
                built_with(
                    &booking.document_spec(self.loaded.journal(), &receivable),
                    &mut self.ids,
                )?
            }
            Basis::Cash => cash_marker(self.loaded, &booking, self.index.settings().cash_deposits)?,
        };
        Ok(shape(booked) == shape(&would))
    }
}

/// A zero-total cash-books invoice's booking at send: the invoice's header
/// tags and description, and one posting per line — each row's cash
/// account, or for a deposit applied the deposit account — with `customer:`
/// and `item:`.
///
/// - Income on receipt (the default): every posting is `$0.00`, a marker
///   that says what was billed, that it was sent, and that nothing moved
///   (the deposit was revenue when it arrived). hledger reads zero postings
///   as balanced.
/// - Liability until final: the real amounts, the rows credited against the
///   deposit liability, which this clears.
fn cash_marker(
    loaded: &Loaded,
    booking: &InvoiceBooking,
    deposits: CashDeposits,
) -> Result<Transaction, BooksError> {
    let tags = std::iter::once(format!("{INVOICE_TAG}: {}", booking.number))
        .chain(
            booking
                .tags
                .iter()
                .map(|(name, value)| format!("{name}: {value}")),
        )
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!("{} {}  ; {tags}\n", booking.date, booking.description);
    for line in &booking.lines {
        let account = line.cash_account().unwrap_or(&line.account);
        let quantity = match deposits {
            // The total is padded to the commodity's precision: `$0.00`.
            CashDeposits::IncomeOnReceipt => Dec::new(0, booking.total.places),
            CashDeposits::LiabilityUntilFinal => {
                line.amount.neg().map_err(|_| BooksError::Overflow)?
            }
        };
        let amount = render_amount(&money(loaded.journal(), &booking.commodity, quantity));
        let mut comment = format!("{CUSTOMER_TAG}: {}", booking.customer);
        if let Some(item) = &line.item {
            comment.push_str(&format!(", item: {item}"));
        }
        text.push_str(&format!("    {account}  {amount}  ; {comment}\n"));
    }
    let parsed = ledgeline_core::parse_journal(&text, "marker.journal")
        .map_err(|e| BooksError::Refused(format!("invoice {}: {e}", booking.number)))?;
    parsed.transactions.into_iter().next().ok_or_else(|| {
        BooksError::Refused(format!(
            "invoice {}'s marker didn't read back",
            booking.number
        ))
    })
}

/// Whether [`InvoiceBooks::rebook`] writes, or only plans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Write the journal.
    Write,
    /// Make every check and Ledgeline's save plan, writing nothing
    /// ([`InvoiceBooks::check_rebook`]).
    DryRun,
}

/// Refused unless `document` is sent: only a sent invoice is re-booked.
fn sent_for_rebook(document: &Document) -> Result<(), BooksError> {
    match document.invoice.header().status {
        Some(Status::Sent) => Ok(()),
        Some(Status::Draft) => Err(status_error(document, "a draft isn't booked; send it")),
        _ => Err(status_error(document, "only a sent invoice is re-booked")),
    }
}

impl InvoiceBooks {
    /// What sending `booking` writes in `loaded`'s basis (only called when
    /// [`books_it`]).
    fn plan_for(
        &self,
        index: &DocumentIndex,
        loaded: &Loaded,
        booking: &InvoiceBooking,
        basis: Basis,
    ) -> Result<Plan, BooksError> {
        Ok(match basis {
            Basis::Accrual => {
                let receivable = receivable_account(index.settings(), loaded.journal())?;
                Plan::Document(booking.document_spec(loaded.journal(), &receivable))
            }
            Basis::Cash => Plan::Marker(cash_marker(
                loaded,
                booking,
                index.settings().cash_deposits,
            )?),
        })
    }

    /// What invoice `number` books, and what it has booked.
    ///
    /// # Errors
    /// As [`InvoiceBooks::document`] and [`InvoiceBooks::booking`];
    /// [`BooksError::Setup`] for accrual books with no receivable account.
    pub fn preview(&self, index: &DocumentIndex, number: &str) -> Result<Preview, BooksError> {
        let document = self.document(index, number)?;
        self.preview_of(index, &document)
    }

    /// [`preview`](Self::preview) of `document` as given: the editor's
    /// unsaved text (invoicer PLAN 4a), read nowhere, so the Books panel
    /// shows what saving and sending would book. Writes nothing.
    ///
    /// # Errors
    /// As [`InvoiceBooks::preview`].
    pub fn preview_of(
        &self,
        index: &DocumentIndex,
        document: &Document,
    ) -> Result<Preview, BooksError> {
        let number = document.number();
        let loaded = self.load()?;
        let booking = self.booking(index, document, &loaded)?;
        let basis = Basis::of(&loaded);
        let mut preview = Preview {
            number: number.to_string(),
            path: document.path.clone(),
            status: document.status_text(),
            basis,
            booking,
            receivable: None,
            would_book: None,
            booked: None,
            in_step: None,
            linked: false,
            generation: loaded.generation(),
        };
        if books_it(basis, &preview.booking) {
            let plan = self.plan_for(index, &loaded, &preview.booking, basis)?;
            let would = plan.transaction(&loaded)?;
            if let Some(found) = booked(&loaded, number)? {
                preview.in_step = Some(shape(&found.transaction) == shape(&would));
                preview.linked = found.linked();
                preview.booked = BookedTransaction::of(&loaded, found.tindex);
            }
            preview.would_book = Some(format_transaction(&would));
            if let Plan::Document(spec) = plan {
                preview.receivable = Some(spec.settlement_account);
            }
        }
        Ok(preview)
    }

    /// **Send** (§4.1): book the invoice, then set `status: sent`,
    /// `sent_date` (when it has none), `sent_via` (when given) and `paid`
    /// (when it has none): `no`, or for a zero total the invoice date.
    ///
    /// - Accrual books book every invoice; a zero total (its deposit covers
    ///   it) with no receivable posting, the rows against the deposit
    ///   liability. Cash books book only a zero total, as a `$0` marker.
    /// - A draft already booked under its number, exactly as its file says,
    ///   is an earlier send whose markdown write failed: only the file is
    ///   written. Booked differently, it is refused.
    /// - A file already `status: sent` but not booked (written by a script,
    ///   say) is booked, and its file only gains what it lacks.
    /// - Refused for an invoice dated on or before `closed-through`.
    ///
    /// # Errors
    /// [`BooksError::Status`] for an invoice already sent (and booked) or
    /// void; [`BooksError::Refused`]; as [`InvoiceBooks::booking`];
    /// [`BooksError::Ops`] from the journal write (nothing written);
    /// [`BooksError::MarkdownNotWritten`] (the journal was written).
    pub fn send(&self, index: &mut DocumentIndex, request: &Send) -> Result<Sent, BooksError> {
        check_date("today", &request.today)?;
        let document = self.document(index, &request.number)?;
        let header = document.invoice.header().clone();
        let already_sent = match header.status {
            Some(Status::Draft) => false,
            Some(Status::Sent) => true,
            Some(Status::Void) => {
                return Err(status_error(
                    &document,
                    "a void invoice can't be sent; copy it to a new number",
                ));
            }
            _ => return Err(status_error(&document, "only a draft invoice can be sent")),
        };
        // A vendor portal that rejects invoices without a PO rejects this
        // one: refused before anything is booked (ARCHITECTURE §3.4).
        if header
            .reference
            .as_deref()
            .is_none_or(|r| r.trim().is_empty())
            && let Ok(customer) = self.customer(index, &document)
            && customer
                .billing_portal
                .as_ref()
                .is_some_and(|p| p.requires_reference)
        {
            return Err(BooksError::Refused(format!(
                "{}'s portal rejects invoices without a `reference`; add the customer's PO as \
                 `reference` first",
                customer.contact.title
            )));
        }
        let mut loaded = self.load()?;
        check_generation(&loaded, request.expected.as_ref())?;
        let booking = self.booking(index, &document, &loaded)?;
        let basis = Basis::of(&loaded);
        let mut journal_written = false;
        let mut booked_now = None;
        if books_it(basis, &booking) {
            let plan = self.plan_for(index, &loaded, &booking, basis)?;
            match booked(&loaded, &request.number)? {
                Some(_) if already_sent => {
                    return Err(status_error(
                        &document,
                        "it is already sent and booked; re-book it after an edit",
                    ));
                }
                Some(found) => {
                    if shape(&found.transaction) != shape(&plan.transaction(&loaded)?) {
                        return Err(BooksError::Refused(format!(
                            "invoice {} is already booked ({}), and not as its file says; mark \
                             it `status: sent` and re-book it, or remove that transaction",
                            request.number, found.transaction.date
                        )));
                    }
                    booked_now = BookedTransaction::of(&loaded, found.tindex);
                }
                None => {
                    check_open(&loaded, &request.number, &booking.date)?;
                    match &plan {
                        Plan::Document(spec) if booking.total.is_zero() => {
                            let mut ids = loaded.id_generator();
                            let built = build_settled_document(spec, &mut || ids.next_id())
                                .map_err(OpsError::from)?;
                            loaded.save(&SaveRequest {
                                source: SourceChange::Add(Box::new(built.transaction)),
                                schedules: built.schedules,
                                today: request.today.clone(),
                                create_file: true,
                                expected: None,
                            })?;
                        }
                        Plan::Document(spec) => {
                            loaded.write_document(spec, &request.today, true, None)?;
                        }
                        Plan::Marker(marker) => {
                            loaded.save(&SaveRequest {
                                source: SourceChange::Add(Box::new(marker.clone())),
                                schedules: Vec::new(),
                                today: request.today.clone(),
                                create_file: false,
                                expected: None,
                            })?;
                        }
                    }
                    journal_written = true;
                    booked_now = booked_text(&loaded, &request.number);
                }
            }
        } else if already_sent {
            return Err(status_error(&document, "it is already sent"));
        }
        let fields = document.invoice.frontmatter();
        let mut edits = Vec::new();
        if !already_sent {
            edits.push(set("status", Status::Sent.as_str()));
        }
        if fields.get("sent_date").is_none_or(Value::is_null) {
            edits.push(set("sent_date", &request.today));
        }
        if let Some(via) = request
            .sent_via
            .as_deref()
            .filter(|via| header.sent_via.as_deref() != Some(via))
        {
            edits.push(set("sent_via", via));
        }
        if fields.get("paid").is_none_or(Value::is_null) {
            // Nothing is owed on a zero total: paid on its date (what the
            // reconciler will say too).
            let paid = if booking.total.is_zero() {
                booking.date.as_str()
            } else {
                "no"
            };
            edits.push(set("paid", paid));
        }
        let markdown_written = !edits.is_empty();
        if markdown_written {
            if journal_written {
                self.finish_fields(index, &document, &edits)?;
            } else {
                self.write_fields(index, &document, &edits)?;
            }
        }
        Ok(Sent {
            number: request.number.clone(),
            path: document.path,
            basis,
            booked: booked_now,
            journal_written,
            markdown_written,
        })
    }

    /// Whether [`InvoiceBooks::rebook`] would refuse `number` once its file
    /// says `text`, checked without writing anything: so a caller about to
    /// rewrite a sent invoice's markdown (the app's editor and Recalculate)
    /// can refuse before the file changes, not after (RC-7).
    ///
    /// `text` is evaluated as the file it will become: the same booking,
    /// plan and Ledgeline save rebook would make, run in memory. So a
    /// dropped schedule, a zero total with payments linked, a row with no
    /// account, or a restatement Ledgeline can't make refuses here too.
    /// Beyond rebook, the invoice must be sent now (on disk), and neither
    /// its booked date nor the date `text` gives it may be on or before
    /// `closed-through` even when the books would not change (I8: void and
    /// reissue instead).
    ///
    /// What it can't foresee is the journal or the file changing between
    /// this check and the writes.
    ///
    /// # Errors
    /// As [`InvoiceBooks::rebook`].
    pub fn check_rebook(
        &self,
        index: &DocumentIndex,
        number: &str,
        text: &str,
        today: &str,
    ) -> Result<(), BooksError> {
        check_date("today", today)?;
        let on_disk = self.document(index, number)?;
        sent_for_rebook(&on_disk)?;
        let document = Document {
            invoice: Invoice::parse(text, &index.settings().words),
            ..on_disk
        };
        let loaded = self.load()?;
        if let Some(found) = booked(&loaded, number)? {
            check_open(&loaded, number, &found.transaction.date)?;
            if let Some(date) = document.invoice.header().date.as_deref() {
                check_open(&loaded, number, date)?;
            }
        }
        let request = Rebook {
            number: number.to_string(),
            today: today.to_string(),
            expected: None,
        };
        self.rebook_of(index, &document, loaded, &request, Mode::DryRun)
            .map(drop)
    }

    /// **Re-book** (§4.5): replace a sent invoice's transaction with what
    /// its file now says, restating its schedules, keeping its payment link.
    /// Nothing is written when the books already agree. Refused when the
    /// booked or the new date is on or before `closed-through`: void and
    /// reissue instead.
    ///
    /// An invoice may move between a positive and a zero total (a deposit
    /// row added or removed); a receivable with payments linked is never
    /// dropped. Cash books book only a zero total, so there is otherwise
    /// nothing to re-book; payments already applied keep the split they
    /// were applied with (a warning says so).
    ///
    /// # Errors
    /// [`BooksError::Status`] for a draft or void invoice;
    /// [`BooksError::Refused`] for one sent but never booked (send books
    /// it); as [`InvoiceBooks::booking`]; [`BooksError::Ops`] (nothing
    /// written).
    pub fn rebook(&self, index: &DocumentIndex, request: &Rebook) -> Result<Rebooked, BooksError> {
        check_date("today", &request.today)?;
        let document = self.document(index, &request.number)?;
        let loaded = self.load()?;
        self.rebook_of(index, &document, loaded, request, Mode::Write)
    }

    /// [`rebook`](Self::rebook) of `document` as given, over `loaded`; in
    /// [`Mode::DryRun`] every check and Ledgeline's save plan run, and
    /// nothing is written.
    fn rebook_of(
        &self,
        index: &DocumentIndex,
        document: &Document,
        mut loaded: Loaded,
        request: &Rebook,
        mode: Mode,
    ) -> Result<Rebooked, BooksError> {
        sent_for_rebook(document)?;
        check_generation(&loaded, request.expected.as_ref())?;
        let booking = self.booking(index, document, &loaded)?;
        let basis = Basis::of(&loaded);
        let mut rebooked = Rebooked {
            number: request.number.clone(),
            path: document.path.clone(),
            basis,
            changed: false,
            previous: None,
            booked: None,
            warnings: Vec::new(),
        };
        let found = booked(&loaded, &request.number)?;
        if basis == Basis::Cash && !applied_payments(&loaded, &request.number).is_empty() {
            rebooked.warnings.push(format!(
                "payments already applied to invoice {} keep the split they were applied with; \
                 undo and apply them again to re-split",
                request.number
            ));
        }
        if basis == Basis::Cash && found.is_none() && !booking.total.is_zero() {
            return Ok(rebooked);
        }
        let Some(found) = found else {
            return Err(BooksError::Refused(format!(
                "invoice {} is sent but not booked; send books it",
                request.number
            )));
        };
        let plan = if books_it(basis, &booking) {
            self.plan_for(index, &loaded, &booking, basis)?
        } else {
            return Err(BooksError::Refused(format!(
                "invoice {} is booked with a zero total, and its total isn't zero any more; in \
                 cash books, void it and reissue it",
                request.number
            )));
        };
        rebooked.previous = BookedTransaction::of(&loaded, found.tindex);
        if shape(&found.transaction) == shape(&plan.transaction(&loaded)?) {
            rebooked.booked = rebooked.previous.clone();
            return Ok(rebooked);
        }
        if found.linked() {
            if booking.total.is_zero() {
                return Err(BooksError::Refused(format!(
                    "invoice {} has payments linked to its receivable; a zero total would drop \
                     it — unlink them first",
                    request.number
                )));
            }
            rebooked.warnings.push(format!(
                "invoice {} has payments linked; the link is kept, and if the total changed it \
                 shows as one to check",
                request.number
            ));
        }
        let save = match &plan {
            Plan::Document(spec) if found.settlement.is_some() && !booking.total.is_zero() => {
                match mode {
                    Mode::DryRun => {
                        loaded.preview_rebook_document(spec, &request.today, true, None)?;
                        rebooked.changed = true;
                        return Ok(rebooked);
                    }
                    Mode::Write => {
                        loaded.rebook_document(spec, &request.today, true, None)?;
                    }
                }
                None
            }
            Plan::Document(spec) => {
                // A zero total on either side: replace it here, as
                // Ledgeline's re-book would, keeping the schedule ids.
                check_open(&loaded, &request.number, &found.transaction.date)?;
                check_open(&loaded, &request.number, &booking.date)?;
                let old_ids = schedule_ids(&found.transaction);
                let mut reused = old_ids.clone().into_iter();
                let mut fresh = loaded.id_generator();
                let built = build_settled_document(spec, &mut || {
                    reused.next().unwrap_or_else(|| fresh.next_id())
                })
                .map_err(OpsError::from)?;
                if let Some(dropped) = old_ids
                    .iter()
                    .find(|id| !built.schedules.iter().any(|s| &s.id == *id))
                {
                    return Err(BooksError::Refused(format!(
                        "re-booking invoice {} would drop its schedule {dropped}; end that \
                         contract first",
                        request.number
                    )));
                }
                Some(SaveRequest {
                    source: SourceChange::Replace {
                        tindex: found.tindex,
                        fingerprint: found.fingerprint,
                        transaction: Box::new(built.transaction),
                    },
                    schedules: built.schedules,
                    today: request.today.clone(),
                    create_file: true,
                    expected: None,
                })
            }
            Plan::Marker(marker) => {
                check_open(&loaded, &request.number, &found.transaction.date)?;
                check_open(&loaded, &request.number, &booking.date)?;
                Some(SaveRequest {
                    source: SourceChange::Replace {
                        tindex: found.tindex,
                        fingerprint: found.fingerprint,
                        transaction: Box::new(marker.clone()),
                    },
                    schedules: Vec::new(),
                    today: request.today.clone(),
                    create_file: false,
                    expected: None,
                })
            }
        };
        rebooked.changed = true;
        if let Some(save) = save {
            match mode {
                Mode::DryRun => {
                    loaded.preview(&save)?;
                    return Ok(rebooked);
                }
                Mode::Write => {
                    loaded.save(&save)?;
                }
            }
        }
        rebooked.booked = booked_text(&loaded, &request.number);
        Ok(rebooked)
    }

    /// **Void** (§4.1): a reversing transaction dated `date` — linked to the
    /// invoice's receivable with `match:` when it has one — then
    /// `status: void`, `void_date`, and no `paid:` (a void invoice has no
    /// payment mirror). A draft never booked and an unbooked cash-books
    /// invoice with no payment applied change in the markdown only; a
    /// booked draft (a send that stopped after the journal) is reversed as
    /// a sent invoice is. A void already booked for the number (an earlier
    /// void whose markdown write failed) is not booked again.
    ///
    /// # Errors
    /// [`BooksError::Status`] for an invoice already void;
    /// [`BooksError::Refused`] for a date after today, a cash-books invoice
    /// with payments applied, or a booking with deferred lines;
    /// [`BooksError::Ops`] when Ledgeline refuses the void (payments linked,
    /// deferred lines, a closed period) — nothing written;
    /// [`BooksError::MarkdownNotWritten`].
    pub fn void(&self, index: &mut DocumentIndex, request: &Void) -> Result<Voided, BooksError> {
        check_date("today", &request.today)?;
        check_date("the void date", &request.date)?;
        if request.date > request.today {
            return Err(BooksError::Refused(format!(
                "a void can't be dated {}, after today ({})",
                request.date, request.today
            )));
        }
        let document = self.document(index, &request.number)?;
        let status = document.invoice.header().status;
        if !matches!(status, Some(Status::Draft | Status::Sent)) {
            return Err(status_error(
                &document,
                "only a draft or sent invoice is voided",
            ));
        }
        let mut loaded = self.load()?;
        check_generation(&loaded, request.expected.as_ref())?;
        let basis = Basis::of(&loaded);
        let mut voided = Voided {
            number: request.number.clone(),
            path: document.path.clone(),
            basis,
            reversal: None,
            link: None,
            journal_written: false,
            markdown_written: false,
        };
        // A draft can be booked too: a send that wrote the journal and
        // stopped before the markdown.
        let booked_draft = status == Some(Status::Draft)
            && (void_reversal(&loaded, &request.number).is_some()
                || booked(&loaded, &request.number)?.is_some());
        if status == Some(Status::Sent) || booked_draft {
            if basis == Basis::Cash && !applied_payments(&loaded, &request.number).is_empty() {
                return Err(BooksError::Refused(format!(
                    "invoice {} has payments applied; undo them before voiding it (or issue a \
                     credit note)",
                    request.number
                )));
            }
            if let Some(at) = void_reversal(&loaded, &request.number) {
                voided.reversal = BookedTransaction::of(&loaded, at);
            } else if let Some(found) = booked(&loaded, &request.number)? {
                if found.settlement.is_some() {
                    let written = loaded.void_document(
                        AccrualRole::Receivable,
                        &request.number,
                        &request.date,
                        None,
                    )?;
                    voided.link = Some(written.link);
                } else {
                    // A zero-total booking: reversed here, as Ledgeline
                    // reverses a document, less the link.
                    if !schedule_ids(&found.transaction).is_empty() {
                        return Err(BooksError::Refused(format!(
                            "invoice {} has deferred lines; end their contracts before voiding it",
                            request.number
                        )));
                    }
                    if request.date < found.transaction.date {
                        return Err(BooksError::Refused(format!(
                            "a void can't be dated {}, before invoice {} ({})",
                            request.date, request.number, found.transaction.date
                        )));
                    }
                    check_open(&loaded, &request.number, &request.date)?;
                    let reversal =
                        settled_reversal(&found.transaction, &request.number, &request.date)?;
                    loaded.save(&SaveRequest {
                        source: SourceChange::Add(Box::new(reversal)),
                        schedules: Vec::new(),
                        today: request.today.clone(),
                        create_file: false,
                        expected: None,
                    })?;
                }
                voided.journal_written = true;
                voided.reversal = void_reversal(&loaded, &request.number)
                    .and_then(|at| BookedTransaction::of(&loaded, at));
            }
        }
        let edits = [
            set("status", Status::Void.as_str()),
            set("void_date", &request.date),
            FieldEdit::Remove("paid".to_string()),
        ];
        if voided.journal_written {
            self.finish_fields(index, &document, &edits)?;
        } else {
            self.write_fields(index, &document, &edits)?;
        }
        voided.markdown_written = true;
        Ok(voided)
    }
}
