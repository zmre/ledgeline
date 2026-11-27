//! Time tracking in the Invoicer (plan 46 Phase 2): a thin layer over
//! [`ledgeline_core::time`] that joins the time book to what the Invoicer
//! knows — customers, the catalog, invoices, timekeeper notes — and adds the
//! operations the app and the CLI share.
//!
//! - [`TimeBooks`]: one books folder's time book (its `[time]` settings and
//!   paths), read and written with [`TimeBooks::write`], which re-reads and
//!   re-applies once when a file changed on disk (T-running "Two machines").
//! - [`link`]: time accounts ↔ customers (`customer:` ↔ `ledger_customer`),
//!   the customer note's time keys, and timekeeper notes (T18).
//! - [`track`]: the slot tracker (T8, T-slots, T-running): state, toggle,
//!   stop, adjust, snap, note.
//! - [`entries`]: Entries (T-entries): list, add, edit, delete, bulk.
//! - [`accounts`]: the Accounts page (T-accounts).
//! - [`report`]: summaries, unbilled and unpaid time (T-dash, B-unbilled).
//! - [`round`]: billing rounding (B-round).
//! - [`bill`]: time → an invoice draft, and releasing it (B-invoice).
//! - [`pay`]: contractor time → an accounts-payable bill (B-ap).
//!
//! Plan 26 rules apply: no clock (every `now` and `today` is a parameter),
//! no globals, all IO through a [`Vault`].

pub mod accounts;
pub mod bill;
pub mod entries;
pub mod link;
pub mod pay;
pub mod report;
pub mod round;
pub mod track;

use crate::books::BooksError;
use crate::draft::DraftError;
use crate::index::DocumentIndex;
use ledgeline_core::reports::periods::iso_from_days;
use ledgeline_core::time::book::{self, Coverage, TimeConfig, TimePaths};
use ledgeline_core::time::files::{TimeFiles, Tracker};
use ledgeline_core::time::model::{Origin, TimeBook, TimeEntry};
use ledgeline_core::time::timeclock::Moment;
use ledgeline_core::time::{TagRole, TimeError, TimeFormat, TimeQty, note_problem};
use ledgeline_core::vault::{Vault, VaultPath};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;

/// The tag a person or organization note carries to be a timekeeper: whose
/// time a card is, and who is paid for it (T18).
pub const TIMEKEEPER_TAG: &str = "timekeeper";

