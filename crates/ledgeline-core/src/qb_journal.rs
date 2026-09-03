//! QuickBooks Online's **Journal** report, parsed and grouped into transactions.
//!
//! Every claim below was measured against a real QuickBooks Online export — 204
//! rows, 46 groups, 102 posting rows — read with this crate's own `calamine`,
//! not with a description of it. See `plans/17-quickbooks-journal-import.md`.
//!
//! # Why this is not a `convert::Tabular`
//!
//! [`convert`](crate::convert) exists to collapse every accepted statement to
//! one table of strings that hledger's CSV rules engine can read. This report
//! cannot go through that pipeline for the reason the WP gives — a rules file
//! has no way to say "keep reading rows until a total line closes the group" —
//! and it cannot even go through the *reader* half, for three separate reasons
//! that each matter on their own:
//!
//! 1. `convert::spreadsheet` renders `Data::Error(_)` as an **empty string**,
//!    which is right for its job and fatal here: `#REF!` in a total row is the
//!    single loudest sign an export has been damaged, and blanking it turns a
//!    corrupted group into one whose total reads as zero.
//! 2. That module trims preamble and trailer rows and drops blank rows from the
//!    middle of the table. Every one of those moves a row, and grouping here is
//!    defined by **adjacency**: which postings sit between a marker and its
//!    closing row.
//! 3. `Tabular` is `Vec<Vec<String>>`, so `Float` and `Error` have already
//!    become the same thing by the time a caller sees it.
//!
//! So the sheet is read directly as `Range<Data>`. What *is* reused is
//! everything that is not the table model: the [`MAX_INPUT_BYTES`] cap, and the
//! four cell-reading primitives whose hard-won reasoning lives in
//! `convert::spreadsheet`'s module docs — [`is_populated`], [`float_text`],
//! [`date_text`] and [`iso_date_text`]. Re-deriving those is how the two
//! readers would come to disagree about a date serial.
//!
//! # The shape of the file
//!
//! Rows 1-3 are a merged title band (company, `Journal`, the date range), row 4
//! is blank, and row 5 is the header. Below it, groups repeat with **no blank
//! row between them**:
//!
//! ```text
//! 612                                                     <- marker: an id, alone
//!     01/01/2017  Journal Entry  2  ...  3000 Member Equity        70,000.00   <- postings
//!     01/01/2017  Journal Entry  2  ...  3900 Retained Earnings  35,131.01
//!     ... eight more ...
//! Total for 612                                    70,120.85    70,120.85     <- closing row
//! ```
//!
//! The file ends with a `TOTAL` row and a generation-timestamp footer.
//!
//! ## The header is not a fixed set of columns
//!
//! QuickBooks Online's report customizer lets a user add and remove columns, and
//! the motivating export had four added (`Item class`, `Balance`,
//! `Customer full name`, `Vendor`). So [`detect`] and [`layout_of`] key off the
//! columns that must be there for the report to be a journal at all — something
//! Debit-like, something Credit-like, something account-name-like — and never on
//! a column count or an exact label list. The marker column has **no header
//! label at all**, so it is found by where the `Total for ` text lands rather
//! than by name or by assuming column A.
//!
//! ## `Balance` is read for nothing, on purpose
//!
//! It is a running figure the *report* computes, and it is worth saying exactly
//! what it is so nobody goes looking for a use: it accumulates each posting's
//! amount **signed by that account's normal balance side**, and resets at every
//! group. Verified across the real export — `3000 Member Equity` credited 70,000
//! gives +70,000, `3900 Retained Earnings` debited 35,131.01 takes it to
//! 34,868.99, and `1520 Computer & Office Equipment` debited 49.99 *adds* to
//! reach 34,918.98. Reproducing it would need each account's declared type,
//! which appears nowhere in the export; it is computed in floating point (the
//! cell above really is stored as `34918.979999999996`); and it is scoped to the
//! report's own date range, so it changes meaning when the user changes the
//! filter. Three independent reasons it can never be a check on anything.
//!
//! # Signed amounts need no account types
//!
//! Exactly one of Debit/Credit is populated on almost every posting row — 54
//! and 48 of the real 102-row sample used to build this module, with no row
//! carrying both nonzero — and `amount = debit if debit else -credit` yields
//! the hledger-signed amount directly. Spot-checked against what the
//! accounting must mean, not against the arithmetic: a deposit debits the
//! bank account (+) and credits equity (−); a card charge credits the card
//! (−, which is how hledger holds a liability) and debits the expense (+); a
//! card payment credits checking (−) and debits the
//! card (+, paying the liability down toward zero). Two zero-amount shapes,
//! both found in real exports larger than the original sample, are exceptions
//! to "exactly one" — neither is a guess, because zero has no sign to guess:
//!
//! ## A row can be "posting-shaped" and carry no posting at all
//!
//! Found in a real, full-size export: a row directly below the marker,
//! repeating the transaction's own date/type/`Num`, with **no account name**
//! and `$0.00` on both Debit and Credit. It is not corrupt — every other cell
//! on it is exactly what a real posting row's would be — it simply names
//! nothing to categorize and moves no money. [`posting`] treats a row as this
//! kind of placeholder, and skips it rather than building a [`QbPosting`]
//! from it, exactly when it has no account **and** both Debit and Credit are
//! absent or exactly zero. That is a safe thing to skip and not merely a
//! convenient one: a `$0.00` posting included in [`close`]'s balance sum or
//! left out of it changes the sum by exactly nothing, so dropping the row
//! cannot turn a balanced group into an unbalanced-looking one or vice versa.
//! A row that names no account but carries a **real, nonzero** amount is a
//! different thing — a genuine "which account does this money belong to"
//! gap — and is still refused as [`QbJournalError::MissingAccount`],
//! unchanged.
//!
//! ## A real account can carry a zero-net leg, with neither side populated
//!
//! Found in a different real export: a Bill Payment (Check) fully offset by
//! a credit memo, whose "2000 Accounts Payable" posting row names a real
//! account but leaves BOTH Debit and Credit blank — QuickBooks apparently
//! sees nothing to write once a leg nets to zero, rather than writing `$0.00`
//! into either column. [`posting`] treats `(None, None)` — and, by the same
//! reasoning, `(Some(zero), Some(zero))`, since a `$0.00` written into either
//! or both columns is the identical amount — as an explicit zero rather than
//! the [`QbJournalError::AmountNotSplit`] refusal a row with a REAL,
//! disagreeing pair of numbers still gets. There is nothing to guess: a debit
//! of zero and a credit of zero are the same value (`-0 == 0`), so no
//! "which number is real" question exists when neither carries any
//! magnitude — unlike two REAL, differing numbers, which stays refused.
//! Verified against real hledger 1.52: a single-posting, `$0.00` transaction
//! (which is what this group becomes once its OTHER row is dropped by the
//! no-account/zero-amount rule above) is accepted — `hledger check` exits 0.
//!
//! # The total row is a formula, and its value is the cached one
//!
//! `Total for 612`'s Debit cell is literally `=I23+I24+…+I32`, and what reaches
//! us is the value Excel **stored** beside the formula. Nothing here evaluates a
//! formula; a cell whose stored value is missing is an error cell and is refused.
//!
//! That has one consequence worth stating, because it looks at first like the
//! balance check is tautological. In an untouched export the total is a sum over
//! the very rows above it and cannot disagree with them — so what the check
//! actually catches is **structural** damage: rows deleted (the formula's
//! references break and the stored value goes stale or becomes `#REF!`), or an
//! amount edited in a spreadsheet that did not recalculate on open. The real
//! sample carries exactly this: rows were removed, and the group whose marker
//! says `6` is closed by a surviving `Total for 11024` whose cells are `#REF!`.
//!
//! ## …and the stored digits are not always the value
//!
//! Excel writes up to seventeen significant digits, so that group's total is
//! stored as `70120.850000000006` and another as `79.989999999999995`. Both are
//! the nearest `f64` to a tidy cent value, and Rust's shortest-round-trip
//! `f64` formatting inverts that exactly — `70120.85` and `79.99` — which is
//! why [`amount_of`] goes through [`float_text`] rather than reading the raw
//! stored digits as text.
//!
//! That inversion is exact for a **single** stored value, but a formula's
//! result is not one — it is the output of Excel's own `SUM`, and IEEE 754
//! addition is not associative: summing enough terms can land on a double
//! that is not the one nearest the "true" decimal answer, even when every
//! addend was itself exact to the cent. The whole-report `TOTAL` row (four
//! hundred–odd values) was already known to drift this way — its shortest
//! form is `65510189.6700001` rather than `65510189.67` — but it was
//! originally assumed a **group**'s total, summing at most ten values in the
//! 204-row sample this module was built against, would always stay inside
//! half a ULP of the tidy value. A real, larger export disproved that: a
//! group with enough postings produced a stored total of
//! `975546.6699999999` against an independently computed `975546.67`.
//!
//! So the comparison is **not** bit-for-bit [`Dec`] equality — it is
//! [`Dec::rounded`] to `computed`'s own precision (see [`close`]). `computed`
//! is an exact sum of exact cent-precision postings and is never itself
//! rounded, so this can only absorb drift on the reported side; it cannot
//! manufacture agreement where a real, cent-or-larger disagreement exists. A
//! group whose stored total genuinely disagrees — an edited amount, a
//! deleted row — still produces [`QbJournalError::MismatchedTotal`] naming
//! both numbers, unchanged.
//!
//! # What is refused, and why nothing is partially imported
//!
//! Every group must balance, and its own closing row must agree, or the whole
//! parse is refused naming which group and why. There is no import-what-parsed
//! mode: a half-imported journal is harder to reason about than one that did not
//! import, and the failure modes here (rows deleted from a report) are exactly
//! the ones that produce plausible-looking wrong numbers.

