//! `convert::delimited` and `convert::spreadsheet` over the committed corpus in
//! `fixtures/import/`, plus the properties that no fixture can pin.
//!
//! Three kinds of check live here and it matters that they stay separate:
//!
//! | Kind | What it proves |
//! | --- | --- |
//! | Per-fixture | One real file of one real shape converts to the table we say it does |
//! | Property | `to_csv` is lossless, and neither `parse` panics on anything |
//! | Adversarial | The **order** of encoding detection, asserted by showing the other order is wrong |
//!
//! The third is the one worth reading. `utf16le_bom_is_decoded_before_any_guess`
//! runs `chardetng` directly against the same bytes and asserts it answers
//! `windows-1252`, which is wrong. That is not a hypothetical: BOM'd UTF-16LE is
//! what Excel's "Unicode Text" export writes, so a reordering of `decode` would
//! silently import every statement exported that way as NUL-separated mojibake.
//! Asserting the trap exists is what stops the guard being "simplified" away.

mod common;

use ledgeline_core::convert::{
    ConvertError, ConvertNote, MAX_INPUT_BYTES, SourceFormat, Tabular, delimited, spreadsheet,
};
use proptest::prelude::*;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Fixture access
// ---------------------------------------------------------------------------

fn import_fixture(relative: &str) -> Vec<u8> {
    let path: PathBuf = common::fixtures_dir().join("import").join(relative);
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {relative} should read: {error}"))
}

fn parse_delimited(relative: &str, format: SourceFormat) -> Tabular {
    delimited::parse(&import_fixture(relative), format)
        .unwrap_or_else(|error| panic!("{relative} should convert: {error}"))
}

fn parse_spreadsheet(relative: &str, format: SourceFormat) -> Tabular {
    spreadsheet::parse(&import_fixture(relative), format)
        .unwrap_or_else(|error| panic!("{relative} should convert: {error}"))
}

fn header(tabular: &Tabular) -> &[String] {
    tabular
        .header
        .as_deref()
        .expect("a delimited or spreadsheet conversion always names its header row")
}

/// The rows as `&str`, which is what every assertion below actually wants.
fn rows(tabular: &Tabular) -> Vec<Vec<&str>> {
    tabular
        .rows
        .iter()
        .map(|row| row.iter().map(String::as_str).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// Delimited: the delimiter
// ---------------------------------------------------------------------------

#[test]
fn tsv_takes_its_delimiter_from_the_extension() {
    let table = parse_delimited("delimited/tab.tsv", SourceFormat::Tsv);

    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    assert_eq!(table.rows.len(), 4);
    assert_eq!(
        rows(&table)[0],
        ["2026-01-05", "COFFEE ROASTERS", "-4.75", "1195.25"]
    );
    // A declared delimiter is not a judgement call, so it owes no note.
    assert_eq!(table.notes, Vec::new());
}

#[test]
fn ssv_takes_its_delimiter_from_the_extension() {
    let table = parse_delimited("delimited/semicolon.ssv", SourceFormat::Ssv);

    assert_eq!(header(&table), ["Datum", "Beschreibung", "Betrag", "Saldo"]);
    // The decimal comma is INSIDE a field, which is the whole point of the
    // fixture: split this file on `,` and every amount becomes two columns.
    assert_eq!(
        rows(&table)[0],
        ["2026-01-05", "KAUFLAND BERLIN", "-42,17", "1195,25"]
    );
    assert_eq!(table.notes, Vec::new());
}

#[test]
fn a_csv_that_is_really_semicolons_is_sniffed_as_semicolons() {
    // The same bytes, offered under the extension that declares nothing. This is
    // the European export the plan says `csv-nose` gets wrong: a preamble-free
    // file whose amounts contain the competing delimiter.
    let bytes = import_fixture("delimited/semicolon.ssv");
    let table = delimited::parse(&bytes, SourceFormat::Csv).expect("should convert");

    assert_eq!(header(&table), ["Datum", "Beschreibung", "Betrag", "Saldo"]);
    assert!(
        table
            .notes
            .contains(&ConvertNote::DelimiterSniffed { delimiter: ';' }),
        "the sniff is a judgement call and must be reported: {:?}",
        table.notes
    );
}

#[test]
fn a_plain_comma_csv_sniffs_as_commas() {
    let table = parse_delimited("delimited/quoted.csv", SourceFormat::Csv);

    assert_eq!(header(&table), ["Date", "Description", "Amount"]);
    assert!(
        table
            .notes
            .contains(&ConvertNote::DelimiterSniffed { delimiter: ',' })
    );
}

#[test]
fn quoting_survives_embedded_delimiters_and_newlines() {
    let table = parse_delimited("delimited/quoted.csv", SourceFormat::Csv);

    assert_eq!(table.rows.len(), 3);
    assert_eq!(rows(&table)[0][1], "SMITH, JOHN & CO");
    assert_eq!(rows(&table)[1][1], "CARD PAYMENT\nTHANK YOU");
    assert_eq!(rows(&table)[2][1], "THEY SAID \"HELLO\"");
    // An embedded newline must not have become an extra record.
    assert!(!table.notes.iter().any(|note| matches!(
        note,
        ConvertNote::RaggedRows { .. } | ConvertNote::PreambleSkipped { .. }
    )));
}

// ---------------------------------------------------------------------------
// Delimited: preamble and ragged rows
// ---------------------------------------------------------------------------

#[test]
fn a_preamble_is_skipped_and_reported() {
    let table = parse_delimited("delimited/preamble.csv", SourceFormat::Csv);

    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    assert_eq!(table.rows.len(), 3);
    assert!(
        table
            .notes
            .contains(&ConvertNote::PreambleSkipped { lines: 2 }),
        "{:?}",
        table.notes
    );
    // The second preamble line contains commas, so a naive "the first record is
    // the header" reading would have produced a two-column table.
    assert!(
        !table
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::RaggedRows { .. }))
    );
}

