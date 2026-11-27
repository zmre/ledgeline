//! The slot tracker (plan 46 T8, T-slots, T-running, T-light).
//!
//! - [`state`]: what Track and the sidebar lamp show — ten slots with today's
//!   totals, the running timer (with the long-running guard), today's total,
//!   the resume hint, and this week's days.
//! - [`toggle`], [`stop`], [`adjust`], [`snap`], [`note`]: the tracker's
//!   writes. Each is one verified save through [`TimeBooks::write`]
//!   (switching is stop + start in one save; T-running).
//!
//! "Mine" is the person's (P-person): their running marker and their time.
//! With no person set, time with no person is mine.

use super::{Qty, TimeBooks, TimeOpError, TimePrefs, Written, check_note, moment_from_epoch};
use ledgeline_core::reports::periods::{Interval, add_days, bucket_key, bucket_start};
use ledgeline_core::time::book::RecordMode;
use ledgeline_core::time::files::{TimeFiles, Tracker};
use ledgeline_core::time::model::{ParsedTime, Running, TimeBook};
use ledgeline_core::time::timeclock::{self, Moment};
use ledgeline_core::time::{TimeFormat, TimeQty, check_account, format_clock, timedot, write};
use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use std::collections::BTreeMap;

/// How many slots there are: `1`–`9`, `0`.
pub const SLOTS: usize = 10;
/// How many of them the user pins (1–5).
pub const PINNED: usize = 5;

/// Why a slot holds its account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlotKind {
    /// Pinned by the user (1–5).
    Pinned,
    /// A recent account not pinned (6–9, and empty pins).
    Recent,
    /// Slot 0: *Other…*, showing the last account chosen through it.
    Other,
}

/// One slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Slot {
    /// The key: 1–9, then 0.
    pub key: u8,
    /// Pinned, recent or other.
    pub kind: SlotKind,
    /// The account; `None` for an empty slot (or Other… with nothing chosen).
    pub account: Option<String>,
    /// For people: `Acme Corp · tax return`.
    pub label: Option<String>,
    /// Mine today, banked (the running timer is in [`TrackState::running`]).
    pub today: Qty,
    /// Whether it is the running one.
    pub running: bool,
    /// The latest note on it today.
    pub last_note: Option<String>,
    /// When it was last used (for recents): ISO date.
    pub last_used: Option<String>,
}

/// A running timer, as the lamp shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunningView {
    /// The account.
    pub account: String,
    /// For people.
    pub label: String,
    /// The slot holding it, if any.
    pub slot: Option<u8>,
    /// When it started: `YYYY-MM-DDTHH:MM:SS`.
    pub since: String,
    /// How long it has run.
    pub elapsed: Qty,
    /// Whose it is.
    pub person: Option<String>,
    /// The file its marker (or `i` line) is in.
    pub file: VaultPath,
    /// `timedot` or `timeclock`.
    pub format: &'static str,
}

/// The long-running guard (T-running): a timer older than the threshold,
/// or started on an earlier day, asks *Keep · Stop at… · Discard*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Guard {
    /// How long it has run.
    pub age: Qty,
    /// Whether it started before today.
    pub previous_day: bool,
    /// The threshold, in minutes (0: only the previous-day rule).
    pub threshold_minutes: i64,
    /// The latest moment mine is known to have stopped before it (the end
    /// of my latest timeclock session), for *Stop at ‹last activity›*.
    pub last_activity: Option<String>,
}

/// The resume hint (T-light, stopped): the most recent account and its key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Last {
    /// The account.
    pub account: String,
    /// For people.
    pub label: String,
    /// Its slot, if it has one.
    pub slot: Option<u8>,
    /// The date it was last used.
    pub date: String,
}

/// One day of the week strip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Day {
    /// ISO date.
    pub date: String,
    /// Mine that day (the running timer included today).
    pub total: Qty,
}