use crate::convert::MAX_INPUT_BYTES;
use crate::convert::spreadsheet::{date_text, float_text, is_populated, iso_date_text};
use crate::decimal::Dec;
use crate::edit::render_dec;
use crate::rules::generate::{DateFormatGuess, guess_date_format};
use calamine::{Data, Ods, Range, Reader, Xls, Xlsx, open_workbook_from_rs};
use std::io::Cursor;
use thiserror::Error;

/// The literal text a closing row puts in the marker column, before the id.
///
/// A literal, and matched as one: QuickBooks writes it in the report's own
/// language, so a localized export simply does not detect — which is the honest
/// answer, and much better than a fuzzy match that groups the wrong rows
/// together and balances anyway.
const TOTAL_FOR: &str = "Total for ";

/// The marker text on the report's own grand-total row, which ends the data.
const REPORT_TOTAL: &str = "TOTAL";

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// One posting line of one QuickBooks transaction.
///
/// `memo`, `class`, `customer` and `vendor` are per-**posting** and not per
/// transaction. That is not a nicety: a single ten-line Journal Entry in the
/// real export carries six different descriptions, and hanging one of them off
/// the transaction loses the other five.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QbPosting {
    /// The raw QuickBooks account name, e.g. `1520 Computer & Office Equipment`.
    ///
    /// Verbatim, including QuickBooks' own `:` between a parent and a sub-account
    /// (`1520 Computer & Office Equipment:1521 …`) — ten of the real export's
    /// eighteen accounts have one. Whatever maps these to hledger accounts has
    /// to cope with that; this module does not touch them.
    pub account: String,
    /// Debit positive, credit negative.
    pub amount: Dec,
    pub memo: Option<String>,
    pub class: Option<String>,
    pub customer: Option<String>,
    pub vendor: Option<String>,
}

/// One QuickBooks transaction: a marker row, its postings and its closing row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QbTransaction {
    /// QuickBooks' own Trans #, e.g. `612`. Not unique across companies and not
    /// ordered — the real export runs `1, 2, 3, 4, 612, 33, 139, 34, …`.
    pub id: String,
    /// Normalized `YYYY-MM-DD`, under [`QbJournal::date_format`].
    pub date: String,
    /// `Deposit`, `Expense`, `Journal Entry`, `Transfer`, `Bill`,
    /// `Credit Card Expense` — the six the real export uses.
    pub transaction_type: String,
    pub num: Option<String>,
    /// The payee, when the report carries a `Name` column and the row fills it.
    pub name: Option<String>,
    pub postings: Vec<QbPosting>,
}

