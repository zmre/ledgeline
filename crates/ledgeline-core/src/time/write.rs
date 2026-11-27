//! Pure text edits on time files, each verified by re-parse (plan 46
//! F-files writing rules, T4, T6, T7, T13).
//!
//! Every function takes a file's text and returns its new text; nothing here
//! touches storage ([`super::files::TimeFiles`] does). The discipline is the
//! journal editor's:
//!
//! - **Untouched lines are byte-for-byte preserved.** An edit names the lines
//!   it changes; only those are re-rendered, and only those are re-aligned.
//!   [`tidy`] is the explicit re-align-everything action.
//! - **Verified.** Each edit states what it means as a change to the file's
//!   *shape* (every non-blank line's meaning: date lines, entries with their
//!   exact quantity, tags and note, markers, comments), and the new text must
//!   parse to exactly that shape, or nothing is returned.
//! - **Writing rules.** Two or more spaces before an amount, never a tab, an
//!   amount as `Nm` or `Nh` ([`TimeQty::written`]), never `mo`/`y`, tags only
//!   on an entry's own line, account names checked ([`check_account`]).
//!
//! Timedot edits: [`add_minutes`] (T6: onto the day's open line), [`insert_entry`],
//! [`set_entry_tag`], [`set_note`], [`set_entry`], [`remove_entry`],
//! [`start_marker`] / [`remove_marker`] (T7),
//! [`tidy`]; [`stop_plan`] rounds a stopped session to minutes per day,
//! conserving the total across midnight. Timeclock edits: [`clock_in`],
//! [`clock_out`] (with seconds, T13), [`set_session`], [`set_session_tag`] and
//! [`remove_session`].

use super::accounts::check_tag_value;
use super::book::{WeekdayStyle, Words};
use super::timeclock::{self, ClockLineKind, Moment, TimeclockFile, split_at_midnight};
use super::timedot::{self, Entry, LineKind, MARKER_PREFIX, MARKER_SINCE, Quantity, TimedotFile};
use super::{
    TagNames, TagRole, TimeError, TimeQty, check_account, format_clock, is_tag_name, with_tag,
};
use crate::parse::parse_tags;

/// Where an amount's right edge sits when a day has no other entry to align
/// with (F-files: column 40).
pub const AMOUNT_EDGE: usize = 40;
/// An entry's indentation when its day has no other entry to copy.
pub const INDENT: &str = "    ";

/// What a timedot edit needs to know to write a new date line.
#[derive(Debug, Clone, Copy)]
pub struct DayStyle<'a> {
    /// Tag names.
    pub tags: &'a TagNames,
    /// Weekday names.
    pub words: &'a Words,
    /// Whether to write the weekday.
    pub weekday: WeekdayStyle,
    /// The person whose file this is: written on a new date line as
    /// `person:`, and the day an edit picks must not belong to someone else.
    pub person: Option<&'a str>,
}

/// The result of [`add_minutes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// The new text.
    pub text: String,
    /// The minutes actually added: less than asked when a day's line would
    /// have gone below zero.
    pub applied: i64,
}

// ---------------------------------------------------------------------------
// Shape: what verification compares
// ---------------------------------------------------------------------------

/// The meaning of one non-blank timedot line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Date {
        date: String,
        description: String,
        tags: Vec<(String, String)>,
    },
    Entry {
        account: String,
        parts: Vec<(Option<char>, TimeQty)>,
        tags: Vec<(String, String)>,
        note: String,
    },
    Marker {
        account: String,
        since: u32,
    },
    Comment(String),
    Invalid(String),
}

fn item_of(text: &str, file: &TimedotFile, line: usize) -> Option<Item> {
    let content = file.lines[line].content(text);
    Some(match &file.lines[line].kind {
        LineKind::Blank => return None,
        LineKind::Date(day) => Item::Date {
            date: day.date.clone(),
            description: day.description.clone(),
            tags: sorted(day.tags.clone()),
        },
        LineKind::Entry(entry) => entry_item(entry),
        LineKind::Marker(marker) => Item::Marker {
            account: marker.account.clone(),
            since: marker.since,
        },
        LineKind::Comment => Item::Comment(content.trim().to_string()),
        LineKind::Invalid => Item::Invalid(content.to_string()),
    })
}

fn entry_item(entry: &Entry) -> Item {
    Item::Entry {
        account: entry.account.clone(),
        parts: entry.quantity.parts(),
        tags: sorted(entry.tags.clone()),
        note: entry.note.clone(),
    }
}

fn sorted(mut tags: Vec<(String, String)>) -> Vec<(String, String)> {
    tags.sort();
    tags
}

/// One intended change, by **old** line index.
enum Change {
    /// The line now means this.
    Replace(usize, Item),
    /// These lines appear before old line `at` (`at` = the line count for the
    /// end).
    Insert(usize, Vec<Item>),
    /// The line is gone.
    Remove(usize),
}

/// Require `new` to parse to `old`'s shape with `changes` applied.
fn verify(
    old_text: &str,
    old: &TimedotFile,
    new_text: &str,
    changes: &[Change],
) -> Result<TimedotFile, TimeError> {
    let mut want = Vec::new();
    for line in 0..=old.lines.len() {
        for change in changes {
            if let Change::Insert(at, items) = change
                && *at == line
            {
                want.extend(items.iter().cloned());
            }
        }
        if line == old.lines.len() {
            break;
        }
        let replaced = changes.iter().find_map(|change| match change {
            Change::Replace(at, item) if *at == line => Some(Some(item.clone())),
            Change::Remove(at) if *at == line => Some(None),
            _ => None,
        });
        match replaced {
            Some(item) => want.extend(item),
            None => want.extend(item_of(old_text, old, line)),
        }
    }
    let new = timedot::parse(new_text);
    let got: Vec<Item> = (0..new.lines.len())
        .filter_map(|line| item_of(new_text, &new, line))
        .collect();
    if got == want {
        Ok(new)
    } else {
        Err(TimeError::Verify(
            "the timedot file would not read back as the edit intended".to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Replace line `at`'s content (its terminator kept); `None` removes the
/// line with its terminator.
fn replace_line(text: &str, file: &TimedotFile, at: usize, with: Option<&str>) -> String {
    let line = &file.lines[at];
    match with {
        Some(content) => format!("{}{content}{}", &text[..line.start], &text[line.end..]),
        None => format!("{}{}", &text[..line.start], &text[line.next..]),
    }
}

/// Insert `lines` before byte offset `at` (a line start, or the end).
fn insert_lines(text: &str, at: usize, lines: &[String]) -> String {
    let nl = newline_of(text);
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str(&text[..at]);
    if at > 0 && !out.ends_with('\n') {
        out.push_str(nl);
    }
    for line in lines {
        out.push_str(line);
        out.push_str(nl);
    }
    out.push_str(&text[at..]);
    out
}

fn width(text: &str) -> usize {
    text.chars().count()
}

/// An entry line: `prefix` (indentation, org stars), the account, padding to
/// put the amount's right edge at `edge` (two spaces at least), the amount,
/// then the comment.
fn render_entry(prefix: &str, account: &str, amount: &str, comment: &str, edge: usize) -> String {
    let head = width(prefix) + width(account);
    let mut line = format!("{prefix}{account}");
    if !amount.is_empty() {
        let pad = edge.saturating_sub(head + width(amount)).max(2);
        line.push_str(&" ".repeat(pad));
        line.push_str(amount);
    }
    if !comment.is_empty() {
        line.push_str("  ; ");
        line.push_str(comment);
    }
    line
}

/// The day block `day` (its date line index) spans: the date line through
/// its last non-blank line.
fn day_span(file: &TimedotFile, day: usize) -> (usize, usize) {
    file.days()
        .into_iter()
        .find(|(start, _)| *start == day)
        .unwrap_or((day, day + 1))
}

/// The right edge of the day's amounts (the widest), skipping line `skip`.
fn day_edge(text: &str, file: &TimedotFile, day: usize, skip: Option<usize>) -> Option<usize> {
    let (start, end) = day_span(file, day);
    (start + 1..end)
        .filter(|at| Some(*at) != skip)
        .filter_map(|at| {
            let entry = file.entry(at)?;
            let (_, q1) = entry.layout.quantity;
            (entry.quantity != Quantity::Empty).then(|| width(&file.lines[at].content(text)[..q1]))
        })
        .max()
}

/// The indentation the day's entries use (the first plain one's), else
/// [`INDENT`].
fn day_indent<'t>(text: &'t str, file: &TimedotFile, day: usize) -> &'t str {
    let (start, end) = day_span(file, day);
    (start + 1..end)
        .find_map(|at| {
            let entry = file.entry(at)?;
            let prefix = &file.lines[at].content(text)[..entry.layout.account_start];
            (!prefix.is_empty() && prefix.chars().all(|c| c == ' ')).then_some(prefix)
        })
        .unwrap_or(INDENT)
}

/// The comment of `entry` as written (without `;`), trimmed.
fn comment_of(entry: &Entry) -> &str {
    entry.comment.as_deref().unwrap_or("")
}

/// Re-render entry line `at` with `amount` (as written) and `comment`,
/// aligned to its day.
fn rerender(
    text: &str,
    file: &TimedotFile,
    at: usize,
    amount: &str,
    comment: &str,
) -> Option<String> {
    let entry = file.entry(at)?;
    let content = file.lines[at].content(text);
    let prefix = &content[..entry.layout.account_start];
    let account = &content[entry.layout.account_start..entry.layout.account_end];
    let edge = day_edge(text, file, entry.day, Some(at))
        .unwrap_or(AMOUNT_EDGE)
        .max(AMOUNT_EDGE.min(width(prefix) + width(account) + 2 + width(amount)));
    Some(render_entry(prefix, account, amount, comment, edge))
}

fn amount_text<'t>(text: &'t str, file: &TimedotFile, at: usize) -> &'t str {
    file.entry(at).map_or("", |entry| {
        let (q0, q1) = entry.layout.quantity;
        &file.lines[at].content(text)[q0..q1]
    })
}

