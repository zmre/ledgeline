//! Posting-row CSV: one row per **posting**, grouped back into transactions
//! (plan 39 Phase 4, import half).
//!
//! A CSV can carry a transaction with more than two accounts only by giving each
//! posting its own row and sharing a key between them. hledger cannot read that
//! shape back, not even its own `print -O csv`: a rules file makes one
//! transaction per record. So this importer is our own Rust, and it produces the
//! same [`QbJournal`] the QuickBooks Journal importer does. The server's
//! multi-posting pipeline (alias mapping of source account names, `id:` dedupe,
//! write) then handles every source alike.
//!
//! # Presets
//!
//! | [`Preset`]               | Grouped by                          | `id:`                       |
//! |--------------------------|-------------------------------------|-----------------------------|
//! | [`Preset::HledgerCsv`]   | `txnidx`                            | the `id:` tag, else a hash  |
//! | [`Preset::Xero`]         | `Journal ID` / `Journal Number`     | `xero-<journal>`            |
//! | [`Preset::GnuCash`]      | `Transaction ID`, carried down      | `gnucash-<guid>`            |
//!
//! Every column is found **by header name**, never by position: hledger 1.99.4
//! swapped the order of `debit` and `credit`, Xero lets a user add and remove
//! report columns, and GnuCash has two layouts. Header names are compared with
//! case, spacing and punctuation folded away, so `Amount Num.` and `amount num`
//! are the same column.
//!
//! ## Grouping strategies
//!
//! - **Key** (hledger, Xero): every row names its transaction. Rows sharing a key
//!   are one transaction, whether or not they are adjacent.
//! - **Key carried down** (GnuCash multi-line): only a transaction's first row
//!   holds its ID, date and description; the split rows under it inherit them.
//!   GnuCash's "simple layout" repeats the ID on every row, which the same rule
//!   reads unchanged.
//! - **Balanced runs** (Xero without a journal column): consecutive rows are cut
//!   into a transaction each time their running total returns to zero. A ledger
//!   sorted by account (GL Detail without `Journal ID`) never balances that way
//!   and is refused at the row where it fails, which is the right outcome:
//!   guessing would join unrelated lines.
//!
//! Every transaction must balance exactly. A file that does not is refused whole,
//! naming the row, because a whole-book import with one transaction quietly
//! dropped is worse than no import.
//!
//! ## What `id:` is
//!
//! The server writes [`QbTransaction::id`] verbatim as the `id:` tag that
//! re-import dedupes on, so it has to be the same every time the same source
//! transaction is exported. Xero journal numbers and GnuCash GUIDs are. hledger's
//! `txnidx` is **not**: it is a position in one report, so the same transaction
//! has a different index in next month's export. An hledger row group uses the
//! transaction's own `id:` tag when its `comment` carries one (so a Ledgeline
//! journal exported and re-imported dedupes against itself) and otherwise a
//! [content hash](content_id) with an occurrence index.
//!
//! ## One commodity per file
//!
//! [`QbPosting::amount`] is a bare number; the server supplies the commodity. A
//! file whose rows hold more than one commodity (hledger's `commodity` column,
//! GnuCash's `Commodity/Currency`) is refused with
//! [`GroupedCsvError::MixedCommodities`] rather than adding `10 AAPL` to dollars.
//!
//! ## What does not come through
//!
//! [`QbTransaction`] has no slot for a secondary date, a transaction comment, a
//! per-posting status or a transaction note, so hledger's `date2`, `comment`
//! (other than its `id:`) and `posting-status`, and GnuCash's `Notes`, `Action`
//! and `Reconcile Date`, are dropped. `docs/imports/other-apps.md` lists these
//! per app.
//!
//! ## Row numbers
//!
//! Errors quote the row a user sees in their spreadsheet: the header's own row
//! plus any preamble lines above it are counted. Blank lines inside the table
//! are dropped by the shared reader before this module sees them, so a file with
//! blank lines in the middle reads a little low below the first one.
//!
//! [`crate::single_entry`] shares this module's table reader, amount and date
//! readers, error type and id rules.

use crate::convert::delimited::{self, MAX_ROWS};
use crate::convert::{self, ConvertError, ConvertNote, MAX_INPUT_BYTES, SourceFormat, Tabular};
use crate::decimal::Dec;
use crate::edit::render_dec;
use crate::qb_journal::{ImportSource, ImportStatus, QbJournal, QbPosting, QbTransaction};
use crate::rules::generate::{DateFormatGuess, guess_date_format};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------

/// Which posting-row export a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    /// hledger's `print -O csv`.
    HledgerCsv,
    /// Xero Journal Report or General Ledger Detail.
    Xero,
    /// GnuCash's "Export Transactions to CSV", multi-line or simple layout.
    GnuCash,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::HledgerCsv, Self::Xero, Self::GnuCash];

    /// The [`ImportSource`] a journal parsed with this preset carries.
    #[must_use]
    pub fn source(self) -> ImportSource {
        match self {
            Self::HledgerCsv => ImportSource::HledgerCsv,
            Self::Xero => ImportSource::Xero,
            Self::GnuCash => ImportSource::GnuCash,
        }
    }

    /// Does this header row belong to this preset? See [`detect_preset`].
    fn matches(self, header: &Header) -> bool {
        match self {
            Self::HledgerCsv => {
                header.has_all(&["txnidx", "account", "postingcomment"])
                    && (header.has("amount") || header.has_all(&["debit", "credit"]))
            }
            Self::Xero => {
                header.find(XERO_KEY).is_some()
                    && header.find(XERO_ACCOUNT).is_some()
                    && header.has_all(&["debit", "credit"])
            }
            Self::GnuCash => {
                header.has("transactionid")
                    && header.find(GNUCASH_ACCOUNT).is_some()
                    && header.find(GNUCASH_AMOUNT).is_some()
            }
        }
    }
}

/// A parsed file plus what the parse decided along the way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parsed {
    pub journal: QbJournal,
    pub report: ImportReport,
}

