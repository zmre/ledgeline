//! The books bridge (PLAN Phase 2b, ARCHITECTURE §4): what an invoice books,
//! and every operation that writes the journal.
//!
//! - [`spec`]: an invoice document to its booking lines and header — pure.
//! - [`InvoiceBooks::preview`]: what an invoice would book, and what it has
//!   booked (the books panel; the reconciler's comparison).
//! - [`InvoiceBooks::send`], [`InvoiceBooks::rebook`],
//!   [`InvoiceBooks::void`]: the lifecycle (§4.1, §4.5).
//! - [`InvoiceBooks::open_deposits`], [`InvoiceBooks::apply_deposit`]:
//!   deposits (§4.4).
//! - [`InvoiceBooks::payment_candidates`], [`InvoiceBooks::apply_payment`],
//!   [`InvoiceBooks::preview_payment`], [`InvoiceBooks::undo_payment`]: Apply
//!   payment in cash books (I17).
//!
//! # The basis decides what is booked
//!
//! Accrual books (`[books] basis = "accrual"`, or inferred by plan 45's
//! gate) get one transaction per sent invoice: the receivable debited with
//! the total, each line credited. Cash books get nothing at send: the
//! invoice is tracked by its markdown, and its payment — a deposit Ledgeline
//! imported — is re-split into its rows by Apply payment.
//!
//! # Write order and crash safety
//!
//! Every operation that writes both sides writes the **journal first**, then
//! the markdown, each conditional on the revision it was read at (the
//! journal through plan 45's `ExternalChange` guard and an optional
//! [`Generation`]; the markdown through its [`Revision`]). If the markdown
//! write fails, [`BooksError::MarkdownNotWritten`] says the books were
//! written; repeating the same operation finishes it (a booked invoice
//! whose file still says `draft` is recognized and only its file is
//! written), and `check` reports the mismatch meanwhile.
//!
//! Results are plain data with `serde::Serialize`, for the CLI's `--json`.
//! `today` is always a parameter. Plan 26 rules apply: no globals, all IO
//! through a [`Vault`].

pub mod deposits;
mod lifecycle;
pub mod payments;
pub mod recognize;
pub mod spec;
pub mod undo;

pub use deposits::{ApplyDeposit, DepositApplied, OpenDeposit, OpenDeposits, WaitingDeposit};
pub(crate) use lifecycle::receipt_postings;
pub use lifecycle::{
    Basis, BookedTransaction, Preview, Rebook, Rebooked, Send, Sent, StepCheck, Void, Voided,
};
pub use payments::{
    ApplyPayment, PaymentApplied, PaymentCandidate, PaymentOptions, PaymentPreview, PaymentUndo,
};
pub use recognize::{Recognize, Recognized};
pub use spec::{BookLine, InvoiceBooking, SpecContext, booking_of};
pub use undo::{UNDO_DIR, UndoRecord};

use crate::booking::BookingError;
use crate::catalog::{self, Catalog};
use crate::customers::{ContactKind, Customer, CustomerLookup, find_customer};
use crate::doc::Invoice;
use crate::draft::{self, DraftError};
use crate::frontmatter::{self, FieldEdit};
use crate::index::DocumentIndex;
use crate::problem::Problem;
use ledgeline_core::Dec;
use ledgeline_core::accrual::ops::{Books, Generation, Loaded, OpsError};
use ledgeline_core::edit::JournalEditor;
use ledgeline_core::vault::{LocalVault, Revision, Vault, VaultError, VaultPath};
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// The largest invoice file read: far beyond any real one.
const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;

