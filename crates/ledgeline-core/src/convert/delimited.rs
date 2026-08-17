//! Delimited text — CSV, TSV and SSV — decoded, split and normalised to one
//! [`Tabular`].
//!
//! Three decisions live here, and each one is a place where a plausible
//! shortcut produces a silently wrong answer rather than an error.
//!
//! # Encoding: BOM, then UTF-8, then a guess — in that order
//!
//! Delimited text declares nothing about its own encoding, so every one of those
//! steps is [`super::encoding`]'s, and [`super::encoding::Guess::Detect`] is what
//! this lane asks for once a BOM and UTF-8 validity have both come up empty.
//! That module owns the reasons the order is **mandatory rather than an
//! optimisation** — chiefly that `chardetng` cannot detect UTF-16 and answers
//! `windows-1252` for the BOM'd UTF-16LE that Excel's "Unicode Text" export
//! writes — and the rule that `1252` means Windows-1252 and never ISO-8859-1.
//!
//! # Delimiter: sniffed by row consistency, not by a crate
//!
//! `csv-sniffer` is unmaintained and panics on untrusted input, `qsv-sniffer`
//! declares itself superseded, and `csv-nose` misfires on European exports that
//! have both a preamble and decimal-comma amounts — silently returning a
//! two-column parse of a three-column file. See the plan's *Preprocessor
//! decisions*. [`sniff_delimiter`] scores each candidate on how consistent the
//! field count is across records and prefers the wider table on a tie, which is
//! what separates `;`-with-decimal-commas (three consistent fields) from reading
//! the same file on `,` (two consistent fields).
//!
//! # Margins: the rows at each end that are not the table
//!
//! Bank exports routinely open with an account line, a date range and a blank
//! line before the header, and close with a disclaimer block below the last
//! transaction. Both are found by [`margins`] — no pattern matching, no keyword
//! list, nothing that is language-specific — and the two ends are found by two
//! different rules, because they are two different questions:
//!
//! - A **preamble** row is one whose width differs from the width the file
//!   settles on. That is a heuristic: the row could in principle be a record we
//!   are throwing away, which is why it is bounded by [`MAX_PREAMBLE_ROWS`] and
//!   always reported.
//! - A **trailer** row is one that could not be a record *at all* — it holds
//!   nothing, or it is narrower than [`MIN_TABLE_FIELDS`], and a transaction
//!   needs at least a date and an amount. That is a statement rather than a
//!   guess, so it needs no bound; and because the test is on the row's own
//!   extent rather than on how it compares to the header, a last row whose final
//!   column happens to be empty is still a record and is still kept.
//!
//! Trimming the trailer is not a nicety. hledger aborts the **entire** read on
//! the first record it cannot parse — one blank line at the bottom of a file
//! costs the user every transaction above it, and the error names the row rather
//! than the file, so it reads as a broken rules file.
//!
//! [`margins`] is deliberately written against a sequence of [`RowShape`]s
//! rather than against records, because a workbook has the same problem and the
//! answer must not drift between the two lanes: [`super::spreadsheet`] hands it
//! how far each row of a sheet extends and gets the same rules applied to it. See
//! that module for why a sheet's row width is a column position rather than a
//! count of populated cells.
//!
//! # Blank rows, wherever they sit
//!
//! A row holding nothing at all is never a transaction — not at the end of a
//! file and not in the middle of one — so [`parse`] drops them from the body too
//! and reports how many. "Blank" here means *no populated cell*, which is a
//! different test from "narrower than the header": `2026-01-09,CORNER MARKET,`
//! is a real transaction whose last column is empty, and it is kept.
//!
//! # `skip N` counts records in the RAW download, not in ours
//!
//! Stripping the preamble moves the header to line 1 — and that silently breaks
//! the user's existing rules file, because `skip` is a number they wrote against
//! the file their bank gave them. hledger has **no header concept at all**: it
//! skips `N` records and treats every remaining one as data, so a rules file
//! written for a three-line preamble says `skip 3`, and run against our
//! header-on-line-1 CSV it skips the header *and the first two transactions*.
//! Verified against hledger 1.52: on a statement of two rows that is **zero
//! transactions and exit code 0** — total loss — and on a longer one it is a
//! silently truncated import that looks like it worked. No error either way,
//! and nothing on stderr.
//!
//! [`align_to_skip`] reconciles the two frames by prepending `N - 1` empty
//! records, so after the skip the first record is our first transaction. It is
//! applied to the copy hledger reads, never to the [`Tabular`] — the preview and
//! every in-memory consumer stay in the frame they were parsed in.
//!
//! Two things about it are not free choices, and both were measured rather than
//! reasoned about:
//!
//! - The padding **cannot be blank lines**. hledger discards those *before*
//!   `skip` counts, so two blank lines plus `skip 3` still eats two
//!   transactions. It has to be a record that is empty but present — `,,` — the
//!   same `,,,,` a spreadsheet's own CSV export writes.
//! - It is `N - 1` and not `N`, because our CSV still has a header and hledger
//!   still spends one of its `N` on it.
//!
//! # What this module does not do
//!
//! It never touches the filesystem and never sees a path, so no error it
//! produces can name one. A malformed record ends the read rather than being
//! skipped past: once the reader has lost the plot, later records are not what
//! the file says, and importing them would be worse than importing fewer.