// ---------------------------------------------------------------------------
// Finding days and lines
// ---------------------------------------------------------------------------

/// The days dated `date` that `person` may write into: no `person:` on the
/// date line, or theirs.
fn days_on(file: &TimedotFile, date: &str, style: &DayStyle<'_>) -> Vec<usize> {
    file.days()
        .into_iter()
        .map(|(start, _)| start)
        .filter(|&start| {
            file.date_line(start).is_some_and(|day| {
                day.date == date
                    && match (style.person, style.tags.value(&day.tags, TagRole::Person)) {
                        (Some(person), Some(theirs)) => person == theirs,
                        _ => true,
                    }
            })
        })
        .collect()
}

/// The day's **open line** for `account` (T6): the first entry for it on that
/// date with no `invoice:`, `bill:` or `nobill:` tag, whose amount is a
/// number or dots in whole minutes (a letters line, or one like `0.33h`,
/// can't take more minutes without changing what it says).
#[must_use]
pub fn open_line(
    file: &TimedotFile,
    date: &str,
    account: &str,
    style: &DayStyle<'_>,
) -> Option<usize> {
    days_on(file, date, style).into_iter().find_map(|day| {
        let (start, end) = day_span(file, day);
        (start + 1..end).find(|&at| {
            file.entry(at).is_some_and(|entry| {
                entry.account == account
                    && !entry_is_settled(entry, style.tags)
                    && match entry.quantity {
                        Quantity::Empty | Quantity::Dots(_) => true,
                        Quantity::Number { .. } => entry.quantity.total().is_whole_minutes(),
                        Quantity::Letters(_) => false,
                    }
            })
        })
    })
}

fn entry_is_settled(entry: &Entry, tags: &TagNames) -> bool {
    [TagRole::Invoice, TagRole::Bill, TagRole::Nobill]
        .into_iter()
        .any(|role| tags.has(&entry.tags, role))
}

/// The date line a new date block gets: `2026-11-26 Thu  ; person: pat`.
#[must_use]
pub fn date_line_text(date: &str, style: &DayStyle<'_>) -> String {
    let mut line = date.to_string();
    if style.weekday == WeekdayStyle::Short {
        let name = style.words.weekday(date);
        if !name.is_empty() {
            line.push(' ');
            line.push_str(name);
        }
    }
    if let Some(person) = style.person {
        line.push_str(&format!(
            "  ; {}: {person}",
            style.tags.name(TagRole::Person)
        ));
    }
    line
}

/// Where a new day for `date` goes: before the first later day, else at the
/// end. Returns the byte offset, the old line index, and whether a blank
/// line must follow (it goes before another day) or precede it.
fn new_day_at(text: &str, file: &TimedotFile, date: &str) -> (usize, usize, bool) {
    let later = file
        .days()
        .into_iter()
        .map(|(start, _)| start)
        .find(|&start| {
            file.date_line(start)
                .is_some_and(|day| day.date.as_str() > date)
        });
    match later {
        Some(at) => (file.lines[at].start, at, true),
        None => (text.len(), file.lines.len(), false),
    }
}

/// The text with a new day for `date` holding `body` lines, and the changes
/// that says. The new lines are placed so days stay in date order.
fn with_new_day(
    text: &str,
    file: &TimedotFile,
    date: &str,
    style: &DayStyle<'_>,
    body: Vec<(String, Item)>,
) -> (String, Vec<Change>) {
    let (at, line, before_day) = new_day_at(text, file, date);
    let head = date_line_text(date, style);
    let mut lines = vec![head.clone()];
    lines.extend(body.iter().map(|(line, _)| line.clone()));
    let ends_blank = text.is_empty() || text.ends_with("\n\n") || text.ends_with("\r\n\r\n");
    if before_day {
        lines.push(String::new());
    } else if !ends_blank {
        lines.insert(0, String::new());
    }
    let new_text = insert_lines(text, at, &lines);
    let header = timedot::parse(&format!("{head}\n"));
    let mut items: Vec<Item> = header
        .date_line(0)
        .map(|day| Item::Date {
            date: day.date.clone(),
            description: day.description.clone(),
            tags: sorted(day.tags.clone()),
        })
        .into_iter()
        .collect();
    items.extend(body.into_iter().map(|(_, item)| item));
    (new_text, vec![Change::Insert(line, items)])
}

/// Where a new entry goes in an existing day: after its last entry (before
/// a trailing marker), else right after the date line.
fn entry_slot(file: &TimedotFile, day: usize) -> usize {
    let (start, end) = day_span(file, day);
    (start + 1..end)
        .rev()
        .find(|&at| matches!(file.lines[at].kind, LineKind::Entry(_)))
        .map_or(start + 1, |last| last + 1)
}

fn line_start(text: &str, file: &TimedotFile, at: usize) -> usize {
    file.lines.get(at).map_or(text.len(), |line| line.start)
}

// ---------------------------------------------------------------------------
// Timedot edits
// ---------------------------------------------------------------------------

