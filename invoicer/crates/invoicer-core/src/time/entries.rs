//! Entries (plan 46 T-entries): the table over the roll-up, and its writes —
//! add, edit, delete and bulk (*Mark billable · Mark nobill · Move to
//! account · Set person*). A line on a sent invoice or a booked bill is
//! locked ([`super::Locks`]); the unlock path is to void or release it.
//!
//! Timedot lines edit in place (account, amount, note; date by moving the
//! line). Timeclock sessions edit as start and end; their tags go on the
//! `o` line.

use super::{
    EntryRef, Locks, Qty, TimeBooks, TimeOpError, Written, check_note, entry_id, find_entries,
    parse_duration,
};
use ledgeline_core::time::files::TimeFiles;
use ledgeline_core::time::model::{TimeBook, TimeEntry};
use ledgeline_core::time::query::{self, Filter, Period};
use ledgeline_core::time::timeclock::Moment;
use ledgeline_core::time::{TagRole, TimeError, check_account, timeclock, timedot, write};
use ledgeline_core::vault::VaultPath;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Which billing state an entry is in (B-states).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Billed {
    /// Billable, with a customer, and no `invoice`/`nobill`.
    Unbilled,
    /// On an invoice.
    Invoiced,
    /// Written off (`nobill`).
    Nobill,
    /// Not billable (its account says no, or no customer).
    NotBillable,
}

impl Billed {
    /// The state of `entry`.
    #[must_use]
    pub fn of(book: &TimeBook, entry: &TimeEntry) -> Self {
        let names = &book.tags;
        if names.has(&entry.tags, TagRole::Invoice) {
            Self::Invoiced
        } else if names.has(&entry.tags, TagRole::Nobill) {
            Self::Nobill
        } else if query::is_billable(book, entry) && query::customer_of(book, entry).is_some() {
            Self::Unbilled
        } else {
            Self::NotBillable
        }
    }

    /// `unbilled`, `invoiced`, `nobill`, `not-billable`, and `billed` for
    /// invoiced or nobill.
    #[must_use]
    pub fn matches(self, wanted: &str) -> bool {
        match wanted {
            "unbilled" => self == Self::Unbilled,
            "invoiced" => self == Self::Invoiced,
            "nobill" => self == Self::Nobill,
            "not-billable" => self == Self::NotBillable,
            "billed" => matches!(self, Self::Invoiced | Self::Nobill),
            _ => true,
        }
    }
}

/// One row of Entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryView {
    /// The id the writes take ([`entry_id`]).
    pub id: String,
    /// ISO date.
    pub date: String,
    /// Who.
    pub person: Option<String>,
    /// The account.
    pub account: String,
    /// How long.
    pub quantity: Qty,
    /// The note.
    pub note: String,
    /// The entry's own tags and its day's.
    pub tags: Vec<(String, String)>,
    /// `invoice:`.
    pub invoice: Option<String>,
    /// `bill:`.
    pub bill: Option<String>,
    /// `nobill:`.
    pub nobill: Option<String>,
    /// The billing state.
    pub billed: Billed,
    /// Billable (resolved).
    pub billable: bool,
    /// The customer (resolved).
    pub customer: Option<String>,
    /// The file.
    pub file: VaultPath,
    /// 1-based lines.
    pub line: u32,
    /// `timedot` or `timeclock`.
    pub format: &'static str,
    /// Why it can't be edited, when it can't.
    pub locked: Option<String>,
}

impl EntryView {
    /// The row for `entry`.
    #[must_use]
    pub fn of(book: &TimeBook, entry: &TimeEntry, locks: &Locks) -> Self {
        let tag = |role| entry.tag(&book.tags, role).map(str::to_string);
        Self {
            id: entry_id(entry),
            date: entry.date.clone(),
            person: entry.person.clone(),
            account: entry.account.clone(),
            quantity: entry.quantity.into(),
            note: entry.note.clone(),
            tags: entry.tags.clone(),
            invoice: tag(TagRole::Invoice),
            bill: tag(TagRole::Bill),
            nobill: tag(TagRole::Nobill),
            billed: Billed::of(book, entry),
            billable: query::is_billable(book, entry),
            customer: query::customer_of(book, entry).map(str::to_string),
            file: entry.source.file.clone(),
            line: entry.source.lines.start,
            format: if matches!(
                entry.origin,
                ledgeline_core::time::model::Origin::Timeclock { .. }
            ) {
                "timeclock"
            } else {
                "timedot"
            },
            locked: locks.reason(book, entry),
        }
    }
}