#[test]
fn ragged_rows_are_kept_and_counted() {
    let table = parse_delimited("delimited/ragged.csv", SourceFormat::Csv);

    assert_eq!(header(&table).len(), 4);
    // Nothing is dropped: the user came here to be shown what is in the file.
    assert_eq!(table.rows.len(), 4);
    assert_eq!(rows(&table)[1], ["2026-01-06", "ATM WITHDRAWAL", "-100.00"]);
    assert_eq!(table.rows[2].len(), 5);
    assert!(
        table.notes.contains(&ConvertNote::RaggedRows { count: 2 }),
        "{:?}",
        table.notes
    );
}

#[test]
fn a_trailer_is_dropped_and_reported() {
    let table = parse_delimited("delimited/trailer.csv", SourceFormat::Csv);

    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    assert_eq!(table.rows.len(), 4);
    assert!(
        table
            .notes
            .contains(&ConvertNote::TrailerSkipped { lines: 4 }),
        "{:?}",
        table.notes
    );
    // The delimited half of the trailer is the case a width rule alone misses:
    // saved out of a spreadsheet, a blank row is `,,,` — as many fields as the
    // table and not one of them populated.
    assert!(
        !rows(&table)
            .iter()
            .any(|row| row.iter().all(|cell| cell.trim().is_empty())),
        "a row of empty commas is not a transaction: {:?}",
        rows(&table)
    );
    // Nothing was left over to report as ragged, because nothing short was kept.
    assert!(
        !table
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::RaggedRows { .. })),
        "{:?}",
        table.notes
    );
}

#[test]
fn a_padded_prose_trailer_is_trimmed_even_though_it_is_as_wide_as_the_table() {
    // `trailer.csv` covers padded BLANK rows; this is padded PROSE. A
    // spreadsheet's CSV export pads every short row out to the table's width, so
    // `Member FDIC,,,` has as many fields as a transaction and is not blank.
    // Counted by fields it "could be a record", the trim stops at it, and
    // hledger then abandons the whole file on the first one it cannot read.
    let table = parse_delimited("delimited/padded-prose.csv", SourceFormat::Csv);

    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    assert_eq!(rows(&table).len(), 4, "{:?}", rows(&table));
    assert_eq!(rows(&table)[0][1], "GROCERY STORE");
    assert_eq!(rows(&table)[3][1], "CORNER MARKET");
    assert!(
        table
            .notes
            .contains(&ConvertNote::PreambleSkipped { lines: 4 }),
        "{:?}",
        table.notes
    );
    assert!(
        table
            .notes
            .contains(&ConvertNote::TrailerSkipped { lines: 3 }),
        "{:?}",
        table.notes
    );
    // The thing that must not come back: prose in the data.
    assert!(
        !rows(&table)
            .iter()
            .any(|row| row.iter().any(|cell| cell.contains("Disclaimer"))),
        "{:?}",
        rows(&table)
    );
}

#[test]
fn a_body_wider_than_its_own_header_does_not_cost_a_transaction() {
    // Every data row ends with the delimiter, so the body is four fields and the
    // header three. By field count the header is the odd row out and is trimmed
    // as preamble -- which makes the first TRANSACTION the header. Under a
    // `skip 1` rules file that transaction then disappears with no error at all,
    // which is the worst shape a bug in this module can take.
    let table = parse_delimited("delimited/trailing-delimiter.csv", SourceFormat::Csv);

    assert_eq!(header(&table), ["Date", "Description", "Amount"]);
    assert_eq!(rows(&table).len(), 4, "{:?}", rows(&table));
    assert_eq!(rows(&table)[0][0], "2026-01-05", "no transaction was eaten");
    assert!(
        !table
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::PreambleSkipped { .. })),
        "the header is not preamble: {:?}",
        table.notes
    );
    // It IS honest about the shape, which is the note the user can act on.
    assert!(
        table.notes.contains(&ConvertNote::RaggedRows { count: 4 }),
        "{:?}",
        table.notes
    );
}

#[test]
fn a_blank_row_between_the_transactions_is_dropped_and_reported() {
    let table = parse_delimited("delimited/trailer.csv", SourceFormat::Csv);

    // hledger abandons the whole read on the first record it cannot parse, so a
    // blank row in the middle costs every transaction after it just as surely as
    // one at the end costs every transaction before it.
    assert!(
        table
            .notes
            .contains(&ConvertNote::BlankRowsDropped { count: 1 }),
        "{:?}",
        table.notes
    );
    assert_eq!(rows(&table)[2][1], "EMPLOYER PAYROLL");
}

