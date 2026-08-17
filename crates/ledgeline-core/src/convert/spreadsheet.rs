//! Workbooks — xls, xlsx, xlsm, xlsb and ods — read through `calamine` and
//! flattened to one [`Tabular`].
//!
//! Everything here is read from an in-memory cursor. The bytes arrive from an
//! upload, this crate never learns where they came from, and so no error it
//! produces can name a path.
//!
//! # A workbook is not a table, and choosing which sheet is a judgement
//!
//! A statement someone saved out of their bank's portal is one sheet of data
//! next to a "Disclaimer" sheet and an empty "Sheet3". [`parse`] takes the first
//! sheet that holds at least [`MIN_TABLE_ROWS`] rows and [`MIN_TABLE_COLUMNS`]
//! columns of actual data, and when more than one sheet qualifies it says so
//! through [`ConvertNote::SheetChosen`] rather than picking quietly. A workbook
//! with no such sheet is [`ConvertError::NoTable`] — a specific answer, not a
//! parse failure, because the file is fine and simply has no statement in it.
//!
//! # Dates: `as_datetime`, never `as_f64` and a division
//!
//! A date cell is a number plus a display format, and `calamine` deliberately
//! exposes **no access to number formats** (the module is private and the
//! request to open it was closed). So `Float(45678.0)` reaches us with nothing
//! attached, and dividing it by anything is how a transaction lands on the wrong
//! day. What `calamine` does give — with the `dates` feature, which is on — is
//! [`calamine::Data::DateTime`], a variant the *reader* produced because the
//! cell's format said date. Only that variant becomes a date here.
//!
//! Two traps that follow from it, both handled in [`date_text`]:
//!
//! - **`Data::as_datetime()` will happily convert an `Int` or a `Float`.** Called
//!   on an amount of `1234.56` it returns 1903-05-18. This module therefore
//!   matches on the variant and never calls the trait method on a number.
//! - **Serial 60 is 1900-02-29, a day that never existed** — Lotus 1-2-3's leap
//!   year bug, preserved by Excel for compatibility. `as_datetime` maps it to
//!   1900-02-28, which is also what serial 59 maps to, so the two collide
//!   silently. Any stamp that reports itself as 1900-02-29 is refused as a date
//!   and emitted as its raw serial instead.
//!
//! The 1904 epoch needs no handling of its own, which contradicts the plan and
//! is worth stating: `calamine` threads the workbook's epoch flag into every
//! [`calamine::ExcelDateTime`] it constructs, so `has_1904_epoch()` is
//! informational and `as_datetime()` is already correct on a Mac-authored
//! workbook. Re-applying the 1,462-day offset on top would move every date four
//! years.
//!
//! # Numbers: the raw value, never a rendered one
//!
//! `1234.56` is emitted as `1234.56` and never as `$1,234.56` — partly because
//! the format is unavailable anyway, but mostly because a rules file matches on
//! this text and a thousands separator would turn one amount into two fields.
//!
//! # Merged cells and the ragged edge
//!
//! A merged range stores its value in the top-left cell and [`calamine::Data::Empty`]
//! in the rest, so a merged title band reads as one populated row followed by
//! empties. [`table_of`] trims fully-blank leading and trailing rows and columns
//! to find the real table, then pads every row to the same width — so a workbook
//! never produces [`ConvertNote::RaggedRows`], because a sheet has no ragged
//! rows to report: it has a rectangle with holes in it.
//!
//! # The preamble: a title is not a header row
//!
//! Trimming the *blank* edges is not enough. A real brokerage export opens with
//! two empty rows, a floating "All Activity Types", another empty row, an
//! "Account Activity for …" line, another empty row, and only then the fifteen
//! column labels. Every one of those rows is inside the trimmed rectangle, so
//! "the first row of the rectangle is the header" reads the title as the header
//! and the whole statement as its body.
//!
//! The delimited lane already solved this by row consistency, and this module
//! calls the same code — [`super::delimited::margins`] — rather than growing a
//! second rule that could answer differently. Nothing here looks at what a cell
//! *says*: no string matching, no row index, no "a title is a row with one cell
//! in it". That last one is the tempting wrong answer, and it turns a genuine
//! one-column sheet into a preamble with nothing after it.
//!
//! What is handed to that rule is [`row_width`] — **one past the row's last
//! populated cell**, not a count of the populated ones. The distinction is the
//! whole thing on a real file. In the export above the header row has fifteen
//! populated cells and the transaction rows have nine, ten or eleven, because
//! `Check Number`, `Cusip` and `Memo` are blank on most of them; scored by
//! population the body agrees on *ten* and the header is discarded along with
//! the titles. Scored by extent every one of those rows ends at column fifteen
//! and the titles end at column one, which is exactly the split we want — and it
//! is also the honest analogue of a delimited record's field count, since
//! `a,b,,,` is five fields and not two.
//!
//! # The trailer: the disclaimer under the transactions
//!
//! The same export closes with twenty-six rows below the last transaction —
//! fourteen entirely blank, twelve holding one paragraph of legal text in column
//! one. Those are not a cosmetic problem. Rendered to CSV they become records
//! hledger cannot read, and hledger abandons the **whole file** on the first
//! one, so a user whose rules file is perfectly correct is told their data will
//! not parse. [`super::delimited::margins`] finds them by the same row-width
//! rule, from the other end: the transaction rows reach column fifteen and every
//! one of the trailer rows reaches column one or column zero.
//!
//! Blank rows are dropped from the *middle* of the table too, for the same
//! reason and with the same guarantee that a real row cannot be caught by it: a
//! transaction has a date in it, and a row with no populated cell at all has
//! nothing.