/// Why a time operation wrote nothing.
#[derive(Debug, Error)]
pub enum TimeOpError {
    /// There is no roll-up yet: setup hasn't run.
    #[error("time tracking isn't set up for these books yet (no {0})")]
    NotSetUp(String),
    /// The engine refused.
    #[error(transparent)]
    Time(#[from] TimeError),
    /// A note hledger would read differently (T-slots "Note"): nothing is
    /// rewritten silently; the suggestion is offered instead.
    #[error("{message}")]
    Note {
        /// The text that can't stay: `re:`, `;`.
        offending: String,
        /// Why.
        message: String,
        /// The note reworded so it can be written.
        suggestion: Option<String>,
    },
    /// The line is on a sent invoice or a booked bill (T-entries).
    #[error("{0}")]
    Locked(String),
    /// No such entry, account, customer or person.
    #[error("{0}")]
    NotFound(String),
    /// The request doesn't read.
    #[error("{0}")]
    BadRequest(String),
    /// The request reads, but the time isn't in a state for it.
    #[error("{0}")]
    Conflict(String),
    /// The books refused (an AP bill).
    #[error(transparent)]
    Books(#[from] BooksError),
    /// The draft couldn't be made.
    #[error(transparent)]
    Draft(#[from] DraftError),
    /// A note or settings file couldn't be read or written.
    #[error("{0}")]
    Storage(String),
}

impl TimeOpError {
    /// The stable code the API and `--json` answer with.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotSetUp(_) => "not-set-up",
            Self::Time(error) => match error {
                TimeError::Path { .. } | TimeError::Parse(_) | TimeError::Invalid(_) => "invalid",
                TimeError::Settings(_) => "settings",
                TimeError::Exists(_) => "exists",
                TimeError::Missing(_) | TimeError::NotFound(_) => "not-found",
                TimeError::Verify(_) => "verify",
                TimeError::ExternalChange(_) => "external-change",
                TimeError::Storage { .. } => "storage",
            },
            Self::Note { .. } => "note-tag",
            Self::Locked(_) => "locked",
            Self::NotFound(_) => "not-found",
            Self::BadRequest(_) => "bad-request",
            Self::Conflict(_) => "conflict",
            Self::Books(error) => error.code(),
            Self::Draft(DraftError::Exists(_) | DraftError::DuplicateNumber { .. }) => "exists",
            Self::Draft(DraftError::UnknownCustomer(_) | DraftError::NotACustomer(_)) => {
                "not-found"
            }
            Self::Draft(_) => "invalid",
            Self::Storage(_) => "storage",
        }
    }
}

/// Refuse `note` when hledger would read it differently ([`note_problem`]),
/// naming the offending text and suggesting a rewrite.
///
/// # Errors
/// [`TimeOpError::Note`].
pub fn check_note(note: &str) -> Result<(), TimeOpError> {
    match note_problem(note) {
        None => Ok(()),
        Some(problem) => Err(TimeOpError::Note {
            offending: problem.offending,
            message: problem.message,
            suggestion: problem.suggestion,
        }),
    }
}

/// The tracker's preferences (`ui-prefs.toml [time]`, plan 46 Settings), as
/// the effective preferences carry them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimePrefs {
    /// Who is tracking (P-person).
    pub person: Option<String>,
    /// Slots 1–5, `""` for an empty pin.
    pub pinned: Vec<String>,
    /// Slot 0: the last account chosen through *Other…*.
    pub other: Option<String>,
    /// The `+`/`−` step, in minutes.
    pub step: i64,
    /// The long-running guard, in minutes (0 is off).
    pub long_running: i64,
    /// The week strip's target, in hours (0 is none).
    pub weekly_target: i64,
}

impl Default for TimePrefs {
    fn default() -> Self {
        Self {
            person: None,
            pinned: Vec::new(),
            other: None,
            step: 5,
            long_running: 240,
            weekly_target: 0,
        }
    }
}