#[test]
fn a_last_record_with_an_empty_final_field_is_not_trimmed() {
    // The negative case. `2026-01-09,CORNER MARKET,-31.18,` is four fields, one
    // of which is empty — a transaction whose running balance the bank did not
    // print. It is the row the trailer trim has to stop at.
    let table = parse_delimited("delimited/trailer.csv", SourceFormat::Csv);

    assert_eq!(
        rows(&table)[3],
        ["2026-01-09", "CORNER MARKET", "-31.18", ""]
    );
}

// ---------------------------------------------------------------------------
// Delimited: encoding
// ---------------------------------------------------------------------------

#[test]
fn windows_1252_is_guessed_and_the_c1_range_survives() {
    let table = parse_delimited("delimited/latin1.csv", SourceFormat::Csv);

    // 0x92, 0x93/0x94 and 0x80 are the four bytes where Windows-1252 and
    // ISO-8859-1 disagree. Reading this file as ISO-8859-1 yields C1 control
    // characters, and a rules file matching on the payee would never fire.
    assert_eq!(rows(&table)[0][1], "MCDONALD\u{2019}S RESTAURANT");
    assert_eq!(
        rows(&table)[1][1],
        "\u{201c}THE CORNER\u{201d} DELICATESSEN"
    );
    assert_eq!(rows(&table)[2][1], "CAF\u{c9} R\u{c9}PUBLIQUE PARIS");
    assert_eq!(rows(&table)[3][1], "ACME GMBH INVOICE \u{20ac}50 FEE");

    let guessed = table
        .notes
        .iter()
        .find_map(|note| match note {
            ConvertNote::EncodingGuessed { label } => Some(label.clone()),
            _ => None,
        })
        .expect("a guessed encoding is owed a note");
    assert_eq!(guessed, "windows-1252");
}

#[test]
fn utf16le_bom_is_decoded_before_any_guess() {
    let bytes = import_fixture("delimited/utf16le-bom.csv");
    assert_eq!(&bytes[..2], [0xFF, 0xFE], "the fixture must carry the BOM");

    // The trap, asserted rather than described: chardetng cannot detect UTF-16
    // at all, and does not decline — it answers windows-1252 with confidence.
    // Let it see these bytes first and every cell comes back NUL-riddled.
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(&bytes, true);
    let wrong = detector.guess(None, chardetng::Utf8Detection::Allow);
    assert_eq!(
        wrong.name(),
        "windows-1252",
        "if this ever stops being true the ordering comment in delimited.rs needs revisiting"
    );
    // `Encoding::decode` would sniff the BOM itself and quietly do the right
    // thing, which is why the naive-path demonstration has to say
    // `decode_without_bom_handling` — that is what a detector-first pipeline
    // actually performs once it believes it already knows the encoding.
    assert!(
        wrong
            .decode_without_bom_handling(&bytes)
            .0
            .contains('\u{0}'),
        "the wrong answer must be visibly wrong, or this test proves nothing"
    );

    let table = delimited::parse(&bytes, SourceFormat::Csv).expect("should convert");
    assert_eq!(header(&table), ["Date", "Description", "Amount"]);
    assert_eq!(rows(&table)[0][1], "CAF\u{c9} R\u{c9}PUBLIQUE");
    assert_eq!(rows(&table)[1][1], "B\u{dc}CHER M\u{dc}NCHEN");
    assert!(
        table
            .rows
            .iter()
            .flatten()
            .all(|cell| !cell.contains('\u{0}')),
        "a NUL anywhere means the BOM lost to the detector"
    );
    // A BOM is a declaration, not a guess, so no note is owed.
    assert!(
        !table
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::EncodingGuessed { .. }))
    );
    // The mark itself must not survive into the first header cell.
    assert!(!header(&table)[0].starts_with('\u{feff}'));
}

#[test]
fn valid_utf8_is_a_fact_and_earns_no_note() {
    let table = parse_delimited("delimited/tab.tsv", SourceFormat::Tsv);
    assert!(
        !table
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::EncodingGuessed { .. }))
    );
}

// ---------------------------------------------------------------------------
// Delimited: refusals and bounds
// ---------------------------------------------------------------------------

#[test]
fn an_empty_input_is_refused_by_name() {
    assert_eq!(
        delimited::parse(&[], SourceFormat::Csv),
        Err(ConvertError::Empty)
    );
    assert_eq!(
        spreadsheet::parse(&[], SourceFormat::Xlsx),
        Err(ConvertError::Empty)
    );
    // A file holding nothing but a byte-order mark has no records either.
    assert_eq!(
        delimited::parse(&[0xEF, 0xBB, 0xBF], SourceFormat::Csv),
        Err(ConvertError::Empty)
    );
}

#[test]
fn each_backend_refuses_the_other_backend_s_formats() {
    assert_eq!(
        delimited::parse(b"a,b\n1,2\n", SourceFormat::Xlsx),
        Err(ConvertError::Unsupported {
            ext: "xlsx".to_string()
        })
    );
    assert_eq!(
        spreadsheet::parse(b"a,b\n1,2\n", SourceFormat::Csv),
        Err(ConvertError::Unsupported {
            ext: "csv".to_string()
        })
    );
}