use super::encoding::{self, Decoded, Guess};
use super::{ConvertError, ConvertNote, MAX_INPUT_BYTES, SourceFormat, Tabular};
use itertools::Itertools;
use std::borrow::Cow;

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

/// How many records any conversion keeps, header and preamble included.
///
/// Lives here rather than in [`super`] because that module is the frozen
/// interface contract for this work package; [`super::spreadsheet`] imports it
/// so both backends cap at the same place and a user cannot tell from the row
/// count which one ran.
///
/// [`MAX_INPUT_BYTES`] already bounds the *bytes*, but not the *records*: 16 MiB
/// of bare line breaks is sixteen million records, each of which would become
/// its own heap-allocated `Vec<String>`. Twenty years of daily transactions is
/// about 7,300 records, so this is more than an order of magnitude past any
/// honest statement and still small enough to hold comfortably.
pub const MAX_ROWS: usize = 100_000;

/// How much of the decoded text [`sniff_delimiter`] scores.
///
/// The delimiter is a property of the file's *shape*, and the shape is settled
/// within the first few dozen records. Scoring 16 MiB four times over — once per
/// candidate — would be three orders of magnitude of work for an answer that
/// stopped changing in the first kilobyte.
const SNIFF_BYTES: usize = 64 * 1024;

/// How many records of the sample each candidate delimiter is scored over.
const SNIFF_ROWS: usize = 200;

/// The delimiters tried when the format does not declare one, in preference
/// order — a tie is broken by this order, so an ambiguous file reads as the
/// comma-separated one its extension claims it is.
///
/// Space is deliberately absent even though hledger's `separator` accepts it: a
/// space-separated scorer treats every description containing a space as a wide
/// ragged row, and would win outright on files that are plainly commas.
const CANDIDATES: [char; 4] = [',', ';', '\t', '|'];

/// The narrowest table [`sniff_delimiter`] and [`margins`] will believe in.
///
/// A "table" of one column is what *every* candidate delimiter reports for a
/// file that contains none of them, so it carries no signal, and acting on it
/// would let a plain text file be reshaped by whichever candidate was tried
/// first. The same threshold is what stops [`margins`] eating a genuinely
/// one-column sheet, where *every* row is one field wide and "narrower than the
/// body" is therefore true of nothing.
///
/// It is also what a **trailer** row is measured against, and there it carries a
/// second meaning: every rules file needs at least a date and an amount, so a
/// row that reaches one field cannot be a transaction whatever it says.
const MIN_TABLE_FIELDS: usize = 2;