/// What Track shows (`GET /api/time/state`, `time status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackState {
    /// Whether the roll-up exists.
    pub configured: bool,
    /// Who is tracking.
    pub person: Option<String>,
    /// `timedot` or `timeclock` (T13).
    pub record: &'static str,
    /// The date the state is for.
    pub today: String,
    /// The moment it was computed: `YYYY-MM-DDTHH:MM:SS`.
    pub now: String,
    /// The `+`/`−` step, in minutes.
    pub step: i64,
    /// Slots 1–9, then 0.
    pub slots: Vec<Slot>,
    /// My running timer.
    pub running: Option<RunningView>,
    /// Everyone else's (another person on a shared folder).
    pub others_running: Vec<RunningView>,
    /// When mine is long-running.
    pub guard: Option<Guard>,
    /// Mine today, banked.
    pub banked_today: Qty,
    /// Mine today with the running timer.
    pub today_total: Qty,
    /// The resume hint.
    pub last: Option<Last>,
    /// Monday to Sunday of this week.
    pub week: Vec<Day>,
    /// The week's target, in hours (0 is none).
    pub weekly_target: i64,
    /// What the parsers found, counted (Entries lists them).
    pub findings: usize,
    /// Where the tracker writes today.
    pub track_file: Option<String>,
}

fn timestamp(at: &Moment) -> String {
    format!("{}T{}", at.date, format_clock(at.seconds))
}

/// Mine: `person`'s, or with no person, what has none.
fn is_mine(of: Option<&str>, person: Option<&str>) -> bool {
    of == person
}

fn my_running<'b>(book: &'b TimeBook, person: Option<&str>) -> Option<&'b Running> {
    book.running
        .iter()
        .find(|running| is_mine(running.person.as_deref(), person))
}

/// `tax-return` → `tax return`.
fn words_of(segment: &str) -> String {
    segment.replace(['-', '_'], " ")
}

/// An account for people (T-slots): `Acme Corp · tax return` when it bills
/// a customer, else `Internal · admin`.
#[must_use]
pub fn label(book: &TimeBook, account: &str) -> String {
    let leaf = account.rsplit(':').next().unwrap_or(account);
    let top = account.split(':').next().unwrap_or(account);
    let customer = book
        .account(account)
        .and_then(|a| a.customer_name())
        .map(str::to_string);
    let head = customer.unwrap_or_else(|| {
        let mut chars = words_of(top).chars().collect::<Vec<_>>();
        if let Some(first) = chars.first_mut() {
            *first = first.to_ascii_uppercase();
        }
        chars.into_iter().collect()
    });
    if account.contains(':') {
        format!("{head} · {}", words_of(leaf))
    } else {
        head
    }
}

