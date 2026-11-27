//! Time tracking (`plans/46-time-tracking.md`): timedot and timeclock files,
//! read and written the way hledger 1.52 reads them.
//!
//! Time lives in its own files, beside the books and never inside them unless
//! the user includes it. A **roll-up** journal (default `time.journal`)
//! includes the accounts file and the time files; whatever it includes is the
//! time book ([`book`]).
//!
//! - [`timedot`] and [`timeclock`] are lossless line-level parsers: every line
//!   is kept with its byte span and kind, so an edit rewrites only the lines it
//!   names ([`write`]).
//! - [`model`] is what the parsers' output becomes: [`model::TimeEntry`],
//!   [`model::TimeAccount`] with tags resolved nearest-ancestor-wins, and the
//!   loaded [`model::TimeBook`].
//! - [`query`] summarises a book: by period, account tree, customer, person
//!   and billable; unbilled and unpaid time; suggested billing periods.
//! - `crate::parse` reads `include`d time files into
//!   [`crate::model::Journal::time_transactions`], never into
//!   `transactions`, so no money report can see an hour (T12).
//!
//! Durations are [`TimeQty`], an exact count of nanoseconds. Every quantity a
//! timedot file can write (decimal hours or minutes with up to ten places,
//! dots, letters) and every timeclock session (whole seconds) is an exact
//! integer there, so Ledgeline never rounds stored time: hledger 1.52's
//! 0.01 h rounding is reported as a finding, not reproduced (T4).
//!
//! Tag names are **roles** ([`TagRole`]) looked up through [`TagNames`], which
//! `[time.tags]` can override (T15). No call site spells a tag literally.

pub mod accounts;
pub mod book;
pub mod files;
pub mod model;
pub mod query;
pub mod timeclock;
pub mod timedot;
pub mod write;

use crate::decimal::Dec;
use crate::view::wire::plain_decimal;
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

/// Why a time operation did nothing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimeError {
    /// A configured or requested path is unusable.
    #[error("`{path}` can't be used: {reason}")]
    Path {
        /// The path as given.
        path: String,
        /// Why.
        reason: String,
    },
    /// The roll-up (or a journal it includes) doesn't parse.
    #[error("the time roll-up does not parse: {0}")]
    Parse(String),
    /// `ledgeline.toml` can't be read or written; the phrase says why.
    #[error("ledgeline.toml {0}")]
    Settings(String),
    /// A file to be created already exists.
    #[error("{0} already exists")]
    Exists(String),
    /// A file the edit needs isn't there.
    #[error("{0} does not exist")]
    Missing(String),
    /// A value can't be written as asked; the message says why.
    #[error("{0}")]
    Invalid(String),
    /// Nothing matched: no such entry, line, marker or session.
    #[error("{0}")]
    NotFound(String),
    /// The edited text did not read back as intended, so nothing is written.
    #[error("the edit did not read back as intended: {0}")]
    Verify(String),
    /// A file changed on disk since it was read; nothing was written.
    #[error("{0} changed on disk since it was read; nothing was written")]
    ExternalChange(String),
    /// Storage failed.
    #[error("could not read or write {path}: {kind:?}")]
    Storage {
        /// The file.
        path: String,
        /// What failed.
        kind: std::io::ErrorKind,
    },
}

/// Nanoseconds per second.
const NANOS_PER_SECOND: i128 = 1_000_000_000;
/// Nanoseconds per minute.
const NANOS_PER_MINUTE: i128 = 60 * NANOS_PER_SECOND;
/// Nanoseconds per hour.
const NANOS_PER_HOUR: i128 = 60 * NANOS_PER_MINUTE;

/// A duration, exactly: a signed count of nanoseconds.
///
/// Negative values are legal (a timedot `-15m` correction). Arithmetic
/// saturates rather than overflows; the parsers refuse any single quantity
/// over [`TimeQty::MAX_HOURS`], so a realistic sum is never near the bound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeQty(i128);