/// How many leading records may be discarded as preamble.
///
/// Real bank preambles run to one to eight lines. Past this the file is not a
/// statement with a header on it, and skipping five thousand records hunting for
/// one would be a far worse answer than reporting the rows as ragged and letting
/// the user look.
const MAX_PREAMBLE_ROWS: usize = 20;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Decode `bytes`, split them on the delimiter `format` implies (or, for CSV, on
/// the one the content implies) and return the table.
///
/// The first surviving record is the header — delimited statements always have
/// one, which is why [`Tabular::header`] is never `None` on this path. Every
/// judgement call made along the way is reported as a [`ConvertNote`] rather
/// than applied silently.
///
/// # Errors
///
/// - [`ConvertError::Empty`] — no bytes, or no records survived decoding (a file
///   holding nothing but a byte-order mark).
/// - [`ConvertError::TooLarge`] — past [`MAX_INPUT_BYTES`]. Checked here as well
///   as at the HTTP boundary so a caller that skips the server cannot get past it.
/// - [`ConvertError::Unsupported`] — `format` is not one of the delimited family.
pub fn parse(bytes: &[u8], format: SourceFormat) -> Result<Tabular, ConvertError> {
    if bytes.is_empty() {
        return Err(ConvertError::Empty);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ConvertError::TooLarge {
            limit: MAX_INPUT_BYTES,
        });
    }

    // Delimited text has nowhere to declare an encoding, so nothing is passed
    // as declared and the residual case falls to the detector.
    let decoded = encoding::decode(bytes, None, &Guess::Detect);
    let delimiter = match format {
        SourceFormat::Tsv => Delimiter::Declared('\t'),
        SourceFormat::Ssv => Delimiter::Declared(';'),
        SourceFormat::Csv => Delimiter::Sniffed(sniff_delimiter(&decoded.text)),
        other => {
            return Err(ConvertError::Unsupported {
                ext: other.as_str().to_string(),
            });
        }
    };

    let (records, truncated) = records(&decoded.text, delimiter.character());
    let margins = margins(&shapes(&records));

    // `take` before `skip`, and both saturating: `margins` counts its trailer
    // from *after* the preamble, so the two runs cannot overlap and the middle
    // is never negative.
    let last = records.len().saturating_sub(margins.trailer);
    let mut body = records.into_iter().take(last).skip(margins.preamble);
    let Some(header) = body.next() else {
        return Err(ConvertError::Empty);
    };
    // A row holding nothing at all is never a transaction, so it goes wherever
    // it sits — but it is a row the user can see, so it is counted and reported.
    let (blank, rows): (Vec<Vec<String>>, Vec<Vec<String>>) =
        body.partition(|record| is_blank(record));
    let ragged = rows.iter().filter(|row| row.len() != header.len()).count();

    Ok(Tabular {
        header: Some(header),
        rows,
        truncated,
        statement: None,
        notes: notes(&decoded, delimiter, margins, blank.len(), ragged),
    })
}

/// Render `tabular` as RFC 4180 CSV: comma separated, LF terminated, and quoted
/// only where quoting changes the reading.
///
/// This is the text `hledger import` is ultimately pointed at, so it is written
/// to be read back by anything: LF rather than CRLF because hledger, the `csv`
/// crate and every unix tool accept it and CRLF is the thing that leaks a stray
/// `\r` into the last column; minimal quoting because a file where every field
/// is quoted is much harder for a user to read when they open it to check what
/// we produced.
#[must_use]
pub fn to_csv(tabular: &Tabular) -> String {
    tabular
        .header
        .iter()
        .chain(tabular.rows.iter())
        .map(|record| render_record(record) + "\n")
        .collect()
}

/// Prepend whatever it takes to make a rules file written for the RAW download
/// read `csv` — which [`to_csv`] has already stripped the preamble from —
/// starting at the same transaction.
///
/// `skip` is the chosen rules file's own `skip` setting
/// (`RulesDoc::settings().skip`), and the answer is `skip - 1` empty records.
/// The module docs argue the whole trap; the short version is that hledger has
/// no header concept, so `skip 3` written for a two-line preamble skips our
/// header **and two transactions**, silently and with exit code 0.
///
/// Both edges are the identity, and deliberately so: `skip 0` and `skip 1` are
/// already correct against a header-on-line-1 file, so a statement whose rules
/// file was written for our shape is handed back byte for byte and nothing about
/// the common case changes.
///
/// The padding is rendered by [`render_record`], which is what makes the width
/// question look after itself: a three-column table pads with `,,`, and a
/// one-column table pads with `""` rather than an empty line — an empty line is
/// the one thing that does **not** work, because hledger drops blank lines
/// before `skip` counts them. Both spellings verified against hledger 1.52.
#[must_use]
pub fn align_to_skip(csv: &str, skip: u32) -> String {
    // Past the record cap the padding cannot change the outcome — a `skip` that
    // large consumes every record the file is allowed to hold whatever we put in
    // front of it — so this is a bound on the arithmetic, not a policy. Without
    // it a rules file reading `skip 4000000000`, which is a typo rather than an
    // attack, would ask for twelve gigabytes of commas.
    let lines = usize::try_from(skip.saturating_sub(1))
        .unwrap_or(MAX_ROWS)
        .min(MAX_ROWS);
    // Zero columns means there is no table to align to — `csv` is empty, or it
    // is a single unterminated line the reader could not make a record of. A
    // record of no fields renders as an empty line, which hledger would discard,
    // so padding here would produce a file that skips its own header instead.
    let columns = columns_of(csv);
    if lines == 0 || columns == 0 {
        return csv.to_string();
    }
    let empty = render_record(&vec![String::new(); columns]) + "\n";
    empty.repeat(lines) + csv
}

