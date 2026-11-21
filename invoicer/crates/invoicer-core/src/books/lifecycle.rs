//! Preview, send, re-book and void (ARCHITECTURE §4.1, §4.5).

use super::spec::{DOCUMENT_TAG, InvoiceBooking, deposit_account, receivable_account};
use super::{BooksError, Document, InvoiceBooks, check_generation};
use crate::doc::Status;
use crate::frontmatter::FieldEdit;
use crate::index::DocumentIndex;
use ledgeline_core::accrual::invoice::{DocumentSpec, build_settled_document};
use ledgeline_core::accrual::ops::{Generation, Loaded, OpsError, SaveRequest, SourceChange};
use ledgeline_core::accrual::{
    AccrualRole, AccrualRoles, INVOICE_TAG, RECOGNITION_TAG, VOID_TAG, is_iso_date,
    is_posted_entry, tag_value,
};
use ledgeline_core::edit::{Fingerprint, format_transaction, render_amount};
use ledgeline_core::matching::{MATCH_TAG, NOMATCH_TAG};
use ledgeline_core::model::{Amount, Tindex, Transaction};
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
    let mut ids = loaded.id_generator();
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
/// covers: no payment will ever arrive to apply, so sending it moves the
/// revenue off the deposit liability).
fn books_it(basis: Basis, booking: &InvoiceBooking) -> bool {
    basis == Basis::Accrual || booking.total.is_zero()
}

impl InvoiceBooks {
    /// The transaction `booking` books in `loaded`'s basis.
    fn spec_for(
        &self,
        index: &DocumentIndex,
        loaded: &Loaded,
        booking: &InvoiceBooking,
        basis: Basis,
    ) -> Result<DocumentSpec, BooksError> {
        Ok(match basis {
            Basis::Accrual => {
                let receivable = receivable_account(index.settings(), loaded.journal())?;
                booking.document_spec(loaded.journal(), &receivable)
            }
            // Cash books: rows into the accounts a payment would have gone
            // to. With a zero total there is no settlement posting, so the
            // settlement account is never written.
            Basis::Cash => booking.cash_document_spec(
                loaded.journal(),
                &deposit_account(index.settings(), loaded.journal()),
            ),
        })
    }

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
        if books_it(basis, &preview.booking) {
            let spec = self.spec_for(index, &loaded, &preview.booking, basis)?;
            let would = built(&loaded, &spec)?;
            if let Some(found) = booked(&loaded, number)? {
                preview.in_step = Some(shape(&found.transaction) == shape(&would));
                preview.linked = found.linked();
                preview.booked = BookedTransaction::of(&loaded, found.tindex);
            }
            preview.would_book = Some(format_transaction(&would));
            if basis == Basis::Accrual {
                preview.receivable = Some(spec.settlement_account);
            }
        }
        Ok(preview)
    }

    /// **Send** (§4.1): book the invoice, then set `status: sent`,
    /// `sent_date` (when it has none), `sent_via` (when given) and `paid: no`
    /// (when it has no `paid`).
    ///
    /// - Accrual books book every invoice; cash books only one with a zero
    ///   total (its deposit covers it). A zero total is booked with no
    ///   receivable posting: the rows against the deposit liability.
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
        let mut booked_now = None;
        if books_it(basis, &booking) {
            let spec = self.spec_for(index, &loaded, &booking, basis)?;
            match booked(&loaded, &request.number)? {
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
                    booked_now = BookedTransaction::of(&loaded, found.tindex);
                }
                None => {
                    check_open(&loaded, &request.number, &booking.date)?;
                    if booking.total.is_zero() {
                        let mut ids = loaded.id_generator();
                        let built = build_settled_document(&spec, &mut || ids.next_id())
                            .map_err(OpsError::from)?;
                        loaded.save(&SaveRequest {
                            source: SourceChange::Add(Box::new(built.transaction)),
                            schedules: built.schedules,
                            today: request.today.clone(),
                            create_file: true,
                            expected: None,
                        })?;
                    } else {
                        loaded.write_document(&spec, &request.today, true, None)?;
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
            booked: booked_now,
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
        let spec = if books_it(basis, &booking) {
            self.spec_for(index, &loaded, &booking, basis)?
        } else {
            return Err(BooksError::Refused(format!(
                "invoice {} is booked with a zero total, and its total isn't zero any more; in \
                 cash books, void it and reissue it",
                request.number
            )));
        };
        rebooked.previous = BookedTransaction::of(&loaded, found.tindex);
        if shape(&found.transaction) == shape(&built(&loaded, &spec)?) {
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
        if found.settlement.is_some() && !booking.total.is_zero() {
            loaded.rebook_document(&spec, &request.today, true, None)?;
        } else {
            // A zero total on either side: replace it here, as Ledgeline's
            // re-book would, keeping the schedule ids.
            check_open(&loaded, &request.number, &found.transaction.date)?;
            check_open(&loaded, &request.number, &booking.date)?;
            let old_ids = schedule_ids(&found.transaction);
            let mut reused = old_ids.clone().into_iter();
            let mut fresh = loaded.id_generator();
            let built = build_settled_document(&spec, &mut || {
                reused.next().unwrap_or_else(|| fresh.next_id())
            })
            .map_err(OpsError::from)?;
            if let Some(dropped) = old_ids
                .iter()
                .find(|id| !built.schedules.iter().any(|s| &s.id == *id))
            {
                return Err(BooksError::Refused(format!(
                    "re-booking invoice {} would drop its schedule {dropped}; end that contract \
                     first",
                    request.number
                )));
            }
            loaded.save(&SaveRequest {
                source: SourceChange::Replace {
                    tindex: found.tindex,
                    fingerprint: found.fingerprint,
                    transaction: Box::new(built.transaction),
                },
                schedules: built.schedules,
                today: request.today.clone(),
                create_file: true,
                expected: None,
            })?;
        }
        rebooked.changed = true;
        rebooked.booked = booked_text(&loaded, &request.number);
        Ok(rebooked)
    }

    /// **Void** (§4.1): a reversing transaction dated `date` — linked to the
    /// invoice's receivable with `match:` when it has one — then
    /// `status: void` and `void_date`. A draft (never booked) and an
    /// unbooked cash-books invoice with no payment applied change in the
    /// markdown only. A void already booked for the number (an earlier void
    /// whose markdown write failed) is not booked again.
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
        if status == Some(Status::Sent) {
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