/// What Entries filters by (the URL contract: `from`, `to`, `person`,
/// `customer`, `billable`, `billed`, `account`, `q`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EntryQuery {
    /// First date (inclusive).
    pub from: Option<String>,
    /// Last date (inclusive).
    pub to: Option<String>,
    /// One person (`""`: unattributed).
    pub person: Option<String>,
    /// One customer.
    pub customer: Option<String>,
    /// Billable or not.
    pub billable: Option<bool>,
    /// `unbilled`, `invoiced`, `nobill`, `not-billable`, `billed`.
    pub billed: Option<String>,
    /// This account or below.
    pub account: Option<String>,
    /// Text in the note or account.
    pub q: Option<String>,
    /// One invoice's time.
    pub invoice: Option<String>,
    /// One bill's time.
    pub bill: Option<String>,
}

/// `from`..=`to` as a [`Period`] (`to` exclusive there).
#[must_use]
pub fn period(from: Option<&str>, to: Option<&str>) -> Period {
    Period {
        from: from.map(str::to_string),
        to: to.map(|to| ledgeline_core::reports::periods::add_days(to, 1)),
    }
}

/// Entries: the rows `query` keeps, in file order, and their total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryList {
    /// The rows.
    pub entries: Vec<EntryView>,
    /// Their total.
    pub total: Qty,
    /// What the parsers found in the files (hledger-compatibility findings).
    pub findings: Vec<FindingView>,
}

/// A parser finding, for Entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindingView {
    /// The file.
    pub file: VaultPath,
    /// 1-based line.
    pub line: u32,
    /// `info`, `warning` or `error`.
    pub severity: &'static str,
    /// What.
    pub message: String,
}

/// The entries `query` keeps.
#[must_use]
pub fn list(book: &TimeBook, query: &EntryQuery, locks: &Locks) -> EntryList {
    let filter = Filter {
        period: period(query.from.as_deref(), query.to.as_deref()),
        person: query.person.as_deref(),
        customer: query.customer.as_deref(),
        billable: query.billable,
        branch: query.account.as_deref(),
    };
    let needle = query.q.as_deref().map(str::to_lowercase);
    let entries: Vec<EntryView> = query::entries(book, &filter)
        .filter(|(_, entry)| {
            query
                .billed
                .as_deref()
                .is_none_or(|wanted| Billed::of(book, entry).matches(wanted))
        })
        .filter(|(_, entry)| {
            query
                .invoice
                .as_deref()
                .is_none_or(|n| entry.tag(&book.tags, TagRole::Invoice) == Some(n))
        })
        .filter(|(_, entry)| {
            query
                .bill
                .as_deref()
                .is_none_or(|n| entry.tag(&book.tags, TagRole::Bill) == Some(n))
        })
        .filter(|(_, entry)| {
            needle.as_deref().is_none_or(|needle| {
                entry.note.to_lowercase().contains(needle)
                    || entry.account.to_lowercase().contains(needle)
            })
        })
        .map(|(_, entry)| EntryView::of(book, entry, locks))
        .collect();
    let total = ledgeline_core::time::TimeQty::sum(
        query::entries(book, &filter)
            .filter(|(_, entry)| entries.iter().any(|v| v.id == entry_id(entry)))
            .map(|(_, entry)| entry.quantity),
    );
    EntryList {
        total: total.into(),
        entries,
        findings: book
            .findings
            .iter()
            .map(|f| FindingView {
                file: f.file.clone(),
                line: f.finding.line,
                severity: match f.finding.severity {
                    ledgeline_core::time::Severity::Info => "info",
                    ledgeline_core::time::Severity::Warning => "warning",
                    ledgeline_core::time::Severity::Error => "error",
                },
                message: f.finding.message.clone(),
            })
            .collect(),
    }
}

/// Entries → Add (`POST /api/time/entries`).
#[derive(Debug, Clone, Deserialize)]
pub struct NewEntry {
    /// ISO date.
    pub date: String,
    /// The account.
    pub account: String,
    /// `45m`, `1h30`, `1.5`, `0:45` ([`parse_duration`]).
    pub duration: String,
    /// The note.
    #[serde(default)]
    pub note: String,
    /// Whose (default: the tracker's person).
    pub person: Option<String>,
}