/// Counts and notes a caller shows beside the parsed transactions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    /// Data rows read below the header.
    pub rows: usize,
    /// Rows that were neither a posting nor a transaction header (a report's
    /// heading or total line), skipped.
    pub skipped_rows: usize,
    /// Whole source transactions left out on purpose: GnuCash voided ones,
    /// Copilot rows marked `excluded`.
    pub skipped_transactions: usize,
    /// Single-entry transfers whose two sides were joined into one transaction.
    pub transfers_paired: usize,
    /// Single-entry transfer rows with no partner, kept against their category.
    pub transfers_unpaired: usize,
    /// What the shared CSV reader decided (encoding, delimiter, preamble).
    pub notes: Vec<ConvertNote>,
}

/// Why a posting-row or single-entry export was refused. Every refusal rejects
/// the whole file. Row numbers are 1-based spreadsheet rows (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum GroupedCsvError {
    #[error("the file is empty")]
    Empty,
    #[error("the file is larger than the {limit}-byte limit")]
    TooLarge { limit: usize },
    #[error("the file could not be read: {detail}")]
    Unreadable { detail: String },
    #[error("the file has more than {limit} rows")]
    TooManyRows { limit: usize },
    #[error("the header row does not match any supported export")]
    NotRecognised,
    #[error("the export has no {column} column")]
    MissingColumn { column: String },
    #[error("row {row} belongs to no transaction (it has no transaction key)")]
    MissingKey { row: usize },
    #[error("row {row} has no account")]
    MissingAccount { row: usize },
    #[error("row {row} has no date")]
    MissingDate { row: usize },
    #[error("row {row} has no amount")]
    MissingAmount { row: usize },
    #[error("row {row} has an unreadable amount: {cell}")]
    MalformedAmount { row: usize, cell: String },
    #[error("row {row} has both a debit and a credit")]
    AmountNotSplit { row: usize },
    #[error("row {row}'s amounts are too large to add up")]
    AmountOverflow { row: usize },
    #[error(
        "the transaction starting at row {row} does not balance (off by {})",
        render_dec(*total, '.')
    )]
    Unbalanced { row: usize, total: Dec },
    #[error(
        "the export holds more than one commodity ({}); export one at a time",
        commodities.join(", ")
    )]
    MixedCommodities { commodities: Vec<String> },
    #[error("no date format reads every date in the export")]
    UnreadableDates,
    #[error("row {row}'s date {value} cannot be read as {format}")]
    UnreadableDate {
        row: usize,
        value: String,
        format: String,
    },
    #[error("the export holds no transactions")]
    NoTransactions,
}

/// Which posting-row preset, if any, `bytes` is.
///
/// Conservative on purpose: a false positive takes an ordinary bank statement
/// away from the rules pipeline it needs. Only a delimited file (by `name` and
/// content, via [`convert::detect`]) whose header carries a preset's
/// distinguishing columns matches:
///
/// - hledger: `txnidx`, `account`, `posting-comment`, and `amount` or
///   `debit`+`credit`;
/// - Xero: a journal column (`Journal ID`, `Journal Number`, `Journal No`,
///   `Journal`), an account column, `Debit` and `Credit`;
/// - GnuCash: `Transaction ID`, `Full Account Name` or `Account Name`, and an
///   amount or value column.
///
/// No bank CSV has a transaction-key column, which is what every rule leans on.
#[must_use]
pub fn detect_preset(name: &str, bytes: &[u8]) -> Option<Preset> {
    let header = header_of(name, bytes)?;
    Preset::ALL
        .into_iter()
        .find(|preset| preset.matches(&header))
}

/// Is `bytes` any posting-row preset? See [`detect_preset`].
#[must_use]
pub fn detect(name: &str, bytes: &[u8]) -> bool {
    detect_preset(name, bytes).is_some()
}

/// Parse `bytes` as whichever preset its header matches.
///
/// # Errors
///
/// [`GroupedCsvError::NotRecognised`] when no preset matches; otherwise any
/// refusal of [`parse_preset`].
pub fn parse(bytes: &[u8]) -> Result<QbJournal, GroupedCsvError> {
    let table = Table::read(bytes)?;
    let header = Header::new(&table.header);
    let preset = Preset::ALL
        .into_iter()
        .find(|preset| preset.matches(&header))
        .ok_or(GroupedCsvError::NotRecognised)?;
    parse_table(preset, &table).map(|parsed| parsed.journal)
}

/// Parse delimited `bytes` as `preset`.
///
/// # Errors
///
/// Any [`GroupedCsvError`].
pub fn parse_preset(preset: Preset, bytes: &[u8]) -> Result<Parsed, GroupedCsvError> {
    parse_table(preset, &Table::read(bytes)?)
}

/// Parse an already-converted table (an `.xlsx` export read by
/// [`convert::convert`], say) as `preset`.
///
/// # Errors
///
/// Any [`GroupedCsvError`].
pub fn parse_tabular(preset: Preset, tabular: &Tabular) -> Result<Parsed, GroupedCsvError> {
    parse_table(preset, &Table::from_tabular(tabular)?)
}

fn parse_table(preset: Preset, table: &Table) -> Result<Parsed, GroupedCsvError> {
    let mut report = table.report();
    let (drafts, format) = match preset {
        Preset::HledgerCsv => hledger(table)?,
        Preset::Xero => xero(table, &mut report)?,
        Preset::GnuCash => gnucash(table, &mut report)?,
    };
    let journal = finish(drafts, format, preset.source(), prefix_of(preset.source()))?;
    Ok(Parsed { journal, report })
}

// ---------------------------------------------------------------------------
// The table, and finding columns by name
// ---------------------------------------------------------------------------

/// A delimited file's header and rows, with what is needed to quote row numbers.
pub(crate) struct Table {
    pub(crate) header: Vec<String>,
    pub(crate) rows: Vec<Vec<String>>,
    /// Spreadsheet row of the header; data row `i` is `first + i + 1`.
    first: usize,
    notes: Vec<ConvertNote>,
}