/// How many fields the first record of `csv` holds.
///
/// Read with the same reader everything else here uses, on a comma, because the
/// only text this is ever asked about is [`to_csv`]'s own output and that is
/// RFC 4180 by construction — quoted fields included, so a `"SMITH, JOHN"` in
/// the header counts as one column rather than two.
fn columns_of(csv: &str) -> usize {
    reader(csv, ',')
        .into_records()
        .next()
        .and_then(Result::ok)
        .map_or(0, |record| record.len())
}

/// One record as an RFC 4180 line, without its terminator.
fn render_record(record: &[String]) -> String {
    match record {
        // RFC 4180 gives an empty line and a record of one empty field exactly
        // the same bytes, so quoting is the only way to tell them apart. This is
        // therefore still "quote when needed" — it is simply a case where the
        // need comes from the record's shape rather than the field's content.
        [only] if only.is_empty() => "\"\"".to_string(),
        _ => record.iter().map(|field| quote(field)).join(","),
    }
}

/// A field, quoted if and only if leaving it bare would change how it reads.
fn quote(field: &str) -> Cow<'_, str> {
    if field.contains([',', '"', '\r', '\n']) || field.starts_with(BOM_CHAR) {
        Cow::Owned(format!("\"{}\"", field.replace('"', "\"\"")))
    } else {
        Cow::Borrowed(field)
    }
}

/// U+FEFF, which every CSV reader — the `csv` crate and hledger included —
/// strips when it is the first thing in a file.
///
/// A cell whose text genuinely begins with one is therefore unrepresentable
/// bare: written as itself in the top-left field it silently disappears on the
/// next read. Quoting moves it off byte zero and it survives, which makes this
/// another case where quoting is *needed* rather than decorative. Cheap enough
/// to apply to every field rather than only the first.
const BOM_CHAR: char = '\u{feff}';

// ---------------------------------------------------------------------------
// Delimiter
// ---------------------------------------------------------------------------

/// Where the delimiter came from, which is what decides whether the user is owed
/// a [`ConvertNote::DelimiterSniffed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delimiter {
    /// The extension said so: `.tsv` is tabs, `.ssv` is semicolons.
    Declared(char),
    /// `.csv` says only "delimited", so the content decided.
    Sniffed(char),
}

impl Delimiter {
    fn character(self) -> char {
        match self {
            Self::Declared(character) | Self::Sniffed(character) => character,
        }
    }
}

/// How consistently one candidate delimiter splits the sample.
///
/// `hits`/`total` is the share of records that agree on a field count, kept as a
/// fraction rather than a float so two candidates can be compared **exactly**,
/// by cross-multiplication. `total` differs between candidates — a quote that is
/// a quote under one delimiter is a literal character under another, and an
/// embedded newline then splits a record in two — so the shares genuinely need
/// the denominators.
#[derive(Debug, Clone, Copy)]
struct Consistency {
    hits: u64,
    total: u64,
    modal_fields: usize,
}

impl Consistency {
    /// Share first, width second.
    ///
    /// Share first because a delimiter that is not in the file at all still
    /// produces a perfectly consistent one-field parse, and the only thing that
    /// distinguishes a real delimiter is that the *count* it produces repeats.
    /// Width second because a European `d;payee;-12,50` reads as three
    /// consistent fields on `;` and two equally consistent fields on `,` — the
    /// tie is real, and the wider reading is the right one.
    fn rank_against(self, other: Self) -> std::cmp::Ordering {
        (self.hits * other.total)
            .cmp(&(other.hits * self.total))
            .then(self.modal_fields.cmp(&other.modal_fields))
    }
}

/// The delimiter `text` is most consistently split by.
///
/// Falls back to a comma when no candidate produces a table at all — a
/// single-column file has no delimiter, and a comma is what an unlabelled data
/// file overwhelmingly is (the same default the rules-file CSV preview applies
/// to an unknown extension).
fn sniff_delimiter(text: &str) -> char {
    let sample = sample_of(text);
    CANDIDATES
        .iter()
        .fold(None, |best: Option<(char, Consistency)>, &candidate| {
            let scored = consistency(sample, candidate);
            if scored.modal_fields < MIN_TABLE_FIELDS {
                return best;
            }
            match best {
                // Strictly-greater, walking the candidates in preference order,
                // so a tie is settled by `CANDIDATES` rather than by iteration
                // order — which is what keeps an ambiguous file reading as the
                // comma-separated one its extension claims it is.
                Some((_, incumbent)) if scored.rank_against(incumbent).is_le() => best,
                _ => Some((candidate, scored)),
            }
        })
        .map_or(',', |(candidate, _)| candidate)
}