impl TimeQty {
    /// The largest single quantity a parser accepts, in hours (about 114
    /// years). Anything larger is a typo, and refusing it keeps every sum far
    /// from `i128`'s bound.
    pub const MAX_HOURS: i128 = 1_000_000;

    /// Zero.
    pub const ZERO: Self = Self(0);

    /// Whole seconds.
    #[must_use]
    pub const fn from_seconds(seconds: i64) -> Self {
        Self(seconds as i128 * NANOS_PER_SECOND)
    }

    /// Whole minutes.
    #[must_use]
    pub const fn from_minutes(minutes: i64) -> Self {
        Self(minutes as i128 * NANOS_PER_MINUTE)
    }

    /// Raw nanoseconds.
    #[must_use]
    pub const fn from_nanos(nanos: i128) -> Self {
        Self(nanos)
    }

    /// `value` units of `unit_nanos` each, rounded half away from zero to the
    /// nanosecond (only a seconds value with ten decimal places needs it).
    /// `None` when it is out of range.
    #[must_use]
    pub fn from_dec(value: Dec, unit_nanos: i128) -> Option<Self> {
        let scale = 10i128.checked_pow(value.places)?;
        let product = value.mantissa.checked_mul(unit_nanos)?;
        let quotient = product / scale;
        let remainder = product % scale;
        let rounded = if remainder.unsigned_abs() * 2 >= scale.unsigned_abs() {
            quotient + product.signum()
        } else {
            quotient
        };
        (rounded.abs() <= Self::MAX_HOURS * NANOS_PER_HOUR).then_some(Self(rounded))
    }

    /// Nanoseconds.
    #[must_use]
    pub const fn nanos(self) -> i128 {
        self.0
    }

    /// Exact seconds, as a decimal.
    #[must_use]
    pub fn seconds(self) -> Dec {
        Dec::new(self.0, 9).normalized()
    }

    /// Minutes: exact whenever the value terminates within nine decimal
    /// places (every timedot quantity not in seconds does), otherwise rounded half away from
    /// zero to nine places (a 20 s timeclock session is 0.333333333 min).
    #[must_use]
    pub fn minutes(self) -> Dec {
        Self::ratio(self.0, NANOS_PER_MINUTE)
    }

    /// Hours, as [`Self::minutes`] is minutes.
    #[must_use]
    pub fn hours(self) -> Dec {
        Self::ratio(self.0, NANOS_PER_HOUR)
    }

    /// Hours as a float, for display and for tolerance comparisons only.
    #[must_use]
    pub fn hours_f64(self) -> f64 {
        self.0 as f64 / NANOS_PER_HOUR as f64
    }

    /// The whole number of minutes, rounded half up (T4: 30 s → 1 m, 1 m 20 s
    /// → 1 m, 1 m 30 s → 2 m). Negative durations round half away from zero.
    #[must_use]
    pub fn round_minutes(self) -> i64 {
        let half = NANOS_PER_MINUTE / 2;
        let rounded = if self.0 >= 0 {
            (self.0 + half) / NANOS_PER_MINUTE
        } else {
            (self.0 - half) / NANOS_PER_MINUTE
        };
        i64::try_from(rounded).unwrap_or(if rounded > 0 { i64::MAX } else { i64::MIN })
    }

    /// Whether this is a whole number of minutes.
    #[must_use]
    pub const fn is_whole_minutes(self) -> bool {
        self.0 % NANOS_PER_MINUTE == 0
    }

