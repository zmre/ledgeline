//! A hand-written reader for QIF (Quicken Interchange Format) and QMTF
//! (Quicken for Mac's QIF variant), as Banktivity, Quicken, Moneydance and
//! GnuCash write them (plan 39 Phase 2; see `plans/39-onboarding-offboarding.md`).
//!
//! This module only reads the file into its own records. Turning those into
//! balanced transactions (transfers, splits, opening balances, ids) is
//! [`crate::qif_import`]'s job.
//!
//! # The shape of a file
//!
//! A QIF file is a run of lines. A line starting with `!` is a header that
//! says what the lines below it are; every other line is one field, keyed by
//! its first character, and a line holding only `^` ends a record:
//!
//! ```text
//! !Account            <- an account record follows; the register below is its
//! NChecking
//! TBank
//! ^
//! !Type:Bank          <- register records for that account
//! D1/ 5'26
//! T-42.50
//! PCorner Grocer
//! LFood:Groceries/Household
//! ^
//! ```
//!
//! - `!Type:Bank|Cash|CCard|Oth A|Oth L|Invst` open a register. Its account is
//!   whichever `!Account` record came last, or none in a single-account file.
//! - `!Account` records list accounts. Between `!Option:AutoSwitch` and
//!   `!Clear:AutoSwitch` they are only a list and do not choose the register's
//!   account.
//! - `!Type:Cat` and `!Type:Class` list categories and classes.
//! - Every other header (`!Type:Memorized`, `!Type:Security`, `!Type:Prices`,
//!   QMTF's `!Type:Tag` …) is skipped, record by record.
//!
//! Banktivity writes one file per currency and repeats the whole account and
//! category list in each; a list entry seen twice is kept once.
//!
//! # Never a panic, never unbounded
//!
//! Input is capped at [`MAX_INPUT_BYTES`], decoded as UTF-8 or else
//! Windows-1252 (what Quicken for Windows writes), and every number is parsed
//! exactly into [`Dec`] with overflow refused rather than wrapped. A record
//! with no fields, an unknown field code or a field outside any register is
//! ignored, because QIF writers disagree about all three.
//!
//! # Dates
//!
//! QIF dates have no declared format. `1/ 5'98`, `01/05/1998`, `5.1.98` and
//! `1998-01-05` all occur, and `01/02/2026` is January or February depending on
//! the writer's locale. [`resolve_dates`] answers that once for the whole file:
//! month-first and day-first are each tried against every date, and the
//! reading that fits them all wins. When both fit, the
//! [`DateFormatGuess::ambiguous`] flag is set so the user is warned, exactly as
//! for a QuickBooks export.

use crate::convert::MAX_INPUT_BYTES;
use crate::decimal::Dec;
use crate::rules::generate::DateFormatGuess;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The file extensions that mean QIF.
pub const EXTENSIONS: [&str; 2] = ["qif", "qmtf"];

/// How much of a value from the file an error message may quote.
const QUOTE_LIMIT: usize = 40;

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// What kind of register a `!Type:` header opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegisterKind {
    Bank,
    Cash,
    CreditCard,
    OtherAsset,
    OtherLiability,
    Investment,
}

/// One `!Account` record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QifAccount {
    pub name: String,
    /// The `T` line as written (`Bank`, `CCard`, `Oth A`, `Invst` …).
    pub kind: Option<String>,
    pub description: Option<String>,
}

/// One `!Type:Cat` record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QifCategory {
    pub name: String,
    pub description: Option<String>,
    /// `I` was present: an income category.
    pub income: bool,
}

/// One `!Type:Class` record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QifClass {
    pub name: String,
    pub description: Option<String>,
}

/// One split line group: `S` (category or `[Account]`), `E` (memo), `$`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QifSplit {
    /// Raw `S` text, class included: `Food:Groceries/Household`, `[Savings]`.
    pub category: Option<String>,
    pub memo: Option<String>,
    /// Same sign convention as the record's total: the splits add up to it.
    pub amount: Option<Dec>,
}