/// A parsed export: its transactions, and how its dates were read.
///
/// The sketch in the WP returned a bare `Vec<QbTransaction>`. It gained the
/// date-format guess because the question turns out to be answerable only from
/// the file as a whole: `01/02/2026` is two different days depending on the
/// QuickBooks account's own date-display preference, and nothing in the export
/// declares which. [`DateFormatGuess::ambiguous`] is set when the catalogue
/// found more than one reading — an export confined to the first twelve days of
/// a month is the case — and a caller that writes transactions without resolving
/// it is filing March in December. The real sample resolves cleanly only because
/// it happens to contain the 17th through the 20th.
#[derive(Debug, Clone, PartialEq)]
pub struct QbJournal {
    pub transactions: Vec<QbTransaction>,
    pub date_format: DateFormatGuess,
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// Why an export was refused. Every variant names the group it is about, because
/// "the import failed" over a 200-row report is not something a user can act on.
///
/// Row numbers are 1-based sheet rows — the numbers down the side of the user's
/// own spreadsheet. Nothing here can carry a path: this module is handed bytes
/// and never learns where they came from, matching `docs/imports.md` § Security.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum QbJournalError {
    #[error("the file is empty")]
    Empty,
    #[error("the file is larger than the {limit}-byte limit")]
    TooLarge { limit: usize },
    #[error("the workbook could not be read")]
    Unreadable,
    #[error("no QuickBooks Journal header was found (needs Debit, Credit and an account column)")]
    NoHeader,
    #[error("row {row} has posting data but no transaction has been opened above it")]
    PostingOutsideGroup { row: usize },
    #[error("row {row} closes transaction {id}, which was never opened")]
    OrphanTotal { id: String, row: usize },
    #[error(
        "transaction {opened} is closed at row {row} by a total for {closed} — \
         the export has had rows removed"
    )]
    TotalIdMismatch {
        opened: String,
        closed: String,
        row: usize,
    },
    #[error("transaction {id} is never closed by a \"Total for {id}\" row")]
    UnclosedGroup { id: String },
    #[error("transaction {id} has no posting rows")]
    EmptyGroup { id: String },
    #[error("row {row} of transaction {id} has no account name")]
    MissingAccount { id: String, row: usize },
    #[error("row {row} of transaction {id} has no transaction date")]
    MissingDate { id: String, row: usize },
    #[error("row {row} of transaction {id} has no transaction type")]
    MissingType { id: String, row: usize },
    #[error("row {row} of transaction {id} must have exactly one of Debit and Credit")]
    AmountNotSplit { id: String, row: usize },
    #[error("row {row} of transaction {id} has an unreadable amount: {cell}")]
    MalformedAmount {
        id: String,
        row: usize,
        cell: String,
    },
    #[error("the closing row for transaction {id} has an unreadable total: {cell}")]
    MalformedTotal { id: String, cell: String },
    #[error(
        "transaction {id} does not balance: debits {}, credits {}",
        render_dec(*debit_total, '.'),
        render_dec(*credit_total, '.')
    )]
    UnbalancedGroup {
        id: String,
        debit_total: Dec,
        credit_total: Dec,
    },
    #[error(
        "transaction {id} sums to {} but its own total row says {}",
        render_dec(*computed, '.'),
        render_dec(*reported, '.')
    )]
    MismatchedTotal {
        id: String,
        computed: Dec,
        reported: Dec,
    },
    #[error("transaction {id}'s amounts are too large to add up")]
    AmountOverflow { id: String },
    #[error("no date format reads every date in the export")]
    UnreadableDates,
    #[error("transaction {id}'s date {value} cannot be read as {format}")]
    UnreadableDate {
        id: String,
        value: String,
        format: String,
    },
    #[error("the export holds no transactions")]
    NoTransactions,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Does `bytes` look like a QuickBooks Journal export?
///
/// Content-based, and deliberately takes no format hint: the question is "are
/// these bytes that report", not "is this valid `.xlsx`". (`convert::spreadsheet`
/// takes the format from its caller for the opposite and equally good reason —
/// there, a file whose extension lies should be refused rather than
/// reinterpreted. Here there is no extension in play.)
///
/// Two conditions, and it needs both:
///
/// 1. A header row carrying something Debit-like **and** something Credit-like
///    **and** something account-name-like — never an exact label set or a column
///    count, since the report's columns are user-customizable.
/// 2. The grouping structure itself: a bare-id marker row, and a
///    `Total for {id}` row closing it whose id is one a marker actually opened.
///
/// The second is what carries the weight. A bank export with `Account Name`,
/// `Debit` and `Credit` columns and a `Total` summary row satisfies the first
/// completely — `fixtures/import/qb-journal/near-miss.xlsx` is exactly that
/// file — and a false positive costs the user the rules-matching flow they
/// needed, so the cheap half of the test never decides on its own.
///
/// Says **yes** on a damaged export, which is the point of keeping detection
/// separate from parsing: a truncated report is still unmistakably this format,
/// and the user is owed the refusal that says so rather than a silent fallback.
#[must_use]
pub fn detect(bytes: &[u8]) -> bool {
    rows_of(bytes).is_ok_and(|rows| layout_of(&rows).is_some_and(|layout| grouped(&rows, &layout)))
}

/// Whether the rows below the header actually pair markers with closing rows.
///
/// One matching pair is enough and is asked for by **id**, not by position: the
/// text `Total for 3` beside a marker row saying `3` is not something any other
/// export shape produces by accident.
fn grouped(rows: &[Vec<Data>], layout: &Layout) -> bool {
    let body = &rows[(layout.header_row + 1).min(rows.len())..];
    let markers: Vec<String> = body
        .iter()
        .filter_map(|row| match classify(row, layout) {
            Row::Marker(id) => Some(id),
            _ => None,
        })
        .collect();
    body.iter().any(|row| match classify(row, layout) {
        Row::Total { id, .. } => markers.contains(&id),
        _ => false,
    })
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse an export and group its rows into transactions.
///
/// # Errors
///
/// Any [`QbJournalError`]. Every one refuses the **whole** file: see the module
/// docs on why there is no partial import for a construct this load-bearing.
pub fn parse(bytes: &[u8]) -> Result<QbJournal, QbJournalError> {
    let rows = rows_of(bytes)?;
    let layout = layout_of(&rows).ok_or(QbJournalError::NoHeader)?;
    let groups = group(&rows, &layout)?;
    if groups.is_empty() {
        return Err(QbJournalError::NoTransactions);
    }
    date(groups)
}

/// The rows of the first sheet that holds anything, as `calamine` read them.
///
/// The sheet is not chosen by name. QuickBooks writes one sheet (`Sheet1`), and
/// a name-based rule would break on a re-saved or localized workbook while
/// buying nothing: [`layout_of`] has to recognise the header anyway, so a sheet
/// that is not the report simply fails that.
fn rows_of(bytes: &[u8]) -> Result<Vec<Vec<Data>>, QbJournalError> {
    if bytes.is_empty() {
        return Err(QbJournalError::Empty);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(QbJournalError::TooLarge {
            limit: MAX_INPUT_BYTES,
        });
    }
    // Each reader checks its own magic bytes and declines in constant time, so
    // trying three costs nothing on the CSV this is asked about most often.
    read_with::<Xlsx<_>>(bytes)
        .or_else(|| read_with::<Xls<_>>(bytes))
        .or_else(|| read_with::<Ods<_>>(bytes))
        .ok_or(QbJournalError::Unreadable)
}

fn read_with<'a, R: Reader<Cursor<&'a [u8]>>>(bytes: &'a [u8]) -> Option<Vec<Vec<Data>>> {
    let mut book: R = open_workbook_from_rs(Cursor::new(bytes)).ok()?;
    let names = book.sheet_names();
    names
        .iter()
        .filter_map(|name| book.worksheet_range(name).ok())
        .map(materialize)
        .find(|rows| !rows.is_empty())
}

/// A sheet range as owned rows, re-anchored to column zero of the sheet.
///
/// `calamine`'s `Range` starts at the first populated cell, so a sheet whose
/// data begins at C4 hands back rows that are two columns short. Every column
/// index this module computes is discovered by scanning those same rows, so the
/// offset is self-consistent — but the *row* offset is not, because errors quote
/// sheet row numbers the user can see. That is what [`Sheet::first_row`] carries.
fn materialize(range: Range<Data>) -> Vec<Vec<Data>> {
    range.rows().map(<[Data]>::to_vec).collect()
}

// ---------------------------------------------------------------------------
// The header, and which column is which
// ---------------------------------------------------------------------------

/// Where each field lives, and where the header row is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Layout {
    header_row: usize,
    /// The unnamed column holding transaction ids and `Total for …` text.
    marker: usize,
    date: usize,
    kind: usize,
    account: usize,
    debit: usize,
    credit: usize,
    num: Option<usize>,
    name: Option<usize>,
    memo: Option<usize>,
    class: Option<usize>,
    customer: Option<usize>,
    vendor: Option<usize>,
}