use super::delimited::{MAX_ROWS, Margins, RowShape, margins};
use super::{ConvertError, ConvertNote, MAX_INPUT_BYTES, SourceFormat, Tabular};
use calamine::{Data, ExcelDateTime, Ods, Range, Reader, Xls, Xlsb, Xlsx, open_workbook_from_rs};
use std::io::{Cursor, Read, Seek};

// ---------------------------------------------------------------------------
// What counts as a table
// ---------------------------------------------------------------------------

/// The fewest rows a sheet may have and still be the statement: a header and one
/// transaction.
const MIN_TABLE_ROWS: usize = 2;

/// The fewest columns a sheet may have and still be the statement. A one-column
/// sheet is a list of notes, a title block or a signature — never something a
/// rules file's `fields` can address, since every rules file needs at least a
/// date and an amount.
const MIN_TABLE_COLUMNS: usize = 2;

/// The detail on a [`ConvertError::Malformed`] from this module.
///
/// Fixed strings, never the underlying reader's message. `calamine`'s errors are
/// very probably free of anything sensitive, but "very probably" is not a
/// property this crate can assert about a dependency's text across versions, and
/// this string reaches a dialog. Same stance, and the same reason, as
/// `rules::discovery`'s refusal to quote an `io::Error`.
const UNREADABLE_WORKBOOK: &str = "the workbook could not be read";

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Read `bytes` as the workbook `format` names and flatten its statement sheet
/// to a table.
///
/// `format` selects the reader rather than letting `calamine` try each in turn:
/// the caller already decided what this file is, and a `.xlsx` that only parses
/// as `.ods` is a file we should refuse rather than quietly re-interpret.
///
/// # Errors
///
/// - [`ConvertError::Empty`] — no bytes at all.
/// - [`ConvertError::TooLarge`] — past [`MAX_INPUT_BYTES`]. Checked here as well
///   as at the HTTP boundary so a caller that skips the server cannot get past it.
/// - [`ConvertError::Unsupported`] — `format` is not read by this backend.
/// - [`ConvertError::Malformed`] — the bytes are not that kind of workbook.
/// - [`ConvertError::NoTable`] — the workbook opened, and no sheet in it holds
///   anything table-shaped.
pub fn parse(bytes: &[u8], format: SourceFormat) -> Result<Tabular, ConvertError> {
    if bytes.is_empty() {
        return Err(ConvertError::Empty);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ConvertError::TooLarge {
            limit: MAX_INPUT_BYTES,
        });
    }

    let source = Cursor::new(bytes);
    match format {
        SourceFormat::Xls => tabulate::<Xls<_>, _>(source, format),
        SourceFormat::Xlsx | SourceFormat::Xlsm => tabulate::<Xlsx<_>, _>(source, format),
        SourceFormat::Xlsb => tabulate::<Xlsb<_>, _>(source, format),
        SourceFormat::Ods => tabulate::<Ods<_>, _>(source, format),
        other => Err(ConvertError::Unsupported {
            ext: other.as_str().to_string(),
        }),
    }
}