#[test]
fn the_size_limit_is_enforced_in_core_not_only_at_the_boundary() {
    let oversize = vec![b'a'; MAX_INPUT_BYTES + 1];
    let expected = Err(ConvertError::TooLarge {
        limit: MAX_INPUT_BYTES,
    });
    assert_eq!(delimited::parse(&oversize, SourceFormat::Csv), expected);
    assert_eq!(spreadsheet::parse(&oversize, SourceFormat::Xlsx), expected);
}

#[test]
fn the_row_cap_is_reported_rather_than_hidden() {
    let mut text = String::from("Date,Description,Amount\n");
    for at in 0..delimited::MAX_ROWS {
        text.push_str(&format!("2026-01-05,ROW {at},-1.00\n"));
    }
    let table = delimited::parse(text.as_bytes(), SourceFormat::Csv).expect("should convert");

    assert!(table.truncated, "a hit cap must be visible to the user");
    assert_eq!(table.rows.len(), delimited::MAX_ROWS - 1);
}

#[test]
fn no_error_ever_names_a_path_or_quotes_a_cell() {
    let errors = [
        delimited::parse(&[], SourceFormat::Csv).unwrap_err(),
        delimited::parse(b"x", SourceFormat::Ofx).unwrap_err(),
        spreadsheet::parse(b"not a workbook at all", SourceFormat::Xlsx).unwrap_err(),
        spreadsheet::parse(
            &import_fixture("spreadsheet/no-table.xlsx"),
            SourceFormat::Xlsx,
        )
        .unwrap_err(),
    ];
    for error in errors {
        let rendered = error.to_string();
        assert!(
            !rendered.contains('/') && !rendered.contains('\\') && !rendered.contains("fixtures"),
            "an error must not disclose a path: {rendered}"
        );
    }
}

// ---------------------------------------------------------------------------
// Spreadsheets
// ---------------------------------------------------------------------------

#[test]
fn xlsx_dates_come_back_as_dates_and_amounts_come_back_raw() {
    let table = parse_spreadsheet("spreadsheet/simple.xlsx", SourceFormat::Xlsx);

    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    assert_eq!(table.rows.len(), 4);
    // Serial 46027 rendered through `as_datetime`, never through `as_f64` and a
    // division. Getting this wrong moves a transaction to another day.
    assert_eq!(rows(&table)[0][0], "2026-01-05");
    assert_eq!(rows(&table)[3][0], "2026-01-09");
    // The Amount column carries a currency number format in the fixture, and
    // calamine gives no access to it. `-54.2`, never `($54.20)`.
    assert_eq!(rows(&table)[0][2], "-54.2");
    assert_eq!(rows(&table)[2][2], "2500");
    assert!(
        table
            .notes
            .contains(&ConvertNote::DatesFromSerial { count: 4 }),
        "{:?}",
        table.notes
    );
    // One sheet, one candidate: nothing was chosen between.
    assert!(
        !table
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::SheetChosen { .. }))
    );
}

#[test]
fn sheet_selection_walks_past_the_cover_and_says_which_it_took() {
    let table = parse_spreadsheet("spreadsheet/multi-sheet.xlsx", SourceFormat::Xlsx);

    assert_eq!(
        table.notes.first(),
        Some(&ConvertNote::SheetChosen {
            name: "Transactions".to_string(),
            of: 3,
        }),
        "{:?}",
        table.notes
    );
    // The table starts at C4, so two blank rows and two blank columns had to go
    // before it was a table at all.
    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    assert_eq!(rows(&table)[0][1], "GROCERY STORE");
    assert_eq!(table.rows.len(), 4);
}

#[test]
fn the_biff_reader_agrees_with_the_xlsx_reader() {
    let modern = parse_spreadsheet("spreadsheet/simple.xlsx", SourceFormat::Xlsx);
    let legacy = parse_spreadsheet("spreadsheet/legacy.xls", SourceFormat::Xls);

    assert_eq!(legacy.header, modern.header);
    assert_eq!(legacy.rows, modern.rows);
    assert!(
        legacy
            .notes
            .contains(&ConvertNote::DatesFromSerial { count: 4 })
    );
}

#[test]
fn ods_dates_are_iso_text_and_so_are_not_serials() {
    let table = parse_spreadsheet("spreadsheet/sheet.ods", SourceFormat::Ods);

    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    assert_eq!(rows(&table)[0][0], "2026-01-05");
    assert_eq!(rows(&table)[0][2], "-54.2");
    // ODS stores `office:date-value` as ISO 8601 text. There was no serial to
    // decode, so there is nothing to warn the user about.
    assert!(
        !table
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::DatesFromSerial { .. }))
    );
}

#[test]
fn a_workbook_with_no_table_is_its_own_answer() {
    let bytes = import_fixture("spreadsheet/no-table.xlsx");
    // Not `Malformed`: the file is perfectly valid and simply holds no statement.
    assert_eq!(
        spreadsheet::parse(&bytes, SourceFormat::Xlsx),
        Err(ConvertError::NoTable)
    );
}

#[test]
fn a_wrong_reader_refuses_rather_than_reinterprets() {
    let xlsx = import_fixture("spreadsheet/simple.xlsx");
    // An xlsx is a zip and an ods is a zip, so this is the case most likely to
    // half-succeed. It must not.
    assert!(matches!(
        spreadsheet::parse(&xlsx, SourceFormat::Xls),
        Err(ConvertError::Malformed { .. })
    ));
}