impl TimePrefs {
    /// Read from the effective preferences' wire object (`{"time": {...}}`).
    #[must_use]
    pub fn from_prefs(prefs: &Value) -> Self {
        let defaults = Self::default();
        let time = &prefs["time"];
        let text = |key: &str| {
            time[key]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let whole = |key: &str, default: i64| time[key].as_i64().unwrap_or(default);
        Self {
            person: text("person"),
            pinned: time["pinned"]
                .as_array()
                .map(|names| {
                    names
                        .iter()
                        .map(|n| n.as_str().unwrap_or("").trim().to_string())
                        .collect()
                })
                .unwrap_or_default(),
            other: text("other"),
            step: whole("step", defaults.step),
            long_running: whole("long-running", defaults.long_running),
            weekly_target: whole("weekly-target", defaults.weekly_target),
        }
    }
}

/// `seconds` since 1970-01-01 00:00 as a [`Moment`].
#[must_use]
pub fn moment_from_epoch(seconds: i64) -> Moment {
    Moment::new(
        iso_from_days(seconds.div_euclid(86_400)),
        u32::try_from(seconds.rem_euclid(86_400)).unwrap_or(0),
    )
}

/// `YYYY-MM-DDTHH:MM[:SS]` (or with a space) as a [`Moment`].
#[must_use]
pub fn parse_moment(text: &str) -> Option<Moment> {
    let text = text.trim();
    let (date, time) = text.split_once(['T', ' '])?;
    let seconds = parse_time_of_day(time)?;
    crate::table::books::is_iso_date(date).then(|| Moment::new(date, seconds))
}

/// `HH:MM` or `HH:MM:SS` as seconds after midnight.
#[must_use]
pub fn parse_time_of_day(text: &str) -> Option<u32> {
    let mut parts = text.trim().split(':');
    let h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let s: u32 = parts.next().map_or(Some(0), |s| s.parse().ok())?;
    (parts.next().is_none() && h < 24 && m < 60 && s < 60).then_some(h * 3600 + m * 60 + s)
}

/// A duration as people type it (T-entries "Add"): `45m`, `2h`, `1h30`,
/// `1h 30m`, `1.5` (hours), `0:45`, `90s`. Never negative.
///
/// # Errors
/// [`TimeOpError::BadRequest`], saying what was typed.
pub fn parse_duration(text: &str) -> Result<TimeQty, TimeOpError> {
    let bad = || {
        TimeOpError::BadRequest(format!(
            "`{text}` isn't a duration: write 45m, 1h30, 1.5 or 0:45"
        ))
    };
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Err(bad());
    }
    if let Some((h, m)) = compact.split_once(':') {
        let hours: i64 = h.parse().map_err(|_| bad())?;
        let minutes: i64 = m.parse().map_err(|_| bad())?;
        if hours < 0 || !(0..60).contains(&minutes) {
            return Err(bad());
        }
        return Ok(TimeQty::from_minutes(hours * 60 + minutes));
    }
    let number = |s: &str, unit_nanos: i128| -> Result<TimeQty, TimeOpError> {
        let value = ledgeline_core::Dec::parse(s, '.').map_err(|_| bad())?;
        if value.mantissa < 0 {
            return Err(bad());
        }
        TimeQty::from_dec(value, unit_nanos).ok_or_else(bad)
    };
    const MINUTE: i128 = 60_000_000_000;
    const HOUR: i128 = 60 * MINUTE;
    if let Some(rest) = compact.strip_suffix('s') {
        return number(rest, MINUTE / 60);
    }
    if let Some(rest) = compact.strip_suffix('m') {
        return match rest.split_once('h') {
            Some((h, m)) => Ok(number(h, HOUR)?.plus(number(m, MINUTE)?)),
            None => number(rest, MINUTE),
        };
    }
    if let Some((h, m)) = compact.split_once('h') {
        let hours = number(h, HOUR)?;
        return if m.is_empty() {
            Ok(hours)
        } else {
            Ok(hours.plus(number(m, MINUTE)?))
        };
    }
    number(&compact, HOUR)
}

/// A stable id for an entry, for the API: its file and its 1-based line
/// (`time/pat/2026-11.timedot@12`, with a letter for one letter of a
/// letters line, `@12b`), or its timeclock session (`…timeclock@s3`). A
/// session cut at midnight is one id.
#[must_use]
pub fn entry_id(entry: &TimeEntry) -> String {
    match entry.origin {
        Origin::Timedot { line, letter } => format!(
            "{}@{}{}",
            entry.source.file,
            line + 1,
            letter.map(String::from).unwrap_or_default()
        ),
        Origin::Timeclock { session } => format!("{}@s{session}", entry.source.file),
    }
}

/// Where an entry id points.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryRef {
    /// The file.
    pub file: VaultPath,
    /// The timedot line index (0-based) or timeclock session index.
    pub at: usize,
    /// Which format.
    pub timeclock: bool,
}

impl EntryRef {
    /// The ref of `entry`.
    #[must_use]
    pub fn of(entry: &TimeEntry) -> Self {
        match entry.origin {
            Origin::Timedot { line, .. } => Self {
                file: entry.source.file.clone(),
                at: line,
                timeclock: false,
            },
            Origin::Timeclock { session } => Self {
                file: entry.source.file.clone(),
                at: session,
                timeclock: true,
            },
        }
    }
}

/// The entries `id` names in `book` (more than one for a letters line or a
/// session cut at midnight), by index.
///
/// # Errors
/// [`TimeOpError::NotFound`].
pub fn find_entries(book: &TimeBook, id: &str) -> Result<Vec<usize>, TimeOpError> {
    let (file, at) = id
        .rsplit_once('@')
        .ok_or_else(|| TimeOpError::BadRequest(format!("`{id}` isn't an entry id")))?;
    let found: Vec<usize> = book
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.source.file.as_str() == file && entry_id(entry) == id)
        .map(|(index, _)| index)
        .collect();
    let line_only: Vec<usize> = if found.is_empty() && !at.starts_with('s') {
        // A letters line named without its letter: all of its letters.
        book.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.source.file.as_str() == file
                    && matches!(entry.origin, Origin::Timedot { line, .. } if (line + 1).to_string() == at)
            })
            .map(|(index, _)| index)
            .collect()
    } else {
        found
    };
    if line_only.is_empty() {
        return Err(TimeOpError::NotFound(format!(
            "no time entry {id} (the file may have changed; read it again)"
        )));
    }
    Ok(line_only)
}