/// Read the header row and the marker column off `rows`, or `None`.
///
/// The three required columns are matched loosely enough to survive QuickBooks'
/// own alternative spellings — `Account`/`Account Name`, `Description`/
/// `Memo/Description` — and strictly enough that `Distribution account number`
/// cannot be taken for the account name, which is the one collision the real
/// export actually contains. `Num` is matched on the **whole** label for the
/// same reason: `Distribution account number` ends in the word `number`.
fn layout_of(rows: &[Vec<Data>]) -> Option<Layout> {
    let (header_row, header) = rows.iter().enumerate().find_map(|(at, row)| {
        let labels: Vec<String> = row.iter().map(|cell| text(cell).to_lowercase()).collect();
        let has = |want: &str| labels.iter().any(|label| label == want);
        // `contains` and not `==` for the account column: the label is `Account`
        // on the stock report and `Account Name` on a customized one.
        let account = labels
            .iter()
            .any(|label| label.contains("account") && !label.contains("number"));
        (has("debit") && has("credit") && account).then_some((at, labels))
    })?;

    let at = |want: &str| header.iter().position(|label| label == want);
    let containing = |want: &str| header.iter().position(|label| label.contains(want));

    // The marker column has no label, so it is located by the thing that
    // identifies it best: the column the `Total for ` text lands in. Falling
    // back to the first unlabelled column matters more than it looks — an export
    // whose last closing row was deleted has a perfectly good header, and
    // without the fallback it would be refused as having none, which is not a
    // report a user could act on. Detection is unaffected either way, because
    // `grouped` asks for a real marker/closing pair separately.
    let marker = rows
        .get(header_row + 1..)?
        .iter()
        .find_map(|row| {
            row.iter()
                .position(|cell| text(cell).starts_with(TOTAL_FOR))
        })
        .or_else(|| header.iter().position(String::is_empty))?;

    let account = header
        .iter()
        .position(|label| label == "account name" || label == "account")
        .or_else(|| {
            header
                .iter()
                .position(|label| label.contains("account") && !label.contains("number"))
        })?;

    let layout = Layout {
        header_row,
        marker,
        date: containing("date")?,
        kind: at("transaction type").or_else(|| at("type"))?,
        account,
        debit: at("debit")?,
        credit: at("credit")?,
        num: at("num"),
        name: at("name"),
        memo: at("description")
            .or_else(|| containing("description"))
            .or_else(|| containing("memo")),
        class: containing("class"),
        customer: containing("customer"),
        vendor: containing("vendor"),
    };
    // A layout whose marker column is also a data column is not this report; it
    // is some other file that happened to contain the words `Total for`.
    (![
        layout.date,
        layout.kind,
        layout.account,
        layout.debit,
        layout.credit,
    ]
    .contains(&layout.marker))
    .then_some(layout)
}

// ---------------------------------------------------------------------------
// Row classification
// ---------------------------------------------------------------------------

/// What one row below the header is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// Nothing populated anywhere.
    Blank,
    /// The marker column holds an id and nothing else on the row is populated.
    Marker(String),
    /// `Total for {id}`.
    Total { id: String },
    /// The report's own grand total: the end of the data.
    ReportTotal,
    /// The marker column is empty and something else is populated.
    Posting,
    /// A populated marker column that is none of the above — trailer prose.
    Other,
}

fn classify(row: &[Data], layout: &Layout) -> Row {
    let marker = row.get(layout.marker).map(text).unwrap_or_default();
    let rest = row
        .iter()
        .enumerate()
        .any(|(at, cell)| at != layout.marker && is_populated(cell));

    if marker.is_empty() {
        return if rest { Row::Posting } else { Row::Blank };
    }
    if let Some(id) = marker.strip_prefix(TOTAL_FOR) {
        return Row::Total {
            id: id.trim().to_string(),
        };
    }
    if marker == REPORT_TOTAL {
        return Row::ReportTotal;
    }
    if rest {
        Row::Other
    } else {
        Row::Marker(marker)
    }
}

// ---------------------------------------------------------------------------
// Grouping
// ---------------------------------------------------------------------------

/// A group before its dates have been normalized.
struct Raw {
    id: String,
    date: String,
    kind: String,
    num: Option<String>,
    name: Option<String>,
    postings: Vec<QbPosting>,
}

/// Walk the rows below the header, pairing markers with closing rows.
///
/// Two rules earn their place here.
///
/// **A marker is confirmed by what follows it.** The title band above the header
/// and the timestamp footer below the data have exactly a marker's shape — one
/// populated cell in column A and nothing else — so a rule that stopped at the
/// shape would open a group on the footer and refuse the file for never closing
/// it. A candidate is a marker only when the next row is a posting row (or the
/// closing row for that same id, so a genuinely empty group is still reported as
/// one rather than being read as prose).
///
/// **A closing row is matched by id, not by position.** This is what catches the
/// real export's damage: rows had been deleted, and the group opened by `6` is
/// closed by a leftover `Total for 11024`. Its four postings balance perfectly,
/// so every arithmetic check passes; only the id knows.
fn group(rows: &[Vec<Data>], layout: &Layout) -> Result<Vec<Raw>, QbJournalError> {
    let body = rows.get(layout.header_row + 1..).unwrap_or_default();
    let sheet_row = |at: usize| layout.header_row + at + 2;

    let mut groups: Vec<Raw> = Vec::new();
    let mut open: Option<(String, usize, Vec<usize>)> = None;

    for (at, row) in body.iter().enumerate() {
        match classify(row, layout) {
            Row::ReportTotal => break,
            Row::Blank => {}
            Row::Other => {}
            Row::Posting => match open.as_mut() {
                Some((_, _, postings)) => postings.push(at),
                None => {
                    return Err(QbJournalError::PostingOutsideGroup { row: sheet_row(at) });
                }
            },
            Row::Marker(id) => {
                if let Some((open_id, ..)) = &open {
                    return Err(QbJournalError::UnclosedGroup {
                        id: open_id.clone(),
                    });
                }
                let confirmed = matches!(
                    body.get(at + 1).map(|next| classify(next, layout)),
                    Some(Row::Posting) | Some(Row::Total { .. })
                );
                if confirmed {
                    open = Some((id, at, Vec::new()));
                }
            }
            Row::Total { id } => {
                let Some((open_id, _, postings)) = open.take() else {
                    return Err(QbJournalError::OrphanTotal {
                        id,
                        row: sheet_row(at),
                    });
                };
                if open_id != id {
                    return Err(QbJournalError::TotalIdMismatch {
                        opened: open_id,
                        closed: id,
                        row: sheet_row(at),
                    });
                }
                groups.push(close(&open_id, body, &postings, row, layout, &sheet_row)?);
            }
        }
    }

    match open {
        Some((id, ..)) => Err(QbJournalError::UnclosedGroup { id }),
        None => Ok(groups),
    }
}