    /// Whether this is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// `self + other`, saturating.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// `self - other`, saturating.
    #[must_use]
    pub const fn minus(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// The sum of `items`.
    pub fn sum(items: impl IntoIterator<Item = Self>) -> Self {
        items.into_iter().fold(Self::ZERO, Self::plus)
    }

    /// What hledger 1.52 shows for this quantity when it came from a unit
    /// (`20m` → 0.33): hours rounded to two places, half away from zero.
    #[must_use]
    pub fn hledger_152_hours(self) -> Dec {
        self.hours().rounded(2).unwrap_or_else(|_| self.hours())
    }

    fn ratio(nanos: i128, per: i128) -> Dec {
        // `per` is 60e9 or 3600e9; scaling by 10^9 keeps nine places, and a
        // terminating ratio normalizes down to its own places.
        let scaled = nanos.saturating_mul(NANOS_PER_SECOND);
        let quotient = scaled / per;
        let remainder = scaled % per;
        let rounded = if remainder.unsigned_abs() * 2 >= per.unsigned_abs() {
            quotient + scaled.signum()
        } else {
            quotient
        };
        Dec::new(rounded, 9).normalized()
    }

    /// The amount as the writer spells it (T4): `Nh` when it is a whole number
    /// of hours, otherwise `Nm`; a non-whole number of minutes is written in
    /// minutes with its decimals (`24.8m`). Never `mo`/`y`, never a space.
    #[must_use]
    pub fn written(self) -> String {
        if self.0 != 0 && self.0 % NANOS_PER_HOUR == 0 {
            format!("{}h", self.0 / NANOS_PER_HOUR)
        } else if self.is_whole_minutes() {
            format!("{}m", self.0 / NANOS_PER_MINUTE)
        } else {
            format!("{}m", plain_decimal(self.minutes()))
        }
    }
}

impl fmt::Display for TimeQty {
    /// `1h 05m`, `45m`, `0m`, `-15m`: whole minutes, rounded half up, for
    /// people. Never parsed back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let minutes = self.round_minutes();
        let sign = if minutes < 0 { "-" } else { "" };
        let minutes = minutes.unsigned_abs();
        match (minutes / 60, minutes % 60) {
            (0, m) => write!(f, "{sign}{m}m"),
            (h, 0) => write!(f, "{sign}{h}h"),
            (h, m) => write!(f, "{sign}{h}h {m:02}m"),
        }
    }
}

/// What a tag means to the engine (T-vocab). Each role has a default name,
/// which `[time.tags]` may rename per books.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TagRole {
    /// `customer:` on a time account: the Invoicer customer it bills.
    Customer,
    /// `billable: yes|no` on a time account; default no.
    Billable,
    /// `item:` on a time account: the catalog code it bills as.
    Item,
    /// `archived: yes` on a time account: hidden from pickers.
    Archived,
    /// `person:` on a date line or entry: who did the work.
    Person,
    /// `invoice:` on an entry: billed on that invoice. The same name as the
    /// books' [`crate::accrual::INVOICE_TAG`].
    Invoice,
    /// `bill:` on an entry: paid through that AP bill (self-bill).
    Bill,
    /// `nobill:` on an entry: written off, never to be billed.
    Nobill,
    /// `approved:` on a date line: reserved for Phase 6 (B-states).
    Approved,
}

impl TagRole {
    /// Every role, in a stable order.
    pub const ALL: [Self; 9] = [
        Self::Customer,
        Self::Billable,
        Self::Item,
        Self::Archived,
        Self::Person,
        Self::Invoice,
        Self::Bill,
        Self::Nobill,
        Self::Approved,
    ];

    /// The role's key in `[time.tags]`, which is also its default name.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Customer => crate::accrual::CUSTOMER_TAG,
            Self::Billable => "billable",
            Self::Item => crate::accrual::ITEM_TAG,
            Self::Archived => "archived",
            Self::Person => "person",
            Self::Invoice => crate::accrual::INVOICE_TAG,
            Self::Bill => "bill",
            Self::Nobill => "nobill",
            Self::Approved => "approved",
        }
    }

    /// The role whose `[time.tags]` key is `key`.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.key() == key)
    }
}

/// The tag name each role is written and read as (T15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagNames(BTreeMap<TagRole, String>);

impl Default for TagNames {
    fn default() -> Self {
        Self(
            TagRole::ALL
                .into_iter()
                .map(|role| (role, role.key().to_string()))
                .collect(),
        )
    }
}