/// My accounts by last use (latest date, then latest in file order),
/// skipping `exclude` and archived accounts.
fn my_recents(book: &TimeBook, person: Option<&str>, exclude: &[&str]) -> Vec<(String, String)> {
    let mut last: BTreeMap<&str, (&str, usize)> = BTreeMap::new();
    for (at, entry) in book.entries.iter().enumerate() {
        if !is_mine(entry.person.as_deref(), person) {
            continue;
        }
        let seen = last
            .entry(entry.account.as_str())
            .or_insert((&entry.date, at));
        if (entry.date.as_str(), at) > *seen {
            *seen = (&entry.date, at);
        }
    }
    let mut ranked: Vec<(&str, (&str, usize))> = last
        .into_iter()
        .filter(|(account, _)| {
            !exclude.contains(account) && !book.account(account).is_some_and(|a| a.is_archived())
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked
        .into_iter()
        .map(|(account, (date, _))| (account.to_string(), date.to_string()))
        .collect()
}

/// The accounts in slots 1–9 and 0 (T8): the pins, empty pins filled from
/// recents, recents in 6–9, and slot 0 the last *Other…* choice.
#[must_use]
pub fn slot_accounts(book: &TimeBook, prefs: &TimePrefs) -> Vec<(u8, SlotKind, Option<String>)> {
    let pins: Vec<Option<String>> = (0..PINNED)
        .map(|at| {
            prefs
                .pinned
                .get(at)
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
        })
        .collect();
    let mut exclude: Vec<&str> = pins.iter().flatten().map(String::as_str).collect();
    exclude.extend(prefs.other.as_deref());
    let mut recents = my_recents(book, prefs.person.as_deref(), &exclude)
        .into_iter()
        .map(|(account, _)| account);
    let mut slots = Vec::with_capacity(SLOTS);
    for (at, pin) in pins.into_iter().enumerate() {
        let key = u8::try_from(at + 1).unwrap_or(0);
        match pin {
            Some(account) => slots.push((key, SlotKind::Pinned, Some(account))),
            None => slots.push((key, SlotKind::Recent, recents.next())),
        }
    }
    for key in 6..=9 {
        slots.push((key, SlotKind::Recent, recents.next()));
    }
    slots.push((0, SlotKind::Other, prefs.other.clone()));
    slots
}

/// The slot holding `account`.
#[must_use]
pub fn slot_of(slots: &[(u8, SlotKind, Option<String>)], account: &str) -> Option<u8> {
    slots
        .iter()
        .find(|(_, _, a)| a.as_deref() == Some(account))
        .map(|(key, _, _)| *key)
}

fn running_view(
    book: &TimeBook,
    running: &Running,
    slots: &[(u8, SlotKind, Option<String>)],
    now: &Moment,
) -> RunningView {
    let elapsed = now.epoch_seconds() - running.since.epoch_seconds();
    RunningView {
        account: running.account.clone(),
        label: label(book, &running.account),
        slot: slot_of(slots, &running.account),
        since: timestamp(&running.since),
        elapsed: TimeQty::from_seconds(elapsed.max(0)).into(),
        person: running.person.clone(),
        file: running.source.file.clone(),
        format: running.format.name(),
    }
}

/// Seconds of `running` that fall on `date`, up to `now`.
fn running_on(running: &Running, date: &str, now: &Moment) -> i64 {
    let day_start = Moment::new(date, 0).epoch_seconds();
    let day_end = day_start + 86_400;
    let start = running.since.epoch_seconds().max(day_start);
    let end = now.epoch_seconds().min(day_end);
    (end - start).max(0)
}

/// The end of my latest timeclock session on `before`'s day, at or before
/// it: the previous entry's end, for snap and *Stop at ‹last activity›*.
fn last_clock_out(book: &TimeBook, person: Option<&str>, before: &Moment) -> Option<Moment> {
    book.sources
        .iter()
        .filter_map(|source| match &source.parsed {
            ParsedTime::Timeclock(file) => Some(file),
            ParsedTime::Timedot(_) => None,
        })
        .flat_map(|file| file.sessions.iter())
        .filter(|session| {
            let theirs = book
                .tags
                .value(&session.tags, ledgeline_core::time::TagRole::Person);
            theirs.is_none() || theirs == person
        })
        .filter_map(|session| session.end.clone())
        .filter(|end| end.date == before.date && end.epoch_seconds() <= before.epoch_seconds())
        .max_by_key(Moment::epoch_seconds)
}

/// What Track shows at `now`, for the person in `prefs`. `book` is `None`
/// before setup: every slot empty, `configured: false`. `stopped_last` is
/// the account the caller last stopped, when it knows (the app does; the
/// files don't say), for the resume hint; else the latest line is used.
#[must_use]
pub fn state(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    prefs: &TimePrefs,
    now: &Moment,
    stopped_last: Option<&str>,
) -> TrackState {
    let person = prefs.person.as_deref();
    let today = now.date.clone();
    let monday =
        bucket_start(&bucket_key(&today, Interval::Weekly)).unwrap_or_else(|_| today.clone());
    let week_dates: Vec<String> = (0..7).map(|d| add_days(&monday, d)).collect();
    let record = match books.config.record {
        RecordMode::Timedot => "timedot",
        RecordMode::Timeclock => "timeclock",
    };
    let track_file = books
        .tracker(person)
        .file_for(&today)
        .ok()
        .map(|path| books.paths.relative(&path));
    let Some(book) = book else {
        return TrackState {
            configured: false,
            person: prefs.person.clone(),
            record,
            today,
            now: timestamp(now),
            step: prefs.step,
            slots: Vec::new(),
            running: None,
            others_running: Vec::new(),
            guard: None,
            banked_today: TimeQty::ZERO.into(),
            today_total: TimeQty::ZERO.into(),
            last: None,
            week: week_dates
                .into_iter()
                .map(|date| Day {
                    date,
                    total: TimeQty::ZERO.into(),
                })
                .collect(),
            weekly_target: prefs.weekly_target,
            findings: 0,
            track_file,
        };
    };
    let accounts = slot_accounts(book, prefs);
    let mine = || {
        book.entries
            .iter()
            .filter(|entry| is_mine(entry.person.as_deref(), person))
    };
    let mut today_by_account: BTreeMap<&str, TimeQty> = BTreeMap::new();
    let mut notes: BTreeMap<&str, &str> = BTreeMap::new();
    let mut by_day: BTreeMap<&str, TimeQty> = BTreeMap::new();
    let mut last_used: BTreeMap<&str, &str> = BTreeMap::new();
    for entry in mine() {
        if entry.date == today {
            let slot = today_by_account.entry(&entry.account).or_default();
            *slot = slot.plus(entry.quantity);
            if !entry.note.is_empty() {
                notes.insert(&entry.account, &entry.note);
            }
        }
        if week_dates.contains(&entry.date) {
            let day = by_day.entry(&entry.date).or_default();
            *day = day.plus(entry.quantity);
        }
        let used = last_used.entry(&entry.account).or_insert(&entry.date);
        if entry.date.as_str() > *used {
            *used = &entry.date;
        }
    }
    let running = my_running(book, person);
    let slots = accounts
        .iter()
        .map(|(key, kind, account)| Slot {
            key: *key,
            kind: *kind,
            label: account.as_deref().map(|a| label(book, a)),
            today: today_by_account
                .get(account.as_deref().unwrap_or(""))
                .copied()
                .unwrap_or_default()
                .into(),
            running: running.is_some_and(|r| Some(r.account.as_str()) == account.as_deref()),
            last_note: account
                .as_deref()
                .and_then(|a| notes.get(a))
                .map(|n| (*n).to_string()),
            last_used: account
                .as_deref()
                .and_then(|a| last_used.get(a))
                .map(|d| (*d).to_string()),
            account: account.clone(),
        })
        .collect();
    let banked = TimeQty::sum(today_by_account.values().copied());
    let running_today = running.map_or(0, |r| running_on(r, &today, now));
    let guard = running.and_then(|r| {
        let age = now.epoch_seconds() - r.since.epoch_seconds();
        let previous_day = r.since.date < today;
        let over = prefs.long_running > 0 && age >= prefs.long_running * 60;
        (previous_day || over).then(|| Guard {
            age: TimeQty::from_seconds(age.max(0)).into(),
            previous_day,
            threshold_minutes: prefs.long_running,
            last_activity: last_clock_out(book, person, &r.since).map(|m| timestamp(&m)),
        })
    });
    let last = mine()
        .enumerate()
        .filter(|(_, entry)| stopped_last.is_none_or(|a| entry.account == a))
        .max_by(|(ia, a), (ib, b)| (a.date.as_str(), ia).cmp(&(b.date.as_str(), ib)))
        .map(|(_, entry)| Last {
            account: entry.account.clone(),
            label: label(book, &entry.account),
            slot: slot_of(&accounts, &entry.account),
            date: entry.date.clone(),
        });
    let week = week_dates
        .iter()
        .map(|date| {
            let banked = by_day.get(date.as_str()).copied().unwrap_or_default();
            let live = running.map_or(0, |r| running_on(r, date, now));
            Day {
                date: date.clone(),
                total: banked.plus(TimeQty::from_seconds(live)).into(),
            }
        })
        .collect();
    TrackState {
        configured: true,
        person: prefs.person.clone(),
        record,
        today: today.clone(),
        now: timestamp(now),
        step: prefs.step,
        slots,
        running: running.map(|r| running_view(book, r, &accounts, now)),
        others_running: book
            .running
            .iter()
            .filter(|r| !is_mine(r.person.as_deref(), person))
            .map(|r| running_view(book, r, &accounts, now))
            .collect(),
        guard,
        banked_today: banked.into(),
        today_total: banked.plus(TimeQty::from_seconds(running_today)).into(),
        last,
        week,
        weekly_target: prefs.weekly_target,
        findings: book.findings.len(),
        track_file,
    }
}

/// A stopped timer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stopped {
    /// The account.
    pub account: String,
    /// When it started.
    pub since: String,
    /// When it stopped.
    pub at: String,
    /// The minutes banked per day (none for a discard or under 30 s).
    pub days: Vec<(String, i64)>,
    /// Whether it was discarded (nothing banked).
    pub discarded: bool,
}

/// What a tracker write did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct TrackOutcome {
    /// The timer stopped, if one was.
    pub stopped: Option<Stopped>,
    /// The account started, if one was.
    pub started: Option<String>,
    /// Minutes really added by an adjust (less than asked at zero).
    pub applied: Option<i64>,
    /// A running timer's new start, after an adjust or snap.
    pub since: Option<String>,
}