/// Which invoice numbers can still be changed: drafts. An entry tagged with
/// any other known invoice (sent, paid, void, or booked with no file) is
/// locked, and so is one on a bill (T-entries).
#[derive(Debug, Clone, Default)]
pub struct Locks {
    /// Numbers whose document is a draft.
    pub drafts: BTreeSet<String>,
    /// Every number with a document or a booking.
    pub known: BTreeSet<String>,
}

impl Locks {
    /// The locks the index and the books' `invoice:` numbers say.
    #[must_use]
    pub fn of(index: &DocumentIndex, booked: &[String]) -> Self {
        let mut locks = Self::default();
        for doc in index.docs() {
            if let Some(number) = doc.number() {
                locks.known.insert(number.to_string());
                if doc.header.status == Some(crate::doc::Status::Draft) {
                    locks.drafts.insert(number.to_string());
                }
            }
        }
        for number in booked {
            locks.known.insert(number.clone());
            locks.drafts.remove(number);
        }
        locks
    }

    /// Why `entry` can't be edited, if it can't.
    #[must_use]
    pub fn reason(&self, book: &TimeBook, entry: &TimeEntry) -> Option<String> {
        if let Some(bill) = entry.tag(&book.tags, TagRole::Bill) {
            return Some(format!(
                "it is on bill {bill}; release the bill to change it"
            ));
        }
        let invoice = entry.tag(&book.tags, TagRole::Invoice)?;
        (self.known.contains(invoice) && !self.drafts.contains(invoice)).then(|| {
            format!("it is on invoice {invoice}, which is sent; void it or release its time to change it")
        })
    }
}

/// One books folder's time book: its settings and paths.
#[derive(Clone)]
pub struct TimeBooks {
    vault: Arc<dyn Vault>,
    main: VaultPath,
    /// `[time]`.
    pub config: TimeConfig,
    /// Where the files are.
    pub paths: TimePaths,
}

impl std::fmt::Debug for TimeBooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeBooks")
            .field("rollup", &self.paths.rollup)
            .finish_non_exhaustive()
    }
}

/// What [`TimeBooks::write`] did.
#[derive(Debug, Clone)]
pub struct Written<T> {
    /// The operation's own result.
    pub done: T,
    /// The files written.
    pub saved: Vec<VaultPath>,
    /// Whether a change on disk made it read and apply again.
    pub retried: bool,
    /// `include` lines added to the roll-up for files it didn't reach.
    pub included: Vec<String>,
}

impl TimeBooks {
    /// The time book of the books whose main journal is `main`.
    ///
    /// # Errors
    /// [`TimeOpError::Time`] when `ledgeline.toml` or a configured path is
    /// unusable.
    pub fn open(vault: Arc<dyn Vault>, main: VaultPath) -> Result<Self, TimeOpError> {
        let config = book::read_config(&*vault, &main)?;
        let paths = book::paths(&main, &config)?;
        Ok(Self {
            vault,
            main,
            config,
            paths,
        })
    }

    /// The vault.
    #[must_use]
    pub fn vault(&self) -> &Arc<dyn Vault> {
        &self.vault
    }

    /// The main journal.
    #[must_use]
    pub fn main(&self) -> &VaultPath {
        &self.main
    }

    /// Whether the roll-up exists.
    #[must_use]
    pub fn is_set_up(&self) -> bool {
        self.vault.stat(&self.paths.rollup).ok().flatten().is_some()
    }

    /// The book, or `None` before setup. Open timeclock sessions count
    /// nothing until stopped; the tracker adds a running timer's time itself.
    ///
    /// # Errors
    /// [`TimeOpError::Time`] when the roll-up doesn't parse.
    pub fn load(&self) -> Result<Option<TimeBook>, TimeOpError> {
        Ok(book::load(&*self.vault, &self.main, &self.config, None)?)
    }