impl Table {
    /// Read delimited bytes through the shared reader ([`delimited::parse`]):
    /// encoding, delimiter sniffing and preamble trimming are all its.
    pub(crate) fn read(bytes: &[u8]) -> Result<Self, GroupedCsvError> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(GroupedCsvError::TooLarge {
                limit: MAX_INPUT_BYTES,
            });
        }
        let tabular = delimited::parse(bytes, SourceFormat::Csv).map_err(from_convert)?;
        Self::from_tabular(&tabular)
    }

    pub(crate) fn from_tabular(tabular: &Tabular) -> Result<Self, GroupedCsvError> {
        if tabular.truncated {
            return Err(GroupedCsvError::TooManyRows { limit: MAX_ROWS });
        }
        let header = tabular.header.clone().ok_or(GroupedCsvError::Empty)?;
        let preamble = tabular
            .notes
            .iter()
            .find_map(|note| match note {
                ConvertNote::PreambleSkipped { lines } => Some(*lines),
                _ => None,
            })
            .unwrap_or(0);
        Ok(Self {
            header,
            rows: tabular.rows.clone(),
            first: preamble + 1,
            notes: tabular.notes.clone(),
        })
    }

    /// The spreadsheet row number of data row `index`.
    pub(crate) fn row_number(&self, index: usize) -> usize {
        self.first + index + 1
    }

    pub(crate) fn report(&self) -> ImportReport {
        ImportReport {
            rows: self.rows.len(),
            notes: self.notes.clone(),
            ..ImportReport::default()
        }
    }
}

/// A trimmed, non-empty cell, or `None`.
pub(crate) fn cell(row: &[String], column: Option<usize>) -> Option<&str> {
    column
        .and_then(|index| row.get(index))
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
}

/// Owned [`cell`].
pub(crate) fn owned(row: &[String], column: Option<usize>) -> Option<String> {
    cell(row, column).map(str::to_string)
}

/// A header row with its names folded for comparison.
pub(crate) struct Header {
    names: Vec<String>,
}

impl Header {
    pub(crate) fn new(raw: &[String]) -> Self {
        Self {
            names: raw.iter().map(|name| fold(name)).collect(),
        }
    }

    /// The first column whose folded name is any of `names` (already folded),
    /// tried in the order given.
    pub(crate) fn find(&self, names: &[&str]) -> Option<usize> {
        names
            .iter()
            .find_map(|wanted| self.names.iter().position(|name| name == wanted))
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        self.find(&[name]).is_some()
    }

    pub(crate) fn has_all(&self, names: &[&str]) -> bool {
        names.iter().all(|name| self.has(name))
    }

    /// [`Self::find`], or a refusal naming the column.
    pub(crate) fn require(&self, names: &[&str], label: &str) -> Result<usize, GroupedCsvError> {
        self.find(names)
            .ok_or_else(|| GroupedCsvError::MissingColumn {
                column: label.to_string(),
            })
    }
}

/// Lowercase ASCII alphanumerics only: `Amount Num.` → `amountnum`,
/// `posting-comment` → `postingcomment`, `Category Group/Category` →
/// `categorygroupcategory`.
pub(crate) fn fold(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The header of a delimited file, or `None` for anything that is not one.
pub(crate) fn header_of(name: &str, bytes: &[u8]) -> Option<Header> {
    convert::detect(name, bytes)
        .ok()
        .filter(|format| format.is_delimited())?;
    let tabular = delimited::parse(bytes, SourceFormat::Csv).ok()?;
    tabular.header.as_deref().map(Header::new)
}

fn from_convert(error: ConvertError) -> GroupedCsvError {
    match error {
        ConvertError::Empty => GroupedCsvError::Empty,
        ConvertError::TooLarge { limit } => GroupedCsvError::TooLarge { limit },
        other => GroupedCsvError::Unreadable {
            detail: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Amounts
// ---------------------------------------------------------------------------

/// The decimal mark a set of amount cells agrees on.
///
/// Exports follow the user's locale, and hledger's own CSV keeps each
/// commodity's display style (`1000,00` for a EUR amount). So the mark is voted
/// on across the file: a cell with both marks says the later one is decimal; a
/// mark used twice is grouping; a lone mark followed by other than three digits
/// is decimal. A lone mark followed by exactly three digits says nothing. With
/// no votes the answer is `.`.
pub(crate) fn decimal_mark<'a>(cells: impl Iterator<Item = &'a str>) -> char {
    let (dots, commas) = cells.filter_map(|text| vote(&digits_only(text))).fold(
        (0usize, 0usize),
        |(dots, commas), mark| match mark {
            '.' => (dots + 1, commas),
            _ => (dots, commas + 1),
        },
    );
    if commas > dots { ',' } else { '.' }
}

fn vote(text: &str) -> Option<char> {
    let last_dot = text.rfind('.');
    let last_comma = text.rfind(',');
    match (last_dot, last_comma) {
        (Some(dot), Some(comma)) => Some(if dot > comma { '.' } else { ',' }),
        (Some(at), None) | (None, Some(at)) => {
            let mark = if last_dot.is_some() { '.' } else { ',' };
            let other = if mark == '.' { ',' } else { '.' };
            if text.matches(mark).count() > 1 {
                Some(other)
            } else {
                let trailing = text[at + 1..].chars().filter(char::is_ascii_digit).count();
                (trailing != 3).then_some(mark)
            }
        }
        (None, None) => None,
    }
}

/// `text` with currency symbols, codes and spaces removed: what is left is the
/// digits, both marks and the sign characters.
fn digits_only(text: &str) -> String {
    text.chars()
        .filter(|c| {
            c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | '(' | ')' | '\u{2212}')
        })
        .collect()
}

/// Read one money cell: `$1,234.56`, `-12.00`, `(12.00)`, `12,00 €`, `USD 5`.
///
/// `Ok(None)` for an empty cell. Anything with no digit at all, or that
/// [`Dec::parse`] refuses, is `Err` with the cell's text.
pub(crate) fn money(text: Option<&str>, mark: char) -> Result<Option<Dec>, String> {
    let Some(text) = text else { return Ok(None) };
    let kept = digits_only(text).replace('\u{2212}', "-");
    if !kept.chars().any(|c| c.is_ascii_digit()) {
        return Err(text.to_string());
    }
    let (parenthesised, body) = match kept
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(inner) => (true, inner.to_string()),
        None => (false, kept.clone()),
    };
    // A trailing minus (`12.00-`) is a common accounting style.
    let (trailing_minus, body) = match body.strip_suffix('-') {
        Some(inner) => (true, inner.to_string()),
        None => (false, body),
    };
    if body.contains(['(', ')']) {
        return Err(text.to_string());
    }
    let value = Dec::parse(&body, mark).map_err(|_| text.to_string())?;
    if parenthesised || trailing_minus {
        value.neg().map(Some).map_err(|_| text.to_string())
    } else {
        Ok(Some(value))
    }
}

/// [`money`], with the refusal naming `row`.
pub(crate) fn money_at(
    text: Option<&str>,
    mark: char,
    row: usize,
) -> Result<Option<Dec>, GroupedCsvError> {
    money(text, mark).map_err(|cell| GroupedCsvError::MalformedAmount { row, cell })
}

/// `debit - credit`, refusing a row that fills both with non-zero values.
pub(crate) fn debit_credit(
    debit: Option<Dec>,
    credit: Option<Dec>,
    row: usize,
) -> Result<Dec, GroupedCsvError> {
    match (debit, credit) {
        (Some(d), Some(c)) if !d.is_zero() && !c.is_zero() => {
            Err(GroupedCsvError::AmountNotSplit { row })
        }
        (None, None) => Err(GroupedCsvError::MissingAmount { row }),
        (d, c) => d
            .unwrap_or_else(Dec::zero)
            .sub(c.unwrap_or_else(Dec::zero))
            .map_err(|_| GroupedCsvError::AmountOverflow { row }),
    }
}

/// The amount columns' cells, for [`decimal_mark`].
pub(crate) fn column_cells<'a>(
    table: &'a Table,
    columns: &'a [Option<usize>],
) -> impl Iterator<Item = &'a str> + 'a {
    table
        .rows
        .iter()
        .flat_map(move |row| columns.iter().filter_map(move |column| cell(row, *column)))
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

/// How a file's dates are read: one format for the whole file, chosen by the
/// same catalogue the rules generator uses ([`guess_date_format`]).
pub(crate) struct Dates {
    pub(crate) guess: DateFormatGuess,
}

impl Dates {
    pub(crate) fn guess<'a>(
        values: impl Iterator<Item = &'a str>,
    ) -> Result<Self, GroupedCsvError> {
        let samples: Vec<String> = values
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        guess_date_format(&samples)
            .map(|guess| Self { guess })
            .ok_or(GroupedCsvError::UnreadableDates)
    }

    pub(crate) fn iso(&self, value: &str, row: usize) -> Result<String, GroupedCsvError> {
        iso_date(&self.guess.format, value).ok_or_else(|| GroupedCsvError::UnreadableDate {
            row,
            value: value.to_string(),
            format: self.guess.format.clone(),
        })
    }
}