impl TagNames {
    /// The defaults with `overrides` applied (role key → name). Unknown keys
    /// and unusable names are skipped and returned as warnings.
    #[must_use]
    pub fn with_overrides<'a>(
        overrides: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> (Self, Vec<String>) {
        let mut names = Self::default();
        let mut warnings = Vec::new();
        for (key, name) in overrides {
            match TagRole::from_key(key) {
                Some(role) if is_tag_name(name) => {
                    names.0.insert(role, name.to_string());
                }
                Some(_) => warnings.push(format!(
                    "time.tags.{key} = \"{name}\" is not a usable tag name; ignored"
                )),
                None => warnings.push(format!("time.tags.{key} is not a time tag; ignored")),
            }
        }
        (names, warnings)
    }

    /// The name `role` is written as.
    #[must_use]
    pub fn name(&self, role: TagRole) -> &str {
        self.0.get(&role).map_or(role.key(), String::as_str)
    }

    /// The value of `role` among `tags` (the first, as hledger's `tag:` finds
    /// any), trimmed.
    #[must_use]
    pub fn value<'t>(&self, tags: &'t [(String, String)], role: TagRole) -> Option<&'t str> {
        let name = self.name(role);
        tags.iter()
            .find(|(tag, _)| tag == name)
            .map(|(_, value)| value.trim())
    }

    /// Whether `role` is present among `tags` at all (any value, even empty).
    #[must_use]
    pub fn has(&self, tags: &[(String, String)], role: TagRole) -> bool {
        let name = self.name(role);
        tags.iter().any(|(tag, _)| tag == name)
    }
}

/// Check an account name before it is written into a time file (F-files
/// writing rules): not empty, no `;` (it would start a comment), no tab or
/// double space (they end the name), no leading `*` (an org heading), no
/// leading or trailing space, no control character, and not date-like (a
/// timedot line starting with a date is a date line).
///
/// # Errors
/// [`TimeError::Invalid`], saying why.
pub fn check_account(name: &str) -> Result<(), TimeError> {
    let refuse = |why: &str| {
        Err(TimeError::Invalid(format!(
            "`{name}` can't be an account: {why}"
        )))
    };
    if name.is_empty() {
        return refuse("it is empty");
    }
    if name.contains(';') {
        return refuse("a `;` would start a comment");
    }
    if name.contains('\t') || name.contains("  ") {
        return refuse("a tab or two spaces end an account name");
    }
    if name.starts_with('*') {
        return refuse("a leading `*` is an org heading");
    }
    if name.starts_with(char::is_whitespace) || name.ends_with(char::is_whitespace) {
        return refuse("it starts or ends with a space");
    }
    if name.chars().any(char::is_control) {
        return refuse("it holds a control character");
    }
    if name.starts_with(['#', '(', '[']) || name.ends_with(':') || name.contains("::") {
        return refuse("hledger would read it differently");
    }
    let first = name.split([' ', ':']).next().unwrap_or("");
    if first.starts_with(|c: char| c.is_ascii_digit()) && first.split(['-', '/', '.']).count() == 3
    {
        return refuse("a name that starts like a date reads as a date line");
    }
    Ok(())
}

/// `comment` (a comment's text after `;`) with tag `name` set to `value`, or
/// removed for `None`. An existing tag is changed in place by
/// [`crate::edit::upsert_tag`] (its spacing kept); a new one is appended as
/// `name: value`, the way time files are written (F-files). Trimmed.
#[must_use]
pub fn with_tag(comment: &str, name: &str, value: Option<&str>) -> String {
    let comment = comment.trim();
    let present = crate::parse::parse_tags(comment)
        .iter()
        .any(|(tag, _)| tag == name);
    match value {
        Some(value) if !present => {
            let tag = if value.trim().is_empty() {
                format!("{name}:")
            } else {
                format!("{name}: {}", value.trim())
            };
            if comment.is_empty() {
                tag
            } else {
                format!("{comment}, {tag}")
            }
        }
        _ => crate::edit::upsert_tag(&format!(" {comment}"), name, value)
            .trim()
            .to_string(),
    }
}

/// Whether `name` can be a tag name: non-empty, no whitespace, none of the
/// characters that end a tag or a comment.
#[must_use]
pub fn is_tag_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, ':' | ',' | ';') || c.is_control())
}