/// One register record, fields as written apart from numbers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QifRecord {
    /// The 1-based line of the record's first field, for error messages.
    pub line: usize,
    /// Raw `D` text; resolved with the whole file's other dates.
    pub date: Option<String>,
    /// `T`, or `U` when there is no `T`.
    pub amount: Option<Dec>,
    pub payee: Option<String>,
    pub memo: Option<String>,
    /// `N` in a non-investment register: a check or reference number.
    pub number: Option<String>,
    /// The `C` line's first character, if any.
    pub status: Option<char>,
    /// Raw `L` text, class included.
    pub category: Option<String>,
    pub splits: Vec<QifSplit>,
    /// `N` in an investment register: `Buy`, `SellX`, `Div` …
    pub action: Option<String>,
    /// `Y`.
    pub security: Option<String>,
    /// `I`.
    pub price: Option<Dec>,
    /// `Q`.
    pub quantity: Option<Dec>,
    /// `O`.
    pub commission: Option<Dec>,
    /// `$` in an investment register: the amount moved to or from `L`.
    pub transfer_amount: Option<Dec>,
}

/// One register: the account it belongs to (when the file named one) and its
/// records, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QifRegister {
    pub account: Option<QifAccount>,
    pub kind: RegisterKind,
    pub records: Vec<QifRecord>,
}

/// A parsed file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QifFile {
    /// Every `!Account` record, first occurrence of each name only.
    pub accounts: Vec<QifAccount>,
    pub categories: Vec<QifCategory>,
    pub classes: Vec<QifClass>,
    pub registers: Vec<QifRegister>,
    /// Headers whose records were skipped (`!Type:Memorized` …), once each.
    pub skipped_sections: Vec<String>,
}

/// Why a file was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum QifError {
    #[error("the file is empty")]
    Empty,
    #[error("the file is larger than the {limit}-byte limit")]
    TooLarge { limit: usize },
    #[error("this file has no QIF header (a line such as !Type:Bank or !Account)")]
    NotQif,
    #[error("the QIF file holds no transactions")]
    NoTransactions,
    #[error("the QIF record at line {line} has no date (D line)")]
    MissingDate { line: usize },
    #[error("line {line}: {value:?} is not a date")]
    UnreadableDate { line: usize, value: String },
    #[error(
        "the dates in this QIF file fit neither month/day nor day/month order (line {line}: {value:?})"
    )]
    InconsistentDates { line: usize, value: String },
    #[error("line {line}: {value:?} is not an amount")]
    UnreadableAmount { line: usize, value: String },
    #[error("the QIF record at line {line} has amounts too large to add up")]
    AmountOverflow { line: usize },
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Does `name` carry a QIF extension?
#[must_use]
pub fn has_qif_extension(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| {
        EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    })
}

/// Do these bytes start like a QIF file, whatever the file is called?
///
/// Conservative on purpose: the first non-blank line must be a QIF header
/// (`!Type:`, `!Account`, `!Option:` or `!Clear:`). No bank CSV starts that
/// way, so an ordinary statement still goes to the rules pipeline.
#[must_use]
pub fn sniff(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let text = String::from_utf8_lossy(head);
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|line| {
            let lower = line.to_ascii_lowercase();
            ["!type:", "!account", "!option:", "!clear:"]
                .iter()
                .any(|prefix| lower.starts_with(prefix))
        })
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Which section the lines being read belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    /// Before any header.
    None,
    AccountList,
    Categories,
    Classes,
    Register(RegisterKind),
    Skipped,
}

/// One field line: its code, its value, and its line number.
type Field = (char, String, usize);

/// Parse `bytes` as QIF.
///
/// # Errors
///
/// [`QifError::Empty`], [`QifError::TooLarge`], [`QifError::NotQif`] when no
/// QIF header is found, or [`QifError::UnreadableAmount`] for a number that is
/// not one. A file with no register records parses; refusing it is the
/// importer's decision.
pub fn parse(bytes: &[u8]) -> Result<QifFile, QifError> {
    if bytes.is_empty() {
        return Err(QifError::Empty);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(QifError::TooLarge {
            limit: MAX_INPUT_BYTES,
        });
    }
    let text = decode(bytes);
    let mut reader = Reader::default();
    for (at, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        reader.line(at + 1, raw)?;
    }
    reader.flush()?;
    if !reader.saw_header {
        return Err(QifError::NotQif);
    }
    Ok(reader.file)
}

/// UTF-8 when the bytes are valid UTF-8, otherwise Windows-1252, which every
/// byte decodes under.
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}

#[derive(Default)]
struct Reader {
    file: QifFile,
    section: Option<Section>,
    autoswitch: bool,
    current_account: Option<QifAccount>,
    fields: Vec<Field>,
    saw_header: bool,
}

impl Reader {
    fn section(&self) -> Section {
        self.section.unwrap_or(Section::None)
    }