/// Add an entry in the person's file for its month, inside its date block.
///
/// # Errors
/// [`TimeOpError`].
pub fn add(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    default_person: Option<&str>,
    new: &NewEntry,
) -> Result<Written<VaultPath>, TimeOpError> {
    if !crate::table::books::is_iso_date(&new.date) {
        return Err(TimeOpError::BadRequest(format!(
            "`{}` isn't a date (YYYY-MM-DD)",
            new.date
        )));
    }
    check_account(&new.account)?;
    check_note(&new.note)?;
    let quantity = parse_duration(&new.duration)?;
    let person = new
        .person
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .or(default_person);
    books.write(book, |_, files| {
        let tracker = books.tracker(person);
        // In timeclock mode the tracker file is a .timeclock: an added
        // entry still goes in the month's timedot file beside it.
        let path = tracker.file_for(&new.date)?;
        if ledgeline_core::time::TimeFormat::of_path(path.as_str())
            == Some(ledgeline_core::time::TimeFormat::Timedot)
        {
            Ok(files.add_entry(&tracker, &new.date, &new.account, quantity, &new.note, &[])?)
        } else {
            let timedot_path = path
                .as_str()
                .strip_suffix(".timeclock")
                .map(|stem| format!("{stem}.timedot"))
                .and_then(|p| VaultPath::new(&p).ok())
                .ok_or_else(|| TimeOpError::BadRequest("no timedot file to add to".into()))?;
            let header = ledgeline_core::time::book::track_header(
                person,
                &new.date,
                ledgeline_core::time::TimeFormat::Timedot,
                &books.config,
            );
            let style = tracker.style();
            files.edit(&timedot_path, Some(&header), |text| {
                write::insert_entry(
                    text,
                    &new.date,
                    &new.account,
                    quantity,
                    &new.note,
                    &[],
                    &style,
                )
            })?;
            Ok(timedot_path)
        }
    })
}

/// Entries → Edit (`PUT /api/time/entries`): what changes. Absent fields
/// stay.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EntryEdit {
    /// The entry ([`entry_id`]).
    pub id: String,
    /// A new date (timedot: the line moves to that day).
    pub date: Option<String>,
    /// A new account.
    pub account: Option<String>,
    /// A new duration (timedot).
    pub duration: Option<String>,
    /// A new note (timedot).
    pub note: Option<String>,
    /// A new start, `YYYY-MM-DDTHH:MM[:SS]` (timeclock).
    pub start: Option<String>,
    /// A new end (timeclock).
    pub end: Option<String>,
}

/// The single entry `id` names, unlocked, with its ref.
fn editable<'b>(
    book: &'b TimeBook,
    id: &str,
    locks: &Locks,
) -> Result<(&'b TimeEntry, EntryRef), TimeOpError> {
    let found = find_entries(book, id)?;
    let entry = &book.entries[found[0]];
    if let Some(reason) = found
        .iter()
        .find_map(|&at| locks.reason(book, &book.entries[at]))
    {
        return Err(TimeOpError::Locked(format!("{id} can't change: {reason}")));
    }
    Ok((entry, EntryRef::of(entry)))
}

fn moment(text: &str) -> Result<Moment, TimeOpError> {
    super::parse_moment(text).ok_or_else(|| {
        TimeOpError::BadRequest(format!("`{text}` isn't a moment (YYYY-MM-DDTHH:MM:SS)"))
    })
}

/// Apply `edit` to one entry.
///
/// # Errors
/// [`TimeOpError::Locked`], [`TimeOpError::Note`], [`TimeOpError`].
pub fn update(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    locks: &Locks,
    edit: &EntryEdit,
) -> Result<Written<()>, TimeOpError> {
    if let Some(note) = &edit.note {
        check_note(note)?;
    }
    if let Some(account) = &edit.account {
        check_account(account)?;
    }
    let quantity = edit.duration.as_deref().map(parse_duration).transpose()?;
    books.write(book, |book, files| {
        let (entry, at) = editable(book, &edit.id, locks)?;
        if at.timeclock {
            if edit.duration.is_some() || edit.note.is_some() || edit.account.is_some() {
                return Err(TimeOpError::BadRequest(
                    "a timeclock session edits as start and end; rename the account to move it"
                        .to_string(),
                ));
            }
            let text = files.text(&at.file)?.to_string();
            let session = timeclock::parse(&text)
                .sessions
                .get(at.at)
                .cloned()
                .ok_or_else(|| TimeOpError::NotFound(format!("no session {}", edit.id)))?;
            let start = edit
                .start
                .as_deref()
                .map(moment)
                .transpose()?
                .unwrap_or(session.start);
            let end = edit.end.as_deref().map(moment).transpose()?.or(session.end);
            files.edit(&at.file, None, |text| {
                write::set_session(text, at.at, &start, end.as_ref())
            })?;
            return Ok(());
        }
        if edit.start.is_some() || edit.end.is_some() {
            return Err(TimeOpError::BadRequest(
                "a timedot line has no start or end; change its duration".to_string(),
            ));
        }
        let moving = edit
            .date
            .as_deref()
            .filter(|date| *date != entry.date)
            .map(str::to_string);
        if let Some(date) = moving {
            if !crate::table::books::is_iso_date(&date) {
                return Err(TimeOpError::BadRequest(format!("`{date}` isn't a date")));
            }
            let own: Vec<(String, String)> = own_tags(files, &at)?;
            let quantity = quantity.unwrap_or(entry.quantity);
            let account = edit
                .account
                .clone()
                .unwrap_or_else(|| entry.account.clone());
            let note = edit.note.clone().unwrap_or_else(|| entry.note.clone());
            files.edit(&at.file, None, |text| write::remove_entry(text, at.at))?;
            let person = entry.person.as_deref();
            let tracker = books.tracker(person);
            files.add_entry(&tracker, &date, &account, quantity, &note, &own)?;
            return Ok(());
        }
        if edit.account.is_some() || quantity.is_some() {
            files.edit(&at.file, None, |text| {
                write::set_entry(text, at.at, edit.account.as_deref(), quantity)
            })?;
        }
        if let Some(note) = &edit.note {
            files.edit(&at.file, None, |text| write::set_note(text, at.at, note))?;
        }
        Ok(())
    })
}

