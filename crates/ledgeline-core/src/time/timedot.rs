//! A lossless, line-level timedot parser, matching hledger 1.52 (plan 46
//! facts 1–4, re-verified 2026-11-26).
//!
//! Every line of the file is kept, with its byte span and what it is:
//!
//! - **Blank** and **comment** lines (`#`, `;`, and `*` org headings before
//!   the first date).
//! - **Date lines**: a date (`2026-11-26`, `2026/11/26` or `2026.11.26`),
//!   optionally after org `*` stars, then a description up to `;` and a
//!   comment whose tags apply to every entry of the day.
//! - **Entries**: `account␣␣quantity  ; comment`, indented or not, with org
//!   stars stripped. The account ends at the first run of two or more
//!   whitespace characters; with only one space or tab before it, hledger
//!   reads the amount as part of the account name and the posting as 0, which
//!   is reported ([`FindingKind::SingleSpaceAmount`]).
//! - **Running markers**: `; ● running ACCOUNT  since HH:MM:SS` inside a day
//!   (T7). hledger reads it as a comment.
//!
//! Quantities ([`Quantity`]):
//!
//! - **dots**, a quarter hour each (spaces between them ignored);
//! - **letters**, a quarter hour each, one posting per distinct letter tagged
//!   `t:LETTER`, letters in sorted order as hledger emits them;
//! - **numbers**, with `.` or `,` as the decimal mark, an optional sign, and a
//!   unit `s m h d w` (hours without one). A space between digits is a digit
//!   group (`1 2` = 12, reported); a space before the unit is an error, as in
//!   hledger. `mo` and `y` are read as 30 and 365 days and reported: hledger
//!   1.52 reads them seven times too large (fact 2).
//!
//! Quantities are exact ([`TimeQty`]); hledger 1.52's rounding of a unit
//! amount to 0.01 h is reported as [`FindingKind::Rounded`], never applied.

use super::{Finding, FindingKind, Severity, TimeQty, line_spans, parse_clock, split_two_spaces};
use crate::decimal::Dec;
use crate::parse::{normalize_date, parse_tags};
use std::ops::Range;

/// Nanoseconds in one unit.
const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// The text a running marker's comment starts with (T7).
pub const MARKER_PREFIX: &str = "● running ";
/// The word before a running marker's start time.
pub const MARKER_SINCE: &str = "since ";

/// A timedot unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// `s`.
    Second,
    /// `m`.
    Minute,
    /// `h`.
    Hour,
    /// `d`: 24 h.
    Day,
    /// `w`: 7 d.
    Week,
    /// `mo`: read as 30 d (hledger 1.52 reads 210 d).
    Month,
    /// `y`: read as 365 d (hledger 1.52 reads 2555 d).
    Year,
}

impl Unit {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "s" => Self::Second,
            "m" => Self::Minute,
            "h" => Self::Hour,
            "d" => Self::Day,
            "w" => Self::Week,
            "mo" => Self::Month,
            "y" => Self::Year,
            _ => return None,
        })
    }

    const fn nanos(self) -> i128 {
        let seconds: i128 = match self {
            Self::Second => 1,
            Self::Minute => 60,
            Self::Hour => 3_600,
            Self::Day => 86_400,
            Self::Week => 604_800,
            Self::Month => 30 * 86_400,
            Self::Year => 365 * 86_400,
        };
        seconds * NANOS_PER_SECOND
    }
}

/// What an entry's amount says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quantity {
    /// Nothing after the account: hledger reads 0.
    Empty,
    /// `....`: a quarter hour per dot.
    Dots(u32),
    /// `aab`: a quarter hour per letter, one posting per distinct letter in
    /// sorted order, each tagged `t:LETTER`.
    Letters(Vec<(char, u32)>),
    /// A number with an optional unit (hours without one).
    Number {
        /// The value as written (decimal mark normalized).
        value: Dec,
        /// The unit, if one was written.
        unit: Option<Unit>,
    },
}

impl Quantity {
    /// The whole quantity.
    #[must_use]
    pub fn total(&self) -> TimeQty {
        TimeQty::sum(self.parts().into_iter().map(|(_, qty)| qty))
    }