fn check_note(note: &str) -> Result<(), TimeError> {
    match super::note_problem(note) {
        None => Ok(()),
        Some(problem) => Err(TimeError::Invalid(match problem.suggestion {
            Some(suggestion) => format!("{}, for example \"{suggestion}\"", problem.message),
            None => problem.message,
        })),
    }
}

/// A comment: `note`, then the tags.
fn comment_text(note: &str, tags: &[(String, String)]) -> String {
    let tags = tags
        .iter()
        .map(|(name, value)| {
            if value.is_empty() {
                format!("{name}:")
            } else {
                format!("{name}: {value}")
            }
        })
        .collect::<Vec<_>>();
    let mut parts: Vec<String> = Vec::new();
    if !note.trim().is_empty() {
        parts.push(note.trim().to_string());
    }
    parts.extend(tags);
    parts.join(", ")
}

fn check_entry_tags(tags: &[(String, String)]) -> Result<(), TimeError> {
    for (name, value) in tags {
        if !is_tag_name(name) {
            return Err(TimeError::Invalid(format!("`{name}` is not a tag name")));
        }
        check_tag_value(value)?;
    }
    Ok(())
}

fn new_entry(
    prefix: &str,
    account: &str,
    quantity: TimeQty,
    note: &str,
    tags: &[(String, String)],
    edge: usize,
) -> (String, Item) {
    let comment = comment_text(note, tags);
    let line = render_entry(prefix, account, &quantity.written(), &comment, edge);
    let parsed = timedot::parse(&format!("2000-01-01\n{line}\n"));
    let item = parsed
        .entry(1)
        .map_or(Item::Invalid(line.clone()), entry_item);
    (line, item)
}

/// Add an entry for `account` on `date`: `quantity` (written `Nm`/`Nh`), a
/// note and tags, after the day's last entry. The day is created (in date
/// order) when the file has none.
///
/// # Errors
/// [`TimeError::Invalid`] for a bad account, note or tag;
/// [`TimeError::Verify`].
pub fn insert_entry(
    text: &str,
    date: &str,
    account: &str,
    quantity: TimeQty,
    note: &str,
    tags: &[(String, String)],
    style: &DayStyle<'_>,
) -> Result<String, TimeError> {
    check_account(account)?;
    check_note(note)?;
    check_entry_tags(tags)?;
    let file = timedot::parse(text);
    let day = days_on(&file, date, style).last().copied();
    let (new_text, changes) = match day {
        Some(day) => {
            let indent = day_indent(text, &file, day).to_string();
            let edge = day_edge(text, &file, day, None).unwrap_or(AMOUNT_EDGE);
            let (line, item) = new_entry(&indent, account, quantity, note, tags, edge);
            let slot = entry_slot(&file, day);
            (
                insert_lines(text, line_start(text, &file, slot), &[line]),
                vec![Change::Insert(slot, vec![item])],
            )
        }
        None => {
            let entry = new_entry(INDENT, account, quantity, note, tags, AMOUNT_EDGE);
            with_new_day(text, &file, date, style, vec![entry])
        }
    };
    verify(text, &file, &new_text, &changes)?;
    Ok(new_text)
}

/// Add `minutes` (negative to take away) to `account` on `date` (T6): onto
/// the day's [`open_line`], or as a new line when there is none. A billed
/// line is never changed. The day's line never goes below zero: a line that
/// reaches zero with no comment is removed, and [`Added::applied`] says how
/// much was really added. Taking away with no open line does nothing.
///
/// # Errors
/// [`TimeError::Invalid`] for a bad account; [`TimeError::Verify`].
pub fn add_minutes(
    text: &str,
    date: &str,
    account: &str,
    minutes: i64,
    style: &DayStyle<'_>,
) -> Result<Added, TimeError> {
    check_account(account)?;
    let file = timedot::parse(text);
    let Some(at) = open_line(&file, date, account, style) else {
        if minutes <= 0 {
            return Ok(Added {
                text: text.to_string(),
                applied: 0,
            });
        }
        return insert_entry(
            text,
            date,
            account,
            TimeQty::from_minutes(minutes),
            "",
            &[],
            style,
        )
        .map(|text| Added {
            text,
            applied: minutes,
        });
    };
    let entry = file
        .entry(at)
        .ok_or_else(|| TimeError::NotFound("no entry".to_string()))?;
    let old = entry.quantity.total();
    let old_minutes = old.round_minutes();
    let new_minutes = (old_minutes + minutes).max(0);
    let applied = new_minutes - old_minutes;
    if applied == 0 {
        return Ok(Added {
            text: text.to_string(),
            applied: 0,
        });
    }
    let comment = comment_of(entry);
    let (new_text, change) = if new_minutes == 0 && comment.is_empty() {
        (replace_line(text, &file, at, None), Change::Remove(at))
    } else {
        let quantity = TimeQty::from_minutes(new_minutes);
        let line = rerender(text, &file, at, &quantity.written(), comment)
            .ok_or_else(|| TimeError::NotFound("no entry".to_string()))?;
        let item = Item::Entry {
            account: entry.account.clone(),
            parts: vec![(None, quantity)],
            tags: sorted(entry.tags.clone()),
            note: entry.note.clone(),
        };
        (
            replace_line(text, &file, at, Some(&line)),
            Change::Replace(at, item),
        )
    };
    verify(text, &file, &new_text, &[change])?;
    Ok(Added {
        text: new_text,
        applied,
    })
}

/// Set tag `tag` to `value` on the entry at line index `line` (or remove it,
/// for `None`), through [`with_tag`]: the note and other tags stay, the
/// amount keeps its spelling, and the line is re-aligned.
///
/// # Errors
/// [`TimeError::NotFound`] when the line isn't an entry;
/// [`TimeError::Invalid`]; [`TimeError::Verify`].
pub fn set_entry_tag(
    text: &str,
    line: usize,
    tag: &str,
    value: Option<&str>,
) -> Result<String, TimeError> {
    if !is_tag_name(tag) {
        return Err(TimeError::Invalid(format!("`{tag}` is not a tag name")));
    }
    if let Some(value) = value {
        check_tag_value(value)?;
    }
    let file = timedot::parse(text);
    let entry = file
        .entry(line)
        .ok_or_else(|| TimeError::NotFound(format!("line {} is not an entry", line + 1)))?;
    let comment = with_tag(comment_of(entry), tag, value);
    let comment = comment.as_str();
    let amount = amount_text(text, &file, line);
    let rendered = rerender(text, &file, line, amount, comment)
        .ok_or_else(|| TimeError::NotFound("no entry".to_string()))?;
    let mut tags: Vec<(String, String)> = entry
        .tags
        .iter()
        .filter(|(n, _)| n != tag)
        .cloned()
        .collect();
    if let Some(value) = value {
        tags.push((tag.to_string(), value.trim().to_string()));
    }
    let item = Item::Entry {
        account: entry.account.clone(),
        parts: entry.quantity.parts(),
        tags: sorted(tags),
        note: entry.note.clone(),
    };
    let new_text = replace_line(text, &file, line, Some(&rendered));
    verify(text, &file, &new_text, &[Change::Replace(line, item)])?;
    Ok(new_text)
}