/// Why a books operation wrote nothing (or, for
/// [`BooksError::MarkdownNotWritten`], only the journal).
#[derive(Debug, Error)]
pub enum BooksError {
    /// No document has the number.
    #[error("no invoice is numbered {0}")]
    NotFound(String),
    /// Several files have the number.
    #[error("invoice number {number} is used by {} files ({}); give each its own number", paths.len(), paths.iter().map(VaultPath::as_str).collect::<Vec<_>>().join(", "))]
    Ambiguous {
        /// The number.
        number: String,
        /// The files.
        paths: Vec<VaultPath>,
    },
    /// The document has errors that stop it from being booked.
    #[error("{path} can't be booked: {}", problems.iter().map(|p| p.message.as_str()).collect::<Vec<_>>().join("; "))]
    Invalid {
        /// The file.
        path: VaultPath,
        /// What is wrong.
        problems: Vec<Problem>,
    },
    /// A bill-table row has no account, or can't be split.
    #[error("{path}: {source}")]
    Booking {
        /// The file.
        path: VaultPath,
        /// Why.
        #[source]
        source: BookingError,
    },
    /// The invoice's status doesn't allow the operation.
    #[error("invoice {number} is {status}: {hint}")]
    Status {
        /// The number.
        number: String,
        /// Its status, as written.
        status: String,
        /// What to do instead.
        hint: String,
    },
    /// The books or the books folder aren't set up for this (no receivable
    /// account, an unknown customer).
    #[error("{0}")]
    Setup(String),
    /// The operation would be wrong for these books (a closed period, a
    /// booked invoice that differs from its file, an overpayment); the
    /// message says which and what to do instead.
    #[error("{0}")]
    Refused(String),
    /// Ledgeline refused or failed the journal write; nothing was written
    /// (but see [`OpsError::RulesNotWritten`]).
    #[error(transparent)]
    Ops(#[from] OpsError),
    /// The journal was written, the markdown wasn't. Repeat the operation to
    /// finish it.
    #[error(
        "invoice {number} was written to the books, but {path} was not updated ({reason}); run it \
         again to finish"
    )]
    MarkdownNotWritten {
        /// The number.
        number: String,
        /// The file.
        path: VaultPath,
        /// Why the write failed.
        reason: String,
    },
    /// The markdown couldn't be written (nothing was written to the books).
    #[error(transparent)]
    Draft(#[from] DraftError),
    /// The markdown couldn't be edited (its frontmatter layout).
    #[error(transparent)]
    Frontmatter(#[from] frontmatter::EditError),
    /// A file couldn't be read.
    #[error("{path}: {source}")]
    Storage {
        /// The file.
        path: VaultPath,
        /// Why.
        #[source]
        source: VaultError,
    },
    /// The arithmetic overflowed.
    #[error("the amounts are too large")]
    Overflow,
}

impl BooksError {
    /// A short, stable name for the error, for `--json` output.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not-found",
            Self::Ambiguous { .. } => "ambiguous",
            Self::Invalid { .. } => "invalid",
            Self::Booking { .. } => "booking",
            Self::Status { .. } => "status",
            Self::Setup(_) => "setup",
            Self::Refused(_) => "refused",
            Self::Ops(OpsError::ExternalChange) => "external-change",
            Self::Ops(OpsError::Stale) => "stale",
            Self::Ops(OpsError::Refused(_)) => "refused",
            Self::Ops(_) => "books",
            Self::MarkdownNotWritten { .. } => "markdown-not-written",
            Self::Draft(DraftError::Vault(VaultError::Changed { .. })) => "external-change",
            Self::Draft(_) => "markdown",
            Self::Frontmatter(_) => "frontmatter",
            Self::Storage { .. } => "storage",
            Self::Overflow => "overflow",
        }
    }
}

/// `Dec` as plain text (`8350.00`), for JSON.
pub(crate) fn dec_text<S: serde::Serializer>(value: &Dec, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&ledgeline_core::edit::render_dec(*value, '.'))
}

/// A document as read for a books operation: its text, the revision it was
/// read at, and its customer.
#[derive(Debug, Clone)]
pub struct Document {
    /// The file.
    pub path: VaultPath,
    /// Its text, parsed.
    pub invoice: Invoice,
    /// The revision it was read at: the markdown write is conditional on it.
    pub revision: Revision,
}

impl Document {
    /// `invoice_num`.
    #[must_use]
    pub fn number(&self) -> &str {
        self.invoice
            .header()
            .number
            .as_ref()
            .map_or("", crate::doc::InvoiceNumber::as_str)
    }

    /// `status`, as written.
    #[must_use]
    pub fn status_text(&self) -> String {
        self.invoice
            .frontmatter()
            .str("status")
            .unwrap_or("(no status)")
            .to_string()
    }
}

/// One books folder: its vault (the main journal's folder) and its
/// Ledgeline books.
#[derive(Clone)]
pub struct InvoiceBooks {
    vault: Arc<dyn Vault>,
    books: Books,
    /// A journal the caller already parsed (the CLI's session), used by the
    /// next [`load`](Self::load) only: after that load the files may change
    /// (this process writes them), so later loads read them again.
    warm: Arc<Mutex<Option<JournalEditor>>>,
}

impl std::fmt::Debug for InvoiceBooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvoiceBooks")
            .field("vault", &self.vault)
            .field("books", &self.books)
            .finish_non_exhaustive()
    }
}