/// Build one group's transaction and check it against its own closing row.
fn close(
    id: &str,
    body: &[Vec<Data>],
    postings: &[usize],
    total: &[Data],
    layout: &Layout,
    sheet_row: &impl Fn(usize) -> usize,
) -> Result<Raw, QbJournalError> {
    let [first, ..] = postings else {
        return Err(QbJournalError::EmptyGroup { id: id.to_string() });
    };
    let head = &body[*first];
    let at = sheet_row(*first);

    let lines: Vec<Posted> = postings
        .iter()
        .map(|at| posting(id, &body[*at], layout, sheet_row(*at)))
        .collect::<Result<Vec<Option<Posted>>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    if lines.is_empty() {
        return Err(QbJournalError::EmptyGroup { id: id.to_string() });
    }

    let sum = |take: fn(&Posted) -> Option<Dec>| {
        lines
            .iter()
            .filter_map(take)
            .try_fold(Dec::zero(), Dec::add)
            .map_err(|_| QbJournalError::AmountOverflow { id: id.to_string() })
    };
    let debit_total = sum(|line| line.debit)?;
    let credit_total = sum(|line| line.credit)?;
    if debit_total != credit_total {
        return Err(QbJournalError::UnbalancedGroup {
            id: id.to_string(),
            debit_total,
            credit_total,
        });
    }

    // The closing row's own two numbers, which in an untouched export are a
    // formula over the rows just checked and in a damaged one are the only thing
    // that knows it. Both columns are compared: a truncation can leave one
    // stale.
    //
    // The comparison is `reported` ROUNDED to `computed`'s own precision, not
    // bit-for-bit equality — see `Dec::rounded`'s own docs for why: a real
    // export was found where a group's total is a `SUM` formula over enough
    // terms that IEEE 754 summation drifts a few units past the last
    // meaningful digit (`975546.6699999999` for a `computed` of `975546.67`).
    // `computed` itself is never rounded — it is an exact sum of exact
    // cent-precision postings and cannot drift — so this can only ever absorb
    // noise on the REPORTED side, never manufacture agreement neither side
    // actually has.
    for (column, computed) in [(layout.debit, debit_total), (layout.credit, credit_total)] {
        let reported = amount_of(total.get(column))
            .map_err(|cell| QbJournalError::MalformedTotal {
                id: id.to_string(),
                cell,
            })?
            .unwrap_or_else(Dec::zero);
        let agrees = reported
            .rounded(computed.places)
            .is_ok_and(|rounded| rounded == computed);
        if !agrees {
            return Err(QbJournalError::MismatchedTotal {
                id: id.to_string(),
                computed,
                reported,
            });
        }
    }

    Ok(Raw {
        id: id.to_string(),
        // Date, type, `Num` and `Name` repeat on every posting row of a group
        // rather than appearing only on the first — verified across all 46 of
        // the real export's groups, where no group varies in any of the four.
        // The first row is read because it is the one that must be there.
        date: cell(head, Some(layout.date)).ok_or_else(|| QbJournalError::MissingDate {
            id: id.to_string(),
            row: at,
        })?,
        kind: cell(head, Some(layout.kind)).ok_or_else(|| QbJournalError::MissingType {
            id: id.to_string(),
            row: at,
        })?,
        num: cell(head, layout.num),
        name: cell(head, layout.name),
        postings: lines.into_iter().map(|line| line.posting).collect(),
    })
}

/// One posting, plus which side of the ledger its amount came from.
struct Posted {
    posting: QbPosting,
    debit: Option<Dec>,
    credit: Option<Dec>,
}

/// One posting row, or `Ok(None)` when it is a placeholder that names no
/// account and moves no money — see the module docs on the row Debit=`$0.00`,
/// no account, immediately after a marker, that a real export was found to
/// contain.
fn posting(
    id: &str,
    row: &[Data],
    layout: &Layout,
    at: usize,
) -> Result<Option<Posted>, QbJournalError> {
    let malformed = |cell| QbJournalError::MalformedAmount {
        id: id.to_string(),
        row: at,
        cell,
    };
    let debit = amount_of(row.get(layout.debit)).map_err(malformed)?;
    let credit = amount_of(row.get(layout.credit)).map_err(&malformed)?;
    let account = cell(row, Some(layout.account));

    // A row naming no account and moving no money on either side is not a
    // posting at all — real double-entry bookkeeping has no such thing, so
    // this is not a corrupted account name, it is a row with nothing in it to
    // categorize. Dropping it changes no arithmetic anywhere: a $0 posting
    // included or excluded sums to the same debit/credit totals, which is
    // exactly what lets this be a skip rather than a guess. A row with a REAL
    // amount and no account is a different thing entirely and still refused
    // below, unchanged.
    if account.is_none() && debit.is_none_or(|d| d.is_zero()) && credit.is_none_or(|c| c.is_zero())
    {
        return Ok(None);
    }

    // Otherwise exactly one side, almost always — 54 debits and 48 credits
    // over the real 102-row sample this module was built against, with no
    // row carrying both or neither. A row that carries both NONZERO numbers
    // is still refused rather than guessed at: which of two numbers is the
    // real amount is not something this module can know.
    //
    // The one exception: a row that names a real account (checked above —
    // this point is only reached once `account` is known to be `Some`) but
    // carries no money on EITHER side. Found in a real export as a genuine
    // zero-net leg of a Bill Payment that a fully offsetting credit reduced
    // to nothing — real, if unusual, business event, not corruption. There
    // is no ambiguity to refuse: a debit of zero and a credit of zero are
    // the identical amount (`-0 == 0`), so whether the cell was left BLANK
    // or written out as literal `$0.00` on one or both sides, the result can
    // only ever be zero. Verified against real hledger: a single-posting,
    // `$0.00` transaction is accepted (`hledger check` exits 0).
    let amount = match (debit, credit) {
        (Some(debit), None) => debit,
        (None, Some(credit)) => credit
            .neg()
            .map_err(|_| QbJournalError::AmountOverflow { id: id.to_string() })?,
        (None, None) => Dec::zero(),
        (Some(debit), Some(credit)) if debit.is_zero() && credit.is_zero() => Dec::zero(),
        _ => {
            return Err(QbJournalError::AmountNotSplit {
                id: id.to_string(),
                row: at,
            });
        }
    };

    Ok(Some(Posted {
        posting: QbPosting {
            account: account.ok_or_else(|| QbJournalError::MissingAccount {
                id: id.to_string(),
                row: at,
            })?,
            amount,
            memo: cell(row, layout.memo),
            class: cell(row, layout.class),
            customer: cell(row, layout.customer),
            vendor: cell(row, layout.vendor),
        },
        debit,
        credit,
    }))
}

// ---------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------

/// One cell's text, trimmed, or `None` when it holds nothing printable.
///
/// The distinction matters more than it looks. Every unused text cell on a real
/// posting row is `Data::String("")` and not `Data::Empty` — the shared-string
/// table has an empty entry and QuickBooks points at it — so a reader that tests
/// the *variant* reports `vendor: Some("")` on every row that has no vendor, and
/// writes a `vendor:` tag with nothing after it into the user's journal.
fn cell(row: &[Data], column: Option<usize>) -> Option<String> {
    let text = text(row.get(column?)?);
    (!text.is_empty()).then_some(text)
}