/// The line index of the marker for `account` in a timedot `text`.
fn marker_line(text: &str, account: &str) -> Result<usize, TimeOpError> {
    timedot::parse(text)
        .markers()
        .find(|(_, marker)| marker.account == account)
        .map(|(line, _)| line)
        .ok_or_else(|| TimeOpError::Conflict(format!("{account} is no longer running")))
}

/// The session index of `running`'s `i` line in a timeclock `text`.
fn session_index(text: &str, running: &Running) -> Result<usize, TimeOpError> {
    let line = usize::try_from(running.source.lines.start)
        .unwrap_or(0)
        .saturating_sub(1);
    timeclock::parse(text)
        .sessions
        .iter()
        .position(|s| s.in_line == line && s.end.is_none())
        .ok_or_else(|| TimeOpError::Conflict(format!("{} is no longer running", running.account)))
}

fn stop_one(
    files: &mut TimeFiles,
    tracker: &Tracker<'_>,
    running: &Running,
    at: &Moment,
    discard: bool,
) -> Result<Stopped, TimeOpError> {
    if at.epoch_seconds() < running.since.epoch_seconds() {
        return Err(TimeOpError::BadRequest(format!(
            "{} can't stop at {}: it started at {}",
            running.account,
            timestamp(at),
            timestamp(&running.since)
        )));
    }
    let days = if discard {
        let file = &running.source.file;
        match running.format {
            TimeFormat::Timedot => files.edit(file, None, |text| {
                write::remove_marker(
                    text,
                    marker_line(text, &running.account)
                        .map_err(|e| ledgeline_core::time::TimeError::NotFound(e.to_string()))?,
                )
            })?,
            TimeFormat::Timeclock => files.edit(file, None, |text| {
                let index = session_index(text, running)
                    .map_err(|e| ledgeline_core::time::TimeError::NotFound(e.to_string()))?;
                write::remove_session(text, index)
            })?,
        }
        Vec::new()
    } else {
        files.stop(tracker, running, at)?
    };
    Ok(Stopped {
        account: running.account.clone(),
        since: timestamp(&running.since),
        at: timestamp(at),
        days,
        discarded: discard,
    })
}