    fn line(&mut self, number: usize, raw: &str) -> Result<(), QifError> {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            return Ok(());
        }
        if trimmed.starts_with('!') {
            // A header with fields still pending ends that record: some
            // writers omit the final `^` before the next section.
            self.flush()?;
            self.header(trimmed);
            return Ok(());
        }
        if trimmed == "^" {
            return self.flush();
        }
        let mut chars = trimmed.chars();
        if let Some(code) = chars.next() {
            self.fields
                .push((code, chars.as_str().trim().to_string(), number));
        }
        Ok(())
    }

    fn header(&mut self, line: &str) {
        let lower: String = line
            .to_ascii_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        self.saw_header |= ["!type:", "!account", "!option:", "!clear:"]
            .iter()
            .any(|prefix| lower.starts_with(prefix));
        let section = match lower.as_str() {
            "!option:autoswitch" => {
                self.autoswitch = true;
                return;
            }
            "!clear:autoswitch" => {
                self.autoswitch = false;
                return;
            }
            "!account" => Section::AccountList,
            "!type:cat" => Section::Categories,
            "!type:class" => Section::Classes,
            "!type:bank" => Section::Register(RegisterKind::Bank),
            "!type:cash" => Section::Register(RegisterKind::Cash),
            "!type:ccard" => Section::Register(RegisterKind::CreditCard),
            "!type:otha" => Section::Register(RegisterKind::OtherAsset),
            "!type:othl" => Section::Register(RegisterKind::OtherLiability),
            "!type:invst" | "!type:port" => Section::Register(RegisterKind::Investment),
            _ => {
                let name = clip(line);
                if !self.file.skipped_sections.contains(&name) {
                    self.file.skipped_sections.push(name);
                }
                Section::Skipped
            }
        };
        if let Section::Register(kind) = section {
            self.file.registers.push(QifRegister {
                account: self.current_account.clone(),
                kind,
                records: Vec::new(),
            });
        }
        self.section = Some(section);
    }

    /// End the record whose fields are pending.
    fn flush(&mut self) -> Result<(), QifError> {
        let fields = std::mem::take(&mut self.fields);
        if fields.is_empty() {
            return Ok(());
        }
        match self.section() {
            Section::None | Section::Skipped => {}
            Section::AccountList => self.account(&fields),
            Section::Categories => self.category(&fields),
            Section::Classes => self.class(&fields),
            Section::Register(kind) => {
                let record = record(kind, &fields)?;
                if let Some(register) = self.file.registers.last_mut() {
                    register.records.push(record);
                }
            }
        }
        Ok(())
    }

    fn account(&mut self, fields: &[Field]) {
        let account = QifAccount {
            name: text_of(fields, 'N').unwrap_or_default(),
            kind: text_of(fields, 'T'),
            description: text_of(fields, 'D'),
        };
        if account.name.is_empty() {
            return;
        }
        if !self.autoswitch {
            self.current_account = Some(account.clone());
        }
        if !self.file.accounts.iter().any(|a| a.name == account.name) {
            self.file.accounts.push(account);
        }
    }

    fn category(&mut self, fields: &[Field]) {
        let category = QifCategory {
            name: text_of(fields, 'N').unwrap_or_default(),
            description: text_of(fields, 'D'),
            income: fields.iter().any(|(code, ..)| *code == 'I'),
        };
        if !category.name.is_empty()
            && !self.file.categories.iter().any(|c| c.name == category.name)
        {
            self.file.categories.push(category);
        }
    }

    fn class(&mut self, fields: &[Field]) {
        let class = QifClass {
            name: text_of(fields, 'N').unwrap_or_default(),
            description: text_of(fields, 'D'),
        };
        if !class.name.is_empty() && !self.file.classes.iter().any(|c| c.name == class.name) {
            self.file.classes.push(class);
        }
    }
}

/// The first non-empty value of `code` among `fields`.
fn text_of(fields: &[Field], code: char) -> Option<String> {
    fields
        .iter()
        .find(|(c, value, _)| *c == code && !value.is_empty())
        .map(|(_, value, _)| value.clone())
}

/// `Some(value)` unless it is empty.
fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