/// Set the note of the entry at line index `line`; its tags stay.
///
/// # Errors
/// As [`set_entry_tag`]; a note can't hold `;`, a line break, or a `word:`
/// (hledger would read a tag).
pub fn set_note(text: &str, line: usize, note: &str) -> Result<String, TimeError> {
    check_note(note)?;
    let file = timedot::parse(text);
    let entry = file
        .entry(line)
        .ok_or_else(|| TimeError::NotFound(format!("line {} is not an entry", line + 1)))?;
    let own_tags: Vec<String> = comment_of(entry)
        .split(',')
        .filter(|segment| crate::parse::tag_in_segment(segment).is_some())
        .map(|segment| segment.trim().to_string())
        .collect();
    let mut parts: Vec<String> = Vec::new();
    if !note.trim().is_empty() {
        parts.push(note.trim().to_string());
    }
    parts.extend(own_tags);
    let comment = parts.join(", ");
    let amount = amount_text(text, &file, line);
    let rendered = rerender(text, &file, line, amount, &comment)
        .ok_or_else(|| TimeError::NotFound("no entry".to_string()))?;
    let item = Item::Entry {
        account: entry.account.clone(),
        parts: entry.quantity.parts(),
        tags: sorted(entry.tags.clone()),
        note: note.trim().to_string(),
    };
    let new_text = replace_line(text, &file, line, Some(&rendered));
    verify(text, &file, &new_text, &[Change::Replace(line, item)])?;
    Ok(new_text)
}

/// Remove the entry at line index `line` (Entries → Delete); every other
/// line is kept byte for byte.
///
/// # Errors
/// [`TimeError::NotFound`] when the line isn't an entry; [`TimeError::Verify`].
pub fn remove_entry(text: &str, line: usize) -> Result<String, TimeError> {
    let file = timedot::parse(text);
    file.entry(line)
        .ok_or_else(|| TimeError::NotFound(format!("line {} is not an entry", line + 1)))?;
    let new_text = replace_line(text, &file, line, None);
    verify(text, &file, &new_text, &[Change::Remove(line)])?;
    Ok(new_text)
}

/// Change the entry at line index `line`: its account and/or its amount
/// (written `Nm`/`Nh`), its note and tags kept, the line re-aligned
/// (Entries → Edit, bulk *Move to account*). A letters line can't take a new
/// amount without losing what its letters say.
///
/// # Errors
/// [`TimeError::NotFound`]; [`TimeError::Invalid`] for a bad account, a
/// negative amount or a letters line; [`TimeError::Verify`].
pub fn set_entry(
    text: &str,
    line: usize,
    account: Option<&str>,
    quantity: Option<TimeQty>,
) -> Result<String, TimeError> {
    if let Some(account) = account {
        check_account(account)?;
    }
    if quantity.is_some_and(|q| q < TimeQty::ZERO) {
        return Err(TimeError::Invalid(
            "an entry's time can't be negative".to_string(),
        ));
    }
    let file = timedot::parse(text);
    let entry = file
        .entry(line)
        .ok_or_else(|| TimeError::NotFound(format!("line {} is not an entry", line + 1)))?;
    if quantity.is_some() && matches!(entry.quantity, Quantity::Letters(_)) {
        return Err(TimeError::Invalid(
            "a letters line can't take a new amount; edit it by hand".to_string(),
        ));
    }
    let content = file.lines[line].content(text);
    let prefix = &content[..entry.layout.account_start];
    let account = account.unwrap_or(&entry.account);
    let written = quantity.map(TimeQty::written);
    let amount = written
        .as_deref()
        .unwrap_or_else(|| amount_text(text, &file, line));
    let edge = day_edge(text, &file, entry.day, Some(line))
        .unwrap_or(AMOUNT_EDGE)
        .max(AMOUNT_EDGE.min(width(prefix) + width(account) + 2 + width(amount)));
    let rendered = render_entry(prefix, account, amount, comment_of(entry), edge);
    let item = Item::Entry {
        account: account.to_string(),
        parts: quantity.map_or_else(|| entry.quantity.parts(), |q| vec![(None, q)]),
        tags: sorted(entry.tags.clone()),
        note: entry.note.clone(),
    };
    let new_text = replace_line(text, &file, line, Some(&rendered));
    verify(text, &file, &new_text, &[Change::Replace(line, item)])?;
    Ok(new_text)
}

/// The marker line (T7): `    ; ● running ACCOUNT  since HH:MM:SS`.
#[must_use]
pub fn marker_text(indent: &str, account: &str, since: u32) -> String {
    format!(
        "{indent}; {MARKER_PREFIX}{account}  {MARKER_SINCE}{}",
        format_clock(since)
    )
}

/// Start the timer: a running marker for `account` at `since` (seconds
/// after midnight) at the end of `date`'s day, which is created when
/// missing (T-running). A file holds one marker at most.
///
/// # Errors
/// [`TimeError::Invalid`] when a marker is already there or the account is
/// bad; [`TimeError::Verify`].
pub fn start_marker(
    text: &str,
    date: &str,
    account: &str,
    since: u32,
    style: &DayStyle<'_>,
) -> Result<String, TimeError> {
    check_account(account)?;
    let file = timedot::parse(text);
    if let Some((_, running)) = file.markers().next() {
        return Err(TimeError::Invalid(format!(
            "{} is already running in this file; stop it first",
            running.account
        )));
    }
    let item = Item::Marker {
        account: account.to_string(),
        since,
    };
    let (new_text, changes) = match days_on(&file, date, style).last().copied() {
        Some(day) => {
            let (_, end) = day_span(&file, day);
            let line = marker_text(day_indent(text, &file, day), account, since);
            (
                insert_lines(text, line_start(text, &file, end), &[line]),
                vec![Change::Insert(end, vec![item])],
            )
        }
        None => with_new_day(
            text,
            &file,
            date,
            style,
            vec![(marker_text(INDENT, account, since), item)],
        ),
    };
    verify(text, &file, &new_text, &changes)?;
    Ok(new_text)
}

/// Remove the marker at line index `line`.
///
/// # Errors
/// [`TimeError::NotFound`] when it isn't a marker; [`TimeError::Verify`].
pub fn remove_marker(text: &str, line: usize) -> Result<String, TimeError> {
    let file = timedot::parse(text);
    if !matches!(
        file.lines.get(line).map(|l| &l.kind),
        Some(LineKind::Marker(_))
    ) {
        return Err(TimeError::NotFound(format!(
            "line {} is not a running marker",
            line + 1
        )));
    }
    let new_text = replace_line(text, &file, line, None);
    verify(text, &file, &new_text, &[Change::Remove(line)])?;
    Ok(new_text)
}

/// A stopped session as whole minutes per day (T4, T-running): the
/// duration from `since` to `now` rounded to the nearest minute, half up,
/// and cut at each midnight so the days' minutes add up to exactly the
/// rounded total. Empty for a session under 30 s (an accidental press) or
/// one that ends before it starts.
#[must_use]
pub fn stop_plan(since: &Moment, now: &Moment) -> Vec<(String, i64)> {
    let mut elapsed = TimeQty::ZERO;
    let mut plan = Vec::new();
    for (date, piece) in split_at_midnight(since, now) {
        let before = elapsed.round_minutes();
        elapsed = elapsed.plus(piece);
        let minutes = elapsed.round_minutes() - before;
        if minutes != 0 {
            plan.push((date, minutes));
        }
    }
    plan
}

