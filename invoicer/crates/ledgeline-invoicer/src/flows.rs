//! The books writes the CLI and the app both run, each one sequence so the
//! two can't drift (RA-5): a write to the books for one invoice, then the
//! books read again, then its `paid:` mirror refreshed.
//!
//! - [`rebook`]: `rebook` (the CLI), Re-book, and the re-book after the
//!   app's save or Recalculate of a sent invoice.
//! - [`void`]: `void` and the app's Void.
//! - [`settle`]: what follows any other write that moved an invoice's
//!   payments (Apply payment, its undo).
//! - [`update_company`]: Update from company, up to the re-book that
//!   follows it for a sent invoice.
//!
//! Each write is retried once when the books or the file changed under it
//! ([`retry_once`]); the result says whether it was, for the CLI's notice.

use invoicer_core::books::{BooksError, Rebook, Rebooked, Void, Voided, retry_once};
use invoicer_core::company;
use invoicer_core::doc::{Invoice, Status};
use invoicer_core::edit::{self, SetOutcome, WriteError};
use invoicer_core::reconcile::{self, MirrorChange, ReconcileOptions, Reconciled};
use invoicer_core::session::{OpenError, Session};
use ledgeline_core::vault::VaultPath;

/// A books write and the paid mirror it moved.
#[derive(Debug)]
pub(crate) struct Written<T> {
    /// What the write did.
    pub done: T,
    /// The invoice's mirror, refreshed: `None` when nothing was looked at
    /// (the books didn't change, or couldn't be read).
    pub mirror: Option<Reconciled>,
}

impl<T> Written<T> {
    /// The invoice's own mirror change, if any (the CLI prints one).
    pub(crate) fn mirror_change(&self) -> Option<MirrorChange> {
        self.mirror
            .as_ref()
            .and_then(|report| report.changed.first().cloned())
    }
}

/// A flow's result, and whether its write was retried.
pub(crate) type Outcome<T> = (Result<Written<T>, BooksError>, bool);

/// Read the journal again when this process's write changed it, so the
/// next read (the mirror, the next request) sees the write.
///
/// # Errors
/// [`OpenError`] when the journal no longer opens; the session is then as
/// it was.
pub(crate) fn reload(session: &mut Session) -> Result<(), OpenError> {
    if session.changed_journal_files().is_empty() {
        return Ok(());
    }
    session.reload_books().map(drop)
}

/// [`reload`], a failure logged: the write is done either way, and the
/// next request reads the books again.
pub(crate) fn reload_logged(session: &mut Session) {
    if let Err(error) = reload(session) {
        eprintln!("ledgeline-invoicer: reload after a write skipped: {error}");
    }
}

/// After a books write for invoice `number`: the books read again
/// ([`reload`], a failure logged) and its `paid:` mirror refreshed from
/// them. `None` when the books can't be read.
pub(crate) fn settle(session: &mut Session, number: &str) -> Option<Reconciled> {
    reload_logged(session);
    let books = session.invoice_books();
    let loaded = books.load().ok()?;
    let (index, _) = session.parts_mut();
    Some(reconcile::reconcile(
        &books,
        index,
        &loaded,
        &ReconcileOptions {
            only: Some(vec![number.to_string()]),
            dry_run: false,
        },
    ))
}

/// **Re-book** sent invoice `number` from its file, then [`settle`] it
/// when the books changed (a new total can move its payment state).
pub(crate) fn rebook(session: &mut Session, number: &str, today: &str) -> Outcome<Rebooked> {
    let books = session.invoice_books();
    let request = Rebook {
        number: number.to_string(),
        today: today.to_string(),
        expected: None,
    };
    let (index, _) = session.parts_mut();
    let (result, retried) = retry_once(|| books.rebook(index, &request));
    let result = match result {
        Ok(rebooked) => {
            let mirror = if rebooked.changed {
                settle(session, number)
            } else {
                None
            };
            Ok(Written {
                done: rebooked,
                mirror,
            })
        }
        Err(error) => {
            // A failed write may still have changed the journal
            // (`RulesNotWritten`): read it again either way.
            reload_logged(session);
            Err(error)
        }
    };
    (result, retried)
}

/// **Void** `request.number`, then [`settle`] it.
pub(crate) fn void(session: &mut Session, request: &Void) -> Outcome<Voided> {
    let books = session.invoice_books();
    let (index, _) = session.parts_mut();
    let (result, retried) = retry_once(|| books.void(index, request));
    let result = match result {
        Ok(voided) => {
            let mirror = settle(session, &request.number);
            Ok(Written {
                done: voided,
                mirror,
            })
        }
        Err(error) => {
            reload_logged(session);
            Err(error)
        }
    };
    (result, retried)
}

/// Why [`update_company`] wrote nothing.
#[derive(Debug)]
pub(crate) enum UpdateRefused {
    /// Void, or no company to copy: a sentence saying so.
    Refused(String),
    /// The file can't be read, edited safely or written.
    Write(WriteError),
    /// A sent invoice the books refuse to re-book (a closed period, …).
    Books(BooksError),
}

/// **Update from company** (`update-company`, the invoice page's action):
/// write the company note's current details into the document at `path`
/// ([`company::update`]). A void invoice is history and refused; with no
/// company nothing is written. A sent invoice follows its other edits: the
/// text is checked as re-booking would book it before the file changes
/// ([`InvoiceBooks::check_rebook`](invoicer_core::books::InvoiceBooks::check_rebook)),
/// and the caller re-books it after ([`rebook`]) when this answers its
/// number.
pub(crate) fn update_company(
    session: &mut Session,
    path: &VaultPath,
    today: &str,
) -> Result<(SetOutcome, Option<String>), UpdateRefused> {
    let (source, revision) =
        edit::read_text(session.vault(), path).map_err(UpdateRefused::Write)?;
    let invoice = Invoice::parse(source.as_str(), &session.settings().words);
    let header = invoice.header();
    let label = header
        .number
        .as_ref()
        .map_or_else(|| path.to_string(), |n| format!("invoice {n}"));
    if header.status == Some(Status::Void) {
        return Err(UpdateRefused::Refused(format!(
            "{label} is void: it keeps the details it was sent with"
        )));
    }
    let index = session.index();
    if company::fields(index.site(), index.settings(), session.vault()).is_none() {
        return Err(UpdateRefused::Refused(
            "there is no company to copy: set `[invoicer] company` to your company's note"
                .to_string(),
        ));
    }
    let rebook = match (header.status, &header.number) {
        (Some(Status::Sent), Some(number)) => Some(number.to_string()),
        _ => None,
    };
    if let Some(number) = &rebook
        && let Some((text, _)) =
            company::current_text(index, session.vault(), &source).map_err(UpdateRefused::Write)?
    {
        session
            .invoice_books()
            .check_rebook(index, number, &text, today)
            .map_err(UpdateRefused::Books)?;
    }
    let (index, vault) = session.parts_mut();
    let outcome =
        company::update(index, vault, path, &source, &revision).map_err(UpdateRefused::Write)?;
    let rebook = rebook.filter(|_| outcome.written);
    Ok((outcome, rebook))
}