    /// The book; [`TimeOpError::NotSetUp`] before setup.
    ///
    /// # Errors
    /// As [`Self::load`].
    pub fn require(&self) -> Result<TimeBook, TimeOpError> {
        self.load()?.ok_or_else(|| self.not_set_up())
    }

    fn not_set_up(&self) -> TimeOpError {
        TimeOpError::NotSetUp(self.paths.relative(&self.paths.rollup))
    }

    /// The tracker for `person`.
    #[must_use]
    pub fn tracker<'a>(&'a self, person: Option<&'a str>) -> Tracker<'a> {
        Tracker {
            paths: &self.paths,
            config: &self.config,
            person,
        }
    }

    /// Run `op` over the book and its files, then save every file it
    /// changed. When a file changed on disk meanwhile, nothing is written:
    /// the book is read again and `op` runs once more over it. `book` is a
    /// book already read (the app's cache), used for the first try.
    ///
    /// A file the save created that the roll-up doesn't reach gets an
    /// `include` line in the roll-up (only once it exists: a glob matching
    /// nothing is an error), so the time just written is read back.
    ///
    /// # Errors
    /// [`TimeOpError::NotSetUp`], whatever `op` refuses, or the save's.
    pub fn write<T>(
        &self,
        book: Option<&TimeBook>,
        mut op: impl FnMut(&TimeBook, &mut TimeFiles) -> Result<T, TimeOpError>,
    ) -> Result<Written<T>, TimeOpError> {
        let mut attempt =
            |book: &TimeBook| -> Result<(T, Vec<VaultPath>, Vec<VaultPath>), TimeOpError> {
                let mut files = TimeFiles::with_book(Arc::clone(&self.vault), book);
                let done = op(book, &mut files)?;
                let created: Vec<VaultPath> = files
                    .pending()
                    .into_iter()
                    .filter(|(_, _, new)| *new)
                    .map(|(path, _, _)| path.clone())
                    .collect();
                let saved = files.save()?;
                Ok((done, saved, created))
            };
        let first = match book {
            Some(book) => attempt(book),
            None => attempt(&self.require()?),
        };
        let (result, retried) = match first {
            Err(TimeOpError::Time(TimeError::ExternalChange(_))) => {
                (attempt(&self.require()?), true)
            }
            other => (other, false),
        };
        let (done, saved, created) = result?;
        let included = self.cover(&created)?;
        Ok(Written {
            done,
            saved,
            retried,
            included,
        })
    }

    /// Add an `include` to the roll-up for each of `files` it doesn't reach.
    fn cover(&self, files: &[VaultPath]) -> Result<Vec<String>, TimeOpError> {
        let rollup = &self.paths.rollup;
        if files.is_empty() || TimeFormat::of_path(rollup.as_str()).is_some() {
            return Ok(Vec::new());
        }
        let (bytes, revision) = self
            .vault
            .read_pinned(rollup, None)
            .map_err(|e| TimeOpError::Storage(format!("{rollup}: {e}")))?;
        let mut text = String::from_utf8(bytes)
            .map_err(|_| TimeOpError::Storage(format!("{rollup} isn't UTF-8")))?;
        let mut added = Vec::new();
        for file in files {
            if TimeFormat::of_path(file.as_str()).is_none() {
                continue;
            }
            if let Coverage::NotCovered(line) =
                book::coverage(&text, rollup, file, Some(&self.paths.track))
            {
                text = book::with_include(&text, &line);
                added.push(line);
            }
        }
        if !added.is_empty() {
            self.vault
                .replace(rollup, text.as_bytes(), &revision)
                .map_err(|e| TimeOpError::Storage(format!("{rollup}: {e}")))?;
        }
        Ok(added)
    }
}