/// The tags on the entry line itself (not its day's).
fn own_tags(files: &mut TimeFiles, at: &EntryRef) -> Result<Vec<(String, String)>, TimeOpError> {
    let text = files.text(&at.file)?;
    let file = timedot::parse(text);
    Ok(file
        .entry(at.at)
        .map(|entry| entry.tags.clone())
        .unwrap_or_default())
}

/// Delete entry `id`.
///
/// # Errors
/// [`TimeOpError::Locked`], [`TimeOpError`].
pub fn delete(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    locks: &Locks,
    id: &str,
) -> Result<Written<()>, TimeOpError> {
    books.write(book, |book, files| {
        let (_, at) = editable(book, id, locks)?;
        files.edit(&at.file, None, |text| {
            if at.timeclock {
                write::remove_session(text, at.at)
            } else {
                write::remove_entry(text, at.at)
            }
        })?;
        Ok(())
    })
}

/// A bulk action (T-entries "Bulk").
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum BulkOp {
    /// Take `nobill` off (back to unbilled, when the account is billable).
    Billable,
    /// Write the time off: `nobill: reason`.
    Nobill {
        /// Why (optional).
        #[serde(default)]
        reason: String,
    },
    /// Move to another account.
    Move {
        /// The account.
        account: String,
    },
    /// Set who did it (`person:` on the entry; empty removes it).
    Person {
        /// The short name.
        person: String,
    },
}

/// Set (or remove) `tag` on the entry at `at`.
pub(crate) fn set_tag(
    files: &mut TimeFiles,
    at: &EntryRef,
    tag: &str,
    value: Option<&str>,
) -> Result<(), TimeError> {
    files.edit(&at.file, None, |text| {
        if at.timeclock {
            write::set_session_tag(text, at.at, tag, value)
        } else {
            write::set_entry_tag(text, at.at, tag, value)
        }
    })
}

/// Apply `op` to every entry in `ids` (each once; a locked one refuses the
/// whole batch, nothing written). Returns how many lines changed.
///
/// # Errors
/// [`TimeOpError::Locked`], [`TimeOpError`].
pub fn bulk(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    locks: &Locks,
    ids: &[String],
    op: &BulkOp,
) -> Result<Written<usize>, TimeOpError> {
    if let BulkOp::Move { account } = op {
        check_account(account)?;
    }
    books.write(book, |book, files| {
        let mut refs: BTreeSet<EntryRef> = BTreeSet::new();
        for id in ids {
            let (_, at) = editable(book, id, locks)?;
            refs.insert(at);
        }
        let names = &book.tags;
        // Last line first: removing a tag can remove the continuation
        // line it was on, which would shift every later line of the file.
        for at in refs.iter().rev() {
            match op {
                BulkOp::Billable => set_tag(files, at, names.name(TagRole::Nobill), None)?,
                BulkOp::Nobill { reason } => {
                    set_tag(files, at, names.name(TagRole::Nobill), Some(reason.trim()))?;
                }
                BulkOp::Move { account } => {
                    if at.timeclock {
                        return Err(TimeOpError::BadRequest(
                            "a timeclock session moves by renaming its account".to_string(),
                        ));
                    }
                    files.edit(&at.file, None, |text| {
                        write::set_entry(text, at.at, Some(account), None)
                    })?;
                }
                BulkOp::Person { person } => {
                    let person = person.trim();
                    set_tag(
                        files,
                        at,
                        names.name(TagRole::Person),
                        (!person.is_empty()).then_some(person),
                    )?;
                }
            }
        }
        Ok(refs.len())
    })
}