/// Run `op`, and once more if the books or the file changed under it
/// ([`BooksError::code`] `external-change`: Ledgeline or an editor saved
/// between the read and the write, and nothing was written). Every
/// operation reads the files afresh, so the second run sees the change.
/// Returns the result and whether it was retried.
///
/// # Errors
/// The second run's error, or the first's when it isn't a change on disk.
pub fn retry_once<T>(
    mut op: impl FnMut() -> Result<T, BooksError>,
) -> (Result<T, BooksError>, bool) {
    match op() {
        Err(error) if error.code() == "external-change" => (op(), true),
        other => (other, false),
    }
}

impl InvoiceBooks {
    /// The books whose main journal is the local file `main_journal`; the
    /// vault is its folder.
    ///
    /// # Errors
    /// [`BooksError::Setup`] when the path has no usable file name.
    pub fn open(main_journal: &Path) -> Result<Self, BooksError> {
        let (vault, main) = LocalVault::for_journal(main_journal)
            .map_err(|e| BooksError::Setup(format!("{}: {e}", main_journal.display())))?;
        Ok(Self::open_in(Arc::new(vault), main))
    }

    /// The books whose main journal is `main` in `vault`.
    #[must_use]
    pub fn open_in(vault: Arc<dyn Vault>, main: VaultPath) -> Self {
        let books = Books::open_in(Arc::clone(&vault), main);
        Self {
            vault,
            books,
            warm: Arc::new(Mutex::new(None)),
        }
    }

    /// These books, whose next [`load`](Self::load) reuses `editor` (a
    /// journal just parsed, as the CLI's session has) instead of reading
    /// the journal again. If the files moved on since, the write is refused
    /// with `ExternalChange` and nothing is written ([`retry_once`] then
    /// reads them afresh).
    #[must_use]
    pub fn with_editor(self, editor: JournalEditor) -> Self {
        if let Ok(mut warm) = self.warm.lock() {
            *warm = Some(editor);
        }
        self
    }

    /// The vault.
    #[must_use]
    pub fn vault(&self) -> &dyn Vault {
        &*self.vault
    }

    /// Ledgeline's books.
    #[must_use]
    pub fn books(&self) -> &Books {
        &self.books
    }

    /// Read the books.
    ///
    /// # Errors
    /// [`BooksError::Ops`].
    pub fn load(&self) -> Result<Loaded, BooksError> {
        let warm = self.warm.lock().ok().and_then(|mut w| w.take());
        Ok(match warm {
            Some(editor) => self.books.load_from(&editor)?,
            None => self.books.load()?,
        })
    }

    /// Read the document at `path` from disk, with its revision.
    ///
    /// # Errors
    /// [`BooksError::Storage`].
    pub fn document_at(
        &self,
        index: &DocumentIndex,
        path: &VaultPath,
    ) -> Result<Document, BooksError> {
        let (bytes, revision) = self
            .vault
            .read_pinned(path, Some(MAX_DOCUMENT_BYTES))
            .map_err(|source| BooksError::Storage {
                path: path.clone(),
                source,
            })?;
        let text = String::from_utf8(bytes).map_err(|_| BooksError::Storage {
            path: path.clone(),
            source: VaultError::from_io(
                path,
                std::io::Error::new(std::io::ErrorKind::InvalidData, "not UTF-8"),
            ),
        })?;
        Ok(Document {
            path: path.clone(),
            invoice: Invoice::parse(text, &index.settings().words),
            revision,
        })
    }

    /// Read the document numbered `number` from disk (not from the index,
    /// which may be behind), with its revision.
    ///
    /// # Errors
    /// [`BooksError::NotFound`], [`BooksError::Ambiguous`],
    /// [`BooksError::Storage`].
    pub fn document(&self, index: &DocumentIndex, number: &str) -> Result<Document, BooksError> {
        let paths: Vec<VaultPath> = index
            .by_number(number)
            .into_iter()
            .map(|d| d.path.clone())
            .collect();
        let path = match paths.as_slice() {
            [] => return Err(BooksError::NotFound(number.to_string())),
            [one] => one.clone(),
            _ => {
                return Err(BooksError::Ambiguous {
                    number: number.to_string(),
                    paths,
                });
            }
        };
        let document = self.document_at(index, &path)?;
        if document.number() != number {
            // Renumbered since the index read it.
            return Err(BooksError::NotFound(number.to_string()));
        }
        Ok(document)
    }