/// One cell as trimmed text. `Data::Error` is empty here; callers that care
/// about it — the two amount columns — go through [`amount_of`] instead.
fn text(cell: &Data) -> String {
    match cell {
        Data::Empty | Data::Error(_) => String::new(),
        Data::String(text) => text.trim().to_string(),
        Data::Bool(flag) => flag.to_string(),
        Data::Int(value) => value.to_string(),
        Data::Float(value) => float_text(*value),
        Data::DateTime(stamp) => date_text(stamp).0,
        Data::DateTimeIso(text) => iso_date_text(text),
        Data::DurationIso(text) => text.clone(),
    }
}

/// An amount cell as an exact decimal, `Ok(None)` when it is blank, and
/// `Err(token)` when it is a formula error or unreadable.
///
/// A `Float` goes through [`float_text`] — shortest-round-trip formatting — and
/// is then parsed exactly. That is the step that turns Excel's stored
/// `70120.850000000006` back into `70120.85`, and it is why no comparison in
/// this module needs a tolerance. See the module docs.
fn amount_of(cell: Option<&Data>) -> Result<Option<Dec>, String> {
    let Some(cell) = cell else { return Ok(None) };
    let text = match cell {
        Data::Empty => return Ok(None),
        // `#REF!` and friends. `convert::spreadsheet` renders these as an empty
        // string, which is right for a rules-file column and catastrophic here.
        Data::Error(error) => return Err(error_text(error)),
        Data::Float(value) => float_text(*value),
        Data::Int(value) => value.to_string(),
        Data::String(text) => text.trim().to_string(),
        other => text(other),
    };
    if text.is_empty() {
        return Ok(None);
    }
    Dec::parse(&text, '.').map(Some).map_err(|_| text)
}