/// The winning sheet: its name, and the rectangle [`table_of`] found on it.
type Chosen = Option<(String, Table)>;

/// Open one workbook with a statically known reader, pick its statement sheet
/// and render it.
fn tabulate<R, RS>(source: RS, format: SourceFormat) -> Result<Tabular, ConvertError>
where
    R: Reader<RS>,
    RS: Read + Seek,
{
    let mut book: R = open_workbook_from_rs(source).map_err(|_| malformed(format))?;
    let names = book.sheet_names();

    // Every sheet is examined, not just sheets up to the first hit, because
    // `SheetChosen` is owed to the user precisely when there was more than one
    // thing it could have been. Bounded by `MAX_INPUT_BYTES`.
    let (chosen, candidates) = names.iter().fold(
        (None, 0usize),
        |(chosen, candidates): (Chosen, usize), name| {
            // A sheet that fails to read is skipped rather than fatal: one
            // corrupt "Disclaimer" tab is no reason to refuse a statement
            // that parsed perfectly on the sheet beside it.
            match book.worksheet_range(name).ok().as_ref().and_then(table_of) {
                Some(table) => (
                    chosen.or_else(|| Some((name.clone(), table))),
                    candidates + 1,
                ),
                None => (chosen, candidates),
            }
        },
    );

    let (name, table) = chosen.ok_or(ConvertError::NoTable)?;
    let truncated = table.rows.len() > MAX_ROWS;
    let rendered: Vec<Rendered> = table
        .rows
        .into_iter()
        .take(MAX_ROWS)
        .map(render_row)
        .collect();
    let serial_dates: usize = rendered.iter().map(|row| row.serial_dates).sum();

    let mut rows = rendered.into_iter().map(|row| row.cells);
    // `table_of` already refused anything with fewer than `MIN_TABLE_ROWS`, so
    // the header is always there; `ok_or` states that rather than asserting it.
    let header = rows.next().ok_or(ConvertError::NoTable)?;

    Ok(Tabular {
        header: Some(header),
        rows: rows.collect(),
        truncated,
        statement: None,
        notes: notes(&name, names.len(), candidates, &table.dropped, serial_dates),
    })
}

/// A [`ConvertError::Malformed`] that quotes nothing the reader said.
fn malformed(format: SourceFormat) -> ConvertError {
    ConvertError::Malformed {
        format,
        detail: UNREADABLE_WORKBOOK.to_string(),
    }
}

/// Everything the conversion decided, in a fixed order so two runs over the same
/// bytes produce the same list.
///
/// The counts in `dropped` are rows of the sheet's *data region* — the rectangle
/// left once the blank edges are gone — so they are the same quantities the
/// delimited lane reports: rows that were there, that we dropped, and that the
/// user would otherwise wonder about. Blank rows at the very top and the very
/// bottom of the *sheet* are not in them, because trimming those is not a
/// judgement call and never was; they are outside the region entirely.
fn notes(
    name: &str,
    sheets: usize,
    candidates: usize,
    dropped: &Dropped,
    serial_dates: usize,
) -> Vec<ConvertNote> {
    let chosen = (candidates > 1).then(|| ConvertNote::SheetChosen {
        name: name.to_string(),
        of: sheets,
    });
    let above = (dropped.margins.preamble > 0).then_some(ConvertNote::PreambleSkipped {
        lines: dropped.margins.preamble,
    });
    let below = (dropped.margins.trailer > 0).then_some(ConvertNote::TrailerSkipped {
        lines: dropped.margins.trailer,
    });
    let empty = (dropped.blanks > 0).then_some(ConvertNote::BlankRowsDropped {
        count: dropped.blanks,
    });
    let dates = (serial_dates > 0).then_some(ConvertNote::DatesFromSerial {
        count: serial_dates,
    });
    [chosen, above, below, empty, dates]
        .into_iter()
        .flatten()
        .collect()
}