/// `value` as `YYYY-MM-DD` under `format`, or `None`.
///
/// Not a second format *matcher*: [`guess_date_format`] already chose a format
/// that reads every value. This only asks which run is the year, month and day,
/// in the order the format's specifiers appear. Unlike the QuickBooks importer's
/// numeric-only helper it reads `%b`/`%B` month names, because Xero writes
/// `15 Jan 2026`.
pub(crate) fn iso_date(format: &str, value: &str) -> Option<String> {
    let order: Vec<char> = format
        .split('%')
        .skip(1)
        .filter_map(|spec| spec.trim_start_matches('-').chars().next())
        .filter(|spec| matches!(spec, 'Y' | 'y' | 'm' | 'd' | 'b' | 'B'))
        .collect();
    let mut numbers = value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|run| !run.is_empty());
    let mut words = value
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|run| !run.is_empty());

    let (mut year, mut month, mut day) = (None, None, None);
    for spec in order {
        match spec {
            'Y' => year = numbers.next()?.parse::<u32>().ok(),
            // hledger's and chrono's window: 69..=99 is the 1900s.
            'y' => {
                let digits: u32 = numbers.next()?.parse().ok()?;
                year = Some(if digits < 69 {
                    2000 + digits
                } else {
                    1900 + digits
                });
            }
            'm' => month = numbers.next()?.parse::<u32>().ok(),
            'd' => day = numbers.next()?.parse::<u32>().ok(),
            _ => month = month_number(words.next()?),
        }
    }
    let (year, month, day) = (year?, month?, day?);
    ((1..=9999).contains(&year)
        && (1..=12).contains(&month)
        && (1..=days_in(year, month)).contains(&day))
    .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

fn month_number(word: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower = word.to_ascii_lowercase();
    let prefix = lower.get(..3)?;
    MONTHS
        .iter()
        .position(|month| *month == prefix)
        .and_then(|index| u32::try_from(index + 1).ok())
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

/// Days since 1970-01-01 of an ISO date this module produced.
pub(crate) fn day_number(iso: &str) -> Option<i64> {
    let mut parts = iso.splitn(3, '-').map(str::parse::<i64>);
    let (y, m, d) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

// ---------------------------------------------------------------------------
// Drafts, ids and the finished journal
// ---------------------------------------------------------------------------

/// A transaction before its id is final. `date` is already ISO.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    /// The source's own stable id, or `None` to use [`content_id`].
    pub(crate) id: Option<String>,
    /// Spreadsheet row of the transaction's first row, for refusals.
    pub(crate) row: usize,
    pub(crate) date: String,
    pub(crate) kind: String,
    pub(crate) num: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) postings: Vec<QbPosting>,
    pub(crate) status: ImportStatus,
}

/// A posting with no QuickBooks-only fields.
pub(crate) fn posting(account: &str, amount: Dec, memo: Option<String>) -> QbPosting {
    QbPosting {
        account: account.to_string(),
        amount,
        memo,
        class: None,
        customer: None,
        vendor: None,
        name: None,
        units: None,
    }
}