/// Re-align every entry (T-files *Tidy file*): entries indented like
/// [`INDENT`] (org-star prefixes kept), numbers and dots in whole minutes
/// rewritten as `Nm`/`Nh`, amounts right-aligned per day at
/// [`AMOUNT_EDGE`] or the day's widest, comments two spaces after; markers
/// re-indented. Letters, other amounts, date lines and comments are kept.
///
/// # Errors
/// [`TimeError::Verify`] (the meaning never changes).
pub fn tidy(text: &str) -> Result<String, TimeError> {
    let file = timedot::parse(text);
    let mut rendered: Vec<Option<String>> = vec![None; file.lines.len()];
    for (start, end) in file.days() {
        let rows: Vec<(usize, String, String, String, String)> = (start + 1..end)
            .filter_map(|at| {
                let entry = file.entry(at)?;
                let content = file.lines[at].content(text);
                let prefix = &content[..entry.layout.account_start];
                let prefix = if prefix.trim().is_empty() {
                    INDENT.to_string()
                } else {
                    prefix.trim_start().to_string()
                };
                let account =
                    content[entry.layout.account_start..entry.layout.account_end].to_string();
                let amount = match entry.quantity {
                    Quantity::Dots(_) | Quantity::Number { .. }
                        if entry.quantity.total().is_whole_minutes() =>
                    {
                        entry.quantity.total().written()
                    }
                    _ => amount_text(text, &file, at).to_string(),
                };
                Some((at, prefix, account, amount, comment_of(entry).to_string()))
            })
            .collect();
        let edge = rows
            .iter()
            .filter(|(_, _, _, amount, _)| !amount.is_empty())
            .map(|(_, prefix, account, amount, _)| {
                width(prefix) + width(account) + 2 + width(amount)
            })
            .max()
            .unwrap_or(0)
            .max(AMOUNT_EDGE);
        for (at, prefix, account, amount, comment) in rows {
            rendered[at] = Some(render_entry(&prefix, &account, &amount, &comment, edge));
        }
        for (at, marker) in file.markers().filter(|(at, _)| (start..end).contains(at)) {
            rendered[at] = Some(marker_text(INDENT, &marker.account, marker.since));
        }
    }
    let mut out = String::with_capacity(text.len());
    for (at, line) in file.lines.iter().enumerate() {
        match &rendered[at] {
            Some(content) => {
                out.push_str(content);
                out.push_str(&text[line.end..line.next]);
            }
            None => out.push_str(&text[line.start..line.next]),
        }
    }
    verify(text, &file, &out, &[])?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Timeclock edits
// ---------------------------------------------------------------------------

/// What verification compares for a timeclock file: each session, and every
/// line that isn't part of one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionShape {
    account: String,
    description: String,
    start: Moment,
    end: Option<Moment>,
    tags: Vec<(String, String)>,
}

fn sessions_of(file: &TimeclockFile) -> Vec<SessionShape> {
    file.sessions
        .iter()
        .map(|s| SessionShape {
            account: s.account.clone(),
            description: s.description.clone(),
            start: s.start.clone(),
            end: s.end.clone(),
            tags: sorted(s.tags.clone()),
        })
        .collect()
}

fn verify_clock(new_text: &str, want: Vec<SessionShape>) -> Result<(), TimeError> {
    let new = timeclock::parse(new_text);
    if new.has_errors() || sessions_of(&new) != want {
        return Err(TimeError::Verify(
            "the timeclock file would not read back as the edit intended".to_string(),
        ));
    }
    Ok(())
}

fn moment_text(at: &Moment) -> String {
    format!("{} {}", at.date, format_clock(at.seconds))
}

fn append_line(text: &str, line: &str) -> String {
    let nl = newline_of(text);
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(nl);
    }
    out.push_str(line);
    out.push_str(nl);
    out
}

/// Clock in (T13): `i DATE HH:MM:SS ACCOUNT  DESCRIPTION  ; tags` at the end.
///
/// # Errors
/// [`TimeError::Invalid`] when the account is already clocked in here, or a
/// value can't be written; [`TimeError::Verify`].
pub fn clock_in(
    text: &str,
    at: &Moment,
    account: &str,
    description: &str,
    tags: &[(String, String)],
) -> Result<String, TimeError> {
    check_account(account)?;
    check_note(description).map_err(|_| {
        TimeError::Invalid("a description can't hold `;`, a line break or a `word:`".to_string())
    })?;
    check_entry_tags(tags)?;
    let file = timeclock::parse(text);
    if file.open_sessions().any(|s| s.account == account) {
        return Err(TimeError::Invalid(format!(
            "{account} is already clocked in"
        )));
    }
    let mut line = format!("i {} {account}", moment_text(at));
    if !description.trim().is_empty() {
        line.push_str("  ");
        line.push_str(description.trim());
    }
    let comment = comment_text("", tags);
    if !comment.is_empty() {
        line.push_str("  ; ");
        line.push_str(&comment);
    }
    let new_text = append_line(text, &line);
    let mut want = sessions_of(&file);
    want.push(SessionShape {
        account: account.to_string(),
        description: description.trim().to_string(),
        start: at.clone(),
        end: None,
        tags: sorted(tags.to_vec()),
    });
    verify_clock(&new_text, want)?;
    Ok(new_text)
}

/// Clock `account` out at `at`: `o DATE HH:MM:SS ACCOUNT` at the end.
///
/// # Errors
/// [`TimeError::NotFound`] when it isn't clocked in; [`TimeError::Invalid`]
/// for an end before its start; [`TimeError::Verify`].
pub fn clock_out(text: &str, at: &Moment, account: &str) -> Result<String, TimeError> {
    let file = timeclock::parse(text);
    let index = file
        .sessions
        .iter()
        .position(|s| s.end.is_none() && s.account == account)
        .ok_or_else(|| TimeError::NotFound(format!("{account} is not clocked in")))?;
    if at.epoch_seconds() < file.sessions[index].start.epoch_seconds() {
        return Err(TimeError::Invalid(
            "a clock-out can't be before its clock-in".to_string(),
        ));
    }
    let new_text = append_line(text, &format!("o {} {account}", moment_text(at)));
    let mut want = sessions_of(&file);
    want[index].end = Some(at.clone());
    verify_clock(&new_text, want)?;
    Ok(new_text)
}

/// The line with its date and time tokens (the 2nd and 3rd words) replaced.
fn retimed(content: &str, at: &Moment) -> Option<String> {
    let mut words = Vec::new();
    let mut rest = content;
    for _ in 0..3 {
        let trimmed = rest.trim_start();
        let lead = &rest[..rest.len() - trimmed.len()];
        let end = trimmed.find([' ', '\t', ';']).unwrap_or(trimmed.len());
        words.push((lead, &trimmed[..end]));
        rest = &trimmed[end..];
    }
    let (code_lead, code) = words.first()?;
    let (date_lead, _) = words.get(1)?;
    let (time_lead, _) = words.get(2)?;
    Some(format!(
        "{code_lead}{code}{date_lead}{}{time_lead}{}{rest}",
        at.date,
        format_clock(at.seconds)
    ))
}

/// Change session `index`'s start and (when it has an `o` line) end,
/// keeping everything else on its lines.
///
/// # Errors
/// [`TimeError::NotFound`]; [`TimeError::Invalid`] for an end before the
/// start, or an end for an open session; [`TimeError::Verify`].
pub fn set_session(
    text: &str,
    index: usize,
    start: &Moment,
    end: Option<&Moment>,
) -> Result<String, TimeError> {
    let file = timeclock::parse(text);
    let session = file
        .sessions
        .get(index)
        .ok_or_else(|| TimeError::NotFound(format!("no session {index}")))?;
    if let Some(end) = end {
        if session.out_line.is_none() {
            return Err(TimeError::Invalid(
                "an open session has no end to set".to_string(),
            ));
        }
        if end.epoch_seconds() < start.epoch_seconds() {
            return Err(TimeError::Invalid(
                "a session can't end before it starts".to_string(),
            ));
        }
    }
    let mut edits: Vec<(usize, Moment)> = vec![(session.in_line, start.clone())];
    if let (Some(out), Some(end)) = (session.out_line, end) {
        edits.push((out, end.clone()));
    }
    edits.sort_by_key(|(line, _)| std::cmp::Reverse(*line));
    let mut new_text = text.to_string();
    for (line, at) in edits {
        let l = &file.lines[line];
        let content = &text[l.start..l.end];
        let replaced = retimed(content, &at)
            .ok_or_else(|| TimeError::Verify("could not retime a clock line".to_string()))?;
        new_text = format!("{}{replaced}{}", &new_text[..l.start], &new_text[l.end..]);
    }
    let mut want = sessions_of(&file);
    want[index].start = start.clone();
    if let Some(end) = end {
        want[index].end = Some(end.clone());
    }
    verify_clock(&new_text, want)?;
    Ok(new_text)
}