    /// The postings hledger makes of it: one, or one per letter.
    #[must_use]
    pub fn parts(&self) -> Vec<(Option<char>, TimeQty)> {
        let quarter = TimeQty::from_minutes(15);
        match self {
            Self::Empty => vec![(None, TimeQty::ZERO)],
            Self::Dots(count) => vec![(None, TimeQty::from_minutes(15 * i64::from(*count)))],
            Self::Letters(groups) => groups
                .iter()
                .map(|(letter, count)| {
                    (
                        Some(*letter),
                        TimeQty::from_nanos(quarter.nanos() * i128::from(*count)),
                    )
                })
                .collect(),
            Self::Number { value, unit } => {
                let nanos = unit.map_or(Unit::Hour.nanos(), Unit::nanos);
                vec![(None, TimeQty::from_dec(*value, nanos).unwrap_or_default())]
            }
        }
    }
}

/// A date line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateLine {
    /// ISO `YYYY-MM-DD`.
    pub date: String,
    /// The text after the date up to `;`, trimmed: hledger's description
    /// (`Wed` on our lines).
    pub description: String,
    /// The comment after `;`, trimmed, if any.
    pub comment: Option<String>,
    /// Its tags, which apply to every entry of the day.
    pub tags: Vec<(String, String)>,
}

/// An entry line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Index of the day's date line in [`TimedotFile::lines`].
    pub day: usize,
    /// The account as hledger reads it.
    pub account: String,
    /// The amount.
    pub quantity: Quantity,
    /// The comment after `;`, trimmed, if any.
    pub comment: Option<String>,
    /// The comment's tags (the entry's own; the day's are on its date line).
    pub tags: Vec<(String, String)>,
    /// The comment's free text: everything that isn't a tag (T-vocab).
    pub note: String,
    /// Byte offsets within the line's content: where the account starts
    /// (after indentation and org stars), where it ends, where the quantity
    /// starts and ends, and where the `;` is.
    pub layout: EntryLayout,
}

/// Where an entry line's parts sit, as byte offsets into the line's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryLayout {
    /// The account's first byte.
    pub account_start: usize,
    /// One past the account's last byte.
    pub account_end: usize,
    /// The quantity's span (empty at `account_end` when there is none).
    pub quantity: (usize, usize),
    /// The `;` that opens the comment.
    pub comment_at: Option<usize>,
}

/// A running marker (T7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// Index of the day's date line.
    pub day: usize,
    /// The account being timed.
    pub account: String,
    /// Seconds after midnight of the day it started.
    pub since: u32,
}

/// What a line is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineKind {
    /// Empty or whitespace.
    Blank,
    /// A comment hledger ignores (including an indented one under an entry,
    /// which does not attach to it: fact 4).
    Comment,
    /// A date line.
    Date(DateLine),
    /// An entry.
    Entry(Entry),
    /// A running marker.
    Marker(Marker),
    /// A line hledger refuses; the finding says why.
    Invalid,
}

/// One line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// 1-based line number.
    pub number: u32,
    /// Byte offset of the line's first byte.
    pub start: usize,
    /// One past its content (before the terminator).
    pub end: usize,
    /// The next line's start (after the terminator).
    pub next: usize,
    /// What it is.
    pub kind: LineKind,
}

impl Line {
    /// The line's content in `text`, without its terminator.
    #[must_use]
    pub fn content<'t>(&self, text: &'t str) -> &'t str {
        &text[self.start..self.end]
    }

    /// The span of the line with its terminator.
    #[must_use]
    pub fn span(&self) -> Range<usize> {
        self.start..self.next
    }
}

/// A parsed timedot file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TimedotFile {
    /// Every line, in order.
    pub lines: Vec<Line>,
    /// What hledger reads differently, or refuses.
    pub findings: Vec<Finding>,
}

/// One posting hledger makes from an entry: an entry, or one letter of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotPosting<'f> {
    /// The line index of the entry.
    pub line: usize,
    /// The day's date line.
    pub day: &'f DateLine,
    /// The entry.
    pub entry: &'f Entry,
    /// The letter, for a letters entry.
    pub letter: Option<char>,
    /// The time.
    pub quantity: TimeQty,
}