/// Toggle `account` at `now` (T-slots): on the running slot it stops; on
/// any other it stops whatever runs, banks it, and starts this one — one
/// save, one lamp lit.
///
/// # Errors
/// [`TimeOpError`].
pub fn toggle(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    person: Option<&str>,
    account: &str,
    now: &Moment,
) -> Result<Written<TrackOutcome>, TimeOpError> {
    check_account(account)?;
    books.write(book, |book, files| {
        let tracker = books.tracker(person);
        let mut out = TrackOutcome::default();
        if let Some(running) = my_running(book, person) {
            out.stopped = Some(stop_one(files, &tracker, running, now, false)?);
            if running.account == account {
                return Ok(out);
            }
        }
        files.start(&tracker, account, now)?;
        out.started = Some(account.to_string());
        Ok(out)
    })
}

/// Start `account` at `now`, stopping whatever else runs; nothing when it
/// already runs (`time start` from a script is idempotent).
///
/// # Errors
/// [`TimeOpError`].
pub fn start(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    person: Option<&str>,
    account: &str,
    now: &Moment,
) -> Result<Written<TrackOutcome>, TimeOpError> {
    check_account(account)?;
    books.write(book, |book, files| {
        let tracker = books.tracker(person);
        let mut out = TrackOutcome::default();
        if let Some(running) = my_running(book, person) {
            if running.account == account {
                return Ok(out);
            }
            out.stopped = Some(stop_one(files, &tracker, running, now, false)?);
        }
        files.start(&tracker, account, now)?;
        out.started = Some(account.to_string());
        Ok(out)
    })
}

/// Stop my timer at `at` (default `now`; the long-running guard's *Stop
/// at…*), or discard it, banking nothing.
///
/// # Errors
/// [`TimeOpError::Conflict`] when nothing runs.
pub fn stop(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    person: Option<&str>,
    at: &Moment,
    discard: bool,
) -> Result<Written<TrackOutcome>, TimeOpError> {
    books.write(book, |book, files| {
        let running = my_running(book, person)
            .ok_or_else(|| TimeOpError::Conflict("no timer is running".to_string()))?;
        let tracker = books.tracker(person);
        Ok(TrackOutcome {
            stopped: Some(stop_one(files, &tracker, running, at, discard)?),
            ..TrackOutcome::default()
        })
    })
}

/// Move `running`'s start to `since` (never after `now`), in its file.
fn restart_at(
    files: &mut TimeFiles,
    tracker: &Tracker<'_>,
    running: &Running,
    since: &Moment,
) -> Result<(), TimeOpError> {
    match running.format {
        TimeFormat::Timedot => {
            files.edit(&running.source.file, None, |text| {
                let line = marker_line(text, &running.account)
                    .map_err(|e| ledgeline_core::time::TimeError::NotFound(e.to_string()))?;
                write::remove_marker(text, line)
            })?;
            files.start(tracker, &running.account, since)?;
        }
        TimeFormat::Timeclock => files.edit(&running.source.file, None, |text| {
            let index = session_index(text, running)
                .map_err(|e| ledgeline_core::time::TimeError::NotFound(e.to_string()))?;
            write::set_session(text, index, since, None)
        })?,
    }
    Ok(())
}