// ---------------------------------------------------------------------------
// Spreadsheets: the preamble
// ---------------------------------------------------------------------------

#[test]
fn a_workbook_preamble_is_skipped_and_reported() {
    let table = parse_spreadsheet("spreadsheet/preamble.xlsx", SourceFormat::Xlsx);
    let plain = parse_spreadsheet("spreadsheet/simple.xlsx", SourceFormat::Xlsx);

    // Trimming the blank edges leaves a title, a blank row, a second title and
    // another blank row sitting above the header. Read the first populated row
    // as the header and this comes back as `["All Activity Types", "", "", ""]`
    // over eight rows of mostly-blank junk.
    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    assert_eq!(
        table.rows, plain.rows,
        "the table is simple.xlsx's, unchanged"
    );
    assert!(
        table
            .notes
            .contains(&ConvertNote::PreambleSkipped { lines: 4 }),
        "dropping a row the user can see is a judgement call and is owed a note: {:?}",
        table.notes
    );
}

// ---------------------------------------------------------------------------
// Spreadsheets: the trailer
// ---------------------------------------------------------------------------

#[test]
fn a_workbook_trailer_is_dropped_and_reported() {
    let table = parse_spreadsheet("spreadsheet/trailer.xlsx", SourceFormat::Xlsx);
    let plain = parse_spreadsheet("spreadsheet/simple.xlsx", SourceFormat::Xlsx);

    assert_eq!(header(&table), ["Date", "Description", "Amount", "Balance"]);
    // The same four transactions as `simple.xlsx`, with the last one's Balance
    // cleared — the row the trim has to stop at.
    assert_eq!(table.rows.len(), 4);
    assert_eq!(table.rows[..3], plain.rows[..3]);

    assert!(
        table
            .notes
            .contains(&ConvertNote::TrailerSkipped { lines: 4 }),
        "two blank rows and two disclaimer paragraphs: {:?}",
        table.notes
    );
    assert!(
        table
            .notes
            .contains(&ConvertNote::BlankRowsDropped { count: 1 }),
        "the blank row between the transactions is owed a note too: {:?}",
        table.notes
    );
}

#[test]
fn a_workbook_last_row_with_an_empty_final_column_is_not_trimmed() {
    // The negative case, and the reason the rule is "too narrow to hold a date
    // and an amount" rather than "narrower than the header". This row reaches
    // column three of four, exactly like the disclaimer rows reach column one —
    // and it is a transaction.
    let table = parse_spreadsheet("spreadsheet/trailer.xlsx", SourceFormat::Xlsx);
    let plain = parse_spreadsheet("spreadsheet/simple.xlsx", SourceFormat::Xlsx);

    let last = table.rows.last().expect("the table has rows");
    let expected = plain.rows.last().expect("the table has rows");
    assert_eq!(last[..3], expected[..3], "the transaction itself is intact");
    assert_eq!(last[3], "", "its Balance is what was blank");
}

#[test]
fn a_single_column_sheet_is_refused_rather_than_reshaped() {
    // A title over a one-column list of balances. Every row holds exactly one
    // populated cell, so the tempting rule — "a row with one cell in it is a
    // title" — would discard the sheet a row at a time; the modal-width rule
    // finds no signal in a one-wide table and skips nothing. Either way this is
    // not a statement, and `NoTable` is the answer rather than a reshaped table
    // built out of the list.
    let bytes = import_fixture("spreadsheet/single-column.xlsx");
    assert_eq!(
        spreadsheet::parse(&bytes, SourceFormat::Xlsx),
        Err(ConvertError::NoTable)
    );
}

/// The column labels a real brokerage "All Activity" export puts on row 7.
///
/// Everything these tests assert is a fact about the file's *shape* — these
/// labels, the column count, the presence of a note, how many rows survive.
/// Nothing is asserted about a payee, an amount or an account, so re-scrubbing
/// the fixture for privacy cannot invalidate any of it.
const ACTIVITY_LABELS: [&str; 15] = [
    "Activity Date",
    "Transaction Date",
    "Account",
    "Institution Name",
    "Activity",
    "Check Number",
    "Card Number",
    "Description",
    "Symbol",
    "Cusip",
    "Memo",
    "Tags",
    "Quantity",
    "Price($)",
    "Amount($)",
];