    /// The customer `document` bills: its `customer:` note, or — for a
    /// person at a `company:` who has no `ledger_customer` of their own —
    /// that company's customer note (a person buying for a company bills
    /// the company, in the books too).
    ///
    /// # Errors
    /// [`BooksError::Setup`] when `customer:` is missing or names no
    /// customer.
    pub fn customer(
        &self,
        index: &DocumentIndex,
        document: &Document,
    ) -> Result<Customer, BooksError> {
        customer_of(
            index,
            &document.path,
            document.invoice.header().invoicee.link.as_deref(),
        )
    }

    /// Every catalog in the books folder (unreadable rows skipped: `check`
    /// reports them).
    fn catalogs(&self, index: &DocumentIndex) -> Vec<Catalog> {
        catalog::load(
            &*self.vault,
            index.site(),
            &index.settings().words,
            index.money(),
        )
        .0
    }

    /// What `document` books, derived from its markdown.
    ///
    /// # Errors
    /// As [`booking_of`] and [`InvoiceBooks::customer`].
    pub fn booking(
        &self,
        index: &DocumentIndex,
        document: &Document,
        loaded: &Loaded,
    ) -> Result<InvoiceBooking, BooksError> {
        let customer = self.customer(index, document)?;
        let catalogs = self.catalogs(index);
        booking_of(
            &document.invoice,
            &document.path,
            &SpecContext {
                settings: index.settings(),
                money: index.money(),
                journal: loaded.journal(),
                customer: &customer,
                catalogs: &catalogs,
                accounts: None,
            },
        )
    }

    /// Write `edits` to `document`'s frontmatter, conditional on the
    /// revision it was read at, and re-index it. A table that already
    /// didn't verify doesn't block a field edit ([`draft::save_fields`]).
    pub(crate) fn write_fields(
        &self,
        index: &mut DocumentIndex,
        document: &Document,
        edits: &[FieldEdit],
    ) -> Result<Revision, BooksError> {
        let source = document.invoice.source();
        let text = frontmatter::edit(source, edits)?;
        Ok(draft::save_fields(
            index,
            &*self.vault,
            &document.path,
            source,
            &text,
            &document.revision,
        )?)
    }

    /// [`write_fields`](Self::write_fields) after a journal write: a failure
    /// is [`BooksError::MarkdownNotWritten`].
    fn finish_fields(
        &self,
        index: &mut DocumentIndex,
        document: &Document,
        edits: &[FieldEdit],
    ) -> Result<Revision, BooksError> {
        self.write_fields(index, document, edits)
            .map_err(|error| BooksError::MarkdownNotWritten {
                number: document.number().to_string(),
                path: document.path.clone(),
                reason: error.to_string(),
            })
    }
}

/// The customer the document at `path` bills by its `customer:` link
/// ([`InvoiceBooks::customer`]), from the index alone: the link may come
/// from the file just read or from the index's header.
///
/// # Errors
/// [`BooksError::Setup`] when `link` is missing or names no customer.
pub(crate) fn customer_of(
    index: &DocumentIndex,
    path: &VaultPath,
    link: Option<&str>,
) -> Result<Customer, BooksError> {
    let words = &index.settings().words;
    let site = index.site();
    let link = link.ok_or_else(|| BooksError::Setup(format!("{path}: `customer` is missing")))?;
    let customer = find_customer(site, path, link, words).map_err(|e| {
        BooksError::Setup(match e {
            CustomerLookup::NotFound => format!("{path}: `customer: {link}` names no note"),
            CustomerLookup::NotACustomer(note) => format!(
                "{path}: `customer: {link}` is {note}, which isn't a person or organization \
                 tagged `{}`",
                words.customer_tag
            ),
        })
    })?;
    let own_ledger_name = site
        .note(&customer.contact.path)
        .is_some_and(|note| note.frontmatter.get("ledger_customer").is_some());
    if customer.contact.kind == ContactKind::Person
        && !own_ledger_name
        && let Some(company) = customer.contact.company.as_deref()
        && let Ok(org) = find_customer(site, &customer.contact.path, company, words)
    {
        return Ok(org);
    }
    Ok(customer)
}

/// Refuse unless the books are as `expected` (when given).
fn check_generation(loaded: &Loaded, expected: Option<&Generation>) -> Result<(), BooksError> {
    match expected {
        Some(expected) if *expected != loaded.generation() => Err(OpsError::Stale.into()),
        _ => Ok(()),
    }
}