/// `+`/`−` (T-slots): add `minutes` (negative to take away) to `account`'s
/// time today. On the running slot it moves the start instead (earlier for
/// `+`), never past now; elsewhere it changes today's open line, never below
/// zero.
///
/// # Errors
/// [`TimeOpError`]; in timeclock mode only the running session can move.
pub fn adjust(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    person: Option<&str>,
    account: &str,
    minutes: i64,
    now: &Moment,
) -> Result<Written<TrackOutcome>, TimeOpError> {
    check_account(account)?;
    books.write(book, |book, files| {
        let tracker = books.tracker(person);
        if let Some(running) = my_running(book, person).filter(|r| r.account == account) {
            let since = (running.since.epoch_seconds() - minutes * 60).min(now.epoch_seconds());
            let since = moment_from_epoch(since);
            restart_at(files, &tracker, running, &since)?;
            return Ok(TrackOutcome {
                applied: Some(minutes),
                since: Some(timestamp(&since)),
                ..TrackOutcome::default()
            });
        }
        let applied = files.add_minutes(&tracker, &now.date, account, minutes)?;
        Ok(TrackOutcome {
            applied: Some(applied),
            ..TrackOutcome::default()
        })
    })
}

/// `⇧−` (T-slots): snap my running timer's start back to `to`, the end of
/// the previous entry. In timeclock mode `to` defaults to the end of my
/// latest session; a timedot file records no end times, so there the
/// caller says when (the app remembers its last stop).
///
/// # Errors
/// [`TimeOpError::Conflict`] when nothing runs or no earlier end is known.
pub fn snap(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    person: Option<&str>,
    to: Option<&Moment>,
) -> Result<Written<TrackOutcome>, TimeOpError> {
    books.write(book, |book, files| {
        let running = my_running(book, person)
            .ok_or_else(|| TimeOpError::Conflict("no timer is running".to_string()))?;
        let to = to
            .cloned()
            .or_else(|| last_clock_out(book, person, &running.since))
            .ok_or_else(|| {
                TimeOpError::Conflict(
                    "no earlier entry's end is known (a timedot file records no times); say when \
                     to snap to"
                        .to_string(),
                )
            })?;
        if to.epoch_seconds() >= running.since.epoch_seconds() {
            return Err(TimeOpError::Conflict(format!(
                "the previous entry ended at {}, after this timer started",
                timestamp(&to)
            )));
        }
        let tracker = books.tracker(person);
        restart_at(files, &tracker, running, &to)?;
        Ok(TrackOutcome {
            since: Some(timestamp(&to)),
            ..TrackOutcome::default()
        })
    })
}

/// Set the note of `account`'s open line on `date` (T-slots `n`), adding
/// a `0m` line to hold it when there is none yet (a timer that has banked
/// nothing). A note hledger would read as a tag is refused with a rewrite.
///
/// # Errors
/// [`TimeOpError::Note`]; [`TimeOpError::Time`]; timeclock files take notes
/// by hand.
pub fn note(
    books: &TimeBooks,
    book: Option<&TimeBook>,
    person: Option<&str>,
    account: &str,
    date: &str,
    text: &str,
) -> Result<Written<()>, TimeOpError> {
    check_account(account)?;
    check_note(text)?;
    books.write(book, |_, files| {
        let tracker = books.tracker(person);
        let path = tracker.file_for(date)?;
        if TimeFormat::of_path(path.as_str()) != Some(TimeFormat::Timedot) {
            return Err(TimeOpError::Conflict(
                "notes go on timedot lines; in timeclock mode write the session's comment by hand"
                    .to_string(),
            ));
        }
        let style = tracker.style();
        let header = ledgeline_core::time::book::track_header(
            person,
            date,
            TimeFormat::Timedot,
            &books.config,
        );
        files.edit(&path, Some(&header), |old| {
            let file = timedot::parse(old);
            match write::open_line(&file, date, account, &style) {
                Some(line) => write::set_note(old, line, text),
                None => write::insert_entry(old, date, account, TimeQty::ZERO, text, &[], &style),
            }
        })?;
        Ok(())
    })
}