// ---------------------------------------------------------------------------
// Finding the table on a sheet
// ---------------------------------------------------------------------------

/// The rectangle of real data on one sheet, and what had to be dropped to get
/// there.
struct Table {
    rows: Vec<Vec<Data>>,
    dropped: Dropped,
}

/// Rows of the data region that were not part of the table. Every one of these
/// is a row the user can see in their spreadsheet, so every one of them is
/// reported.
struct Dropped {
    /// Found by [`margins`]: title rows above the header, disclaimer rows below
    /// the last transaction.
    margins: Margins,
    /// Rows inside the table holding nothing at all. Reported as
    /// [`ConvertNote::BlankRowsDropped`].
    blanks: usize,
}

/// The rectangle of real data on one sheet, or `None` if there is not enough of
/// it to be a statement.
///
/// `calamine`'s [`Range`] already starts at the first populated cell, but "first
/// populated" is not "first row of the table", and it fails in two different
/// ways that need two different answers:
///
/// 1. A sheet whose A1 holds a merged title band starts one or more **blank**
///    rows above the header. Those are trimmed — on both axes, at both ends —
///    and every surviving row is padded to the full width, which is what turns a
///    merged range's trail of `Empty` cells into ordinary empty fields. This is
///    not a judgement call and is not reported.
/// 2. What is left can still open with a **populated** preamble — a floating
///    title, a date range, an account line — and close with a populated
///    **trailer**, which on a real export is a disclaimer block twenty-six rows
///    deep. Both are found by row consistency ([`margins`]) and both *are*
///    reported, because dropping a row the user can see is a judgement call.
/// 3. Whatever survives can still have blank rows scattered through it, which
///    are dropped and counted. They are not a judgement call about *what* the
///    row is — a row with no populated cell holds nothing — but they are still
///    rows the user can see, so they are still reported.
///
/// The order matters and is the one thing here worth stating twice. The margins
/// are found before the columns are trimmed, so a title in column A of a sheet
/// whose table starts at column C cannot drag column A into the table — and
/// neither can a disclaimer paragraph under it, which is the same trap from the
/// other end. They are found after the blank edges are trimmed, so those never
/// count against [`MAX_PREAMBLE_ROWS`] and never get reported as something we
/// skipped. Row widths are unaffected by the column trim either way — it
/// subtracts the same offset from every row — so nothing is lost by this order.
fn table_of(range: &Range<Data>) -> Option<Table> {
    let rows: Vec<&[Data]> = range.rows().collect();
    let (first_row, last_row) = extent(rows.iter().map(|row| row.iter().any(is_populated)))?;
    let region = rows.get(first_row..=last_row)?;

    let shapes: Vec<RowShape> = region.iter().copied().map(shape_of).collect();
    let margins = margins(&shapes);
    // `margins` counts its trailer from after the preamble, so this range is
    // always well-ordered; `get` states that rather than trusting it.
    let body = region.get(margins.preamble..region.len().checked_sub(margins.trailer)?)?;

    // The header cannot be caught by this. It is the first row whose width is
    // the modal one, and the modal width is at least `MIN_TABLE_FIELDS`, so it
    // has a populated cell in it.
    let (blank, kept): (Vec<&[Data]>, Vec<&[Data]>) =
        body.iter().copied().partition(|row| row_width(row) == 0);

    let width = kept.iter().map(|row| row.len()).max().unwrap_or(0);
    let (first_column, last_column) = extent((0..width).map(|column| {
        kept.iter()
            .any(|row| row.get(column).is_some_and(is_populated))
    }))?;

    let table: Vec<Vec<Data>> = kept
        .iter()
        .map(|row| {
            (first_column..=last_column)
                .map(|column| row.get(column).cloned().unwrap_or(Data::Empty))
                .collect()
        })
        .collect();

    let columns = last_column.checked_sub(first_column)? + 1;
    (table.len() >= MIN_TABLE_ROWS && columns >= MIN_TABLE_COLUMNS).then_some(Table {
        rows: table,
        dropped: Dropped {
            margins,
            blanks: blank.len(),
        },
    })
}