/// Score one candidate over the sample.
fn consistency(sample: &str, delimiter: char) -> Consistency {
    let counts = reader(sample, delimiter)
        .into_records()
        .map_while(Result::ok)
        .take(SNIFF_ROWS)
        .map(|record| record.len())
        .counts();

    // Ties on the tally go to the wider record: a file whose preamble is as long
    // as its body should still be read as the table it contains.
    let modal = counts
        .iter()
        .max_by_key(|(fields, hits)| (**hits, **fields))
        .map(|(fields, hits)| (*fields, *hits));

    let total: usize = counts.values().sum();
    Consistency {
        hits: as_u64(modal.map_or(0, |(_, hits)| hits)),
        total: as_u64(total),
        modal_fields: modal.map_or(0, |(fields, _)| fields),
    }
}

/// The leading slice of `text` the sniffer scores, cut at a line break so the
/// last record it sees is a whole one.
fn sample_of(text: &str) -> &str {
    if text.len() <= SNIFF_BYTES {
        return text;
    }
    let cut = text.as_bytes()[..SNIFF_BYTES]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or_else(
            // 64 KiB with no line break in it: there is no whole record to cut
            // at, so fall back to the nearest character boundary, which is never
            // more than three bytes back.
            || {
                (0..=SNIFF_BYTES)
                    .rev()
                    .find(|at| text.is_char_boundary(*at))
                    .unwrap_or(0)
            },
            |at| at + 1,
        );
    text.get(..cut).unwrap_or(text)
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// A `csv` reader configured the one way this crate ever configures one.
///
/// `flexible` because a statement whose trailer row is short must still import —
/// the user came here to be shown what is in the file, including the mess — and
/// `has_headers(false)` because *which* record is the header is decided after
/// the preamble has been found, not by the reader.
fn reader(text: &str, delimiter: char) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .delimiter(delimiter_byte(delimiter))
        .flexible(true)
        .has_headers(false)
        .from_reader(text.as_bytes())
}

/// Every record, capped at [`MAX_ROWS`], plus whether the cap was reached.
fn records(text: &str, delimiter: char) -> (Vec<Vec<String>>, bool) {
    let mut kept: Vec<Vec<String>> = reader(text, delimiter)
        .into_records()
        // A malformed record ends the read rather than being skipped past: once
        // the reader has lost the plot the later records are not what the file
        // says, and importing them would be worse than importing fewer.
        .map_while(Result::ok)
        .take(MAX_ROWS + 1)
        .map(|record| record.iter().map(str::to_string).collect())
        .collect();
    let truncated = kept.len() > MAX_ROWS;
    kept.truncate(MAX_ROWS);
    (kept, truncated)
}

/// The width most rows agree on, or `None` when there are none.
fn modal_width(widths: impl Iterator<Item = usize>) -> Option<usize> {
    widths
        .counts()
        .into_iter()
        // Same tie-break as [`consistency`], for the same reason.
        .max_by_key(|(fields, hits)| (*hits, *fields))
        .map(|(fields, _)| fields)
}

/// How one row looks to [`margins`].
///
/// A *shape* rather than the row itself because both input lanes have this
/// problem and their answers must not drift apart. A delimited record's width is
/// its field count; a sheet row's is one past its last populated cell, which
/// [`super::spreadsheet`] computes and explains. `blank` is deliberately its own
/// field rather than `width == 0`: a record of fifteen empty fields is fifteen
/// fields wide and holds nothing, and `,,,,,,,,,,,,,,` is exactly what a
/// statement's trailer looks like once a spreadsheet has been saved as CSV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RowShape {
    pub width: usize,
    pub blank: bool,
}

/// How many rows at each end of a region are not part of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Margins {
    /// Leading rows that are not the table. Reported as
    /// [`ConvertNote::PreambleSkipped`].
    pub preamble: usize,
    /// Trailing rows that are not the table. Reported as
    /// [`ConvertNote::TrailerSkipped`].
    pub trailer: usize,
}

