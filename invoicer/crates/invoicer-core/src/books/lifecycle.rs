//! Preview, send, re-book and void (ARCHITECTURE §4.1, §4.5).

use super::spec::{InvoiceBooking, receivable_account};
use super::{BooksError, Document, InvoiceBooks, check_generation};
use crate::doc::Status;
use crate::frontmatter::FieldEdit;
use crate::index::DocumentIndex;
use ledgeline_core::accrual::documents::FoundDocument;
use ledgeline_core::accrual::invoice::{DocumentSpec, build_document};
use ledgeline_core::accrual::ops::{Generation, Loaded};
use ledgeline_core::accrual::{
    AccrualRole, INVOICE_TAG, RECOGNITION_TAG, VOID_TAG, is_iso_date, tag_value,
};
use ledgeline_core::edit::format_transaction;
use ledgeline_core::matching::{MATCH_TAG, NOMATCH_TAG};
use ledgeline_core::model::{Tindex, Transaction};
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

/// The transaction `spec` builds (with placeholder schedule ids).
fn built(loaded: &Loaded, spec: &DocumentSpec) -> Result<Transaction, BooksError> {
    let mut ids = loaded.id_generator();
    Ok(build_document(spec, &mut ids)
        .map_err(ledgeline_core::accrual::ops::OpsError::from)?
        .transaction)
}

/// The one booked invoice numbered `number`, if any.
fn booked_document(loaded: &Loaded, number: &str) -> Result<Option<FoundDocument>, BooksError> {
    if loaded
        .documents()
        .get(AccrualRole::Receivable, number)
        .len()
        > 1
    {
        return Err(BooksError::Refused(format!(
            "invoice {number} is booked more than once; remove the extra transaction by hand"
        )));
    }
    Ok(loaded.find_document(number))
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

/// Cash books: the transactions a payment of `number` was applied to.
pub(super) fn applied_payments<'j>(loaded: &'j Loaded, number: &str) -> Vec<&'j Transaction> {
    loaded
        .journal()
        .transactions
        .iter()
        .filter(|txn| {
            tag_value(&txn.tags, INVOICE_TAG) == Some(number)
                && tag_value(&txn.tags, VOID_TAG).is_none()
        })
        .collect()
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

impl InvoiceBooks {
    /// What invoice `number` books, and what it has booked.
    ///
    /// # Errors
    /// As [`InvoiceBooks::document`] and [`InvoiceBooks::booking`];
    /// [`BooksError::Setup`] for accrual books with no receivable account.
    pub fn preview(&self, index: &DocumentIndex, number: &str) -> Result<Preview, BooksError> {
        let document = self.document(index, number)?;
        let loaded = self.load()?;
        let booking = self.booking(index, &document, &loaded)?;
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
        if basis == Basis::Accrual {
            let receivable = receivable_account(index.settings(), loaded.journal())?;
            let spec = preview.booking.document_spec(loaded.journal(), &receivable);
            let would = built(&loaded, &spec)?;
            if let Some(found) = booked_document(&loaded, number)? {
                preview.in_step = Some(shape(&found.transaction) == shape(&would));
                preview.linked = tag_value(
                    &found.transaction.postings[found.settlement.posting].tags,
                    MATCH_TAG,
                )
                .is_some();
                preview.booked = BookedTransaction::of(&loaded, found.tindex);
            }
            preview.would_book = Some(format_transaction(&would));
            preview.receivable = Some(receivable);
        }
        Ok(preview)
    }

    /// **Send** (§4.1): book the invoice — accrual books only — then set
    /// `status: sent`, `sent_date` (when it has none), `sent_via` (when
    /// given) and `paid: no` (when it has no `paid`).
    ///
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
        let mut loaded = self.load()?;
        check_generation(&loaded, request.expected.as_ref())?;
        let booking = self.booking(index, &document, &loaded)?;
        let basis = Basis::of(&loaded);
        let mut journal_written = false;
        let mut booked = None;
        if basis == Basis::Accrual {
            let receivable = receivable_account(index.settings(), loaded.journal())?;
            let spec = booking.document_spec(loaded.journal(), &receivable);
            match booked_document(&loaded, &request.number)? {
                Some(_) if already_sent => {
                    return Err(status_error(
                        &document,
                        "it is already sent and booked; re-book it after an edit",
                    ));
                }
                Some(found) => {
                    if shape(&found.transaction) != shape(&built(&loaded, &spec)?) {
                        return Err(BooksError::Refused(format!(
                            "invoice {} is already booked ({}), and not as its file says; mark \
                             it `status: sent` and re-book it, or remove that transaction",
                            request.number, found.transaction.date
                        )));
                    }
                    booked = BookedTransaction::of(&loaded, found.tindex);
                }
                None => {
                    if let Some(closed) = loaded.config().closed_through.as_deref()
                        && booking.date.as_str() <= closed
                    {
                        return Err(BooksError::Refused(format!(
                            "invoice {} is dated {}, on or before closed-through {closed}; date \
                             it in an open period",
                            request.number, booking.date
                        )));
                    }
                    loaded.write_document(&spec, &request.today, true, None)?;
                    journal_written = true;
                    booked = loaded
                        .find_document(&request.number)
                        .and_then(|found| BookedTransaction::of(&loaded, found.tindex));
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
            edits.push(set("paid", "no"));
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
            booked,
            journal_written,
            markdown_written,
        })
    }