/// Whether a `yes|no` style value says yes (`yes`, `y`, `true`, `1`), case
/// insensitively. Anything else is no.
#[must_use]
pub fn is_yes(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "y" | "true" | "1"
    )
}

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Worth knowing; nothing is wrong.
    Info,
    /// hledger reads this differently from what was probably meant.
    Warning,
    /// hledger refuses this line, or Ledgeline can't read it.
    Error,
}

/// Which hledger-compatibility problem a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FindingKind {
    /// A line hledger can't parse; the message says why.
    Invalid,
    /// `account 30m` or `account<TAB>30m`: hledger reads the amount as part
    /// of the account name and the posting as 0 (fact 1).
    SingleSpaceAmount,
    /// A `mo` or `y` unit: hledger 1.52 reads them seven times too large
    /// (fact 2). Ledgeline reads 30 and 365 days.
    MonthYearUnit,
    /// A space inside a number (`1 2`): hledger reads it as a digit group,
    /// `12`.
    SpaceInNumber,
    /// hledger 1.52 rounds this line to 0.01 h (`20m` → 0.33); Ledgeline keeps
    /// the exact minutes, so their totals can differ slightly (fact 2).
    Rounded,
    /// An entry with no amount: hledger reads 0.
    NoAmount,
    /// A timeclock session that can't be paired (fact 9).
    Pairing,
    /// A `b`, `h` or `O` timeclock line: ignored.
    Ignored,
}

/// One thing a parser noticed about a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// 1-based line number.
    pub line: u32,
    /// How serious.
    pub severity: Severity,
    /// What.
    pub kind: FindingKind,
    /// For people.
    pub message: String,
}

impl Finding {
    fn new(line: u32, severity: Severity, kind: FindingKind, message: impl Into<String>) -> Self {
        Self {
            line,
            severity,
            kind,
            message: message.into(),
        }
    }
}

/// Which time format a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeFormat {
    /// `.timedot`.
    Timedot,
    /// `.timeclock`.
    Timeclock,
}

impl TimeFormat {
    /// The format a path's extension names, if it names one.
    #[must_use]
    pub fn of_path(path: &str) -> Option<Self> {
        let ext = path.rsplit_once('.')?.1;
        match ext {
            "timedot" => Some(Self::Timedot),
            "timeclock" => Some(Self::Timeclock),
            _ => None,
        }
    }

    /// The `include` prefix (`timedot:`) and file extension.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Timedot => "timedot",
            Self::Timeclock => "timeclock",
        }
    }
}

/// Split text into lines, each as `(start, content_end, next_start)` byte
/// offsets: the content excludes its `\n` or `\r\n` terminator, and the last
/// line may have none.
pub(crate) fn line_spans(text: &str) -> Vec<(usize, usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::with_capacity(text.len() / 24 + 1);
    let mut start = 0;
    while start < bytes.len() {
        let newline = memchr_newline(&bytes[start..]).map(|at| start + at);
        let (content_end, next) = match newline {
            Some(at) => {
                let end = if at > start && bytes[at - 1] == b'\r' {
                    at - 1
                } else {
                    at
                };
                (end, at + 1)
            }
            None => (bytes.len(), bytes.len()),
        };
        spans.push((start, content_end, next));
        start = next;
    }
    spans
}

fn memchr_newline(bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|&b| b == b'\n')
}

/// Split `text` at its first run of two or more whitespace characters (spaces
/// or tabs), hledger's account/amount separator: `(name, rest)` with the
/// name's lone tabs turned into spaces as hledger does, and `rest` after the
/// run. No run: the whole text is the name and `rest` is empty.
pub(crate) fn split_two_spaces(text: &str) -> (String, &str) {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if matches!(bytes[at], b' ' | b'\t')
            && at + 1 < bytes.len()
            && matches!(bytes[at + 1], b' ' | b'\t')
        {
            let mut end = at;
            while end < bytes.len() && matches!(bytes[end], b' ' | b'\t') {
                end += 1;
            }
            return (text[..at].replace('\t', " "), &text[end..]);
        }
        at += 1;
    }
    (text.trim_end().replace('\t', " "), "")
}