/// The rows at each end of `rows` that are not records.
///
/// The two ends are found by two different rules because they are two different
/// questions; the module docs argue why. In short:
///
/// - **Preamble**: a leading run of rows whose width differs from the one the
///   file settles on. A guess, so it is bounded by [`MAX_PREAMBLE_ROWS`] — past
///   that the file is not a statement with a header on it, and saying so through
///   [`ConvertNote::RaggedRows`] beats discarding most of it.
/// - **Trailer**: a trailing run of rows that could not be records at all —
///   blank, or narrower than [`MIN_TABLE_FIELDS`]. Not a guess, so not bounded:
///   there is no file in which a row holding nothing, or holding one field, is
///   the transaction we should have kept.
///
/// Nothing is trimmed at either end when the region has no table shape to begin
/// with ([`MIN_TABLE_FIELDS`]) — in a genuinely one-column file *every* row is
/// one field wide, so "too narrow to be a record" is true of all of them and
/// acting on it would eat the file.
///
/// **Blank rows do not vote on the modal width.** They are not records, so they
/// have nothing to say about what a record looks like, and letting them vote is
/// self-defeating: a file with more blank rows than transactions would elect a
/// modal width of zero and then refuse to trim the blank rows, which is the one
/// case where trimming them matters most.
pub(super) fn margins(rows: &[RowShape]) -> Margins {
    let modal = modal_width(rows.iter().filter(|row| !row.blank).map(|row| row.width)).unwrap_or(0);
    if modal < MIN_TABLE_FIELDS {
        return Margins::default();
    }
    let leading = rows
        .iter()
        .take(MAX_PREAMBLE_ROWS + 1)
        // NARROWER than the table, not merely different from it. A preamble line
        // is a title or an account line: it does not reach as far as the table
        // does. A row that reaches FURTHER is the opposite case and trimming it
        // costs a transaction -- a trailing column populated on only a few rows,
        // a "Notes" or "Check #", makes the modal one short of the header's own
        // extent, and `!=` then eats the header and promotes the first
        // transaction into it, silently, under a `skip 1` rules file.
        .take_while(|row| !could_be_a_record(row) || row.width < modal)
        .count();
    let preamble = if leading > MAX_PREAMBLE_ROWS {
        0
    } else {
        leading
    };
    // Counted from after the preamble so the two runs can never overlap on a
    // file that is nothing but margins.
    let trailer = rows
        .get(preamble..)
        .unwrap_or_default()
        .iter()
        .rev()
        .take_while(|row| !could_be_a_record(row))
        .count();
    Margins { preamble, trailer }
}

/// Whether a row could carry a transaction at all.
///
/// Deliberately **not** "as wide as the header": a row whose last column is
/// empty reaches one short of the header and is a perfectly ordinary
/// transaction. What disqualifies a row is holding nothing, or not reaching far
/// enough to hold both of the two fields every rules file needs.
///
fn could_be_a_record(row: &RowShape) -> bool {
    !row.blank && row.width >= MIN_TABLE_FIELDS
}

/// Every record's shape, in order.
fn shapes(records: &[Vec<String>]) -> Vec<RowShape> {
    records
        .iter()
        .map(|record| RowShape {
            width: populated_extent(record),
            blank: is_blank(record),
        })
        .collect()
}

/// Whether a record holds nothing at all.
///
/// Whitespace does not count as content, matching
/// [`super::spreadsheet`]'s cell test: a lone space is what a spreadsheet leaves
/// behind when someone "clears" a cell, and hledger cannot read one as a date
/// either.
fn is_blank(record: &[String]) -> bool {
    record.iter().all(|field| field.trim().is_empty())
}

/// One past the record's last populated field, using the same "whitespace is not
/// content" test as [`is_blank`].
///
/// Trailing empties are what a spreadsheet's CSV export pads every short row
/// with, so they say nothing about how far the row actually reaches. Zero for a
/// blank record, which makes the extent test subsume the blank one.
fn populated_extent(record: &[String]) -> usize {
    record
        .iter()
        .rposition(|field| !field.trim().is_empty())
        .map_or(0, |last| last + 1)
}

// ---------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------

/// Everything the conversion decided, in a fixed order so two runs over the same
/// bytes produce the same list.
fn notes(
    decoded: &Decoded,
    delimiter: Delimiter,
    margins: Margins,
    blanks: usize,
    ragged: usize,
) -> Vec<ConvertNote> {
    let encoding = decoded
        .guessed
        .clone()
        .map(|label| ConvertNote::EncodingGuessed { label });
    let sniffed = match delimiter {
        Delimiter::Declared(_) => None,
        Delimiter::Sniffed(character) => Some(ConvertNote::DelimiterSniffed {
            delimiter: character,
        }),
    };
    let above = (margins.preamble > 0).then_some(ConvertNote::PreambleSkipped {
        lines: margins.preamble,
    });
    let below = (margins.trailer > 0).then_some(ConvertNote::TrailerSkipped {
        lines: margins.trailer,
    });
    let empty = (blanks > 0).then_some(ConvertNote::BlankRowsDropped { count: blanks });
    let uneven = (ragged > 0).then_some(ConvertNote::RaggedRows { count: ragged });
    [encoding, sniffed, above, below, empty, uneven]
        .into_iter()
        .flatten()
        .collect()
}