    /// **Re-book** (§4.5): replace a sent invoice's transaction with what
    /// its file now says, restating its schedules, keeping its payment link.
    /// Nothing is written when the books already agree. Refused when the
    /// booked or the new date is on or before `closed-through`: void and
    /// reissue instead.
    ///
    /// Cash books book nothing at send, so there is nothing to re-book;
    /// payments already applied keep the split they were applied with (a
    /// warning says so).
    ///
    /// # Errors
    /// [`BooksError::Status`] for a draft or void invoice;
    /// [`BooksError::Refused`] for one sent but never booked (send books
    /// it); as [`InvoiceBooks::booking`]; [`BooksError::Ops`] (nothing
    /// written).
    pub fn rebook(&self, index: &DocumentIndex, request: &Rebook) -> Result<Rebooked, BooksError> {
        check_date("today", &request.today)?;
        let document = self.document(index, &request.number)?;
        match document.invoice.header().status {
            Some(Status::Sent) => {}
            Some(Status::Draft) => {
                return Err(status_error(&document, "a draft isn't booked; send it"));
            }
            _ => return Err(status_error(&document, "only a sent invoice is re-booked")),
        }
        let mut loaded = self.load()?;
        check_generation(&loaded, request.expected.as_ref())?;
        let booking = self.booking(index, &document, &loaded)?;
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
        if basis == Basis::Cash {
            if !applied_payments(&loaded, &request.number).is_empty() {
                rebooked.warnings.push(format!(
                    "payments already applied to invoice {} keep the split they were applied \
                     with; undo and apply them again to re-split",
                    request.number
                ));
            }
            return Ok(rebooked);
        }
        let receivable = receivable_account(index.settings(), loaded.journal())?;
        let spec = booking.document_spec(loaded.journal(), &receivable);
        let found = booked_document(&loaded, &request.number)?.ok_or_else(|| {
            BooksError::Refused(format!(
                "invoice {} is sent but not booked; send books it",
                request.number
            ))
        })?;
        rebooked.previous = BookedTransaction::of(&loaded, found.tindex);
        if shape(&found.transaction) == shape(&built(&loaded, &spec)?) {
            rebooked.booked = rebooked.previous.clone();
            return Ok(rebooked);
        }
        if tag_value(
            &found.transaction.postings[found.settlement.posting].tags,
            MATCH_TAG,
        )
        .is_some()
        {
            rebooked.warnings.push(format!(
                "invoice {} has payments linked; the link is kept, and if the total changed it \
                 shows as one to check",
                request.number
            ));
        }
        loaded.rebook_document(&spec, &request.today, true, None)?;
        rebooked.changed = true;
        rebooked.booked = loaded
            .find_document(&request.number)
            .and_then(|found| BookedTransaction::of(&loaded, found.tindex));
        Ok(rebooked)
    }

    /// **Void** (§4.1): in accrual books, a reversing transaction dated
    /// `date`, linked to the invoice's receivable with `match:`; then
    /// `status: void` and `void_date`. A draft (never booked) and a cash-books
    /// invoice with no payment applied change in the markdown only. A void
    /// already booked for the number (an earlier void whose markdown write
    /// failed) is not booked again.
    ///
    /// # Errors
    /// [`BooksError::Status`] for an invoice already void;
    /// [`BooksError::Refused`] for a date after today, or a cash-books
    /// invoice with payments applied; [`BooksError::Ops`] when Ledgeline
    /// refuses the void (payments linked, deferred lines, a closed period)
    /// — nothing written; [`BooksError::MarkdownNotWritten`].
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
        if status == Some(Status::Sent) {
            if basis == Basis::Cash {
                if !applied_payments(&loaded, &request.number).is_empty() {
                    return Err(BooksError::Refused(format!(
                        "invoice {} has payments applied; undo them before voiding it (or \
                         issue a credit note)",
                        request.number
                    )));
                }
            } else if let Some(at) = void_reversal(&loaded, &request.number) {
                voided.reversal = BookedTransaction::of(&loaded, at);
            } else if booked_document(&loaded, &request.number)?.is_some() {
                let written = loaded.void_document(
                    AccrualRole::Receivable,
                    &request.number,
                    &request.date,
                    None,
                )?;
                voided.journal_written = true;
                voided.link = Some(written.link);
                voided.reversal = void_reversal(&loaded, &request.number)
                    .and_then(|at| BookedTransaction::of(&loaded, at));
            }
        }
        let edits = [
            set("status", Status::Void.as_str()),
            set("void_date", &request.date),
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