/// Remove session `index`: its `i` line, the indented comments under it,
/// and its `o` line; every other line kept byte for byte.
///
/// # Errors
/// [`TimeError::NotFound`]; [`TimeError::Verify`].
pub fn remove_session(text: &str, index: usize) -> Result<String, TimeError> {
    let file = timeclock::parse(text);
    let session = file
        .sessions
        .get(index)
        .ok_or_else(|| TimeError::NotFound(format!("no session {index}")))?;
    let mut doomed = vec![session.in_line];
    doomed.extend(
        (session.in_line + 1..file.lines.len())
            .take_while(|&at| matches!(file.lines[at].kind, ClockLineKind::SessionComment(_))),
    );
    doomed.extend(session.out_line);
    doomed.sort_unstable();
    let mut new_text = String::with_capacity(text.len());
    let mut at = 0;
    for line in doomed {
        let l = &file.lines[line];
        new_text.push_str(&text[at..l.start]);
        at = l.next;
    }
    new_text.push_str(&text[at..]);
    let mut want = sessions_of(&file);
    want.remove(index);
    verify_clock(&new_text, want)?;
    Ok(new_text)
}

/// Set (or remove) `tag` on session `index` (T-entries: tags go on the `o`
/// line). A tag already on one of the session's lines is changed where it
/// is; a new one goes on the `o` line, or the `i` line of an open session.
/// Removing takes it off every line of the session.
///
/// # Errors
/// [`TimeError::NotFound`]; [`TimeError::Invalid`]; [`TimeError::Verify`].
pub fn set_session_tag(
    text: &str,
    index: usize,
    tag: &str,
    value: Option<&str>,
) -> Result<String, TimeError> {
    if !is_tag_name(tag) {
        return Err(TimeError::Invalid(format!("`{tag}` is not a tag name")));
    }
    if let Some(value) = value {
        check_tag_value(value)?;
    }
    let file = timeclock::parse(text);
    let session = file
        .sessions
        .get(index)
        .ok_or_else(|| TimeError::NotFound(format!("no session {index}")))?;
    let next_in = file
        .sessions
        .get(index + 1)
        .map_or(file.lines.len(), |next| next.in_line);
    let lines: Vec<usize> = (session.in_line..next_in.max(session.in_line + 1))
        .filter(|&at| {
            at == session.in_line
                || Some(at) == session.out_line
                || matches!(file.lines[at].kind, ClockLineKind::SessionComment(_))
                    && session.out_line.is_none_or(|out| at < out)
        })
        .collect();
    let comment_on = |at: usize| -> (usize, String) {
        let line = &file.lines[at];
        let content = &text[line.start..line.end];
        match content.find(';') {
            Some(semi) => (semi, content[semi + 1..].to_string()),
            None => (content.trim_end().len(), String::new()),
        }
    };
    let has = |at: usize| parse_tags(&comment_on(at).1).iter().any(|(n, _)| n == tag);
    let targets: Vec<usize> = match value {
        None => lines.iter().copied().filter(|&at| has(at)).collect(),
        Some(_) => vec![
            lines
                .iter()
                .copied()
                .find(|&at| has(at))
                .unwrap_or(session.out_line.unwrap_or(session.in_line)),
        ],
    };
    let mut new_text = text.to_string();
    for &at in targets.iter().rev() {
        let line = &file.lines[at];
        let content = &text[line.start..line.end];
        let (semi, comment) = comment_on(at);
        let updated = with_tag(&comment, tag, value);
        let updated = updated.as_str();
        let head = content[..semi].trim_end();
        let rebuilt = match (
            updated.is_empty(),
            matches!(line.kind, ClockLineKind::SessionComment(_)),
        ) {
            (true, true) => None,
            (true, false) => Some(head.to_string()),
            (false, true) => Some(format!("{head} {updated}")),
            (false, false) => Some(format!("{head}  ; {updated}")),
        };
        new_text = match rebuilt {
            Some(rebuilt) => format!(
                "{}{rebuilt}{}",
                &new_text[..line.start],
                &new_text[line.end..]
            ),
            None => format!("{}{}", &new_text[..line.start], &new_text[line.next..]),
        };
    }
    // `upsert_tag` changes the first occurrence on the line it edits, which
    // is the session's first occurrence (tags are in line order).
    let mut want = sessions_of(&file);
    let mut tags = session.tags.clone();
    match value {
        None => tags.retain(|(name, _)| name != tag),
        Some(value) => match tags.iter_mut().find(|(name, _)| name == tag) {
            Some(found) => found.1 = value.trim().to_string(),
            None => tags.push((tag.to_string(), value.trim().to_string())),
        },
    }
    want[index].tags = sorted(tags);
    verify_clock(&new_text, want)?;
    Ok(new_text)
}

// ---------------------------------------------------------------------------
// Renaming an account (T-accounts)
// ---------------------------------------------------------------------------

/// One line a rename changes, for the preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedLine {
    /// 1-based line number.
    pub line: u32,
    /// The line before.
    pub before: String,
    /// The line after.
    pub after: String,
}

fn renamed(account: &str, old: &str, new: &str) -> Option<String> {
    super::model::is_under(account, old).then(|| format!("{new}{}", &account[old.len()..]))
}

/// Rename `old` and every account below it to `new` in a timedot file: each
/// entry and marker naming one, re-aligned; tags (an `invoice:` above all)
/// and every other line kept.
///
/// # Errors
/// [`TimeError::Invalid`] for a bad name; [`TimeError::Verify`].
pub fn rename_in_timedot(
    text: &str,
    old: &str,
    new: &str,
) -> Result<(String, Vec<RenamedLine>), TimeError> {
    check_account(new)?;
    let file = timedot::parse(text);
    let mut out = text.to_string();
    let mut changes = Vec::new();
    let mut preview = Vec::new();
    for at in (0..file.lines.len()).rev() {
        let line = &file.lines[at];
        let content = line.content(text);
        let (rendered, item) = match &line.kind {
            LineKind::Entry(entry) => {
                let Some(to) = renamed(&entry.account, old, new) else {
                    continue;
                };
                let prefix = &content[..entry.layout.account_start];
                let amount = amount_text(text, &file, at);
                let edge = day_edge(text, &file, entry.day, Some(at)).unwrap_or(AMOUNT_EDGE);
                let rendered = render_entry(prefix, &to, amount, comment_of(entry), edge);
                let Item::Entry {
                    parts, tags, note, ..
                } = entry_item(entry)
                else {
                    continue;
                };
                (
                    rendered,
                    Item::Entry {
                        account: to,
                        parts,
                        tags,
                        note,
                    },
                )
            }
            LineKind::Marker(marker) => {
                let Some(to) = renamed(&marker.account, old, new) else {
                    continue;
                };
                let indent = &content[..content.len() - content.trim_start().len()];
                (
                    marker_text(indent, &to, marker.since),
                    Item::Marker {
                        account: to,
                        since: marker.since,
                    },
                )
            }
            _ => continue,
        };
        preview.push(RenamedLine {
            line: line.number,
            before: content.to_string(),
            after: rendered.clone(),
        });
        out = format!("{}{rendered}{}", &out[..line.start], &out[line.end..]);
        changes.push(Change::Replace(at, item));
    }
    verify(text, &file, &out, &changes)?;
    preview.reverse();
    Ok((out, preview))
}