/// Apply `edits` to the frontmatter of the note at `path`, by key (§5.5),
/// and save it over the revision read: how a customer's `time_items` rule
/// and a timekeeper's answers are remembered (an explicit action).
///
/// # Errors
/// [`TimeOpError::Storage`] when the note can't be read, edited safely or
/// written (it changed meanwhile).
pub(crate) fn save_note_fields(
    index: &mut DocumentIndex,
    vault: &dyn Vault,
    path: &VaultPath,
    edits: &[crate::frontmatter::FieldEdit],
) -> Result<(), TimeOpError> {
    let (text, revision) =
        crate::edit::read_text(vault, path).map_err(|e| TimeOpError::Storage(e.to_string()))?;
    let edited = crate::frontmatter::edit(&text, edits)
        .map_err(|e| TimeOpError::Storage(format!("{path}: {e}")))?;
    if edited != text {
        vault
            .replace(path, edited.as_bytes(), &revision)
            .map_err(|e| TimeOpError::Storage(format!("{path}: {e}")))?;
        index.refresh(vault, std::slice::from_ref(path));
    }
    Ok(())
}

/// Hours as plain decimal text, exact (`2.5`, `0.333333333`).
#[must_use]
pub fn hours_text(quantity: TimeQty) -> String {
    ledgeline_core::edit::render_dec(quantity.hours(), '.')
}

/// Hours to two places, half up (`0.33`): what an invoice row writes.
#[must_use]
pub fn hours_2(quantity: TimeQty) -> String {
    let rounded = quantity
        .hours()
        .rounded(2)
        .unwrap_or_else(|_| quantity.hours());
    ledgeline_core::edit::render_dec(rounded, '.')
}

/// Whole seconds, rounded half up, for JSON.
#[must_use]
pub fn seconds_of(quantity: TimeQty) -> i64 {
    let nanos = quantity.nanos();
    i64::try_from((nanos + nanos.signum() * 500_000_000) / 1_000_000_000).unwrap_or(0)
}

/// The quantity, for JSON: exact seconds, exact hours and the display text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Qty {
    /// Whole seconds.
    pub seconds: i64,
    /// Hours, exact, as plain decimal text.
    pub hours: String,
    /// For people: `1h 05m`.
    pub text: String,
}

impl From<TimeQty> for Qty {
    fn from(quantity: TimeQty) -> Self {
        Self {
            seconds: seconds_of(quantity),
            hours: hours_text(quantity),
            text: quantity.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_people_type_them() {
        let minutes = |text: &str| parse_duration(text).map(TimeQty::round_minutes).ok();
        assert_eq!(minutes("45m"), Some(45));
        assert_eq!(minutes("2h"), Some(120));
        assert_eq!(minutes("1h30"), Some(90));
        assert_eq!(minutes("1h 30m"), Some(90));
        assert_eq!(minutes("1.5"), Some(90));
        assert_eq!(minutes("0:45"), Some(45));
        assert_eq!(minutes("90s"), Some(2));
        for bad in ["", "x", "-1", "1:75", "h"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn moments_and_prefs() {
        let at = parse_moment("2026-11-26T14:05:12").expect("moment");
        assert_eq!(at, Moment::new("2026-11-26", 50_712));
        assert_eq!(moment_from_epoch(at.epoch_seconds()), at);
        assert_eq!(
            parse_moment("2026-11-26 14:05"),
            Some(Moment::new("2026-11-26", 50_700))
        );
        assert_eq!(parse_moment("2026-11-26"), None);
        let prefs = TimePrefs::from_prefs(&serde_json::json!({
            "time": {"person": "pat", "pinned": ["a", "", "b"], "other": null, "step": 10}
        }));
        assert_eq!(prefs.person.as_deref(), Some("pat"));
        assert_eq!(prefs.pinned, ["a", "", "b"]);
        assert_eq!(prefs.other, None);
        assert_eq!(prefs.step, 10);
        assert_eq!(prefs.long_running, 240);
        assert_eq!(TimePrefs::from_prefs(&Value::Null), TimePrefs::default());
    }

    #[test]
    fn notes_with_tags_are_refused() {
        assert!(check_note("reviewed schedules").is_ok());
        let error = check_note("call re: K-1").expect_err("a tag");
        assert_eq!(error.code(), "note-tag");
        match error {
            TimeOpError::Note {
                offending,
                suggestion,
                ..
            } => {
                assert_eq!(offending, "re:");
                assert_eq!(suggestion.as_deref(), Some("call re K-1"));
            }
            other => panic!("{other}"),
        }
    }
}