/// Build one register record from its fields.
fn record(kind: RegisterKind, fields: &[Field]) -> Result<QifRecord, QifError> {
    let investment = kind == RegisterKind::Investment;
    let mut record = QifRecord {
        line: fields.first().map_or(0, |(_, _, line)| *line),
        ..QifRecord::default()
    };
    let mut total_u = None;
    for (code, value, line) in fields {
        let number = || amount(value, *line);
        match code {
            'D' => record.date = record.date.take().or_else(|| non_empty(value)),
            'T' => record.amount = record.amount.or(number()?),
            'U' => total_u = total_u.or(number()?),
            'P' => record.payee = record.payee.take().or_else(|| non_empty(value)),
            'M' => record.memo = record.memo.take().or_else(|| non_empty(value)),
            'N' if investment => record.action = non_empty(value),
            'N' => record.number = non_empty(value),
            'C' => record.status = value.chars().next(),
            'L' => record.category = non_empty(value),
            'S' => record.splits.push(QifSplit {
                category: non_empty(value),
                ..QifSplit::default()
            }),
            'E' => {
                let split = open_split(&mut record.splits, |split| split.memo.is_some());
                split.memo = non_empty(value);
            }
            '$' if investment => record.transfer_amount = number()?,
            '$' => {
                let parsed = number()?;
                let split = open_split(&mut record.splits, |split| split.amount.is_some());
                split.amount = parsed;
            }
            'Y' if investment => record.security = non_empty(value),
            'I' if investment => record.price = number()?,
            'Q' if investment => record.quantity = number()?,
            'O' if investment => record.commission = number()?,
            // `A` address lines, `%` split percentages, `F` flags, `X`
            // invoice fields, `1`–`7` loan fields and anything else: kept by
            // no importer here, and QIF writers vary too much to refuse them.
            _ => {}
        }
    }
    record.amount = record.amount.or(total_u);
    Ok(record)
}

/// The split an `E` or `$` line belongs to: the last one, unless it already
/// has that field (or there is none), in which case a new one is opened. Some
/// writers put `$` before `S`; this keeps them paired.
fn open_split(splits: &mut Vec<QifSplit>, taken: impl Fn(&QifSplit) -> bool) -> &mut QifSplit {
    if splits.last().is_none_or(&taken) {
        splits.push(QifSplit::default());
    }
    let last = splits.len() - 1;
    &mut splits[last]
}

/// An amount field, exactly. `Ok(None)` for an empty value.
///
/// QIF writers use `1,234.56` almost everywhere, but European locales write
/// `1.234,56`. The decimal mark is the last of `.`/`,` when both appear, a
/// lone `,` followed by one or two digits, or a lone `.`; any other `.`/`,` is
/// a digit-group separator.
fn amount(value: &str, line: usize) -> Result<Option<Dec>, QifError> {
    let unreadable = || QifError::UnreadableAmount {
        line,
        value: clip(value),
    };
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '$' && *c != '\u{a0}')
        .collect();
    if cleaned.is_empty() {
        return Ok(None);
    }
    let (sign, digits) = match cleaned.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", cleaned.strip_prefix('+').unwrap_or(&cleaned)),
    };
    if digits.is_empty()
        || !digits
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
        || !digits.chars().any(|c| c.is_ascii_digit())
    {
        return Err(unreadable());
    }
    let mark = decimal_mark(digits);
    Dec::parse_with_mark(&format!("{sign}{digits}"), mark)
        .map(Some)
        .map_err(|_| unreadable())
}

fn decimal_mark(digits: &str) -> Option<char> {
    let dots = digits.matches('.').count();
    let commas = digits.matches(',').count();
    match (dots, commas) {
        (0, 0) => None,
        (_, 0) => (dots == 1).then_some('.'),
        (0, _) => {
            let after = digits.rsplit(',').next().map_or(0, str::len);
            (commas == 1 && (1..=2).contains(&after)).then_some(',')
        }
        _ => {
            let last_dot = digits.rfind('.');
            let last_comma = digits.rfind(',');
            if last_dot > last_comma {
                Some('.')
            } else {
                Some(',')
            }
        }
    }
}

/// `value`, cut short for an error message.
fn clip(value: &str) -> String {
    let clipped: String = value.chars().take(QUOTE_LIMIT).collect();
    if clipped.len() < value.len() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

/// Which of a date's first two numbers is the month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    MonthFirst,
    DayFirst,
}

/// One date's numbers before the file's order is known.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DateParts {
    first: u32,
    second: u32,
    third: u32,
    /// `1998-01-05`: the year leads, and the order question does not arise.
    year_first: bool,
    /// The last number had four digits (or the first, when `year_first`).
    long_year: bool,
    /// The separators, in order, with padding spaces dropped.
    separators: [char; 2],
}