/// How one sheet row looks to [`margins`].
///
/// A sheet row is blank exactly when it has no populated cell, which is exactly
/// when its extent is zero — so unlike a delimited record, where
/// `,,,,,,,,,,,,,,` is fifteen fields of nothing, the two facts coincide here.
fn shape_of(row: &[Data]) -> RowShape {
    let width = row_width(row);
    RowShape {
        width,
        blank: width == 0,
    }
}

/// How far a row extends: one past its last populated cell, and zero for a row
/// with none.
///
/// **Not** a count of populated cells, and the difference is what makes the rule
/// work on a real export rather than only on a tidy one — see the module docs. A
/// row is as wide as the last thing in it, exactly as a delimited record is as
/// wide as its last field whether or not that field has anything in it.
fn row_width(row: &[Data]) -> usize {
    row.iter().rposition(is_populated).map_or(0, |at| at + 1)
}

/// The first and last index for which `populated` is true, or `None` when none
/// of them is.
fn extent(populated: impl Iterator<Item = bool>) -> Option<(usize, usize)> {
    let indices: Vec<usize> = populated
        .enumerate()
        .filter_map(|(at, populated)| populated.then_some(at))
        .collect();
    Some((*indices.first()?, *indices.last()?))
}

/// Whether a cell counts as data when deciding where the table starts.
///
/// A cell holding only whitespace does not. Spreadsheets acquire those by the
/// dozen — a space typed to "clear" a cell, a formula returning `""` — and
/// letting one of them anchor the top-left corner would drag the header row out
/// of the table.
fn is_populated(cell: &Data) -> bool {
    match cell {
        Data::Empty => false,
        Data::String(text) => !text.trim().is_empty(),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Rendering cells
// ---------------------------------------------------------------------------

/// One rendered row, and how many of its cells were date serials.
struct Rendered {
    cells: Vec<String>,
    serial_dates: usize,
}

fn render_row(row: Vec<Data>) -> Rendered {
    let cells: Vec<(String, bool)> = row.iter().map(cell_text).collect();
    Rendered {
        serial_dates: cells.iter().filter(|(_, serial)| *serial).count(),
        cells: cells.into_iter().map(|(text, _)| text).collect(),
    }
}

/// One cell as text, plus whether it was a date **serial** rendered as a date.
fn cell_text(cell: &Data) -> (String, bool) {
    match cell {
        Data::Empty => (String::new(), false),
        Data::String(text) => (text.clone(), false),
        Data::Bool(flag) => (flag.to_string(), false),
        Data::Int(value) => (value.to_string(), false),
        Data::Float(value) => (float_text(*value), false),
        Data::DateTime(stamp) => date_text(stamp),
        // ODS stores dates as ISO 8601 text rather than as a serial, so there is
        // nothing to decode and nothing to warn about — but the time half is
        // still dropped, so every backend emits the same `YYYY-MM-DD` shape.
        Data::DateTimeIso(text) => (iso_date_text(text), false),
        Data::DurationIso(text) => (text.clone(), false),
        // `#N/A`, `#DIV/0!`: a formula that did not evaluate has no value. The
        // error token is a spreadsheet artefact, and putting it where a rules
        // file expects an amount would be worse than the empty cell this
        // effectively is.
        Data::Error(_) => (String::new(), false),
    }
}

/// A date cell as `YYYY-MM-DD`, or its raw serial when it cannot honestly be one.
///
/// The 1904 epoch is *not* re-applied here: `calamine` records the workbook's
/// epoch inside every [`ExcelDateTime`] it builds, so `as_datetime` has already
/// accounted for it. See the module docs.
fn date_text(stamp: &ExcelDateTime) -> (String, bool) {
    if stamp.is_duration() || is_phantom_leap_day(stamp) {
        return (float_text(stamp.as_f64()), false);
    }
    stamp.as_datetime().map_or_else(
        || (float_text(stamp.as_f64()), false),
        // `NaiveDate`'s `Display` is ISO 8601 `YYYY-MM-DD`, which is the one
        // date format every hledger `date-format` rule can be written against.
        |when| (when.date().to_string(), true),
    )
}

/// Whether `stamp` is serial 60, the 1900-02-29 that never happened.
///
/// Excel inherited Lotus 1-2-3's belief that 1900 was a leap year and keeps it
/// for compatibility, so every 1900-epoch serial at or below 59 is off by one
/// from the truth and serial 60 names a date that does not exist.
/// `as_datetime()` resolves it to 1900-02-28 — the *same* answer it gives for
/// serial 59 — so believing it would silently merge two different cells onto one
/// day. The test is on the reported date rather than on the serial value because
/// a 1904-epoch workbook's serial 60 is a perfectly ordinary 1904-03-01, and the
/// epoch flag itself is private.
fn is_phantom_leap_day(stamp: &ExcelDateTime) -> bool {
    let (year, month, day, ..) = stamp.to_ymd_hms_milli();
    (year, month, day) == (1900, 2, 29)
}

/// A float as text, at full precision and with no currency formatting.
///
/// Rust's `f64` `Display` is the shortest representation that round-trips, and
/// never uses exponent notation — so `1234.56` stays `1234.56` rather than
/// becoming `1234.5600000000001` or `1.23456e3`, and hledger can read every one
/// of them.
fn float_text(value: f64) -> String {
    value.to_string()
}

/// The date half of an ISO 8601 timestamp.
fn iso_date_text(text: &str) -> String {
    text.split_once('T')
        .map_or(text, |(date, _)| date)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_never_read_as_dates() {
        // The trap: `Data::as_datetime()` would turn this amount into 1903-05-18.
        assert_eq!(
            cell_text(&Data::Float(1234.56)),
            ("1234.56".to_string(), false)
        );
        assert_eq!(cell_text(&Data::Int(45678)), ("45678".to_string(), false));
    }

    #[test]
    fn the_phantom_leap_day_is_refused() {
        let phantom = ExcelDateTime::new(60.0, calamine::ExcelDateTimeType::DateTime, false);
        assert!(
            is_phantom_leap_day(&phantom),
            "serial 60 on the 1900 epoch is 1900-02-29"
        );
        let (text, serial) = date_text(&phantom);
        assert!(!serial, "a day that never existed is not a date");
        assert_eq!(text, "60");
    }

    #[test]
    fn real_serials_become_iso_dates() {
        let stamp = ExcelDateTime::new(45678.0, calamine::ExcelDateTimeType::DateTime, false);
        assert_eq!(date_text(&stamp), ("2025-01-21".to_string(), true));
    }

    #[test]
    fn iso_timestamps_lose_their_time() {
        assert_eq!(iso_date_text("2026-01-15T10:30:00"), "2026-01-15");
        assert_eq!(iso_date_text("2026-01-15"), "2026-01-15");
    }

    /// A sheet written the way it reads, `.` for an empty cell.
    fn sheet(lines: &[&str]) -> Vec<Vec<Data>> {
        lines
            .iter()
            .map(|line| {
                line.split('|')
                    .map(|cell| match cell.trim() {
                        "." => Data::Empty,
                        text => Data::String(text.to_string()),
                    })
                    .collect()
            })
            .collect()
    }

    fn margins_of(rows: &[Vec<Data>]) -> Margins {
        let shapes: Vec<RowShape> = rows.iter().map(Vec::as_slice).map(shape_of).collect();
        margins(&shapes)
    }

    fn preamble_of(rows: &[Vec<Data>]) -> usize {
        margins_of(rows).preamble
    }

    #[test]
    fn a_row_is_as_wide_as_its_last_populated_cell() {
        let rows = sheet(&["a | . | c | .", "a | . | . | .", ". | . | . | ."]);
        assert_eq!(
            rows.iter().map(|row| row_width(row)).collect::<Vec<_>>(),
            [3, 1, 0]
        );
    }

    #[test]
    fn a_floating_title_is_preamble_and_the_header_is_not() {
        // The real export's shape: a title, a blank row, a second title, a blank
        // row, then the labels. The header is the *widest* row, and the rows
        // under it have holes in them — which is why this is scored on extent
        // and not on how many cells are populated.
        let rows = sheet(&[
            "All Activity Types | . | . | .",
            ". | . | . | .",
            "Account Activity for All Accounts | . | . | .",
            ". | . | . | .",
            "Date | Description | Memo | Amount",
            "2026-01-05 | GROCERY STORE | . | -54.20",
            "2026-01-06 | ATM WITHDRAWAL | . | -100.00",
            "2026-01-07 | EMPLOYER PAYROLL | . | 2500.00",
        ]);
        assert_eq!(preamble_of(&rows), 4);
    }

    #[test]
    fn a_single_column_sheet_is_not_read_as_all_preamble() {
        // Every row holds exactly one populated cell, so the tempting rule —
        // "a row with one cell in it is a title" — discards the entire sheet.
        // A one-wide table carries no signal, so nothing is skipped.
        let rows = sheet(&[
            "Daily Closing Balance",
            "Balance",
            "1200.00",
            "1100.00",
            "3600.00",
            "3568.82",
        ]);
        assert_eq!(preamble_of(&rows), 0);
    }

    #[test]
    fn a_body_row_with_one_cell_in_it_is_still_a_body_row() {
        // The header matches the modal width on the first row, so the run of
        // preamble is empty and the sparse rows below are kept — a rule keyed on
        // "one populated cell" would have eaten the second row here.
        let rows = sheet(&[
            "Date | Amount",
            "2026-01-05 | .",
            "2026-01-06 | -10.00",
            "2026-01-07 | -20.00",
        ]);
        assert_eq!(preamble_of(&rows), 0);
    }

    #[test]
    fn a_disclaimer_block_under_the_transactions_is_trailer() {
        // The real export's other end: blank rows and one-cell paragraphs, all
        // of them below the last transaction. Scored by extent the body reaches
        // column four and every one of these reaches column one or zero.
        let rows = sheet(&[
            "Date | Description | Memo | Amount",
            "2026-01-05 | GROCERY STORE | . | -54.20",
            "2026-01-06 | ATM WITHDRAWAL | . | -100.00",
            ". | . | . | .",
            ". | . | . | .",
            "Balances shown are as of the statement date. | . | . | .",
            ". | . | . | .",
            "Big Brokerage is a member SIPC. | . | . | .",
        ]);
        assert_eq!(
            margins_of(&rows),
            Margins {
                preamble: 0,
                trailer: 5
            }
        );
    }

    #[test]
    fn a_last_transaction_with_an_empty_final_column_survives() {
        // The row reaches column three of four because `Amount` is blank on it.
        // A rule spelled "narrower than the header" trims it and the user
        // silently loses a transaction; the rule is "too narrow to be one".
        let rows = sheet(&[
            "Date | Description | Memo | Amount",
            "2026-01-05 | GROCERY STORE | . | -54.20",
            "2026-01-06 | PENDING CHARGE | seen | .",
        ]);
        assert_eq!(margins_of(&rows), Margins::default());
    }

    #[test]
    fn error_cells_read_as_empty() {
        assert_eq!(
            cell_text(&Data::Error(calamine::CellErrorType::Div0)),
            (String::new(), false)
        );
    }
}