impl TimedotFile {
    /// Whether any finding is an error (hledger refuses the file).
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }

    /// The date line at `index`, if that is one.
    #[must_use]
    pub fn date_line(&self, index: usize) -> Option<&DateLine> {
        match &self.lines.get(index)?.kind {
            LineKind::Date(date) => Some(date),
            _ => None,
        }
    }

    /// The entry at `index`, if that is one.
    #[must_use]
    pub fn entry(&self, index: usize) -> Option<&Entry> {
        match &self.lines.get(index)?.kind {
            LineKind::Entry(entry) => Some(entry),
            _ => None,
        }
    }

    /// Every posting hledger reads, in file order.
    pub fn postings(&self) -> impl Iterator<Item = DotPosting<'_>> {
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(at, line)| match &line.kind {
                LineKind::Entry(entry) => Some((at, entry)),
                _ => None,
            })
            .flat_map(move |(at, entry)| {
                let day = self.date_line(entry.day);
                entry
                    .quantity
                    .parts()
                    .into_iter()
                    .filter_map(move |(letter, quantity)| {
                        Some(DotPosting {
                            line: at,
                            day: day?,
                            entry,
                            letter,
                            quantity,
                        })
                    })
            })
    }

    /// Every running marker, with its line index.
    pub fn markers(&self) -> impl Iterator<Item = (usize, &Marker)> {
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(at, line)| match &line.kind {
                LineKind::Marker(marker) => Some((at, marker)),
                _ => None,
            })
    }

    /// The days, each as `(date line index, one past its last non-blank
    /// line)`, in file order.
    #[must_use]
    pub fn days(&self) -> Vec<(usize, usize)> {
        let starts: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| matches!(line.kind, LineKind::Date(_)))
            .map(|(at, _)| at)
            .collect();
        starts
            .iter()
            .enumerate()
            .map(|(n, &start)| {
                let limit = starts.get(n + 1).copied().unwrap_or(self.lines.len());
                let end = (start + 1..limit)
                    .rev()
                    .find(|&at| !matches!(self.lines[at].kind, LineKind::Blank))
                    .map_or(start + 1, |last| last + 1);
                (start, end)
            })
            .collect()
    }
}

/// Parse `text` as timedot.
#[must_use]
pub fn parse(text: &str) -> TimedotFile {
    let text_body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bom = text.len() - text_body.len();
    let mut file = TimedotFile::default();
    let mut day: Option<usize> = None;
    for (n, (start, end, next)) in line_spans(text).into_iter().enumerate() {
        let number = u32::try_from(n + 1).unwrap_or(u32::MAX);
        let content_start = if n == 0 { start + bom } else { start };
        let content = &text[content_start.min(end)..end];
        let mut problems = Vec::new();
        let kind = classify(content, day, &mut problems);
        if let LineKind::Date(_) = kind {
            day = Some(file.lines.len());
        }
        file.findings.extend(
            problems
                .into_iter()
                .map(|(severity, kind, message)| Finding::new(number, severity, kind, message)),
        );
        file.lines.push(Line {
            number,
            start,
            end,
            next,
            kind,
        });
    }
    file
}

type Problem = (Severity, FindingKind, String);

fn classify(content: &str, day: Option<usize>, problems: &mut Vec<Problem>) -> LineKind {
    let trimmed = content.trim_start();
    if trimmed.is_empty() {
        return LineKind::Blank;
    }
    if trimmed.starts_with(['#', ';']) {
        return day
            .and_then(|day| marker(trimmed, day))
            .map_or(LineKind::Comment, LineKind::Marker);
    }
    let body = trimmed.trim_start_matches('*').trim_start();
    if let Some(date) = date_line(body) {
        return LineKind::Date(date);
    }
    let Some(day) = day else {
        if trimmed.starts_with('*') {
            // An org heading before the first day.
            return LineKind::Comment;
        }
        problems.push((
            Severity::Error,
            FindingKind::Invalid,
            "hledger expects a date line (or a comment) before any entry".to_string(),
        ));
        return LineKind::Invalid;
    };
    let account_start = content.len() - body.len();
    match entry(content, account_start, day, problems) {
        Some(entry) => LineKind::Entry(entry),
        None => LineKind::Invalid,
    }
}

/// A date line's parts, when `body` (stars and indentation gone) starts with
/// a date.
fn date_line(body: &str) -> Option<DateLine> {
    let token_end = body
        .find(|c: char| c.is_whitespace() || c == ';')
        .unwrap_or(body.len());
    let token = &body[..token_end];
    if !token.starts_with(|c: char| c.is_ascii_digit()) || token.split(['-', '/', '.']).count() != 3
    {
        return None;
    }
    let date = normalize_date(token, None).ok()?;
    let rest = &body[token_end..];
    let (description, comment) = split_comment(rest);
    let comment = comment.map(str::trim).filter(|c| !c.is_empty());
    Some(DateLine {
        date,
        description: description.trim().to_string(),
        tags: comment.map(parse_tags).unwrap_or_default(),
        comment: comment.map(str::to_string),
    })
}