#[test]
fn a_real_brokerage_export_finds_the_header_under_its_title_block() {
    let table = parse_spreadsheet(
        "spreadsheet/real-brokerage-preamble.xlsx",
        SourceFormat::Xlsx,
    );

    assert_eq!(header(&table), ACTIVITY_LABELS);
    // The bug's signature, asserted directly: reading the first populated row as
    // the header gives one label and fourteen empty strings. A header cell that
    // is blank is a header row that is not a header row.
    assert!(
        header(&table).iter().all(|label| !label.trim().is_empty()),
        "every column must be named: {:?}",
        header(&table)
    );
    assert!(
        table
            .rows
            .iter()
            .all(|row| row.len() == ACTIVITY_LABELS.len()),
        "a sheet is a rectangle; every row is as wide as the header"
    );
    // Exact, because the disclaimer block below the transactions is now trimmed
    // as well. 34 is every transaction row and nothing else; leaving the trailer
    // in gives 60, and losing the body would show up as single digits.
    assert_eq!(table.rows.len(), REAL_BROKERAGE_TXNS);

    let skipped = table
        .notes
        .iter()
        .find_map(|note| match note {
            ConvertNote::PreambleSkipped { lines } => Some(*lines),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the skipped title rows are owed a note: {:?}", table.notes));
    assert!(skipped > 0);
}

/// How many transaction rows the real export holds. A property of the file, not
/// of any row in it, so re-scrubbing cannot change it without the scrubber
/// deliberately adding or removing a transaction.
const REAL_BROKERAGE_TXNS: usize = 34;

/// How many rows sit below the last transaction: fourteen entirely blank and
/// twelve holding one paragraph of legal text in column one.
const REAL_BROKERAGE_TRAILER: usize = 26;

#[test]
fn a_real_brokerage_export_drops_the_disclaimer_block_under_its_transactions() {
    // The bug this fixture caught. `hledger` abandons the whole read on the
    // first record it cannot parse, so the blank row immediately after the last
    // transaction cost the user all 34 of them:
    //
    //     could not parse "" as a date using date format "%m/%d/%Y"
    //     record: ,,,,,,,,,,,,,,
    //
    // and the candidate scorer, seeing a hard failure, ranked their perfectly
    // good rules file at zero.
    let table = parse_spreadsheet(
        "spreadsheet/real-brokerage-preamble.xlsx",
        SourceFormat::Xlsx,
    );

    assert_eq!(table.rows.len(), REAL_BROKERAGE_TXNS);
    assert!(
        table.notes.contains(&ConvertNote::TrailerSkipped {
            lines: REAL_BROKERAGE_TRAILER
        }),
        "ignoring the last {REAL_BROKERAGE_TRAILER} rows of someone's file is owed a note: {:?}",
        table.notes
    );

    // Structural, so a re-scrub cannot invalidate it. Every surviving row is a
    // full-width record, and not one of them opens with prose — a disclaimer
    // paragraph left in the table would be a single populated cell in column
    // one, padded out to fifteen.
    assert!(
        table
            .rows
            .iter()
            .all(|row| row.len() == ACTIVITY_LABELS.len()),
        "a sheet is a rectangle; every row is as wide as the header"
    );
    for row in &table.rows {
        let populated = row.iter().filter(|cell| !cell.trim().is_empty()).count();
        assert!(
            populated > 1,
            "a row with one populated cell is a paragraph, not a transaction: {row:?}"
        );
        assert!(
            !row.is_empty() && !row[0].trim().is_empty(),
            "every record's first column is populated: {row:?}"
        );
        // A date is short. Prose is not, and it is what the trailer is made of.
        assert!(
            row[0].trim().chars().count() <= 32,
            "the first cell of a record is a date, not a sentence: {:?}",
            row[0]
        );
    }
    // No row is blank, at either end or in the middle.
    assert!(
        !table
            .rows
            .iter()
            .any(|row| row.iter().all(|cell| cell.trim().is_empty())),
        "a blank row is never a transaction"
    );
}

#[test]
fn a_newline_inside_a_cell_survives_the_whole_pipeline() {
    // The `Description` column of the real export carries a second line —
    // `"…FEE\nTransaction Date : 08/12/2026"`. It has to survive the workbook
    // read, and it has to survive `to_csv`, whose output is what `hledger
    // import` is ultimately pointed at.
    let table = parse_spreadsheet(
        "spreadsheet/real-brokerage-preamble.xlsx",
        SourceFormat::Xlsx,
    );
    let multiline = table
        .rows
        .iter()
        .flatten()
        .filter(|cell| cell.contains('\n'))
        .count();
    assert!(multiline > 0, "the fixture must carry an embedded newline");

    let rendered = delimited::to_csv(&table);
    let back = delimited::parse(rendered.as_bytes(), SourceFormat::Csv).expect("should convert");

    // Unquoted, each of those newlines would end a record early and the table
    // would come back with more rows than it went in with, every one of them
    // ragged.
    assert_eq!(back.header, table.header);
    assert_eq!(back.rows, table.rows);
    assert!(
        !back
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::RaggedRows { .. })),
        "{:?}",
        back.notes
    );
}

// ---------------------------------------------------------------------------
// The whole point, through real hledger
// ---------------------------------------------------------------------------

/// Opts in to running the converted CSV through a real hledger. Default-skipped
/// so `cargo test` stays hermetic, exactly like `LEDGELINE_HLEDGER_MATCH_CHECK`
/// in `matching.rs`. Run by `just hledger-checks`.
const HLEDGER_OPT_IN: &str = "LEDGELINE_HLEDGER_CONVERT_CHECK";