/// Split a QIF date into its three numbers.
///
/// Separators are any of `/ - . '`; spaces are padding (Quicken writes
/// `1/ 5'98`). Anything else, or a count of numbers other than three, is not
/// a date.
fn date_parts(raw: &str) -> Option<DateParts> {
    let mut runs: Vec<&str> = Vec::with_capacity(3);
    let mut separators: Vec<char> = Vec::with_capacity(2);
    let mut start: Option<usize> = None;
    for (at, c) in raw.char_indices() {
        if c.is_ascii_digit() {
            start.get_or_insert(at);
            continue;
        }
        if let Some(from) = start.take() {
            runs.push(&raw[from..at]);
        }
        match c {
            ' ' => {}
            '/' | '-' | '.' | '\'' => separators.push(c),
            _ => return None,
        }
    }
    if let Some(from) = start {
        runs.push(&raw[from..]);
    }
    let [a, b, c] = runs[..] else { return None };
    let [s1, s2] = separators[..] else {
        return None;
    };
    if [a, b, c].iter().any(|run| run.len() > 4) {
        return None;
    }
    let number = |run: &str| run.parse::<u32>().ok();
    let year_first = a.len() >= 3;
    Some(DateParts {
        first: number(a)?,
        second: number(b)?,
        third: number(c)?,
        year_first,
        long_year: if year_first {
            a.len() == 4
        } else {
            c.len() >= 3
        },
        separators: [s1, s2],
    })
}

impl DateParts {
    /// `(year, month, day)` under `order`, when that is a real calendar date.
    fn read(&self, order: DateOrder) -> Option<(u32, u32, u32)> {
        let (year, month, day) = if self.year_first {
            (self.first, self.second, self.third)
        } else {
            match order {
                DateOrder::MonthFirst => (self.third, self.first, self.second),
                DateOrder::DayFirst => (self.third, self.second, self.first),
            }
        };
        // A two-digit year uses hledger's (and chrono's) window: 69–99 is the
        // twentieth century, 00–68 the twenty-first. Quicken's `'` separator
        // marks years from 2000, which the window reads the same way.
        let year = if self.long_year {
            year
        } else if year < 69 {
            2000 + year
        } else {
            1900 + year
        };
        ((1..=12).contains(&month) && (1..=days_in(year, month)).contains(&day))
            .then_some((year, month, day))
    }
}

fn days_in(year: u32, month: u32) -> u32 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The whole file's dates, read under one order.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDates {
    /// `YYYY-MM-DD`, one per input, in input order.
    pub iso: Vec<String>,
    pub guess: DateFormatGuess,
    pub order: DateOrder,
}

/// Read every date in a file under the one order that fits them all.
///
/// `dates` pairs each raw `D` value with its line, for error messages. When
/// month-first and day-first both read every date, month-first wins unless
/// day-first keeps more consecutive dates in order (registers are almost
/// always sorted by date), and the guess is flagged ambiguous either way.
///
/// # Errors
///
/// [`QifError::UnreadableDate`] for a value that is not a date under any
/// order; [`QifError::InconsistentDates`] when each order fails on some date.
pub fn resolve_dates(dates: &[(&str, usize)]) -> Result<ResolvedDates, QifError> {
    let parts: Vec<DateParts> = dates
        .iter()
        .map(|(raw, line)| {
            date_parts(raw).ok_or_else(|| QifError::UnreadableDate {
                line: *line,
                value: clip(raw),
            })
        })
        .collect::<Result<_, _>>()?;

    let reads = |order: DateOrder| -> Option<Vec<(u32, u32, u32)>> {
        parts.iter().map(|part| part.read(order)).collect()
    };
    let month_first = reads(DateOrder::MonthFirst);
    let day_first = reads(DateOrder::DayFirst);
    let order_questioned = parts.iter().any(|part| !part.year_first);

    let (order, read, ambiguous) = match (month_first, day_first) {
        (Some(m), Some(d)) if order_questioned && m != d => {
            if in_order_count(&d) > in_order_count(&m) {
                (DateOrder::DayFirst, d, true)
            } else {
                (DateOrder::MonthFirst, m, true)
            }
        }
        (Some(m), _) => (DateOrder::MonthFirst, m, false),
        (None, Some(d)) => (DateOrder::DayFirst, d, false),
        (None, None) => {
            // Name the first date month-first cannot read: that is the
            // reading most QIF writers use, so it is the likeliest culprit.
            let at = parts
                .iter()
                .position(|part| part.read(DateOrder::MonthFirst).is_none())
                .unwrap_or(0);
            let (raw, line) = dates.get(at).copied().unwrap_or(("", 0));
            return Err(QifError::InconsistentDates {
                line,
                value: clip(raw),
            });
        }
    };

    Ok(ResolvedDates {
        iso: read
            .iter()
            .map(|(y, m, d)| format!("{y:04}-{m:02}-{d:02}"))
            .collect(),
        guess: DateFormatGuess {
            format: parts
                .iter()
                .find(|part| !part.year_first)
                .or_else(|| parts.first())
                .map_or_else(|| "%Y-%m-%d".to_string(), |part| format_of(part, order)),
            confidence: if ambiguous { 0.5 } else { 1.0 },
            ambiguous,
        },
        order,
    })
}