fn split_comment(text: &str) -> (&str, Option<&str>) {
    match text.find(';') {
        Some(at) => (&text[..at], Some(&text[at + 1..])),
        None => (text, None),
    }
}

/// A running marker, when a comment line (`trimmed` starts at its `;`) is
/// one.
fn marker(trimmed: &str, day: usize) -> Option<Marker> {
    let rest = trimmed.strip_prefix(';')?.trim_start();
    let rest = rest.strip_prefix(MARKER_PREFIX)?;
    let (account, after) = split_two_spaces(rest);
    let since = after.trim().strip_prefix(MARKER_SINCE)?.trim();
    let account = account.trim();
    (!account.is_empty()).then_some(())?;
    Some(Marker {
        day,
        account: account.to_string(),
        since: parse_clock(since)?,
    })
}

fn entry(
    content: &str,
    account_start: usize,
    day: usize,
    problems: &mut Vec<Problem>,
) -> Option<Entry> {
    let body = &content[account_start..];
    let (main, comment) = split_comment(body);
    let comment_at = comment.map(|_| account_start + main.len());
    let (account, rest) = split_two_spaces(main);
    let account = account.trim_end().to_string();
    if account.is_empty() {
        problems.push((
            Severity::Error,
            FindingKind::Invalid,
            "an entry needs an account name".to_string(),
        ));
        return None;
    }
    // Tabs in the name became spaces, so its byte length is unchanged.
    let raw_account_end = account_start + account.len();
    let quantity_text = rest.trim_end();
    let quantity_start = account_start + (main.len() - rest.len());
    let quantity_end = quantity_start + quantity_text.len();
    let quantity = match parse_quantity(quantity_text, problems) {
        Ok(quantity) => quantity,
        Err(message) => {
            problems.push((Severity::Error, FindingKind::Invalid, message));
            return None;
        }
    };
    if quantity == Quantity::Empty {
        match single_space_amount(&account) {
            Some(amount) => problems.push((
                Severity::Warning,
                FindingKind::SingleSpaceAmount,
                format!(
                    "only one space or tab before `{amount}`: hledger reads it as part of \
                     the account name and the amount as 0; put two spaces before the amount"
                ),
            )),
            None => problems.push((
                Severity::Info,
                FindingKind::NoAmount,
                "this entry has no amount; hledger reads 0".to_string(),
            )),
        }
    }
    let comment = comment.map(str::trim).filter(|c| !c.is_empty());
    let mut tags = comment.map(parse_tags).unwrap_or_default();
    if let Quantity::Letters(groups) = &quantity {
        tags.extend(
            groups
                .iter()
                .map(|(letter, _)| ("t".to_string(), letter.to_string())),
        );
    }
    let note = comment
        .map(|comment| crate::edit::free_text(comment).join(", "))
        .unwrap_or_default();
    Some(Entry {
        day,
        account,
        quantity,
        comment: comment.map(str::to_string),
        tags,
        note,
        layout: EntryLayout {
            account_start,
            account_end: raw_account_end,
            quantity: (quantity_start, quantity_end),
            comment_at,
        },
    })
}

/// The amount-looking last word of an account written with a single space
/// before it, if any.
fn single_space_amount(account: &str) -> Option<&str> {
    let (_, last) = account.rsplit_once(' ')?;
    let mut ignored = Vec::new();
    match parse_quantity(last, &mut ignored) {
        Ok(Quantity::Number { .. } | Quantity::Dots(_)) => Some(last),
        _ => None,
    }
}

/// Parse a quantity. `Err` is a line hledger refuses.
fn parse_quantity(text: &str, problems: &mut Vec<Problem>) -> Result<Quantity, String> {
    if text.is_empty() {
        return Ok(Quantity::Empty);
    }
    if text.bytes().any(|b| b.is_ascii_digit()) {
        return parse_number(text, problems);
    }
    if text.chars().all(|c| c == '.' || c == ' ') {
        let dots = text.chars().filter(|&c| c == '.').count();
        return u32::try_from(dots)
            .map(Quantity::Dots)
            .map_err(|_| "too many dots".to_string());
    }
    if text.chars().all(|c| c.is_alphabetic() || c == ' ') {
        let mut groups: Vec<(char, u32)> = Vec::new();
        for letter in text.chars().filter(|c| *c != ' ') {
            match groups.iter_mut().find(|(seen, _)| *seen == letter) {
                Some((_, count)) => *count += 1,
                None => groups.push((letter, 1)),
            }
        }
        groups.sort_by_key(|(letter, _)| *letter);
        return Ok(Quantity::Letters(groups));
    }
    Err(format!(
        "`{text}` is not a timedot quantity (dots, letters, or a number with a unit)"
    ))
}

