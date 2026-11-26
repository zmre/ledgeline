//! The books writes the CLI and the app both run, each one sequence so the
//! two can't drift (RA-5): a write to the books for one invoice, then the
//! books read again, then its `paid:` mirror refreshed.
//!
//! - [`rebook`]: `rebook` (the CLI), Re-book, and the re-book after the
//!   app's save or Recalculate of a sent invoice.
//! - [`void`]: `void` and the app's Void.
//! - [`settle`]: what follows any other write that moved an invoice's
//!   payments (Apply payment, its undo).
//!
//! Each write is retried once when the books or the file changed under it
//! ([`retry_once`]); the result says whether it was, for the CLI's notice.

use invoicer_core::books::{BooksError, Rebook, Rebooked, Void, Voided, retry_once};
use invoicer_core::reconcile::{self, MirrorChange, ReconcileOptions, Reconciled};
use invoicer_core::session::{OpenError, Session};

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