/// How many dates are on or after the one before them.
fn in_order_count(dates: &[(u32, u32, u32)]) -> usize {
    dates.windows(2).filter(|pair| pair[0] <= pair[1]).count()
}

/// A `date-format`-style description of how `part` was read, e.g. `%m/%d'%y`.
fn format_of(part: &DateParts, order: DateOrder) -> String {
    let year = if part.long_year { "%Y" } else { "%y" };
    let [s1, s2] = part.separators;
    let (a, b, c) = if part.year_first {
        (year, "%m", "%d")
    } else {
        match order {
            DateOrder::MonthFirst => ("%m", "%d", year),
            DateOrder::DayFirst => ("%d", "%m", year),
        }
    };
    format!("{a}{s1}{b}{s2}{c}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Dec {
        Dec::parse(text, '.').expect("test literal parses")
    }

    #[test]
    fn a_single_account_bank_file_parses() {
        let file = parse(
            b"!Type:Bank\nD01/05/2026\nT-42.50\nPCorner Grocer\nMWeekly shop\nN1001\nC*\nLFood:Groceries/Household\n^\n",
        )
        .expect("parses");
        assert_eq!(file.registers.len(), 1);
        let register = &file.registers[0];
        assert_eq!(register.kind, RegisterKind::Bank);
        assert_eq!(register.account, None);
        let record = &register.records[0];
        assert_eq!(record.line, 2);
        assert_eq!(record.date.as_deref(), Some("01/05/2026"));
        assert_eq!(record.amount, Some(dec("-42.50")));
        assert_eq!(record.payee.as_deref(), Some("Corner Grocer"));
        assert_eq!(record.memo.as_deref(), Some("Weekly shop"));
        assert_eq!(record.number.as_deref(), Some("1001"));
        assert_eq!(record.status, Some('*'));
        assert_eq!(record.category.as_deref(), Some("Food:Groceries/Household"));
    }

    #[test]
    fn splits_pair_category_memo_and_amount() {
        let file = parse(
            b"!Type:Bank\nD1/5/26\nT-100.00\nSFood\nEbread\n$-60.00\nS[Savings]\n$-40.00\n^\n",
        )
        .expect("parses");
        let splits = &file.registers[0].records[0].splits;
        assert_eq!(splits.len(), 2);
        assert_eq!(splits[0].category.as_deref(), Some("Food"));
        assert_eq!(splits[0].memo.as_deref(), Some("bread"));
        assert_eq!(splits[0].amount, Some(dec("-60")));
        assert_eq!(splits[1].category.as_deref(), Some("[Savings]"));
        assert_eq!(splits[1].memo, None);
        assert_eq!(splits[1].amount, Some(dec("-40")));
    }

    #[test]
    fn an_account_record_names_the_register_that_follows() {
        let file = parse(
            b"!Account\nNChecking\nTBank\n^\n!Type:Bank\nD1/5/26\nT1\n^\n!Account\nNVisa\nTCCard\n^\n!Type:CCard\nD1/6/26\nT-2\n^\n",
        )
        .expect("parses");
        let names: Vec<_> = file
            .registers
            .iter()
            .map(|r| r.account.as_ref().map(|a| a.name.clone()))
            .collect();
        assert_eq!(
            names,
            vec![Some("Checking".to_string()), Some("Visa".to_string())]
        );
        assert_eq!(file.registers[1].kind, RegisterKind::CreditCard);
    }

    #[test]
    fn an_autoswitch_list_does_not_choose_the_register_account() {
        let file = parse(
            b"!Option:AutoSwitch\n!Account\nNChecking\nTBank\n^\nNSavings\nTBank\n^\n!Clear:AutoSwitch\n!Account\nNChecking\nTBank\n^\n!Type:Bank\nD1/5/26\nT1\n^\n",
        )
        .expect("parses");
        assert_eq!(file.accounts.len(), 2, "the repeated Checking is kept once");
        assert_eq!(
            file.registers[0].account.as_ref().map(|a| a.name.as_str()),
            Some("Checking")
        );
    }

    #[test]
    fn repeated_category_and_class_lists_are_kept_once() {
        let file = parse(
            b"!Type:Cat\nNFood\nE\n^\nNSalary\nI\n^\n!Type:Class\nNHousehold\n^\n!Type:Cat\nNFood\nE\n^\n!Type:Class\nNHousehold\n^\n",
        )
        .expect("parses");
        assert_eq!(file.categories.len(), 2);
        assert!(file.categories[1].income);
        assert_eq!(file.classes.len(), 1);
    }

    #[test]
    fn investment_fields_are_read_in_an_investment_register() {
        let file = parse(
            b"!Type:Invst\nD1/5/26\nNBuyX\nYACME\nI15.00\nQ10\nO4.95\nT154.95\nL[Checking]\n$154.95\n^\n",
        )
        .expect("parses");
        let record = &file.registers[0].records[0];
        assert_eq!(record.action.as_deref(), Some("BuyX"));
        assert_eq!(record.number, None);
        assert_eq!(record.security.as_deref(), Some("ACME"));
        assert_eq!(record.price, Some(dec("15")));
        assert_eq!(record.quantity, Some(dec("10")));
        assert_eq!(record.commission, Some(dec("4.95")));
        assert_eq!(record.transfer_amount, Some(dec("154.95")));
        assert!(record.splits.is_empty());
    }

    #[test]
    fn unknown_sections_are_skipped_and_named() {
        let file = parse(b"!Type:Memorized\nPNetflix\nT-9.99\n^\n!Type:Bank\nD1/5/26\nT1\n^\n")
            .expect("parses");
        assert_eq!(file.registers[0].records.len(), 1);
        assert_eq!(file.skipped_sections, vec!["!Type:Memorized".to_string()]);
    }

    #[test]
    fn a_missing_final_caret_still_ends_the_record() {
        let file = parse(b"!Type:Bank\nD1/5/26\nT1\n!Type:Cat\nNFood\n^\n").expect("parses");
        assert_eq!(file.registers[0].records.len(), 1);
        assert_eq!(file.categories.len(), 1);
    }

    #[test]
    fn crlf_bom_and_windows_1252_are_read() {
        let mut bytes =
            b"\xEF\xBB\xBF!Type:Bank\r\nD1/5/26\r\nT1\r\nPCaf\xC3\xA9\r\n^\r\n".to_vec();
        let file = parse(&bytes).expect("utf-8 parses");
        assert_eq!(file.registers[0].records[0].payee.as_deref(), Some("Café"));
        bytes = b"!Type:Bank\r\nD1/5/26\r\nT1\r\nPCaf\xE9\r\n^\r\n".to_vec();
        let file = parse(&bytes).expect("1252 parses");
        assert_eq!(file.registers[0].records[0].payee.as_deref(), Some("Café"));
    }

    #[test]
    fn refusals() {
        assert_eq!(parse(b""), Err(QifError::Empty));
        assert_eq!(parse(b"Date,Amount\n1/5/26,1\n"), Err(QifError::NotQif));
        assert!(matches!(
            parse(b"!Type:Bank\nD1/5/26\nTabc\n^\n"),
            Err(QifError::UnreadableAmount { line: 3, .. })
        ));
        let big = vec![b' '; MAX_INPUT_BYTES + 1];
        assert!(matches!(parse(&big), Err(QifError::TooLarge { .. })));
    }

    #[test]
    fn amounts_in_either_locale() {
        let read = |value: &str| amount(value, 1).expect("reads").expect("present");
        assert_eq!(read("1,234.56"), dec("1234.56"));
        assert_eq!(read("-1.234,56"), dec("-1234.56"));
        assert_eq!(read("12,5"), dec("12.5"));
        assert_eq!(read("1,234"), dec("1234"));
        assert_eq!(read("$ 12.00"), dec("12"));
        assert_eq!(read("+3"), dec("3"));
        assert_eq!(amount("", 1), Ok(None));
        assert!(amount("1e5", 1).is_err());
        assert!(amount("-", 1).is_err());
        assert!(amount("99999999999999999999999999999999999999999", 1).is_err());
    }

    #[test]
    fn detection() {
        assert!(sniff(b"!Type:Bank\nD1/5/26\n"));
        assert!(sniff(b"\xEF\xBB\xBF\n  !Account\n"));
        assert!(sniff(b"!Option:AutoSwitch\n"));
        assert!(!sniff(b"Date,Description,Amount\n"));
        assert!(!sniff(b"OFXHEADER:100\n"));
        assert!(has_qif_extension("Export.QIF"));
        assert!(has_qif_extension("x.qmtf"));
        assert!(!has_qif_extension("x.csv"));
        assert!(!has_qif_extension("qif"));
    }

    fn resolve(raw: &[&str]) -> Result<ResolvedDates, QifError> {
        let dates: Vec<(&str, usize)> = raw.iter().map(|value| (*value, 1)).collect();
        resolve_dates(&dates)
    }

    #[test]
    fn month_first_wins_when_a_day_is_above_twelve() {
        let dates = resolve(&["1/5/2026", "1/17/2026"]).expect("reads");
        assert_eq!(dates.iso, vec!["2026-01-05", "2026-01-17"]);
        assert_eq!(dates.order, DateOrder::MonthFirst);
        assert!(!dates.guess.ambiguous);
        assert_eq!(dates.guess.format, "%m/%d/%Y");
    }

    #[test]
    fn day_first_wins_when_the_first_number_is_above_twelve() {
        let dates = resolve(&["5.1.98", "17.1.98"]).expect("reads");
        assert_eq!(dates.iso, vec!["1998-01-05", "1998-01-17"]);
        assert_eq!(dates.order, DateOrder::DayFirst);
        assert!(!dates.guess.ambiguous);
        assert_eq!(dates.guess.format, "%d.%m.%y");
    }

    #[test]
    fn quicken_apostrophe_years_and_padding() {
        let dates = resolve(&["1/ 5'98", "12/31'04", "2/29'24"]).expect("reads");
        assert_eq!(dates.iso, vec!["1998-01-05", "2004-12-31", "2024-02-29"]);
        assert_eq!(dates.guess.format, "%m/%d'%y");
    }

    #[test]
    fn iso_dates_need_no_order() {
        let dates = resolve(&["2026-01-05", "2026-02-01"]).expect("reads");
        assert_eq!(dates.iso, vec!["2026-01-05", "2026-02-01"]);
        assert!(!dates.guess.ambiguous);
    }

    #[test]
    fn small_days_are_ambiguous_and_the_sorted_reading_is_preferred() {
        // Month-first reads Jan 2, Feb 1, Mar 1 (sorted). Day-first reads
        // Feb 1, Jan 2, Jan 3: out of order.
        let dates = resolve(&["01/02/2026", "02/01/2026", "03/01/2026"]).expect("reads");
        assert!(dates.guess.ambiguous);
        assert_eq!(dates.order, DateOrder::MonthFirst);
        // Here day-first is the sorted one.
        let dates = resolve(&["02/01/2026", "01/02/2026", "01/03/2026"]).expect("reads");
        assert!(dates.guess.ambiguous);
        assert_eq!(dates.order, DateOrder::DayFirst);
        assert_eq!(dates.iso, vec!["2026-01-02", "2026-02-01", "2026-03-01"]);
    }

    #[test]
    fn identical_readings_are_not_ambiguous() {
        let dates = resolve(&["3/3/2026"]).expect("reads");
        assert!(!dates.guess.ambiguous);
    }

    #[test]
    fn dates_neither_order_reads_are_refused() {
        assert!(matches!(
            resolve(&["1/13/2026", "13/1/2026"]),
            Err(QifError::InconsistentDates { .. })
        ));
        assert!(matches!(
            resolve(&["yesterday"]),
            Err(QifError::UnreadableDate { .. })
        ));
        assert!(matches!(
            resolve(&["2/30/2026"]),
            Err(QifError::InconsistentDates { .. })
        ));
        assert!(matches!(
            resolve(&["1/2/3/4"]),
            Err(QifError::UnreadableDate { .. })
        ));
    }

    proptest::proptest! {
        /// Hostile bytes never panic: they parse or are refused.
        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..2048)) {
            let _ = parse(&bytes);
            let _ = sniff(&bytes);
        }

        /// Arbitrary lines under a register header never panic either — this
        /// reaches the field and number readers, which raw bytes rarely do.
        #[test]
        fn arbitrary_register_lines_never_panic(
            lines in proptest::collection::vec("[DTUPMNCLSE$YIQO^!%A][ -~]{0,24}", 0..64)
        ) {
            let text = format!("!Type:Invst\n{}\n!Type:Bank\n{}\n", lines.join("\n"), lines.join("\n"));
            let _ = parse(text.as_bytes());
        }

        /// Arbitrary date-ish strings never panic.
        #[test]
        fn arbitrary_dates_never_panic(raw in proptest::collection::vec("[0-9/'. -]{0,14}", 1..8)) {
            let dates: Vec<(&str, usize)> = raw.iter().map(|value| (value.as_str(), 1)).collect();
            let _ = resolve_dates(&dates);
        }
    }
}