fn parse_number(text: &str, problems: &mut Vec<Problem>) -> Result<Quantity, String> {
    let refuse = || format!("`{text}` is not a timedot quantity");
    let (negative, unsigned) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let unit_at = unsigned
        .find(|c: char| c.is_alphabetic())
        .unwrap_or(unsigned.len());
    let (number, unit_text) = unsigned.split_at(unit_at);
    let unit = if unit_text.is_empty() {
        None
    } else {
        Some(Unit::parse(unit_text).ok_or_else(|| {
            format!("`{unit_text}` is not a timedot unit (s, m, h, d, w)")
        })?)
    };
    if number.is_empty()
        || number.ends_with(' ')
        || number.starts_with(' ')
        || !number
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | ' '))
    {
        return Err(refuse());
    }
    if number.contains(' ') {
        if number
            .split(' ')
            .any(|group| group.is_empty() || !group.starts_with(|c: char| c.is_ascii_digit()))
        {
            return Err(refuse());
        }
        problems.push((
            Severity::Warning,
            FindingKind::SpaceInNumber,
            format!("hledger reads the space in `{number}` as a digit group separator"),
        ));
    }
    let digits: String = number
        .chars()
        .filter(|c| *c != ' ')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    if digits.matches('.').count() > 1 || digits == "." {
        return Err(refuse());
    }
    let value = Dec::parse(&digits, '.').map_err(|_| refuse())?;
    let value = if negative {
        value.neg().map_err(|_| refuse())?
    } else {
        value
    };
    let nanos = unit.map_or(Unit::Hour.nanos(), Unit::nanos);
    let quantity = TimeQty::from_dec(value, nanos)
        .ok_or_else(|| format!("`{text}` is more than a million hours"))?;
    if matches!(unit, Some(Unit::Month | Unit::Year)) {
        problems.push((
            Severity::Warning,
            FindingKind::MonthYearUnit,
            "hledger 1.52 reads `mo` and `y` seven times too large; Ledgeline reads 30 and \
             365 days. Write hours or days instead"
                .to_string(),
        ));
    }
    if unit.is_some() && quantity.hledger_152_hours() != quantity.hours() {
        problems.push((
            Severity::Info,
            FindingKind::Rounded,
            format!(
                "hledger 1.52 rounds `{text}` to {} h; Ledgeline keeps the exact time",
                crate::view::wire::plain_decimal(quantity.hledger_152_hours())
            ),
        ));
    }
    Ok(Quantity::Number { value, unit })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hours(file: &TimedotFile) -> Vec<(String, String, Option<char>)> {
        file.postings()
            .map(|p| {
                (
                    p.entry.account.clone(),
                    crate::view::wire::plain_decimal(p.quantity.hledger_152_hours()),
                    p.letter,
                )
            })
            .collect()
    }

    /// The probe file from 2026-11-26, with hledger 1.52's `print` amounts.
    #[test]
    fn matches_hledger_on_the_probe_file() {
        let text = "# comment\n; another\n* Org heading before date\n\
** 2026-11-01 Sun description here  ; person: pat, day:1\n\
*** clients:acme  ....  ; letters?\n\
clients:acme:a  1.5\n\
    clients:acme:b  20m  ; invoice: 108, a note\n\
    clients:acme:c   1,5h\n\
    clients:acme:d  aabbc\n\
    clients:acme:e 30m\n\
\tclients:acme:f\t30m\n\
    clients:acme:g  1 2\n\
    clients:acme:h  -15m\n\
    clients:acme:i  1mo\n\
    clients:acme:j  90s\n\
    clients:acme:k  .. ..\n\
    ; indented comment\n\
2026/11/02\n    x  1d\n    y  1w\n2026.11.03 Tue\n    z  2h\n";
        let file = parse(text);
        assert!(!file.has_errors(), "{:?}", file.findings);
        let day = file.date_line(3).expect("date");
        assert_eq!(day.date, "2026-11-01");
        assert_eq!(day.description, "Sun description here");
        assert_eq!(day.tags[0], ("person".into(), "pat".into()));
        let got = hours(&file);
        let want: Vec<(&str, &str, Option<char>)> = vec![
            ("clients:acme", "1.00", None),
            ("clients:acme:a", "1.50", None),
            ("clients:acme:b", "0.33", None),
            ("clients:acme:c", "1.50", None),
            ("clients:acme:d", "0.50", Some('a')),
            ("clients:acme:d", "0.50", Some('b')),
            ("clients:acme:d", "0.25", Some('c')),
            ("clients:acme:e 30m", "0.00", None),
            ("clients:acme:f 30m", "0.00", None),
            ("clients:acme:g", "12.00", None),
            ("clients:acme:h", "-0.25", None),
            // hledger 1.52 reads 5040; Ledgeline 720, with a finding.
            ("clients:acme:i", "720.00", None),
            ("clients:acme:j", "0.03", None),
            ("clients:acme:k", "1.00", None),
            ("x", "24.00", None),
            ("y", "168.00", None),
            ("z", "2.00", None),
        ];
        let want: Vec<(String, String, Option<char>)> = want
            .into_iter()
            .map(|(a, h, l)| (a.to_string(), h.to_string(), l))
            .collect();
        assert_eq!(got, want);
        let kinds: Vec<FindingKind> = file.findings.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&FindingKind::SingleSpaceAmount));
        assert!(kinds.contains(&FindingKind::SpaceInNumber));
        assert!(kinds.contains(&FindingKind::MonthYearUnit));
        assert!(kinds.contains(&FindingKind::Rounded));
        let b = file.entry(6).expect("entry");
        assert_eq!(b.note, "a note");
        assert_eq!(b.tags, vec![("invoice".into(), "108".into())]);
        assert_eq!(b.quantity.total(), TimeQty::from_minutes(20));
        let d = file.entry(8).expect("letters");
        assert!(d.tags.contains(&("t".into(), "a".into())));
        assert_eq!(file.lines[16].kind, LineKind::Comment);
    }

    #[test]
    fn refuses_what_hledger_refuses() {
        for bad in [
            "    c  1.5 h",
            "    a  ..ab",
            "    g  0.5.",
            "    h  .a",
            "    i  1.5h2",
            "    k  3 parsecs",
        ] {
            let file = parse(&format!("2026-11-01\n{bad}\n"));
            assert!(file.has_errors(), "{bad}");
            assert_eq!(file.lines[1].kind, LineKind::Invalid, "{bad}");
        }
        assert!(parse("x  1h\n2026-11-01\n").has_errors());
        let ok = parse("2026-11-01\n  j  1.\n  k  .5\n  d  +1h\n  e  ab ba\n  f  1,500\n  b\n");
        assert!(!ok.has_errors(), "{:?}", ok.findings);
        let totals: Vec<i64> = ok.postings().map(|p| p.quantity.round_minutes()).collect();
        assert_eq!(totals, vec![60, 30, 60, 30, 30, 90, 0]);
    }

    #[test]
    fn org_headings_and_markers() {
        let text = "* Top\n2026-11-01\n* Heading words\n** sub:acct  1h\n    # hash\n  a  ba\n    ; ● running clients:acme:advisory  since 14:05:12\n; ● running x  since 1:00\n";
        let file = parse(text);
        assert_eq!(file.lines[0].kind, LineKind::Comment);
        assert_eq!(file.entry(2).expect("heading").account, "Heading words");
        assert_eq!(file.entry(3).expect("sub").account, "sub:acct");
        assert_eq!(file.lines[4].kind, LineKind::Comment);
        let markers: Vec<_> = file.markers().collect();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].1.account, "clients:acme:advisory");
        assert_eq!(markers[0].1.since, 14 * 3600 + 5 * 60 + 12);
        assert_eq!(file.days(), vec![(1, 8)]);
    }

    #[test]
    fn layout_points_at_the_parts() {
        let line = "    clients:acme  45m  ; invoice: 108";
        let file = parse(&format!("2026-11-26\n{line}\r\n"));
        let entry = file.entry(1).expect("entry");
        let l = entry.layout;
        assert_eq!(&line[l.account_start..l.account_end], "clients:acme");
        assert_eq!(&line[l.quantity.0..l.quantity.1], "45m");
        assert_eq!(&line[l.comment_at.expect("comment")..], "; invoice: 108");
        assert_eq!(file.lines[1].content(&format!("2026-11-26\n{line}\r\n")), line);
    }
}
