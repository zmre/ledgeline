//! **Recognize** (ARCHITECTURE §4.1): post plan 45's due recognition
//! entries through a date, never after today — the same
//! [`Loaded::post_due`](ledgeline_core::accrual::ops::Loaded::post_due)
//! Ledgeline's **Post through ‹date›** calls, so both apps post the same
//! entries the same way, and a repeat posts nothing.

use super::{BooksError, InvoiceBooks, check_generation};
use ledgeline_core::accrual::is_iso_date;
use ledgeline_core::accrual::ops::{Generation, PostDue};
use serde::Serialize;

/// **Recognize** through a date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recognize {
    /// Today: nothing after it is ever posted.
    pub today: String,
    /// Post entries dated on or before this (at most today); `None` is
    /// today.
    pub through: Option<String>,
    /// Also fold the due entries in closed periods into one adjustment per
    /// schedule, dated today (Ledgeline's catch-up).
    pub include_closed: bool,
    /// List what would be posted, write nothing.
    pub dry_run: bool,
    /// Refuse unless the books are still as listed.
    pub expected: Option<Generation>,
}

/// One recognition entry: its schedule and date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecognitionEntry {
    /// The schedule id (`recognition:`).
    pub id: String,
    /// Its date (`recognized:`).
    pub date: String,
}

/// What [`InvoiceBooks::recognize`] posted (or, dry, would post).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recognized {
    /// Today.
    pub today: String,
    /// The last date posted.
    pub through: String,
    /// Whether the journal was written.
    pub written: bool,
    /// The entries posted (or due), oldest first.
    pub entries: Vec<RecognitionEntry>,
    /// The schedules given a closed-period adjustment (with
    /// `include_closed`).
    pub adjustments: Vec<String>,
    /// Due entries in closed periods left unposted (without
    /// `include_closed`): Ledgeline's catch-up posts them as adjustments.
    pub closed: Vec<RecognitionEntry>,
}

impl InvoiceBooks {
    /// **Recognize**: every open due recognition entry dated on or before
    /// `through` (today when not given), in one write. Never after today.
    ///
    /// # Errors
    /// [`BooksError::Refused`] for a date that isn't one or is after today;
    /// [`BooksError::Ops`] (`Stale`, `ExternalChange`: nothing written).
    pub fn recognize(&self, request: &Recognize) -> Result<Recognized, BooksError> {
        let through = request.through.as_deref().unwrap_or(&request.today);
        for (what, date) in [("today", request.today.as_str()), ("--through", through)] {
            if !is_iso_date(date) {
                return Err(BooksError::Refused(format!(
                    "{what} `{date}` is not a date (YYYY-MM-DD)"
                )));
            }
        }
        if through > request.today.as_str() {
            return Err(BooksError::Refused(format!(
                "recognition is never posted after today ({}); {through} is later",
                request.today
            )));
        }
        let mut loaded = self.load()?;
        check_generation(&loaded, request.expected.as_ref())?;
        let due = loaded.due(&request.today);
        let entry = |e: &ledgeline_core::accrual::due::DueEntry| RecognitionEntry {
            id: e.id.clone(),
            date: e.recognized.clone(),
        };
        let closed: Vec<RecognitionEntry> = due
            .closed
            .iter()
            .filter(|e| e.recognized.as_str() <= through)
            .map(entry)
            .collect();
        let mut out = Recognized {
            today: request.today.clone(),
            through: through.to_string(),
            written: false,
            entries: due.through(through).iter().map(entry).collect(),
            adjustments: Vec::new(),
            closed: if request.include_closed {
                Vec::new()
            } else {
                closed.clone()
            },
        };
        if request.dry_run {
            if request.include_closed {
                let mut ids: Vec<String> = closed.into_iter().map(|e| e.id).collect();
                ids.dedup();
                out.adjustments = ids;
            }
            return Ok(out);
        }
        let posted = loaded.post_due(&PostDue {
            today: request.today.clone(),
            through: Some(through.to_string()),
            only: None,
            include_closed: request.include_closed,
            expected: None,
        })?;
        out.written = !posted.entries.is_empty() || !posted.adjustments.is_empty();
        out.entries = posted
            .entries
            .into_iter()
            .map(|(id, date)| RecognitionEntry { id, date })
            .collect();
        out.adjustments = posted.adjustments;
        Ok(out)
    }
}