// ---------------------------------------------------------------------------
// Small conversions
// ---------------------------------------------------------------------------

/// The delimiter as the single byte `csv::ReaderBuilder` takes.
///
/// [`CANDIDATES`] and the declared delimiters are all ASCII, so the fallback is
/// unreachable; it is spelled out rather than asserted because a panic in a
/// preview would take down a request that was only ever asked to look. Same
/// reasoning, and same shape, as `rules::discovery::delimiter_byte`.
fn delimiter_byte(delimiter: char) -> u8 {
    u8::try_from(u32::from(delimiter)).unwrap_or(b',')
}

/// A count as a `u64`. Every caller's value is bounded by [`SNIFF_ROWS`], so the
/// saturating arm cannot be reached; it exists so the conversion needs no `as`.
fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The encoding tests that used to live here moved with the code they
    // exercise, into `super::encoding`.

    #[test]
    fn a_lone_empty_field_is_distinguishable_from_a_blank_line() {
        assert_eq!(render_record(&[String::new()]), "\"\"");
        assert_eq!(render_record(&[String::new(), String::new()]), ",");
    }

    #[test]
    fn quoting_is_minimal() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("a,b"), "\"a,b\"");
        assert_eq!(quote("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(quote("two\nlines"), "\"two\nlines\"");
        assert_eq!(quote("semi;colon"), "semi;colon");
    }

    #[test]
    fn a_leading_byte_order_mark_is_quoted_so_it_survives_a_re_read() {
        // Bare, this cell would be byte zero of the file and every CSV reader
        // would strip it as a BOM.
        assert_eq!(quote("\u{feff}Date"), "\"\u{feff}Date\"");
        assert_eq!(quote("Date\u{feff}"), "Date\u{feff}");
    }

    /// Three columns and two transactions — the shape `to_csv` produces once a
    /// preamble has been stripped, and therefore the shape a rules file's `skip`
    /// disagrees with.
    const CONVERTED: &str = "Date,Description,Amount\n2026-01-05,A,-1.00\n2026-01-06,B,-2.00\n";

    #[test]
    fn a_skip_of_zero_or_one_already_lines_up_and_is_left_alone() {
        // `skip 1` is a rules file written for a file with a header and no
        // preamble, which is exactly what we produce. Byte for byte, or the fix
        // would be changing every import that works today.
        assert_eq!(align_to_skip(CONVERTED, 1), CONVERTED);
        assert_eq!(align_to_skip(CONVERTED, 0), CONVERTED);
    }

    #[test]
    fn a_skip_of_n_prepends_n_minus_one_empty_records() {
        // `skip 3`: two preamble lines and a header in the raw download. After
        // three records are skipped the first record must be `2026-01-05`.
        assert_eq!(align_to_skip(CONVERTED, 3), format!(",,\n,,\n{CONVERTED}"));
        assert_eq!(align_to_skip(CONVERTED, 2), format!(",,\n{CONVERTED}"));
        assert_eq!(
            align_to_skip(CONVERTED, 5),
            format!(",,\n,,\n,,\n,,\n{CONVERTED}")
        );
    }

    #[test]
    fn the_padding_is_as_wide_as_the_table() {
        // Not decoration: a human opening the file sees a rectangle, and our own
        // re-read counts these as blank ROWS rather than as a ragged margin.
        for columns in 2..6 {
            let header: Vec<String> = (0..columns).map(|n| format!("c{n}")).collect();
            let csv = to_csv(&Tabular {
                header: Some(header),
                rows: vec![vec!["x".to_string(); columns]],
                ..Tabular::default()
            });
            let padded = align_to_skip(&csv, 2);
            let first = padded.lines().next().expect("a padding line");
            assert_eq!(first.matches(',').count(), columns - 1, "{columns} columns");
        }
    }

    #[test]
    fn a_one_column_table_pads_with_a_quoted_empty_field_not_a_blank_line() {
        // The case that would silently defeat the whole fix. hledger discards
        // truly blank lines BEFORE `skip` counts them, so a bare `\n` here would
        // leave the header inside the skipped run and eat a transaction.
        // `""` is a record, and hledger counts it (verified against 1.52).
        let csv = "Amount\n-1.00\n";
        assert_eq!(align_to_skip(csv, 3), format!("\"\"\n\"\"\n{csv}"));
    }

    #[test]
    fn an_empty_csv_has_no_table_to_align_to() {
        // No header, no columns, nothing a `skip` could be counted against —
        // and padding it would invent records in a file that has none.
        assert_eq!(align_to_skip("", 3), "");
        assert_eq!(align_to_skip("", 0), "");
    }

    #[test]
    fn an_absurd_skip_is_bounded_by_the_record_cap() {
        // A typo in someone's rules file, not an attack — but `skip 4000000000`
        // would ask for twelve gigabytes of commas if this were unbounded.
        let padded = align_to_skip(CONVERTED, u32::MAX);
        assert_eq!(padded.lines().count(), MAX_ROWS + CONVERTED.lines().count());
    }

    /// A row reaching `width` fields.
    fn wide(width: usize) -> RowShape {
        RowShape {
            width,
            blank: false,
        }
    }

    /// A row holding nothing. Its extent is zero however many empty fields it
    /// was padded out to, which is what makes `,,,` and `` the same shape.
    fn empty() -> RowShape {
        RowShape {
            width: 0,
            blank: true,
        }
    }

    /// The body has to outnumber the odd rows for the modal width to be the
    /// table's, so it is one longer than the run in front of it in both halves.
    fn odd_then_table(odd: usize) -> Vec<RowShape> {
        std::iter::repeat_n(wide(1), odd)
            .chain(std::iter::repeat_n(wide(2), MAX_PREAMBLE_ROWS + 2))
            .collect()
    }

    #[test]
    fn preamble_is_bounded() {
        assert_eq!(
            margins(&odd_then_table(MAX_PREAMBLE_ROWS)).preamble,
            MAX_PREAMBLE_ROWS
        );
        // One past the bound the file is not a statement with a header on it,
        // and reporting the rows as ragged beats discarding twenty-one of them.
        assert_eq!(margins(&odd_then_table(MAX_PREAMBLE_ROWS + 1)).preamble, 0);
    }

    #[test]
    fn a_one_field_file_has_no_margins_to_find() {
        // Every row is one field wide, so "narrower than the body" is true of
        // nothing — and so is "too narrow to be a record", which would otherwise
        // trim the whole file away from the bottom.
        let one_wide: Vec<RowShape> = std::iter::repeat_n(wide(1), 8).collect();
        assert_eq!(margins(&one_wide), Margins::default());
    }

    #[test]
    fn a_trailer_is_trimmed_however_wide_its_blank_rows_are() {
        // The shape a statement has once a spreadsheet has been saved as CSV:
        // the disclaimer paragraphs are one field, and the blank rows between
        // them are as wide as the table because they are all its commas.
        let rows = [
            wide(4),
            wide(4),
            wide(4),
            empty(),
            wide(1),
            empty(),
            wide(1),
        ];
        assert_eq!(
            margins(&rows),
            Margins {
                preamble: 0,
                trailer: 4
            }
        );
    }

    #[test]
    fn a_last_row_with_an_empty_final_column_is_not_a_trailer() {
        // `2026-01-09,CORNER MARKET,-31.18,` — a real transaction that reaches
        // one short of the header. A rule spelled "narrower than the table"
        // eats it; the rule is "too narrow to hold a date and an amount".
        let rows = [wide(4), wide(4), wide(3)];
        assert_eq!(margins(&rows), Margins::default());
    }

    #[test]
    fn a_trailer_is_not_bounded_the_way_a_preamble_is() {
        // Unlike a preamble, a trailing run of unusable rows is not a heuristic
        // that might be discarding records — so a long one is trimmed rather
        // than abandoned.
        let rows: Vec<RowShape> = std::iter::repeat_n(wide(3), 3)
            .chain(std::iter::repeat_n(empty(), MAX_PREAMBLE_ROWS * 5))
            .collect();
        assert_eq!(margins(&rows).trailer, MAX_PREAMBLE_ROWS * 5);
    }

    #[test]
    fn a_blank_record_is_blank_however_it_is_spelled() {
        assert!(is_blank(&[]));
        assert!(is_blank(&[String::new(), String::new()]));
        assert!(is_blank(&[" ".to_string(), "\t".to_string()]));
        assert!(!is_blank(&[String::new(), "0".to_string()]));
    }
}