/// The `id:` prefix for a source's hashed ids.
pub(crate) fn prefix_of(source: ImportSource) -> &'static str {
    match source {
        ImportSource::QuickBooks => "qb",
        ImportSource::Qif => "qif",
        ImportSource::Xero => "xero",
        ImportSource::GnuCash => "gnucash",
        ImportSource::HledgerCsv => "hledger",
        ImportSource::Ynab => "ynab",
        ImportSource::Monarch => "monarch",
        ImportSource::Copilot => "copilot",
    }
}

/// A content hash of what a transaction *is*: its date, payee, and each
/// posting's account and amount, sorted so posting order does not matter.
///
/// Memos are left out on purpose. A memo is the field users most often tidy in
/// the source app between exports, and an edited memo should not turn a
/// transaction already imported into a "new" one. Two transactions that are
/// identical by this measure are told apart by the occurrence index
/// [`finish`] appends, in file order.
///
/// FNV-1a 64, written out here because the id is persisted in journals and so
/// must never change with a Rust release (`std`'s `DefaultHasher` may).
fn content_id(draft: &Draft) -> String {
    let mut legs: Vec<String> = draft
        .postings
        .iter()
        .map(|p| {
            format!(
                "{}\u{1f}{}",
                p.account,
                render_dec(p.amount.normalized(), '.')
            )
        })
        .collect();
    legs.sort();
    let text = format!(
        "{}\u{1e}{}\u{1e}{}",
        draft.date,
        draft.name.as_deref().unwrap_or_default(),
        legs.join("\u{1e}")
    );
    format!("{:016x}", fnv1a(text.as_bytes()))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Check every draft balances, give hashed ones their ids, and wrap them up.
pub(crate) fn finish(
    drafts: Vec<Draft>,
    date_format: DateFormatGuess,
    source: ImportSource,
    prefix: &str,
) -> Result<QbJournal, GroupedCsvError> {
    if drafts.is_empty() {
        return Err(GroupedCsvError::NoTransactions);
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    let transactions = drafts
        .into_iter()
        .map(|draft| {
            let total = draft
                .postings
                .iter()
                .try_fold(Dec::zero(), |sum, p| sum.add(p.amount))
                .map_err(|_| GroupedCsvError::AmountOverflow { row: draft.row })?;
            if !total.is_zero() {
                return Err(GroupedCsvError::Unbalanced {
                    row: draft.row,
                    total,
                });
            }
            let id = match &draft.id {
                Some(id) => id.clone(),
                None => {
                    let hash = content_id(&draft);
                    let occurrence = seen.entry(hash.clone()).or_default();
                    let id = format!("{prefix}-{hash}-{occurrence}");
                    *occurrence += 1;
                    id
                }
            };
            Ok(QbTransaction {
                id,
                date: draft.date,
                transaction_type: draft.kind,
                num: draft.num,
                name: draft.name,
                postings: draft.postings,
                status: draft.status,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QbJournal {
        transactions,
        date_format,
        source,
        account_kinds: std::collections::BTreeMap::new(),
    })
}

// ---------------------------------------------------------------------------
// Grouping
// ---------------------------------------------------------------------------

/// Row indices grouped by key, in order of each key's first appearance.
fn group_by_key(keys: impl Iterator<Item = (usize, String)>) -> Vec<(String, Vec<usize>)> {
    let mut position: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, key) in keys {
        match position.get(&key) {
            Some(&at) => groups[at].1.push(index),
            None => {
                position.insert(key.clone(), groups.len());
                groups.push((key, vec![index]));
            }
        }
    }
    groups
}

/// Consecutive rows cut wherever their running total returns to zero.
///
/// Every row of a run must carry the run's first date (`line.2`). Without that
/// rule a ledger sorted by account, which always sums to zero at its very last
/// row, would come back as one enormous transaction; with it, such a file is
/// refused at the first row whose run fails to close before the date changes.
fn balanced_runs<'a>(
    lines: &[(usize, Dec, Option<&'a str>)],
    table: &Table,
) -> Result<Vec<Vec<usize>>, GroupedCsvError> {
    let mut runs = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut run_date: Option<&'a str> = None;
    let mut total = Dec::zero();
    for &(index, amount, date) in lines {
        if let Some(&first) = current.first()
            && date != run_date
        {
            return Err(GroupedCsvError::Unbalanced {
                row: table.row_number(first),
                total,
            });
        }
        if current.is_empty() {
            run_date = date;
        }
        total = total
            .add(amount)
            .map_err(|_| GroupedCsvError::AmountOverflow {
                row: table.row_number(index),
            })?;
        current.push(index);
        if total.is_zero() && current.len() > 1 {
            runs.push(std::mem::take(&mut current));
        }
    }
    match current.first() {
        None => Ok(runs),
        Some(&first) => Err(GroupedCsvError::Unbalanced {
            row: table.row_number(first),
            total,
        }),
    }
}

/// Refuse a file whose rows name more than one commodity.
fn one_commodity<'a>(values: impl Iterator<Item = &'a str>) -> Result<(), GroupedCsvError> {
    let set: BTreeSet<&str> = values.collect();
    if set.len() > 1 {
        return Err(GroupedCsvError::MixedCommodities {
            commodities: set
                .into_iter()
                .map(|c| {
                    if c.is_empty() {
                        "(none)".to_string()
                    } else {
                        c.to_string()
                    }
                })
                .collect(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// hledger print -O csv
// ---------------------------------------------------------------------------

/// hledger 1.52's columns: `txnidx,date,date2,status,code,description,comment,
/// account,amount,commodity,credit,debit,posting-status,posting-comment`.
/// `amount` is signed; when a file lacks it, `debit - credit` is used.
fn hledger(table: &Table) -> Result<(Vec<Draft>, DateFormatGuess), GroupedCsvError> {
    let header = Header::new(&table.header);
    let key = header.require(&["txnidx"], "txnidx")?;
    let date = header.require(&["date"], "date")?;
    let account = header.require(&["account"], "account")?;
    let amount = header.find(&["amount"]);
    let (debit, credit) = (header.find(&["debit"]), header.find(&["credit"]));
    if amount.is_none() && (debit.is_none() || credit.is_none()) {
        return Err(GroupedCsvError::MissingColumn {
            column: "amount".to_string(),
        });
    }
    let status = header.find(&["status"]);
    let code = header.find(&["code"]);
    let description = header.find(&["description"]);
    let comment = header.find(&["comment"]);
    let commodity = header.find(&["commodity"]);
    let memo = header.find(&["postingcomment"]);

    one_commodity(
        table
            .rows
            .iter()
            .map(|row| cell(row, commodity).unwrap_or_default()),
    )?;
    let mark = decimal_mark(column_cells(table, &[amount, debit, credit]));

    let keyed = table
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            owned(row, Some(key))
                .map(|key| (index, key))
                .ok_or(GroupedCsvError::MissingKey {
                    row: table.row_number(index),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let dates = Dates::guess(table.rows.iter().filter_map(|row| cell(row, Some(date))))?;
    let drafts = group_by_key(keyed.into_iter())
        .into_iter()
        .map(|(_, indices)| {
            let first_index = indices[0];
            let first = &table.rows[first_index];
            let row = table.row_number(first_index);
            let postings = indices
                .iter()
                .map(|&index| {
                    let line = &table.rows[index];
                    let at = table.row_number(index);
                    let name = cell(line, Some(account))
                        .ok_or(GroupedCsvError::MissingAccount { row: at })?;
                    let value = match amount {
                        Some(column) => money_at(cell(line, Some(column)), mark, at)?
                            .ok_or(GroupedCsvError::MissingAmount { row: at })?,
                        None => debit_credit(
                            money_at(cell(line, debit), mark, at)?,
                            money_at(cell(line, credit), mark, at)?,
                            at,
                        )?,
                    };
                    Ok(posting(name, value, owned(line, memo)))
                })
                .collect::<Result<Vec<_>, GroupedCsvError>>()?;
            let raw_date = cell(first, Some(date)).ok_or(GroupedCsvError::MissingDate { row })?;
            Ok(Draft {
                id: cell(first, comment).and_then(|text| tag_value(text, "id")),
                row,
                date: dates.iso(raw_date, row)?,
                kind: "Transaction".to_string(),
                num: owned(first, code),
                name: owned(first, description),
                postings,
                status: match cell(first, status) {
                    Some("*") => ImportStatus::Cleared,
                    Some("!") => ImportStatus::Pending,
                    _ => ImportStatus::Unmarked,
                },
            })
        })
        .collect::<Result<Vec<_>, GroupedCsvError>>()?;
    Ok((drafts, dates.guess))
}

/// The value of hledger tag `name` in a comment: `name: value` where the value
/// runs to the next comma or line end. `None` when absent or empty.
pub(crate) fn tag_value(comment: &str, name: &str) -> Option<String> {
    comment
        .split([',', '\n'])
        .filter_map(|part| part.split_once(':'))
        .find(|(tag, _)| tag.split_whitespace().last() == Some(name))
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

// ---------------------------------------------------------------------------
// Xero Journal Report / General Ledger Detail
// ---------------------------------------------------------------------------

const XERO_KEY: &[&str] = &["journalid", "journalnumber", "journalno", "journal"];
const XERO_ACCOUNT: &[&str] = &["account", "accountname"];

/// Xero: one row per journal line, `Debit`/`Credit` positive.
///
/// The account is `Account Code` and `Account` joined (`200 Sales`), like the
/// QuickBooks importer's numbered names, so two accounts sharing a name stay
/// apart. Rows with no account are a report's heading or total lines (GL
/// Detail writes `Total Sales` in its *Date* column) and are skipped (counted). Reversal journals are ordinary
/// journals and are imported as they are.
fn xero(
    table: &Table,
    report: &mut ImportReport,
) -> Result<(Vec<Draft>, DateFormatGuess), GroupedCsvError> {
    let header = Header::new(&table.header);
    let key = header.find(XERO_KEY);
    let date = header.require(&["date"], "Date")?;
    let account = header.require(XERO_ACCOUNT, "Account")?;
    let code = header.find(&["accountcode", "code"]);
    let debit = header.find(&["debit"]);
    let credit = header.find(&["credit"]);
    let net = header.find(&["net", "netamount", "amount"]);
    if net.is_none() && (debit.is_none() || credit.is_none()) {
        return Err(GroupedCsvError::MissingColumn {
            column: "Debit and Credit".to_string(),
        });
    }
    let description = header.find(&["description"]);
    let reference = header.find(&["reference"]);
    let source = header.find(&["source", "sourcetype"]);
    let contact = header.find(&["contact", "contactname"]);

    let mark = decimal_mark(column_cells(table, &[debit, credit, net]));
    let lines: Vec<usize> = (0..table.rows.len())
        .filter(|&index| cell(&table.rows[index], Some(account)).is_some())
        .collect();
    report.skipped_rows += table.rows.len() - lines.len();

    let amount_of = |index: usize| -> Result<Dec, GroupedCsvError> {
        let row = &table.rows[index];
        let at = table.row_number(index);
        if debit.is_some() && credit.is_some() {
            debit_credit(
                money_at(cell(row, debit), mark, at)?,
                money_at(cell(row, credit), mark, at)?,
                at,
            )
        } else {
            money_at(cell(row, net), mark, at)?.ok_or(GroupedCsvError::MissingAmount { row: at })
        }
    };

    let groups: Vec<(Option<String>, Vec<usize>)> = match key {
        Some(key) => {
            let keyed = lines
                .iter()
                .map(|&index| {
                    owned(&table.rows[index], Some(key))
                        .map(|k| (index, k))
                        .ok_or(GroupedCsvError::MissingKey {
                            row: table.row_number(index),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            group_by_key(keyed.into_iter())
                .into_iter()
                .map(|(k, indices)| (Some(k), indices))
                .collect()
        }
        None => {
            let amounts = lines
                .iter()
                .map(|&index| {
                    amount_of(index)
                        .map(|amount| (index, amount, cell(&table.rows[index], Some(date))))
                })
                .collect::<Result<Vec<_>, _>>()?;
            balanced_runs(&amounts, table)?
                .into_iter()
                .map(|indices| (None, indices))
                .collect()
        }
    };

    let dates = Dates::guess(
        lines
            .iter()
            .filter_map(|&i| cell(&table.rows[i], Some(date))),
    )?;
    let drafts = groups
        .into_iter()
        .map(|(journal, indices)| {
            let first = &table.rows[indices[0]];
            let row = table.row_number(indices[0]);
            let postings = indices
                .iter()
                .map(|&index| {
                    let line = &table.rows[index];
                    let at = table.row_number(index);
                    let name = cell(line, Some(account))
                        .ok_or(GroupedCsvError::MissingAccount { row: at })?;
                    let full = match cell(line, code) {
                        Some(code) if !name.starts_with(code) => format!("{code} {name}"),
                        _ => name.to_string(),
                    };
                    Ok(posting(&full, amount_of(index)?, owned(line, description)))
                })
                .collect::<Result<Vec<_>, GroupedCsvError>>()?;
            let raw_date = cell(first, Some(date)).ok_or(GroupedCsvError::MissingDate { row })?;
            Ok(Draft {
                id: journal.map(|k| format!("xero-{k}")),
                row,
                date: dates.iso(raw_date, row)?,
                kind: owned(first, source).unwrap_or_else(|| "Journal".to_string()),
                num: owned(first, reference),
                name: owned(first, contact),
                postings,
                status: ImportStatus::Unmarked,
            })
        })
        .collect::<Result<Vec<_>, GroupedCsvError>>()?;
    Ok((drafts, dates.guess))
}

// ---------------------------------------------------------------------------
// GnuCash multi-line CSV
// ---------------------------------------------------------------------------

const GNUCASH_ACCOUNT: &[&str] = &["fullaccountname", "accountname"];
const GNUCASH_AMOUNT: &[&str] = &["valuenum", "amountnum", "valuewithsym", "amountwithsym"];

/// GnuCash: `Transaction ID`, `Date`, `Number`, `Description`,
/// `Commodity/Currency` and `Void Reason` sit on a transaction's first row and
/// are carried down to its split rows. A row with an account is a split; its
/// `Value Num.` (in the transaction's currency, so the splits sum to zero) is
/// the amount, debit positive. `Amount Num.` is used only when there is no value
/// column. A split reconciled (`y`) or cleared (`c`) marks the transaction
/// cleared. Voided transactions are skipped and counted.
fn gnucash(
    table: &Table,
    report: &mut ImportReport,
) -> Result<(Vec<Draft>, DateFormatGuess), GroupedCsvError> {
    let header = Header::new(&table.header);
    let key = header.require(&["transactionid"], "Transaction ID")?;
    let account = header.require(GNUCASH_ACCOUNT, "Full Account Name")?;
    let amount = header.require(GNUCASH_AMOUNT, "Value Num.")?;
    let date = header.require(&["date"], "Date")?;
    let number = header.find(&["number", "num"]);
    let description = header.find(&["description"]);
    let currency = header.find(&["commoditycurrency", "currency"]);
    let void = header.find(&["voidreason"]);
    let memo = header.find(&["memo"]);
    let reconcile = header.find(&["reconcile"]);

    let mark = decimal_mark(column_cells(table, &[Some(amount)]));

    // Carry the key down to the split rows; remember each transaction's header.
    struct Head {
        row: usize,
        date: Option<String>,
        num: Option<String>,
        name: Option<String>,
        currency: Option<String>,
        void: bool,
    }
    let mut heads: HashMap<String, Head> = HashMap::new();
    let mut current: Option<String> = None;
    let mut splits: Vec<(usize, String)> = Vec::new();
    for (index, row) in table.rows.iter().enumerate() {
        let at = table.row_number(index);
        if let Some(id) = owned(row, Some(key)) {
            heads.entry(id.clone()).or_insert_with(|| Head {
                row: at,
                date: owned(row, Some(date)),
                num: owned(row, number),
                name: owned(row, description),
                currency: cell(row, currency)
                    .map(|c| c.rsplit("::").next().unwrap_or(c).to_string()),
                void: cell(row, void).is_some(),
            });
            current = Some(id);
        }
        if cell(row, Some(account)).is_some() {
            let id = current
                .clone()
                .ok_or(GroupedCsvError::MissingKey { row: at })?;
            splits.push((index, id));
        } else if cell(row, Some(key)).is_none() {
            report.skipped_rows += 1;
        }
    }

    one_commodity(heads.values().filter_map(|head| head.currency.as_deref()))?;

    let groups = group_by_key(splits.into_iter());
    let live: Vec<(String, Vec<usize>)> = groups
        .into_iter()
        .filter(|(id, _)| !heads.get(id).is_some_and(|head| head.void))
        .collect();
    report.skipped_transactions += heads.values().filter(|head| head.void).count();

    let dates = Dates::guess(
        live.iter()
            .filter_map(|(id, _)| heads.get(id).and_then(|head| head.date.as_deref())),
    )?;
    let drafts = live
        .into_iter()
        .map(|(id, indices)| {
            let head = &heads[&id];
            let postings = indices
                .iter()
                .map(|&index| {
                    let line = &table.rows[index];
                    let at = table.row_number(index);
                    let name = cell(line, Some(account))
                        .ok_or(GroupedCsvError::MissingAccount { row: at })?;
                    let value = money_at(cell(line, Some(amount)), mark, at)?
                        .ok_or(GroupedCsvError::MissingAmount { row: at })?;
                    Ok(posting(name, value, owned(line, memo)))
                })
                .collect::<Result<Vec<_>, GroupedCsvError>>()?;
            let cleared = indices.iter().any(|&index| {
                cell(&table.rows[index], reconcile)
                    .is_some_and(|mark| matches!(mark.to_ascii_lowercase().as_str(), "y" | "c"))
            });
            let raw_date = head
                .date
                .as_deref()
                .ok_or(GroupedCsvError::MissingDate { row: head.row })?;
            Ok(Draft {
                id: Some(format!("gnucash-{id}")),
                row: head.row,
                date: dates.iso(raw_date, head.row)?,
                kind: "Transaction".to_string(),
                num: head.num.clone(),
                name: head.name.clone(),
                postings,
                status: if cleared {
                    ImportStatus::Cleared
                } else {
                    ImportStatus::Unmarked
                },
            })
        })
        .collect::<Result<Vec<_>, GroupedCsvError>>()?;
    Ok((drafts, dates.guess))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Dec {
        Dec::parse(text, '.').expect("literal")
    }

    #[test]
    fn folds_header_names() {
        assert_eq!(fold("Amount Num."), "amountnum");
        assert_eq!(fold("\u{feff}posting-comment"), "postingcomment");
        assert_eq!(fold("Category Group/Category"), "categorygroupcategory");
    }

    #[test]
    fn votes_on_the_decimal_mark() {
        assert_eq!(decimal_mark(["1.234,56", "12,00"].into_iter()), ',');
        assert_eq!(decimal_mark(["1,234.56", "12.00"].into_iter()), '.');
        assert_eq!(decimal_mark(["1000,00", "645,00"].into_iter()), ',');
        assert_eq!(decimal_mark(["1,000", "2,000"].into_iter()), '.');
        assert_eq!(decimal_mark(["1.000.000"].into_iter()), ',');
        assert_eq!(decimal_mark(std::iter::empty()), '.');
    }

    #[test]
    fn reads_money_in_every_common_shape() {
        assert_eq!(money(Some("$1,234.56"), '.'), Ok(Some(dec("1234.56"))));
        assert_eq!(money(Some("(12.00)"), '.'), Ok(Some(dec("-12.00"))));
        assert_eq!(money(Some("12.00-"), '.'), Ok(Some(dec("-12.00"))));
        assert_eq!(money(Some("1.234,50 €"), ','), Ok(Some(dec("1234.50"))));
        assert_eq!(money(Some("\u{2212}5"), '.'), Ok(Some(dec("-5"))));
        assert_eq!(money(None, '.'), Ok(None));
        assert!(money(Some("n/a"), '.').is_err());
        assert!(money(Some("1(2)"), '.').is_err());
    }

    #[test]
    fn iso_dates_with_month_names_and_validation() {
        assert_eq!(
            iso_date("%d %b %Y", "15 Jan 2026").as_deref(),
            Some("2026-01-15")
        );
        assert_eq!(
            iso_date("%-d %b %Y", "1 Sep 2026").as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(
            iso_date("%b %d, %Y", "March 3, 2026").as_deref(),
            Some("2026-03-03")
        );
        assert_eq!(
            iso_date("%m/%d/%y", "02/29/24").as_deref(),
            Some("2024-02-29")
        );
        assert_eq!(iso_date("%m/%d/%Y", "02/30/2026"), None);
        assert_eq!(iso_date("%Y-%m-%d", "2026-13-01"), None);
        assert_eq!(iso_date("%Y-%m-%d", "garbage"), None);
    }

    #[test]
    fn day_numbers_count_days() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(
            day_number("2026-03-01")
                .zip(day_number("2026-02-28"))
                .map(|(a, b)| a - b),
            Some(1)
        );
        assert_eq!(
            day_number("2024-03-01")
                .zip(day_number("2024-02-28"))
                .map(|(a, b)| a - b),
            Some(2)
        );
    }

    #[test]
    fn reads_the_id_tag_from_an_hledger_comment() {
        assert_eq!(
            tag_value("id: abc-123, trip: x", "id").as_deref(),
            Some("abc-123")
        );
        assert_eq!(tag_value("trip: x\nid: 9", "id").as_deref(), Some("9"));
        // hledger reads this as one `note` tag whose value contains `id: 7`.
        assert_eq!(tag_value("note: paid id: 7", "id"), None);
        assert_eq!(tag_value("myid: 7", "id"), None);
        assert_eq!(tag_value("id:", "id"), None);
    }

    #[test]
    fn content_ids_ignore_posting_order_and_memos() {
        let draft = |postings: Vec<QbPosting>| Draft {
            id: None,
            row: 2,
            date: "2026-01-01".to_string(),
            kind: "Transaction".to_string(),
            num: None,
            name: Some("Shop".to_string()),
            postings,
            status: ImportStatus::Unmarked,
        };
        let a = draft(vec![
            posting("A", dec("1.0"), Some("x".to_string())),
            posting("B", dec("-1"), None),
        ]);
        let b = draft(vec![
            posting("B", dec("-1.00"), None),
            posting("A", dec("1"), None),
        ]);
        assert_eq!(content_id(&a), content_id(&b));
    }

    #[test]
    fn balanced_runs_cut_at_zero_and_refuse_leftovers() {
        let table = Table {
            header: vec![],
            rows: vec![vec![]; 5],
            first: 1,
            notes: vec![],
        };
        let day = Some("2026-01-01");
        let lines = [
            (0, dec("5"), day),
            (1, dec("-5"), day),
            (2, dec("3"), day),
            (3, dec("-1"), day),
            (4, dec("-2"), day),
        ];
        assert_eq!(
            balanced_runs(&lines, &table),
            Ok(vec![vec![0, 1], vec![2, 3, 4]])
        );
        let moved = [(0, dec("5"), day), (1, dec("-5"), Some("2026-01-02"))];
        assert_eq!(
            balanced_runs(&moved, &table),
            Err(GroupedCsvError::Unbalanced {
                row: 2,
                total: dec("5")
            })
        );
        assert_eq!(
            balanced_runs(&lines[..4], &table),
            Err(GroupedCsvError::Unbalanced {
                row: 4,
                total: dec("2")
            })
        );
    }
}