#[test]
fn the_real_export_imports_through_real_hledger() {
    // The end-to-end statement of the bug. Everything above asserts our own
    // shape; this asserts hledger's opinion of it, which is the only opinion
    // that decides whether the user's import works.
    //
    // With the trailer left in, this run exits non-zero with
    //
    //     could not parse "" as a date using date format "%m/%d/%Y"
    //     record: ,,,,,,,,,,,,,,
    //
    // and produces ZERO transactions — not 34 with one skipped. One unparseable
    // record abandons the entire read.
    if std::env::var_os(HLEDGER_OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {HLEDGER_OPT_IN}=1 to run the converted CSV through hledger");
        return;
    }

    let table = parse_spreadsheet(
        "spreadsheet/real-brokerage-preamble.xlsx",
        SourceFormat::Xlsx,
    );
    let dir =
        std::env::temp_dir().join(format!("ledgeline_convert_hledger_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let csv = dir.join("activity.csv");
    std::fs::write(&csv, delimited::to_csv(&table)).expect("write the converted CSV");

    let rules = common::fixtures_dir().join("import/spreadsheet/brokerage-activity.rules");
    let output = std::process::Command::new("hledger")
        .arg("print")
        .arg("-f")
        .arg(&csv)
        .arg("--rules")
        .arg(&rules)
        .args(["-O", "json"])
        .output()
        .unwrap_or_else(|error| panic!("could not run hledger: {error}"));

    assert!(
        output.status.success(),
        "hledger exited {} over the converted CSV:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let entries: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("hledger's output is JSON");
    let count = entries.as_array().map_or(0, Vec::len);
    assert_eq!(
        count, REAL_BROKERAGE_TXNS,
        "every transaction row must become a transaction, and nothing else may"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn converted_and_aligned_imports_what_the_raw_download_does() {
    // The `skip` frame mismatch, end to end, against the only oracle that
    // decides it. `preamble.csv` is a raw download: a title line, an account
    // line, a header, three transactions. `preamble.csv.rules` is the rules file
    // its owner wrote for it, and it says `skip 3` — counted against THAT file.
    //
    // Convert it and the preamble is gone, so the header is line 1. Hand the
    // same rules file the same statement and hledger spends its three skips on
    // the header and the first two transactions, and imports the third one
    // alone. **One of three, exit code 0, nothing on stderr.** On a statement
    // with two rows or fewer the same arithmetic reads zero — which is the shape
    // the bug was found in — but the partial answer asserted here is the worse
    // one to live with: a user who checks the count sees transactions arrive.
    //
    // Three counts are taken rather than one, and the middle one is the reason
    // this test cannot be replaced by an assertion about padding text:
    //
    //   raw       — what the user gets today from their terminal
    //   unaligned — what Ledgeline gave them: the bug, pinned
    //   aligned   — what `align_to_skip` restores
    //
    // Asserting only `aligned == raw` would still pass if the fixture happened
    // to have no preamble for `skip` to disagree about. Asserting the wrong
    // number alone would pass with the alignment deleted. Together they cannot
    // both hold unless the fix is doing its job.
    if std::env::var_os(HLEDGER_OPT_IN).is_none_or(|value| value.is_empty()) {
        eprintln!("skipping: set {HLEDGER_OPT_IN}=1 to run the aligned CSV through hledger");
        return;
    }

    let raw_path = common::fixtures_dir().join("import/delimited/preamble.csv");
    let rules = common::fixtures_dir().join("import/delimited/preamble.csv.rules");
    let table = parse_delimited("delimited/preamble.csv", SourceFormat::Csv);
    let skip = 3;

    let dir = std::env::temp_dir().join(format!("ledgeline_align_hledger_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");

    let unaligned_csv = delimited::to_csv(&table);
    let unaligned = dir.join("unaligned.csv");
    std::fs::write(&unaligned, &unaligned_csv).expect("write the converted CSV");

    let aligned = dir.join("aligned.csv");
    std::fs::write(
        &aligned,
        delimited::align_to_skip(&unaligned_csv, skip).as_bytes(),
    )
    .expect("write the aligned CSV");

    let raw_count = hledger_txns(&raw_path, &rules);
    let unaligned_count = hledger_txns(&unaligned, &rules);
    let aligned_count = hledger_txns(&aligned, &rules);

    assert_eq!(
        raw_count, 3,
        "the fixture's own rules file must read the fixture's own download"
    );
    assert_eq!(
        unaligned_count, 1,
        "the bug: `skip 3` over a header-on-line-1 CSV eats two transactions, and says so nowhere"
    );
    assert_eq!(
        aligned_count, raw_count,
        "a converted statement must import exactly what the download it came from imports"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// How many transactions real hledger reads out of `data` under `rules`.
///
/// `-O json` and a length, never a scrape of the rendered ledger — the same rule
/// `rules::matching` holds to, and for the same reason: a display format is not
/// a data format.
///
/// A non-zero exit is a panic rather than a zero, because the failure this is
/// used to measure produces exit **0** and the two must never be confused.
fn hledger_txns(data: &std::path::Path, rules: &std::path::Path) -> usize {
    let output = std::process::Command::new("hledger")
        .arg("print")
        .arg("-f")
        .arg(data)
        .arg("--rules")
        .arg(rules)
        .args(["-O", "json"])
        .output()
        .unwrap_or_else(|error| panic!("could not run hledger: {error}"));
    assert!(
        output.status.success(),
        "hledger exited {} over {}:\n{}",
        output.status,
        data.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let entries: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("hledger's output is JSON");
    entries.as_array().map_or(0, Vec::len)
}

// ---------------------------------------------------------------------------
// `to_csv`
// ---------------------------------------------------------------------------

#[test]
fn to_csv_quotes_only_what_needs_it() {
    let table = Tabular {
        header: Some(vec!["Date".into(), "Description".into(), "Amount".into()]),
        rows: vec![
            vec!["2026-01-05".into(), "SMITH, JOHN".into(), "-54.20".into()],
            vec!["2026-01-06".into(), "SAID \"HI\"".into(), "-1.00".into()],
            vec!["2026-01-07".into(), "TWO\nLINES".into(), "-2.00".into()],
            vec![
                "2026-01-08".into(),
                "semi;colon|pipe\ttab".into(),
                "-3.00".into(),
            ],
        ],
        ..Tabular::default()
    };

    assert_eq!(
        delimited::to_csv(&table),
        "Date,Description,Amount\n\
         2026-01-05,\"SMITH, JOHN\",-54.20\n\
         2026-01-06,\"SAID \"\"HI\"\"\",-1.00\n\
         2026-01-07,\"TWO\nLINES\",-2.00\n\
         2026-01-08,semi;colon|pipe\ttab,-3.00\n"
    );
}

#[test]
fn to_csv_emits_the_header_first_and_nothing_at_all_for_an_empty_table() {
    assert_eq!(delimited::to_csv(&Tabular::default()), "");
    assert_eq!(
        delimited::to_csv(&Tabular {
            header: Some(vec!["a".into(), "b".into()]),
            ..Tabular::default()
        }),
        "a,b\n"
    );
}

#[test]
fn a_fixture_round_trips_through_to_csv_and_back() {
    let table = parse_delimited("delimited/quoted.csv", SourceFormat::Csv);
    let rendered = delimited::to_csv(&table);
    let back = delimited::parse(rendered.as_bytes(), SourceFormat::Csv).expect("should convert");

    assert_eq!(back.header, table.header);
    assert_eq!(back.rows, table.rows);
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// Cell content that is deliberately hostile to a naive writer: every candidate
/// delimiter, the quote character, and both line-ending bytes.
fn hostile_cell() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just(','),
            Just(';'),
            Just('\t'),
            Just('|'),
            Just('"'),
            Just('\n'),
            Just('\r'),
            Just(' '),
            any::<char>(),
        ],
        0..6,
    )
    .prop_map(|characters| characters.into_iter().collect())
}

/// Records of at least two fields.
///
/// One-field records are excluded for a stated reason rather than an accidental
/// one: RFC 4180 gives a lone empty field and an empty line the same bytes, so a
/// width-1 table is the single shape where the format itself is lossy. `to_csv`
/// quotes that case (see its unit test), but a width-1 table cannot arise from
/// either backend — the delimited reader needs a delimiter to make one and the
/// spreadsheet reader refuses anything under `MIN_TABLE_COLUMNS`.
fn hostile_record() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(hostile_cell(), 2..5)
}

proptest! {
    /// `to_csv` is lossless: whatever is in a cell, the `csv` crate reads back
    /// exactly the cell. This is the obligation the whole import pipeline rests
    /// on, because the text `to_csv` produces is what `hledger import` is
    /// pointed at.
    #[test]
    fn to_csv_round_trips_through_the_csv_crate(
        header in hostile_record(),
        body in proptest::collection::vec(hostile_record(), 0..6),
    ) {
        let table = Tabular {
            header: Some(header.clone()),
            rows: body.clone(),
            ..Tabular::default()
        };

        let rendered = delimited::to_csv(&table);
        let read_back: Vec<Vec<String>> = csv::ReaderBuilder::new()
            .flexible(true)
            .has_headers(false)
            .from_reader(rendered.as_bytes())
            .into_records()
            .map(|record| {
                record
                    .expect("to_csv output must always be readable")
                    .iter()
                    .map(str::to_string)
                    .collect()
            })
            .collect();

        let expected: Vec<Vec<String>> = std::iter::once(header).chain(body).collect();
        prop_assert_eq!(read_back, expected);
    }

    /// No byte string makes the delimited reader panic. It is fed uploads, so
    /// "returns an error" and "does not abort the process" are different claims
    /// and only the second one is about safety.
    #[test]
    fn delimited_parse_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        for format in [SourceFormat::Csv, SourceFormat::Tsv, SourceFormat::Ssv] {
            let _ = delimited::parse(&bytes, format);
        }
    }
}

proptest! {
    // Fewer cases: each one drives four workbook readers over the bytes.
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn spreadsheet_parse_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        for format in [
            SourceFormat::Xls,
            SourceFormat::Xlsx,
            SourceFormat::Xlsb,
            SourceFormat::Ods,
        ] {
            let _ = spreadsheet::parse(&bytes, format);
        }
    }

    /// Random bytes are almost never a plausible workbook, so they exercise the
    /// "reject immediately" path and little else. Truncating a real one reaches
    /// much further in — a valid zip directory pointing at half a worksheet.
    #[test]
    fn a_truncated_workbook_never_panics(cut in 0usize..5089) {
        let bytes = import_fixture("spreadsheet/simple.xlsx");
        let _ = spreadsheet::parse(&bytes[..cut.min(bytes.len())], SourceFormat::Xlsx);
    }
}