/// A formula error as the token a user sees in their own spreadsheet.
fn error_text(error: &calamine::CellErrorType) -> String {
    use calamine::CellErrorType::{Div0, GettingData, NA, Name, Null, Num, Ref, Value};
    match error {
        Div0 => "#DIV/0!",
        NA => "#N/A",
        Name => "#NAME?",
        Null => "#NULL!",
        Num => "#NUM!",
        Ref => "#REF!",
        Value => "#VALUE!",
        GettingData => "#GETTING_DATA",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

/// Resolve the export's date format once, over every group, and normalize.
///
/// Deliberately a whole-file question. `01/02/2026` is January 2nd or February
/// 1st depending on a QuickBooks *account* preference the export does not
/// record, and the only evidence available is whether some other date in the
/// same file has a component above twelve. [`guess_date_format`] already answers
/// exactly this — over the catalogue that `rules::matching` is tested against
/// the hledger binary with — so it is reused rather than re-derived, and its
/// `ambiguous` flag is passed through to the caller instead of being resolved by
/// a coin toss here.
fn date(groups: Vec<Raw>) -> Result<QbJournal, QbJournalError> {
    let samples: Vec<String> = groups.iter().map(|group| group.date.clone()).collect();
    let guess = guess_date_format(&samples).ok_or(QbJournalError::UnreadableDates)?;

    let transactions = groups
        .into_iter()
        .map(|group| {
            Ok(QbTransaction {
                date: iso_date(&guess.format, &group.date).ok_or_else(|| {
                    QbJournalError::UnreadableDate {
                        id: group.id.clone(),
                        value: group.date.clone(),
                        format: guess.format.clone(),
                    }
                })?,
                id: group.id,
                transaction_type: group.kind,
                num: group.num,
                name: group.name,
                postings: group.postings,
            })
        })
        .collect::<Result<_, QbJournalError>>()?;

    Ok(QbJournal {
        transactions,
        date_format: guess,
    })
}

/// `value` as `YYYY-MM-DD`, given the `date-format` that reads it.
///
/// **Not** a second `date-format` reader, and it is worth being precise about
/// why, because this codebase already refused to grow one
/// (`rules::generate::readable_formats` reuses `rules::matching`'s). The
/// question of whether `format` *reads* `value` has already been answered — by
/// that same shared reader, inside [`guess_date_format`]. All that is left is
/// which of the value's digit runs is the month and which is the day, and this
/// asks the format string in the order its specifiers appear.
///
/// Numeric formats only. A `%b`/`%B` month name would need a name table and is
/// not a shape QuickBooks' date-display preferences can produce, so it returns
/// `None` and becomes a named refusal rather than a silent misreading.
fn iso_date(format: &str, value: &str) -> Option<String> {
    let order: Vec<char> = format
        .split('%')
        .skip(1)
        .filter_map(|spec| spec.trim_start_matches('-').chars().next())
        .filter(|spec| matches!(spec, 'Y' | 'y' | 'm' | 'd'))
        .collect();
    let [first, second, third] = order[..] else {
        return None;
    };

    let runs: Vec<u32> = value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|run| !run.is_empty())
        .filter_map(|run| run.parse().ok())
        .collect();
    // A datetime format contributes more runs; the date is always the first
    // three, in every shape the catalogue carries.
    let [a, b, c, ..] = runs[..] else { return None };

    let mut year = None;
    let mut month = None;
    let mut day = None;
    for (spec, digits) in [(first, a), (second, b), (third, c)] {
        match spec {
            // hledger's own two-digit-year window, and chrono's: 69..=99 is the
            // twentieth century, 00..=68 the twenty-first.
            'y' => {
                year = Some(if digits < 69 {
                    2000 + digits
                } else {
                    1900 + digits
                })
            }
            'Y' => year = Some(digits),
            'm' => month = Some(digits),
            'd' => day = Some(digits),
            _ => return None,
        }
    }
    let (year, month, day) = (year?, month?, day?);
    ((1..=12).contains(&month) && (1..=31).contains(&day))
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sheet written the way it reads. `.` is a genuinely absent cell and `~`
    /// is an *empty string*, which is what the real export puts in its unused
    /// text cells and is not the same thing at all.
    fn sheet(lines: &[&str]) -> Vec<Vec<Data>> {
        lines
            .iter()
            .map(|line| {
                line.split('|')
                    .map(|cell| match cell.trim() {
                        "." => Data::Empty,
                        "~" => Data::String(String::new()),
                        "#REF!" => Data::Error(calamine::CellErrorType::Ref),
                        text => text
                            .parse::<f64>()
                            .map_or_else(|_| Data::String(text.to_string()), Data::Float),
                    })
                    .collect()
            })
            .collect()
    }

    /// The customized header, one balanced group, and its closing row.
    fn one_group() -> Vec<Vec<Data>> {
        sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Expense | Checking | . | 25.00",
            ". | 01/17/2026 | Expense | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 25.00 | 25.00",
        ])
    }

    fn parsed(rows: &[Vec<Data>]) -> Result<QbJournal, QbJournalError> {
        let layout = layout_of(rows).ok_or(QbJournalError::NoHeader)?;
        date(group(rows, &layout)?)
    }

    #[test]
    fn a_group_becomes_a_transaction() {
        let journal = parsed(&one_group()).expect("parses");
        let [transaction] = &journal.transactions[..] else {
            panic!("one group");
        };
        assert_eq!(transaction.id, "7");
        assert_eq!(transaction.date, "2026-01-17");
        assert_eq!(transaction.postings.len(), 2);
        assert_eq!(
            transaction.postings[0].amount,
            Dec::parse("-25.00", '.').expect("literal")
        );
    }

    #[test]
    fn an_empty_string_in_the_marker_column_is_not_a_marker() {
        // The trap `Data::Empty`-only matching falls into: a re-saved workbook
        // can spell a blank marker cell as an empty string, and read as a marker
        // every posting row opens a group of its own.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            "~ | 01/17/2026 | Expense | Checking | . | 25.00",
            "~ | 01/17/2026 | Expense | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 25.00 | 25.00",
        ]);
        assert_eq!(parsed(&rows).expect("parses").transactions.len(), 1);
    }

    #[test]
    fn a_title_band_and_a_footer_are_not_markers() {
        // Both have a marker's exact shape — one populated cell, nothing else —
        // and a rule keyed on the shape alone opens a group on each and refuses
        // the file for never closing them.
        let rows = sheet(&[
            "Northwind Widgets LLC | . | . | . | . | .",
            "Journal | . | . | . | . | .",
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Expense | Checking | . | 25.00",
            ". | 01/17/2026 | Expense | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 25.00 | 25.00",
            " Wednesday, September 02, 2026 10:31 AM GMT-06:00 | . | . | . | . | .",
        ]);
        assert_eq!(parsed(&rows).expect("parses").transactions.len(), 1);
    }

    #[test]
    fn a_group_whose_debits_and_credits_disagree_is_refused() {
        // Constructed: an untouched export's total row is a formula over these
        // very rows and cannot disagree with them, so there is no real file of
        // this shape to keep in the corpus.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Expense | Checking | . | 25.00",
            ". | 01/17/2026 | Expense | Supplies | 30.00 | .",
            "Total for 7 | . | . | . | 30.00 | 25.00",
        ]);
        match parsed(&rows) {
            Err(QbJournalError::UnbalancedGroup {
                id,
                debit_total,
                credit_total,
            }) => {
                assert_eq!(id, "7");
                assert_eq!(debit_total, Dec::parse("30.00", '.').expect("literal"));
                assert_eq!(credit_total, Dec::parse("25.00", '.').expect("literal"));
            }
            other => panic!("expected UnbalancedGroup, got {other:?}"),
        }
    }

    #[test]
    fn a_row_with_both_a_debit_and_a_credit_is_refused() {
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Expense | Checking | 25.00 | 25.00",
            ". | 01/17/2026 | Expense | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 50.00 | 25.00",
        ]);
        assert!(matches!(
            parsed(&rows),
            Err(QbJournalError::AmountNotSplit { .. })
        ));
    }

    #[test]
    fn a_zero_amount_row_with_no_account_is_skipped_not_refused() {
        // The shape a real, full-size export was found to contain: a row
        // right after the marker repeating the date/type but naming no
        // account, with $0.00 on both sides. Refusing it for "no account
        // name" is too strict — it moves no money either way.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Journal Entry | . | 0.00 | .",
            ". | 01/17/2026 | Journal Entry | Checking | . | 25.00",
            ". | 01/17/2026 | Journal Entry | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 25.00 | 25.00",
        ]);
        let journal = parsed(&rows).expect("parses");
        let [transaction] = &journal.transactions[..] else {
            panic!("one group");
        };
        assert_eq!(
            transaction.postings.len(),
            2,
            "the zero/no-account placeholder row is not counted as a posting"
        );
    }

    #[test]
    fn a_zero_amount_row_with_no_account_and_a_blank_debit_too_is_also_skipped() {
        // The placeholder need not populate Debit at all — Credit blank AND
        // Debit blank, only date/type/Num, is the same "moves no money" case.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Journal Entry | . | . | .",
            ". | 01/17/2026 | Journal Entry | Checking | . | 25.00",
            ". | 01/17/2026 | Journal Entry | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 25.00 | 25.00",
        ]);
        let journal = parsed(&rows).expect("parses");
        assert_eq!(journal.transactions[0].postings.len(), 2);
    }

    #[test]
    fn a_row_with_a_real_amount_and_no_account_is_still_refused() {
        // The safety property the skip above must never weaken: real money
        // with nowhere to categorize it is exactly the case this format's
        // strictness exists to catch, and stays a hard refusal.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Journal Entry | . | 25.00 | .",
            ". | 01/17/2026 | Journal Entry | Supplies | . | 25.00",
            "Total for 7 | . | . | . | 25.00 | 25.00",
        ]);
        match parsed(&rows) {
            Err(QbJournalError::MissingAccount { id, .. }) => assert_eq!(id, "7"),
            other => panic!("expected MissingAccount, got {other:?}"),
        }
    }

    #[test]
    fn a_group_of_only_zero_amount_no_account_rows_is_refused_as_empty() {
        // The pathological case: every row after the marker is a placeholder,
        // so nothing real was ever posted. Still refused, just under the same
        // "no posting rows" message an actually-empty group gets.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Journal Entry | . | 0.00 | .",
            "Total for 7 | . | . | . | . | .",
        ]);
        assert_eq!(
            parsed(&rows),
            Err(QbJournalError::EmptyGroup { id: "7".into() })
        );
    }

    #[test]
    fn a_real_account_with_neither_debit_nor_credit_populated_posts_zero() {
        // The exact shape reported against a real export: a Bill Payment
        // (Check) fully offset by a credit memo, whose Accounts Payable leg
        // names a real account but leaves both Debit and Credit blank.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7513 | . | . | . | . | .",
            ". | 01/10/2023 | Bill Payment (Check) | . | 0.00 | .",
            ". | 01/10/2023 | Bill Payment (Check) | 2000 Accounts Payable | . | .",
            "Total for 7513 | . | . | . | 0.00 | 0.00",
        ]);
        let journal = parsed(&rows).expect("parses despite the zero-net leg");
        let [transaction] = &journal.transactions[..] else {
            panic!("one group");
        };
        // The first row has no account and is dropped by the existing
        // no-account/zero-amount rule; only the Accounts Payable leg
        // survives, posted at exactly zero.
        assert_eq!(transaction.postings.len(), 1);
        assert_eq!(transaction.postings[0].account, "2000 Accounts Payable");
        assert_eq!(transaction.postings[0].amount, Dec::zero());
    }

    #[test]
    fn a_real_account_with_both_sides_explicitly_zero_also_posts_zero() {
        // The same reasoning applies whether the zero was left BLANK or
        // written out as literal `$0.00` on one or both sides: there is no
        // "which number is real" question when neither carries any
        // magnitude.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Journal Entry | Checking | . | 25.00",
            ". | 01/17/2026 | Journal Entry | Suspense | 0.00 | 0.00",
            ". | 01/17/2026 | Journal Entry | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 25.00 | 25.00",
        ]);
        let journal = parsed(&rows).expect("parses");
        let [transaction] = &journal.transactions[..] else {
            panic!("one group");
        };
        assert_eq!(transaction.postings.len(), 3);
        let suspense = transaction
            .postings
            .iter()
            .find(|posting| posting.account == "Suspense")
            .expect("the suspense posting survives");
        assert_eq!(suspense.amount, Dec::zero());
    }

    #[test]
    fn a_group_that_is_never_closed_is_refused() {
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Expense | Checking | . | 25.00",
            ". | 01/17/2026 | Expense | Supplies | 25.00 | .",
        ]);
        assert_eq!(
            parsed(&rows),
            Err(QbJournalError::UnclosedGroup { id: "7".into() })
        );
    }

    #[test]
    fn a_ref_error_in_a_posting_amount_is_refused_by_name() {
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/17/2026 | Expense | Checking | . | #REF!",
            ". | 01/17/2026 | Expense | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 25.00 | 25.00",
        ]);
        match parsed(&rows) {
            Err(QbJournalError::MalformedAmount { cell, .. }) => assert_eq!(cell, "#REF!"),
            other => panic!("expected MalformedAmount, got {other:?}"),
        }
    }

    #[test]
    fn the_report_total_row_ends_the_data_rather_than_opening_a_group() {
        let mut rows = one_group();
        rows.push(sheet(&["TOTAL | . | . | . | 25.00 | 25.00"]).remove(0));
        rows.push(sheet(&["stray prose | . | . | . | . | ."]).remove(0));
        assert_eq!(parsed(&rows).expect("parses").transactions.len(), 1);
    }

    // The excessive precision IS the test. These are the literal digit strings
    // Excel stored in the real export's total rows, and clippy's suggested
    // `70_120.85` would delete the property under test by writing the value the
    // parser is supposed to recover.
    #[allow(clippy::excessive_precision)]
    #[test]
    fn seventeen_stored_digits_read_back_as_the_cent_value() {
        // Excel's own spelling of the nearest double to 70120.85. Compared as
        // text, or in f64 against a re-summed total, this refuses a good file.
        assert_eq!(
            amount_of(Some(&Data::Float(70120.850000000006))),
            Ok(Some(Dec::parse("70120.85", '.').expect("literal")))
        );
        assert_eq!(
            amount_of(Some(&Data::Float(79.989999999999995))),
            Ok(Some(Dec::parse("79.99", '.').expect("literal")))
        );
    }

    #[test]
    fn a_total_that_drifted_past_a_cent_of_float_summation_noise_still_agrees() {
        // The exact discrepancy reported against a real, larger export: a
        // group's total-row formula, cached after summing enough terms, does
        // not land on the double nearest the tidy cent value — the parser
        // must round the REPORTED side to the computed sum's own precision
        // before comparing, not refuse a well-formed file.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7237 | . | . | . | . | .",
            ". | 01/17/2026 | Journal Entry | Checking | . | 975546.67",
            ". | 01/17/2026 | Journal Entry | Supplies | 975546.67 | .",
            "Total for 7237 | . | . | . | 975546.6699999999 | 975546.6699999999",
        ]);
        let journal = parsed(&rows).expect("parses despite the drifted total");
        assert_eq!(journal.transactions[0].postings.len(), 2);
    }

    #[test]
    fn an_error_cell_is_never_read_as_a_blank_amount() {
        // `convert::spreadsheet::cell_text` renders this as an empty string,
        // which here would read as a total of zero.
        assert_eq!(
            amount_of(Some(&Data::Error(calamine::CellErrorType::Ref))),
            Err("#REF!".to_string())
        );
        assert_eq!(amount_of(Some(&Data::Empty)), Ok(None));
        assert_eq!(amount_of(Some(&Data::String(String::new()))), Ok(None));
    }

    #[test]
    fn iso_dates_come_from_the_formats_own_specifier_order() {
        assert_eq!(
            iso_date("%m/%d/%Y", "01/17/2026").as_deref(),
            Some("2026-01-17")
        );
        assert_eq!(
            iso_date("%d/%m/%Y", "01/17/2026"),
            None,
            "month 17 is not a month"
        );
        assert_eq!(
            iso_date("%-m/%-d/%Y", "1/7/2026").as_deref(),
            Some("2026-01-07"),
            "the relaxed spelling reads the same way"
        );
        assert_eq!(
            iso_date("%Y-%m-%d", "2026-01-17").as_deref(),
            Some("2026-01-17")
        );
        assert_eq!(
            iso_date("%m/%d/%y", "01/17/26").as_deref(),
            Some("2026-01-17")
        );
        assert_eq!(
            iso_date("%m/%d/%Y %H:%M", "01/17/2026 13:45").as_deref(),
            Some("2026-01-17"),
            "the time is surplus runs, not part of the date"
        );
        assert_eq!(iso_date("%b %d, %Y", "Jan 17, 2026"), None, "no name table");
    }

    #[test]
    fn a_customized_column_set_is_read_by_name_and_not_by_position() {
        // The account column sits to the LEFT of the dates here and there are
        // four columns the stock report does not have. Nothing may be positional.
        let rows = sheet(&[
            ". | Vendor | Account Name | Item class | Transaction date | Transaction type | Credit | Debit | Balance",
            "7 | . | . | . | . | . | . | . | .",
            ". | Acme | Checking | . | 01/17/2026 | Expense | 25.00 | . | 25.00",
            ". | Acme | Supplies | Retail | 01/17/2026 | Expense | . | 25.00 | 50.00",
            "Total for 7 | . | . | . | . | . | 25.00 | 25.00 | .",
        ]);
        let journal = parsed(&rows).expect("parses");
        let [transaction] = &journal.transactions[..] else {
            panic!("one group");
        };
        assert_eq!(transaction.postings[0].account, "Checking");
        assert_eq!(transaction.postings[0].vendor.as_deref(), Some("Acme"));
        assert_eq!(transaction.postings[0].class, None);
        assert_eq!(transaction.postings[1].class.as_deref(), Some("Retail"));
    }

    #[test]
    fn a_file_whose_dates_are_all_before_the_thirteenth_says_it_is_ambiguous() {
        // Nothing in the data can tell month-first from day-first, so the answer
        // has to be "here is a reading, and it is a coin toss" rather than a
        // silently chosen one.
        let rows = sheet(&[
            ". | Transaction date | Transaction type | Account Name | Debit | Credit",
            "7 | . | . | . | . | .",
            ". | 01/02/2026 | Expense | Checking | . | 25.00",
            ". | 01/02/2026 | Expense | Supplies | 25.00 | .",
            "Total for 7 | . | . | . | 25.00 | 25.00",
        ]);
        let journal = parsed(&rows).expect("parses");
        assert!(journal.date_format.ambiguous);
        assert_eq!(journal.date_format.format, "%m/%d/%Y");
    }

    #[test]
    fn a_sheet_with_no_debit_and_credit_pair_has_no_layout() {
        let rows = sheet(&[
            "Date | Description | Amount",
            "2026-01-17 | GROCERY STORE | -54.20",
        ]);
        assert_eq!(layout_of(&rows), None);
    }
}