/// A time of day, `HH:MM` or `HH:MM:SS`, as seconds since midnight.
pub(crate) fn parse_clock(text: &str) -> Option<u32> {
    let mut parts = text.split(':');
    let hours: u32 = two_digits(parts.next()?)?;
    let minutes: u32 = two_digits(parts.next()?)?;
    let seconds: u32 = match parts.next() {
        Some(s) => two_digits(s)?,
        None => 0,
    };
    if parts.next().is_some() || hours > 23 || minutes > 59 || seconds > 59 {
        return None;
    }
    Some(hours * 3600 + minutes * 60 + seconds)
}

fn two_digits(text: &str) -> Option<u32> {
    (text.len() == 2 || text.len() == 1)
        .then_some(())
        .filter(|()| text.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|()| text.parse().ok())
}

/// `HH:MM:SS` for seconds since midnight.
#[must_use]
pub fn format_clock(seconds: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_round_and_print() {
        assert_eq!(TimeQty::from_seconds(29).round_minutes(), 0);
        assert_eq!(TimeQty::from_seconds(30).round_minutes(), 1);
        assert_eq!(TimeQty::from_seconds(80).round_minutes(), 1);
        assert_eq!(TimeQty::from_seconds(90).round_minutes(), 2);
        assert_eq!(TimeQty::from_minutes(-15).round_minutes(), -15);
        assert_eq!(
            plain_decimal(TimeQty::from_minutes(20).hours()),
            "0.333333333"
        );
        assert_eq!(
            plain_decimal(TimeQty::from_minutes(20).hledger_152_hours()),
            "0.33"
        );
        assert_eq!(plain_decimal(TimeQty::from_minutes(90).hours()), "1.5");
        assert_eq!(TimeQty::from_minutes(120).written(), "2h");
        assert_eq!(TimeQty::from_minutes(130).written(), "130m");
        assert_eq!(TimeQty::from_seconds(90).written(), "1.5m");
        assert_eq!(TimeQty::from_minutes(0).written(), "0m");
        assert_eq!(TimeQty::from_minutes(65).to_string(), "1h 05m");
        assert_eq!(TimeQty::from_minutes(-15).to_string(), "-15m");
        assert_eq!(
            TimeQty::from_dec(Dec::new(15, 1), NANOS_PER_HOUR),
            Some(TimeQty::from_minutes(90))
        );
        assert_eq!(
            TimeQty::from_dec(Dec::new(10_000_001, 0), NANOS_PER_HOUR),
            None
        );
    }

    #[test]
    fn tag_names_take_overrides() {
        let (names, warnings) =
            TagNames::with_overrides([("invoice", "rechnung"), ("nope", "x"), ("bill", "a b")]);
        assert_eq!(names.name(TagRole::Invoice), "rechnung");
        assert_eq!(names.name(TagRole::Bill), "bill");
        assert_eq!(warnings.len(), 2);
        let tags = vec![("rechnung".to_string(), " 7 ".to_string())];
        assert_eq!(names.value(&tags, TagRole::Invoice), Some("7"));
    }

    #[test]
    fn splitting_and_clocks() {
        assert_eq!(split_two_spaces("a:b  1h"), ("a:b".to_string(), "1h"));
        assert_eq!(split_two_spaces("a b 1h"), ("a b 1h".to_string(), ""));
        assert_eq!(split_two_spaces("a\t1h"), ("a 1h".to_string(), ""));
        assert_eq!(split_two_spaces("a \t1h"), ("a".to_string(), "1h"));
        assert_eq!(parse_clock("14:05"), Some(50_700));
        assert_eq!(parse_clock("14:05:12"), Some(50_712));
        assert_eq!(parse_clock("24:00"), None);
        assert_eq!(format_clock(50_712), "14:05:12");
        assert_eq!(
            line_spans("a\r\nb\n\nc"),
            vec![(0, 1, 3), (3, 4, 5), (5, 5, 6), (6, 7, 7)]
        );
    }
}