/// [`rename_in_timedot`] for a timeclock file: the account on each `i` and
/// `o` line naming one.
///
/// # Errors
/// As [`rename_in_timedot`].
pub fn rename_in_timeclock(
    text: &str,
    old: &str,
    new: &str,
) -> Result<(String, Vec<RenamedLine>), TimeError> {
    check_account(new)?;
    let file = timeclock::parse(text);
    let mut out = text.to_string();
    let mut preview = Vec::new();
    for line in file.lines.iter().rev() {
        let account = match &line.kind {
            ClockLineKind::In(clock_in) => Some(clock_in.account.as_str()),
            ClockLineKind::Out(clock_out) => clock_out.account.as_deref(),
            _ => None,
        };
        let Some((account, to)) = account.and_then(|a| renamed(a, old, new).map(|to| (a, to)))
        else {
            continue;
        };
        let content = &text[line.start..line.end];
        // The account follows the date and time: the first match after the
        // third word.
        let after_time = content
            .match_indices(char::is_whitespace)
            .filter(|(at, _)| content[..*at].split_whitespace().count() == 3)
            .map(|(at, _)| at)
            .next()
            .unwrap_or(content.len());
        let Some(found) = content[after_time..]
            .find(account)
            .map(|at| at + after_time)
        else {
            continue;
        };
        let rendered = format!(
            "{}{to}{}",
            &content[..found],
            &content[found + account.len()..]
        );
        preview.push(RenamedLine {
            line: line.number,
            before: content.to_string(),
            after: rendered.clone(),
        });
        out = format!("{}{rendered}{}", &out[..line.start], &out[line.end..]);
    }
    let want = sessions_of(&file)
        .into_iter()
        .map(|mut session| {
            if let Some(to) = renamed(&session.account, old, new) {
                session.account = to;
            }
            session
        })
        .collect();
    verify_clock(&out, want)?;
    preview.reverse();
    Ok((out, preview))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Time — pat — November 2026\n# Written by Ledgeline.\n\n\
2026-11-25 Wed  ; person: pat\n    clients:acme:tax-return         130m  ; reviewed schedules C and E\n    internal:admin                   15m\n\n\
2026-11-26 Thu  ; person: pat\n    clients:acme:tax-return          45m  ; invoice: 108\n    clients:acme:tax-return          20m  ; call with Fred\n    clients:acme:advisory             1h\n";

    fn style<'a>(tags: &'a TagNames, words: &'a Words, person: Option<&'a str>) -> DayStyle<'a> {
        DayStyle {
            tags,
            words,
            weekday: WeekdayStyle::Short,
            person,
        }
    }

    fn lines_of(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    #[test]
    fn adds_to_the_open_line_and_never_a_billed_one() {
        let (tags, words) = (TagNames::default(), Words::default());
        let s = style(&tags, &words, Some("pat"));
        let added =
            add_minutes(FILE, "2026-11-26", "clients:acme:tax-return", 25, &s).expect("add");
        assert_eq!(added.applied, 25);
        let lines = lines_of(&added.text);
        assert_eq!(
            lines[8],
            "    clients:acme:tax-return          45m  ; invoice: 108"
        );
        assert_eq!(
            lines[9],
            "    clients:acme:tax-return          45m  ; call with Fred"
        );
        // Every other line is untouched.
        let before = lines_of(FILE);
        for (n, line) in lines.iter().enumerate().filter(|(n, _)| *n != 9) {
            assert_eq!(*line, before[n]);
        }
        let added = add_minutes(FILE, "2026-11-26", "clients:acme:advisory", 30, &s).expect("add");
        assert_eq!(
            lines_of(&added.text)[10],
            "    clients:acme:advisory            90m"
        );
        let added = add_minutes(FILE, "2026-11-26", "clients:acme:advisory", -90, &s).expect("sub");
        assert_eq!(added.applied, -60);
        assert!(!added.text.contains("advisory"));
        let added = add_minutes(FILE, "2026-11-26", "internal:sales", 15, &s).expect("new line");
        assert!(added.text.ends_with(
            "    clients:acme:advisory             1h\n    internal:sales                   15m\n"
        ));
        let unchanged = add_minutes(FILE, "2026-11-26", "internal:sales", -5, &s).expect("noop");
        assert_eq!(unchanged.applied, 0);
        assert_eq!(unchanged.text, FILE);
    }

    #[test]
    fn new_days_go_in_date_order() {
        let (tags, words) = (TagNames::default(), Words::default());
        let s = style(&tags, &words, Some("pat"));
        let text = insert_entry(
            FILE,
            "2026-11-27",
            "internal:admin",
            TimeQty::from_minutes(120),
            "filing",
            &[],
            &s,
        )
        .expect("append");
        assert!(text.ends_with("1h\n\n2026-11-27 Fri  ; person: pat\n    internal:admin                    2h  ; filing\n"), "{text}");
        let text = insert_entry(
            FILE,
            "2026-11-24",
            "internal:admin",
            TimeQty::from_minutes(5),
            "",
            &[],
            &s,
        )
        .expect("before");
        assert!(text.contains("\n\n2026-11-24 Tue  ; person: pat\n    internal:admin                    5m\n\n2026-11-25 Wed"), "{text}");
        let none = DayStyle {
            person: None,
            weekday: WeekdayStyle::None,
            ..s
        };
        let text = insert_entry(
            "",
            "2026-11-24",
            "a",
            TimeQty::from_minutes(5),
            "",
            &[],
            &none,
        )
        .expect("empty");
        assert_eq!(
            text,
            "2026-11-24\n    a                                 5m\n"
        );
        // Someone else's day is not ours to write into.
        let sam = style(&tags, &words, Some("sam"));
        let text = insert_entry(
            FILE,
            "2026-11-26",
            "a",
            TimeQty::from_minutes(5),
            "",
            &[],
            &sam,
        )
        .expect("sam");
        assert!(text.ends_with(
            "2026-11-26 Thu  ; person: sam\n    a                                 5m\n"
        ));
        assert!(
            insert_entry(
                FILE,
                "2026-11-26",
                "a;b",
                TimeQty::from_minutes(5),
                "",
                &[],
                &s
            )
            .is_err()
        );
        assert!(
            insert_entry(
                FILE,
                "2026-11-26",
                "a",
                TimeQty::from_minutes(5),
                "re: K-1",
                &[],
                &s
            )
            .is_err()
        );
    }

    #[test]
    fn tags_and_notes_on_odd_lines() {
        let odd =
            "* Org\n** 2026-11-01\n*** clients:acme  ....  ; letters\n    x\tdots  ..\n  y  ab\n";
        let file = timedot::parse(odd);
        assert!(!file.has_errors());
        let text = set_entry_tag(odd, 2, "invoice", Some("108")).expect("tag");
        // The day's (ragged) column is kept; only the tag is new.
        assert_eq!(
            lines_of(&text)[2],
            "*** clients:acme  ....  ; letters, invoice: 108"
        );
        let text = set_entry_tag(odd, 4, "invoice", Some("9")).expect("letters");
        assert_eq!(lines_of(&text)[4], "  y                 ab  ; invoice: 9");
        let text = set_entry_tag(&text, 4, "invoice", None).expect("untag");
        assert_eq!(lines_of(&text)[4], "  y                 ab");
        let text = set_note(FILE, 8, "first pass").expect("note");
        assert_eq!(
            lines_of(&text)[8],
            "    clients:acme:tax-return          45m  ; first pass, invoice: 108"
        );
        let text = set_note(&text, 8, "").expect("clear");
        assert_eq!(
            lines_of(&text)[8],
            "    clients:acme:tax-return          45m  ; invoice: 108"
        );
        assert!(set_entry_tag(FILE, 0, "invoice", Some("1")).is_err());
    }

    #[test]
    fn markers_start_and_stop() {
        let (tags, words) = (TagNames::default(), Words::default());
        let s = style(&tags, &words, Some("pat"));
        let text =
            start_marker(FILE, "2026-11-26", "clients:acme:advisory", 50_712, &s).expect("start");
        assert!(text.ends_with("1h\n    ; ● running clients:acme:advisory  since 14:05:12\n"));
        assert!(start_marker(&text, "2026-11-26", "x", 1, &s).is_err());
        let file = timedot::parse(&text);
        let (at, _) = file.markers().next().expect("marker");
        assert_eq!(remove_marker(&text, at).expect("stop"), FILE);
        let text = start_marker(FILE, "2026-11-28", "x", 60, &s).expect("new day");
        assert!(
            text.ends_with(
                "\n\n2026-11-28 Sat  ; person: pat\n    ; ● running x  since 00:01:00\n"
            )
        );
    }

    #[test]
    fn stop_rounds_half_up_and_conserves_across_midnight() {
        let at = |date: &str, h: u32, m: u32, s: u32| Moment::new(date, h * 3600 + m * 60 + s);
        assert!(stop_plan(&at("2026-11-26", 9, 0, 0), &at("2026-11-26", 9, 0, 29)).is_empty());
        assert_eq!(
            stop_plan(&at("2026-11-26", 9, 0, 0), &at("2026-11-26", 9, 0, 30)),
            vec![("2026-11-26".into(), 1)]
        );
        assert_eq!(
            stop_plan(&at("2026-11-26", 9, 0, 0), &at("2026-11-26", 9, 1, 20)),
            vec![("2026-11-26".into(), 1)]
        );
        assert_eq!(
            stop_plan(&at("2026-11-26", 9, 0, 0), &at("2026-11-26", 9, 1, 30)),
            vec![("2026-11-26".into(), 2)]
        );
        assert_eq!(
            stop_plan(&at("2026-11-30", 23, 40, 40), &at("2026-12-01", 0, 20, 0)),
            vec![("2026-11-30".into(), 19), ("2026-12-01".into(), 20)]
        );
    }

    #[test]
    fn tidy_realigns_everything_and_keeps_meaning() {
        let messy = "2026-11-01 Sun\n  a  1.5\n\tb:c\t\t....  ;  note\n        d  ab\n  e  0.33\n    ; ● running a  since 9:00\n";
        let text = tidy(messy).expect("tidy");
        assert_eq!(
            text,
            "2026-11-01 Sun\n    a                                90m\n    b:c                               1h  ; note\n    d                                 ab\n    e                               0.33\n    ; ● running a  since 09:00:00\n"
        );
        assert_eq!(tidy(&text).expect("again"), text);
    }

    #[test]
    fn entries_change_and_go_and_sessions_go() {
        let text = set_entry(FILE, 8, None, Some(TimeQty::from_minutes(90))).expect("qty");
        assert_eq!(
            lines_of(&text)[8],
            "    clients:acme:tax-return          90m  ; invoice: 108"
        );
        let text = set_entry(&text, 9, Some("clients:acme:advisory"), None).expect("move");
        assert_eq!(
            lines_of(&text)[9],
            "    clients:acme:advisory            20m  ; call with Fred"
        );
        assert!(set_entry(&text, 7, None, None).is_err(), "a date line");
        assert!(set_entry(&text, 8, None, Some(TimeQty::from_minutes(-5))).is_err());
        let text = remove_entry(&text, 10).expect("remove");
        assert_eq!(lines_of(&text).len(), 10);
        assert!(!text.contains("1h"));
        assert!(remove_entry(&text, 6).is_err(), "a blank line");
        let letters = "2026-11-26\n    a  aab\n";
        assert!(set_entry(letters, 1, None, Some(TimeQty::from_minutes(5))).is_err());

        let clock = "i 2026-11-26 09:00:00 a  x\n  ; person: pat\no 2026-11-26 10:00:00\n\
i 2026-11-26 11:00:00 b\no 2026-11-26 11:30:00\n";
        let text = remove_session(clock, 0).expect("remove");
        assert_eq!(text, "i 2026-11-26 11:00:00 b\no 2026-11-26 11:30:00\n");
        assert!(remove_session(clock, 2).is_err());
    }

    #[test]
    fn timeclock_edits_keep_seconds() {
        let start = Moment::new("2026-11-26", 9 * 3600 + 5);
        let text = clock_in(
            "",
            &start,
            "clients:acme",
            "audit",
            &[("person".into(), "pat".into())],
        )
        .expect("in");
        assert_eq!(
            text,
            "i 2026-11-26 09:00:05 clients:acme  audit  ; person: pat\n"
        );
        assert!(clock_in(&text, &start, "clients:acme", "", &[]).is_err());
        let end = Moment::new("2026-11-26", 10 * 3600 + 30 * 60 + 17);
        let text = clock_out(&text, &end, "clients:acme").expect("out");
        assert!(text.ends_with("o 2026-11-26 10:30:17 clients:acme\n"));
        let file = timeclock::parse(&text);
        assert_eq!(file.sessions[0].duration(None), TimeQty::from_seconds(5412));
        let text = set_session_tag(&text, 0, "invoice", Some("108")).expect("tag");
        assert!(text.ends_with("o 2026-11-26 10:30:17 clients:acme  ; invoice: 108\n"));
        let text = set_session_tag(&text, 0, "person", None).expect("untag");
        assert!(text.starts_with("i 2026-11-26 09:00:05 clients:acme  audit\n"));
        let text = set_session(&text, 0, &Moment::new("2026-11-26", 8 * 3600), Some(&end))
            .expect("retime");
        assert!(text.starts_with("i 2026-11-26 08:00:00 clients:acme  audit\n"));
        assert!(clock_out(&text, &end, "clients:acme").is_err());
        let (renamed, preview) = rename_in_timeclock(&text, "clients", "kunden").expect("rename");
        assert_eq!(preview.len(), 2);
        assert!(renamed.starts_with("i 2026-11-26 08:00:00 kunden:acme  audit\n"));
    }

    #[test]
    fn renames_entries_and_markers() {
        let text = format!("{FILE}    ; ● running clients:acme:advisory  since 14:05:12\n");
        let (out, preview) =
            rename_in_timedot(&text, "clients:acme", "clients:acme-corp").expect("rename");
        assert_eq!(preview.len(), 5);
        assert!(out.contains("    clients:acme-corp:tax-return     45m  ; invoice: 108\n"));
        assert!(out.ends_with("; ● running clients:acme-corp:advisory  since 14:05:12\n"));
        assert!(out.contains("    internal:admin                   15m\n"));
    }
}
