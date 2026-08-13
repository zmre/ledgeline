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
//! # Preamble: leading rows that are not the table
//!
//! Bank exports routinely open with an account line, a date range and a blank
//! line before the header. Those rows have a different field count from the
//! body, which is exactly how [`preamble_rows`] finds them — no pattern
//! matching, no keyword list, nothing that is language-specific.
//!
//! [`preamble_rows`] is deliberately written against a sequence of *widths*
//! rather than against records, because a workbook has the same problem and the
//! answer must not drift between the two lanes: [`super::spreadsheet`] hands it
//! how far each row of a sheet extends and gets the same rule applied to it. See
//! that module for why a sheet's row width is a column position rather than a
//! count of populated cells.
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

/// The narrowest table [`sniff_delimiter`] and [`preamble_rows`] will believe in.
///
/// A "table" of one column is what *every* candidate delimiter reports for a
/// file that contains none of them, so it carries no signal, and acting on it
/// would let a plain text file be reshaped by whichever candidate was tried
/// first. The same threshold is what stops [`preamble_rows`] eating a genuinely
/// one-column sheet, where *every* row is one field wide and "narrower than the
/// body" is therefore true of nothing.
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
    let preamble = preamble_rows(records.iter().map(Vec::len));

    let mut body = records.into_iter().skip(preamble);
    let Some(header) = body.next() else {
        return Err(ConvertError::Empty);
    };
    let rows: Vec<Vec<String>> = body.collect();
    let ragged = rows.iter().filter(|row| row.len() != header.len()).count();

    Ok(Tabular {
        header: Some(header),
        rows,
        truncated,
        statement: None,
        notes: notes(&decoded, delimiter, preamble, ragged),
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

/// How many leading rows are preamble rather than table, given how wide each row
/// is.
///
/// A row is preamble exactly when its width differs from the one the file
/// settles on, which is why this needs no keyword list, no row index and no
/// notion of what a title looks like — and so works the same in every language.
///
/// `widths` is a *sequence of widths* rather than the rows themselves because
/// both input lanes have this problem and their answers must not drift apart. A
/// delimited record's width is its field count; a sheet row's is one past its
/// last populated cell, which [`super::spreadsheet`] computes and explains.
///
/// Nothing is skipped when the file has no table shape to begin with
/// ([`MIN_TABLE_FIELDS`]) or when the run of odd rows is longer than any real
/// preamble ([`MAX_PREAMBLE_ROWS`]) — in the second case the file is not what we
/// think it is, and saying so through [`ConvertNote::RaggedRows`] beats
/// discarding most of it.
pub(super) fn preamble_rows(widths: impl Iterator<Item = usize> + Clone) -> usize {
    let modal = modal_width(widths.clone()).unwrap_or(0);
    if modal < MIN_TABLE_FIELDS {
        return 0;
    }
    let leading = widths
        .take(MAX_PREAMBLE_ROWS + 1)
        .take_while(|width| *width != modal)
        .count();
    if leading > MAX_PREAMBLE_ROWS {
        0
    } else {
        leading
    }
}

// ---------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------

/// Everything the conversion decided, in a fixed order so two runs over the same
/// bytes produce the same list.
fn notes(
    decoded: &Decoded,
    delimiter: Delimiter,
    preamble: usize,
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
    let skipped = (preamble > 0).then_some(ConvertNote::PreambleSkipped { lines: preamble });
    let uneven = (ragged > 0).then_some(ConvertNote::RaggedRows { count: ragged });
    [encoding, sniffed, skipped, uneven]
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

    /// The body has to outnumber the odd rows for the modal width to be the
    /// table's, so it is one longer than the run in front of it in both halves.
    fn odd_then_table(odd: usize) -> impl Iterator<Item = usize> + Clone {
        std::iter::repeat_n(1, odd).chain(std::iter::repeat_n(2, MAX_PREAMBLE_ROWS + 2))
    }

    #[test]
    fn preamble_is_bounded() {
        assert_eq!(
            preamble_rows(odd_then_table(MAX_PREAMBLE_ROWS)),
            MAX_PREAMBLE_ROWS
        );
        // One past the bound the file is not a statement with a header on it,
        // and reporting the rows as ragged beats discarding twenty-one of them.
        assert_eq!(preamble_rows(odd_then_table(MAX_PREAMBLE_ROWS + 1)), 0);
    }

    #[test]
    fn a_one_field_file_has_no_preamble_to_find() {
        // Every row is one field wide, so "narrower than the body" is true of
        // nothing and there is no signal to act on.
        assert_eq!(preamble_rows(std::iter::repeat_n(1, 8)), 0);
    }
}
