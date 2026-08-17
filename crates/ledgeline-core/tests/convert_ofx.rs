//! OFX 1.x (SGML), OFX 2.x (XML) and QFX → `Tabular`.
//!
//! Every fixture under `fixtures/import/ofx/` is asserted here, and each
//! assertion names the trap its fixture exists to pin — see that directory's
//! `README.md`. Malformed input is exercised from byte literals rather than
//! files, because a broken `.ofx` on disk invites a well-meaning repair that
//! silently deletes the test.

use ledgeline_core::convert::{
    ConvertError, ConvertNote, MAX_INPUT_BYTES, SourceFormat, StatementMeta, Tabular, ofx,
};
use proptest::prelude::*;
use std::path::PathBuf;

/// The column shape every OFX conversion emits. Spelled out rather than
/// imported so that changing it is a deliberate edit in two places.
const HEADER: [&str; 7] = [
    "date", "amount", "name", "memo", "trntype", "fitid", "checknum",
];

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/import/ofx")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{name} is readable: {e}"))
}

fn parsed(name: &str) -> Tabular {
    ofx::parse(&fixture(name)).unwrap_or_else(|e| panic!("{name} parses: {e}"))
}

fn header() -> Option<Vec<String>> {
    Some(HEADER.iter().map(|c| (*c).to_string()).collect())
}

fn row(cells: [&str; 7]) -> Vec<String> {
    cells.iter().map(|c| (*c).to_string()).collect()
}

/// Column index by name, so a test reads as the field it means.
fn cell<'a>(row: &'a [String], column: &str) -> &'a str {
    let index = HEADER
        .iter()
        .position(|c| *c == column)
        .unwrap_or_else(|| panic!("{column} is a column"));
    row.get(index).map_or("", String::as_str)
}

/// A minimal, valid OFX 1.x statement with one transaction carrying `memo`.
/// Used for the entity matrix, where a whole fixture per case would obscure
/// what is being compared.
fn statement_with_memo(memo: &str) -> Vec<u8> {
    format!(
        "<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\n\
         <CURDEF>USD\n\
         <BANKTRANLIST>\n\
         <STMTTRN>\n\
         <TRNTYPE>DEBIT\n\
         <DTPOSTED>20260101\n\
         <TRNAMT>-1.00\n\
         <FITID>X1\n\
         <MEMO>{memo}\n\
         </STMTTRN>\n\
         </BANKTRANLIST>\n\
         </STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>\n"
    )
    .into_bytes()
}

/// [`statement_with_memo`], with every `\x01` replaced by the raw byte `0x92` —
/// a right single quotation mark in Windows-1252 and a lone continuation byte
/// (so, invalid) in UTF-8. Rust string literals cannot carry it directly.
fn statement_with_cp1252_memo(memo: &str) -> Vec<u8> {
    statement_with_memo(memo)
        .into_iter()
        .map(|byte| if byte == 0x01 { 0x92 } else { byte })
        .collect()
}

const EVERY_FIXTURE: [&str; 9] = [
    "bank-v1.ofx",
    "two-accounts.ofx",
    "bank-v2.ofx",
    "creditcard.qfx",
    "citi-creditline.ofx",
    "tz-dates.ofx",
    "entities.ofx",
    "hybrid-xml-header.ofx",
    "balances.ofx",
];

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

#[test]
fn every_statement_fixture_is_rectangular() {
    for name in EVERY_FIXTURE {
        let tabular = parsed(name);
        assert_eq!(tabular.header, header(), "{name} header");
        assert!(!tabular.rows.is_empty(), "{name} has rows");
        assert!(!tabular.truncated, "{name} is not truncated");
        for row in &tabular.rows {
            assert_eq!(row.len(), HEADER.len(), "{name} row width");
        }
    }
}

#[test]
fn every_statement_fixture_dates_are_iso() {
    for name in EVERY_FIXTURE {
        for row in parsed(name).rows {
            let date = cell(&row, "date");
            assert_eq!(date.len(), 10, "{name}: {date} is YYYY-MM-DD");
            assert!(
                date.chars().enumerate().all(|(i, c)| match i {
                    4 | 7 => c == '-',
                    _ => c.is_ascii_digit(),
                }),
                "{name}: {date} is YYYY-MM-DD"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// bank-v1 / bank-v2 — one statement, two dialects
// ---------------------------------------------------------------------------

#[test]
fn bank_v1_sgml_reads_unclosed_leaf_tags() {
    let tabular = parsed("bank-v1.ofx");
    assert_eq!(
        tabular.rows,
        vec![
            row([
                "2026-01-05",
                "-42.17",
                "SAFEWAY #1234",
                "SAFEWAY STORE 1234 PORTLAND OR",
                "DEBIT",
                "2026010500001",
                "",
            ]),
            row([
                "2026-01-12",
                "-1200.00",
                "CHECK 1041",
                "",
                "CHECK",
                "2026011200002",
                "1041",
            ]),
            row([
                "2026-01-15",
                "2500.0",
                "ACME CORP PAYROLL",
                "DIRECT DEPOSIT",
                "CREDIT",
                "2026011500003",
                "",
            ]),
        ]
    );
}

#[test]
fn bank_v1_keeps_the_amount_text_the_bank_wrote() {
    // `2500.0` is what the file says. Parsing to a float and re-rendering it as
    // `2500.00` would be the first step of a class of silent rewrites.
    let tabular = parsed("bank-v1.ofx");
    let credit = tabular.rows.last().expect("three transactions");
    assert_eq!(cell(credit, "amount"), "2500.0");
}

#[test]
fn bank_v1_emits_name_and_memo_as_separate_columns() {
    // `NAME` truncates at 32 characters, which is why the real payee lives in
    // `MEMO`. Collapsing them throws away whichever one the rules need.
    let tabular = parsed("bank-v1.ofx");
    let debit = tabular.rows.first().expect("three transactions");
    assert_eq!(cell(debit, "name"), "SAFEWAY #1234");
    assert_eq!(cell(debit, "memo"), "SAFEWAY STORE 1234 PORTLAND OR");
    assert_ne!(cell(debit, "name"), cell(debit, "memo"));
}

#[test]
fn bank_v1_reports_the_statement_it_volunteered() {
    let tabular = parsed("bank-v1.ofx");
    assert_eq!(
        tabular.statement,
        Some(StatementMeta {
            account_hint: Some("6789".to_string()),
            currency: Some("USD".to_string()),
            ledger_balance: Some("3257.83".to_string()),
            balance_as_of: Some("2026-01-31".to_string()),
        })
    );
}

#[test]
fn sgml_and_xml_dialects_of_one_statement_are_identical() {
    // The load-bearing test for "never branch on the declared version". One is
    // CRLF SGML with unclosed leaves; the other is indented, fully closed XML
    // under an OFX 2.x header. Nothing about the output may differ.
    assert_eq!(parsed("bank-v1.ofx"), parsed("bank-v2.ofx"));
}

#[test]
fn a_clean_parse_says_nothing() {
    assert_eq!(parsed("bank-v1.ofx").notes, Vec::new());
    assert_eq!(parsed("bank-v2.ofx").notes, Vec::new());
}

// ---------------------------------------------------------------------------
// QFX, credit lines, investments
// ---------------------------------------------------------------------------

#[test]
fn qfx_is_ofx_plus_intu_tags() {
    // The `.` in `INTU.BID` is a legal tag character; a scanner that stopped a
    // tag name at it would misread the whole `SONRS`.
    let tabular = parsed("creditcard.qfx");
    assert_eq!(tabular.rows.len(), 3);
    assert_eq!(
        tabular
            .statement
            .as_ref()
            .and_then(|s| s.account_hint.clone()),
        Some("1111".to_string())
    );
    // Card sign convention comes through untouched: purchases negative, the
    // payment positive, and a negative closing balance.
    assert_eq!(cell(&tabular.rows[0], "amount"), "-64.50");
    assert_eq!(cell(&tabular.rows[1], "amount"), "500.00");
    assert_eq!(
        tabular.statement.and_then(|s| s.ledger_balance),
        Some("-1284.62".to_string())
    );
}

#[test]
fn qfx_content_is_detected_regardless_of_extension() {
    assert!(ofx::looks_like_ofx(&fixture("creditcard.qfx")));
}

#[test]
fn a_credit_line_shipped_as_a_bank_statement_still_parses() {
    // Citi delivers a credit card as BANKMSGSRSV1/STMTRS with
    // ACCTTYPE=CREDITLINE. Routing on message set returns nothing here.
    let tabular = parsed("citi-creditline.ofx");
    assert_eq!(tabular.rows.len(), 2);
    assert_eq!(cell(&tabular.rows[0], "name"), "COSTCO WHSE #0123");
    assert_eq!(
        tabular.statement.and_then(|s| s.ledger_balance),
        Some("-879.14".to_string())
    );
}

#[test]
fn an_investment_statement_is_refused_by_name() {
    assert_eq!(
        ofx::parse(&fixture("investment.ofx")),
        Err(ConvertError::InvestmentStatement)
    );
}

#[test]
fn an_xml_header_over_an_sgml_body_is_read_as_sgml() {
    // Legal nowhere, shipped by real banks. The header only ever chooses the
    // decoder.
    let tabular = parsed("hybrid-xml-header.ofx");
    assert_eq!(tabular.rows.len(), 2);
    assert_eq!(cell(&tabular.rows[1], "amount"), "1450.00");
    assert_eq!(
        tabular.statement.and_then(|s| s.currency),
        Some("AUD".to_string())
    );
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

#[test]
fn dates_keep_the_statements_own_calendar_day() {
    let dates: Vec<String> = parsed("tz-dates.ofx")
        .rows
        .iter()
        .map(|row| cell(row, "date").to_string())
        .collect();
    assert_eq!(
        dates,
        vec![
            // 8, 10, 12 and 14 digits.
            "2026-01-01",
            "2026-01-02",
            "2026-01-03",
            "2026-01-04",
            // Local midnight under a zone whose name is wrong for the season.
            // Normalising to UTC lands this on the 4th.
            "2026-01-05",
            // Fractional offset.
            "2026-01-06",
            // Unsigned zero offset.
            "2026-01-07",
        ]
    );
}

// ---------------------------------------------------------------------------
// Entities — the property that disqualified every candidate crate
// ---------------------------------------------------------------------------

#[test]
fn entity_matrix() {
    // One pass, whitespace preserved, unknown and raw `&` left alone.
    let cases: [(&str, &str); 16] = [
        // The headline regression: `ofx-rs` returns `ATT` for this.
        ("AT &amp; T", "AT & T"),
        ("AT&amp;T", "AT&T"),
        ("R &amp;amp; D", "R &amp; D"),
        // Numeric references, decimal and hex, either case of the `x`.
        ("caf&#233;", "café"),
        ("caf&#xE9;", "café"),
        ("caf&#XE9;", "café"),
        ("&#38;", "&"),
        // A bank that escaped its own escapes. One pass is the whole point:
        // the user asked for the literal text, not a quotation mark.
        ("SAY &amp;quot;HI&amp;quot;", "SAY &quot;HI&quot;"),
        ("&#x26;#x26;", "&#x26;"),
        // Raw, unescaped `&` — routine in real memos, and a hard error in
        // `ofx-rs`. It must survive verbatim, including when a stray `;`
        // downstream makes it look like an entity.
        ("P&G HOME PRODUCTS", "P&G HOME PRODUCTS"),
        ("TICKETS A&B; SERVICE", "TICKETS A&B; SERVICE"),
        ("50% OFF ROOMS & SUITES", "50% OFF ROOMS & SUITES"),
        ("&amp", "&amp"),
        // Undeclared in OFX; it is not HTML.
        ("NON&nbsp;ENTITY", "NON&nbsp;ENTITY"),
        // Not a character.
        ("&#999999999;", "&#999999999;"),
        // The rest of the predefined set.
        (
            "A &lt;B&gt; C &apos;q&apos; &quot;d&quot;",
            "A <B> C 'q' \"d\"",
        ),
    ];
    for (input, expected) in cases {
        let tabular = ofx::parse(&statement_with_memo(input))
            .unwrap_or_else(|e| panic!("{input:?} parses: {e}"));
        let memo = tabular
            .rows
            .first()
            .map(|row| cell(row, "memo").to_string())
            .unwrap_or_default();
        assert_eq!(memo, expected, "input {input:?}");
    }
}

#[test]
fn the_entities_fixture_decodes_without_losing_whitespace() {
    let tabular = parsed("entities.ofx");
    let names: Vec<&str> = tabular.rows.iter().map(|r| cell(r, "name")).collect();
    let memos: Vec<&str> = tabular.rows.iter().map(|r| cell(r, "memo")).collect();
    assert_eq!(
        names,
        vec![
            "AT & T",
            "café du monde",
            "SAY &quot;HI&quot;",
            "P&G HOME PRODUCTS",
            "A <B> C",
            "TICKETS A&B; SERVICE",
        ]
    );
    assert_eq!(
        memos,
        vec![
            "Payment to AT & T Mobility",
            "café noir, beignets",
            "double-escaped by the bank, decoded once",
            "50% OFF ROOMS & SUITES",
            "'single' and \"double\"",
            "NON&nbsp;ENTITY STAYS PUT",
        ]
    );
}

// ---------------------------------------------------------------------------
// Arithmetic validation
// ---------------------------------------------------------------------------

#[test]
fn an_opening_and_closing_pair_that_reconciles_is_silent() {
    let tabular = parsed("balances.ofx");
    assert_eq!(tabular.notes, Vec::new());
    assert_eq!(
        tabular.statement.and_then(|s| s.ledger_balance),
        Some("2257.83".to_string())
    );
}

#[test]
fn an_opening_and_closing_pair_that_does_not_reconcile_is_noted() {
    // Edited in memory rather than committed as a fixture: a file that is wrong
    // on disk gets "fixed" by the next reader.
    let text = String::from_utf8(fixture("balances.ofx")).expect("fixture is UTF-8");
    let broken = text.replace("<BALAMT>2257.83", "<BALAMT>2257.84");
    let tabular = ofx::parse(broken.as_bytes()).expect("still parses");
    assert_eq!(
        tabular.notes,
        vec![ConvertNote::BalanceMismatch {
            expected: "2257.84".to_string(),
            computed: "2257.83".to_string(),
        }]
    );
    // The mismatch is a note, never an error: the rows are still the best
    // available reading of the file.
    assert_eq!(tabular.rows.len(), 3);
}

#[test]
fn a_percentage_in_the_balance_list_is_not_an_opening_balance() {
    // `balances.ofx` carries a BALTYPE=PERCENT interest rate beside the opening
    // balance. Reading it as the opening would report a false mismatch.
    assert_eq!(parsed("balances.ofx").notes, Vec::new());
}

#[test]
fn a_lone_closing_balance_makes_no_arithmetic_claim() {
    // Every other fixture has a LEDGERBAL and no opening balance. There is
    // nothing to add it to, so it is recorded and left alone.
    //
    // `two-accounts.ofx` is excluded from the note assertion rather than from
    // the balance one: it carries a `StatementChosen`, which is the whole reason
    // it exists, and its first statement's balance is still recorded normally.
    for name in EVERY_FIXTURE
        .iter()
        .filter(|n| !["balances.ofx", "two-accounts.ofx"].contains(n))
    {
        let tabular = parsed(name);
        assert!(
            tabular
                .statement
                .as_ref()
                .is_some_and(|s| s.ledger_balance.is_some()),
            "{name} has a closing balance"
        );
        assert_eq!(tabular.notes, Vec::new(), "{name} makes no claim");
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn detection_is_on_content_not_extension() {
    for name in EVERY_FIXTURE {
        assert!(ofx::looks_like_ofx(&fixture(name)), "{name} looks like OFX");
    }
    assert!(ofx::looks_like_ofx(&fixture("investment.ofx")));

    for (label, bytes) in [
        ("empty", &b""[..]),
        ("csv", &b"date,amount,payee\n2026-01-01,-5.00,Store\n"[..]),
        ("zip", &b"PK\x03\x04\x14\x00\x00\x00\x08\x00"[..]),
        ("journal", &b"2026-01-01 Opening\n  assets:bank  $10\n"[..]),
        ("prose", "an ofx statement is not this file".as_bytes()),
    ] {
        assert!(!ofx::looks_like_ofx(bytes), "{label} is not OFX");
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

#[test]
fn a_utf16_statement_reads_the_same_as_its_utf8_twin() {
    // BOM sniffing has to come first: `chardetng` cannot detect UTF-16 at all
    // and answers windows-1252 for exactly these bytes.
    let text = String::from_utf8(fixture("bank-v2.ofx")).expect("fixture is UTF-8");
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    assert_eq!(ofx::parse(&utf16), Ok(parsed("bank-v2.ofx")));
}

#[test]
fn an_undeclared_high_byte_encoding_is_guessed_and_said_so() {
    // No ENCODING/CHARSET header and not valid UTF-8, so the decoder has to
    // choose — and has to admit that it chose.
    let tabular = ofx::parse(&statement_with_cp1252_memo("MOE\x01S TAVERN")).expect("parses");
    assert_eq!(
        tabular.notes,
        vec![ConvertNote::EncodingGuessed {
            label: "windows-1252".to_string()
        }]
    );
    assert_eq!(
        tabular.rows.first().map(|r| cell(r, "memo")),
        Some("MOE\u{2019}S TAVERN")
    );
}

#[test]
fn a_declared_windows_1252_charset_is_believed() {
    let declared = [
        &b"OFXHEADER:100\nDATA:OFXSGML\nENCODING:USASCII\nCHARSET:1252\n\n"[..],
        &statement_with_cp1252_memo("MOE\x01S TAVERN")[..],
    ]
    .concat();
    let tabular = ofx::parse(&declared).expect("parses");
    assert_eq!(tabular.notes, Vec::new(), "declared, so not a guess");
    assert_eq!(
        tabular.rows.first().map(|r| cell(r, "memo")),
        Some("MOE\u{2019}S TAVERN")
    );
}

// ---------------------------------------------------------------------------
// Tolerance and refusal
// ---------------------------------------------------------------------------

#[test]
fn an_empty_memo_does_not_swallow_the_amount() {
    // `<MEMO>` with no value opens exactly like an aggregate. Only the arrival
    // of `</STMTTRN>` tells them apart, and until it does, TRNAMT looks like a
    // child of MEMO.
    let bytes = statement_with_memo("");
    let tabular = ofx::parse(&bytes).expect("parses");
    let first = tabular.rows.first().expect("one transaction");
    assert_eq!(cell(first, "memo"), "");
    assert_eq!(cell(first, "amount"), "-1.00");
    assert_eq!(cell(first, "date"), "2026-01-01");
}

#[test]
fn a_truncated_statement_still_yields_its_transactions() {
    let text = String::from_utf8(fixture("bank-v1.ofx")).expect("fixture is UTF-8");
    let cut = text
        .find("</BANKTRANLIST>")
        .expect("the fixture has a transaction list");
    let tabular = ofx::parse(&text.as_bytes()[..cut]).expect("parses");
    assert_eq!(tabular.rows.len(), 3);
    // The closing balance came after the cut, so nothing is claimed about it.
    assert_eq!(
        tabular.statement.and_then(|s| s.ledger_balance),
        None,
        "no balance was read"
    );
}

#[test]
fn empty_input_is_refused_as_empty() {
    assert_eq!(ofx::parse(b""), Err(ConvertError::Empty));
    assert_eq!(ofx::parse(b"   \r\n\t "), Err(ConvertError::Empty));
}

#[test]
fn oversize_input_is_refused_before_it_is_decoded() {
    let huge = vec![b'<'; MAX_INPUT_BYTES + 1];
    assert_eq!(
        ofx::parse(&huge),
        Err(ConvertError::TooLarge {
            limit: MAX_INPUT_BYTES
        })
    );
}

#[test]
fn non_ofx_input_is_malformed_not_a_panic() {
    let error = ofx::parse(b"date,amount\n2026-01-01,-5.00\n").expect_err("not OFX");
    assert_eq!(
        error,
        ConvertError::Malformed {
            format: SourceFormat::Ofx,
            detail: "the file does not contain an OFX document".to_string(),
        }
    );
}

#[test]
fn an_ofx_document_without_a_statement_says_so() {
    let error =
        ofx::parse(b"OFXHEADER:100\n\n<OFX>\n<SIGNONMSGSRSV1>\n</SIGNONMSGSRSV1>\n</OFX>\n")
            .expect_err("no statement");
    assert_eq!(
        error,
        ConvertError::Malformed {
            format: SourceFormat::Ofx,
            detail: "no bank or credit card statement was found".to_string(),
        }
    );
}

#[test]
fn no_error_discloses_a_path_or_user_data() {
    // The rule `docs/imports.md` § Security already holds the rules API to: an
    // error quotes neither a path nor the caller's content.
    let secret = "/Users/someone/private/statements/acct-4111111111111111.ofx";
    let inputs: Vec<Vec<u8>> = vec![
        secret.as_bytes().to_vec(),
        format!("<OFX><MEMO>{secret}</MEMO></OFX>").into_bytes(),
        fixture("investment.ofx"),
        vec![b'<'; MAX_INPUT_BYTES + 1],
        Vec::new(),
    ];
    for bytes in inputs {
        if let Err(error) = ofx::parse(&bytes) {
            let message = error.to_string();
            assert!(!message.contains('/'), "leaked a path: {message}");
            assert!(!message.contains("4111"), "leaked user data: {message}");
        }
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// Fragments chosen to hit the scanner's corners far more often than random
/// bytes would: unbalanced close tags, bare `<`, half-written entities.
fn markup_fragment() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "<OFX>",
        "</OFX>",
        "<STMTRS>",
        "</STMTRS>",
        "<STMTTRN>",
        "</STMTTRN>",
        "<BANKTRANLIST>",
        "<TRNAMT>",
        "<DTPOSTED>",
        "<MEMO>",
        "</MEMO>",
        "<!-- c -->",
        "<?xml?>",
        "<INTU.BID>",
        "20260101",
        "-1.00",
        "&amp;",
        "&#233;",
        "&",
        "<",
        ">",
        "/",
        "\n",
        " ",
        "\u{00e9}",
    ])
    .prop_map(str::to_string)
}

proptest! {
    /// The only hard guarantee: arbitrary bytes never panic. The parser reads
    /// untrusted uploads, so a panic is a denial of service.
    #[test]
    fn parse_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = ofx::parse(&bytes);
        let _ = ofx::looks_like_ofx(&bytes);
    }

    #[test]
    fn parse_never_panics_on_arbitrary_markup(
        fragments in prop::collection::vec(markup_fragment(), 0..200),
    ) {
        let text = fragments.concat();
        let _ = ofx::parse(text.as_bytes());
    }

    /// Whatever comes back is usable: the row width is the header width, every
    /// time, so the preview and the CSV writer never meet a ragged row.
    #[test]
    fn any_successful_parse_is_rectangular(
        fragments in prop::collection::vec(markup_fragment(), 0..200),
    ) {
        let text = fragments.concat();
        if let Ok(tabular) = ofx::parse(text.as_bytes()) {
            let width = tabular.header.as_ref().map_or(0, Vec::len);
            prop_assert_eq!(width, HEADER.len());
            for row in &tabular.rows {
                prop_assert_eq!(row.len(), width);
            }
        }
    }

    /// Text with no markup and no `&` survives a round trip through a MEMO
    /// byte for byte, modulo the leading/trailing whitespace OFX cannot
    /// represent. This is the anti-`ofx-rs` property: nothing is dropped.
    #[test]
    fn plain_memo_text_survives(memo in "[a-zA-Z0-9 ,.'#*()/-]{1,60}") {
        let expected = memo.trim();
        prop_assume!(!expected.is_empty());
        let tabular = ofx::parse(&statement_with_memo(&memo)).map_err(|e| e.to_string());
        let got = tabular
            .as_ref()
            .ok()
            .and_then(|t| t.rows.first())
            .map(|row| cell(row, "memo"))
            .unwrap_or_default();
        prop_assert_eq!(got, expected);
    }
}

// ---------------------------------------------------------------------------
// The real statements
// ---------------------------------------------------------------------------
//
// `real-creditline-v102.{ofx,qfx}` are one genuine credit-line statement as
// delivered by a real institution, in both dialects, with payees, amounts and
// institution identifiers replaced (see the directory README). Every assertion
// below is STRUCTURAL — nothing depends on a payee string, an amount or an
// account number — because the scrub that made them publishable is allowed to
// happen again without invalidating a single test.
//
// They earn their place by pinning things no hand-written fixture would have
// thought to: a credit card delivered as `BANKMSGSRSV1/STMTRS` with
// `ACCTTYPE=CREDITLINE`, and a `NAME` one character under the 32-char limit.

#[test]
fn a_real_credit_line_is_read_from_the_bank_message_set() {
    // The trap: this is a CREDIT CARD, and the issuer ships it as STMTRS inside
    // BANKMSGSRSV1 rather than as CCSTMTRS. Routing on the message set — the
    // obvious implementation — finds a bank account or nothing at all.
    let table = parsed("real-creditline-v102.ofx");
    assert_eq!(table.header, header());
    assert_eq!(table.rows.len(), 26);
}

#[test]
fn the_two_dialects_of_one_statement_agree_on_every_transaction() {
    // Same statement, delivered as OFX 1.x SGML and as QFX. The dialects differ
    // in their signon block, not their data, so dates and amounts must match
    // row for row. This is the strongest single check on the scanner: the two
    // files exercise different tag-closing shapes and must still agree.
    let ofx = parsed("real-creditline-v102.ofx");
    let qfx = parsed("real-creditline-v102.qfx");
    assert_eq!(ofx.rows.len(), qfx.rows.len());
    for (a, b) in ofx.rows.iter().zip(qfx.rows.iter()) {
        assert_eq!(cell(a, "date"), cell(b, "date"));
        assert_eq!(cell(a, "amount"), cell(b, "amount"));
        assert_eq!(cell(a, "trntype"), cell(b, "trntype"));
    }
}

#[test]
fn a_real_statement_yields_its_closing_balance_and_a_masked_account() {
    // The balance is what pre-fills the assertion field in the UI, so its
    // presence is the feature. The account hint must be a masked fragment: the
    // last four characters and nothing more ever leaves this crate.
    for name in ["real-creditline-v102.ofx", "real-creditline-v102.qfx"] {
        let table = parsed(name);
        let meta = table
            .statement
            .unwrap_or_else(|| panic!("{name} carries statement metadata"));
        assert!(meta.ledger_balance.is_some(), "{name} has a ledger balance");
        assert_eq!(meta.currency.as_deref(), Some("USD"), "{name}");
        let hint = meta
            .account_hint
            .unwrap_or_else(|| panic!("{name} carries an account hint"));
        assert_eq!(hint.chars().count(), 4, "{name} is masked to four chars");
    }
}

#[test]
fn real_dates_are_the_local_calendar_day_of_a_fourteen_digit_timestamp() {
    // Every DTPOSTED in these files is the 14-digit form. Reading them as
    // instants and normalising to UTC moves a transaction across midnight.
    let table = parsed("real-creditline-v102.ofx");
    for row in &table.rows {
        let date = cell(row, "date");
        assert_eq!(date.len(), 10, "{date} is YYYY-MM-DD");
        assert!(date.starts_with("2026-"), "{date}");
    }
}

#[test]
fn a_real_payee_at_the_truncation_boundary_survives_whole() {
    // Banks truncate NAME at exactly 32 characters. The longest payee in this
    // statement is 31 — one under the limit — which is precisely where an
    // off-by-one in the value scanner would show up and nowhere else.
    let table = parsed("real-creditline-v102.ofx");
    let longest = table
        .rows
        .iter()
        .map(|row| cell(row, "name").chars().count())
        .max()
        .unwrap_or_default();
    assert_eq!(longest, 31, "the boundary payee is read whole");
    assert!(
        table.rows.iter().all(|row| !cell(row, "name").is_empty()),
        "no real payee reads as empty"
    );
}

#[test]
fn the_real_statements_are_detected_as_the_dialect_they_are() {
    use ledgeline_core::convert::detect;
    // Dispatch is on CONTENT, with the name only breaking the OFX/QFX tie.
    assert_eq!(
        detect(
            "real-creditline-v102.ofx",
            &fixture("real-creditline-v102.ofx")
        ),
        Ok(SourceFormat::Ofx)
    );
    assert_eq!(
        detect(
            "real-creditline-v102.qfx",
            &fixture("real-creditline-v102.qfx")
        ),
        Ok(SourceFormat::Qfx)
    );
}

#[test]
fn quickbooks_web_connect_is_read_end_to_end_as_its_own_dialect() {
    use ledgeline_core::convert::{convert, detect};
    // A `.qbo` is Web Connect: the same OFX 1.x SGML a `.qfx` carries, under a
    // third extension. Asserted through the WHOLE path -- detect, then
    // dispatch -- because the two used to disagree in a way no test could see.
    // `.qbo` was folded into `SourceFormat::Qfx`, so it parsed perfectly while
    // being impossible to publish in `/api/import/capabilities`, and the SPA
    // refused the file before the engine ever saw it.
    let bytes = fixture("real-creditline-v102.qfx");
    assert_eq!(detect("webconnect.qbo", &bytes), Ok(SourceFormat::Qbo));

    let qbo = convert(SourceFormat::Qbo, &bytes).expect("a .qbo converts");
    assert_eq!(
        qbo,
        parsed("real-creditline-v102.qfx"),
        "the dialect label changes nothing about what is read"
    );
    assert!(!qbo.rows.is_empty(), "the fixture carries transactions");
}

#[test]
fn no_real_fixture_still_carries_an_institution_name() {
    // A guard on the scrub itself, not on the parser. If someone refreshes
    // these fixtures from a new download and forgets to anonymise, this fails
    // before the file reaches a public commit.
    for name in ["real-creditline-v102.ofx", "real-creditline-v102.qfx"] {
        let text = String::from_utf8_lossy(&fixture(name)).to_uppercase();
        for leaked in ["CITIBANK", "CHASE", "WELLS FARGO", "AMEX", "CAPITAL ONE"] {
            assert!(!text.contains(leaked), "{name} still names {leaked}");
        }
    }
}

// ---------------------------------------------------------------------------
// More than one statement, and a raw `<` in a value
// ---------------------------------------------------------------------------

#[test]
fn a_download_of_several_accounts_reads_the_first_and_says_so() {
    // "Download all my accounts" is one file per bank, not one per account. Only
    // the first statement is imported -- each statement's transactions belong to
    // its own account and a rules file names one `account1` for the whole
    // import, so merging them would post the savings rows to checking -- but the
    // rest must not vanish in silence.
    //
    // The arithmetic safety net cannot catch this on its own: the first
    // statement reconciles perfectly against its OWN closing balance, so the
    // file looks entirely healthy while two thirds of it is missing.
    let table = parsed("two-accounts.ofx");

    assert_eq!(table.rows.len(), 1, "{:?}", table.rows);
    assert_eq!(cell(&table.rows[0], "name"), "GROCERY STORE");
    assert!(
        table
            .notes
            .contains(&ConvertNote::StatementChosen { of: 2 }),
        "the other statement must be reported, not discarded: {:?}",
        table.notes
    );
    // The one that WAS read is the one the metadata describes.
    let meta = table.statement.expect("statement metadata");
    assert_eq!(meta.account_hint.as_deref(), Some("1111"));
}

#[test]
fn one_statement_earns_no_statement_note() {
    // The negative case: a note on every ordinary download would be noise, and
    // noise is how a real warning gets ignored.
    let table = parsed("bank-v1.ofx");
    assert!(
        !table
            .notes
            .iter()
            .any(|note| matches!(note, ConvertNote::StatementChosen { .. })),
        "{:?}",
        table.notes
    );
}

#[test]
fn a_raw_less_than_in_a_value_does_not_delete_the_transaction() {
    // Banks write `A < B` rather than the `A &lt; B` the spec asks for. Read as
    // a tag, the stray `<` swallows the enclosing `</STMTTRN>`, the next close
    // demotes STMTTRN to an empty leaf, and the WHOLE TRANSACTION disappears --
    // from a file that still parses, still reports no note, and still balances
    // if the amounts happen to work out. Three in, three out.
    let ofx = "OFXHEADER:100\nDATA:OFXSGML\n\n\
        <OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\n\
        <CURDEF>USD\n\
        <BANKTRANLIST>\n\
        <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260105<TRNAMT>-10.00<NAME>ALPHA</STMTTRN>\n\
        <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260106<TRNAMT>-20.00<NAME>BRAVO<MEMO>A < B REPAIRS</STMTTRN>\n\
        <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260107<TRNAMT>-30.00<NAME>CHARLIE</STMTTRN>\n\
        </BANKTRANLIST>\n\
        </STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>\n";
    let table = ofx::parse(ofx.as_bytes()).expect("parses");

    assert_eq!(table.rows.len(), 3, "{:?}", table.rows);
    assert_eq!(cell(&table.rows[1], "name"), "BRAVO");
    assert_eq!(cell(&table.rows[1], "memo"), "A < B REPAIRS");
    assert_eq!(cell(&table.rows[2], "name"), "CHARLIE");
}

#[test]
fn a_raw_less_than_does_not_mangle_the_payee_either() {
    // The other half. In NAME rather than MEMO the row survives but the payee is
    // truncated at the `<` and everything after it is lost -- silent payee
    // mangling, which is the `ofx-rs` failure this hand-rolled parser exists to
    // avoid rather than reproduce.
    let ofx = "OFXHEADER:100\nDATA:OFXSGML\n\n\
        <OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\n\
        <CURDEF>USD\n\
        <BANKTRANLIST>\n\
        <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260105<TRNAMT>-10.00<NAME>A < B REPAIRS<MEMO>CARD 1234</STMTTRN>\n\
        </BANKTRANLIST>\n\
        </STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>\n";
    let table = ofx::parse(ofx.as_bytes()).expect("parses");

    assert_eq!(table.rows.len(), 1, "{:?}", table.rows);
    assert_eq!(cell(&table.rows[0], "name"), "A < B REPAIRS");
    assert_eq!(cell(&table.rows[0], "memo"), "CARD 1234");
}

#[test]
fn a_properly_escaped_less_than_still_reads_as_one() {
    // The spec-compliant spelling must be unaffected by the tolerance above.
    let ofx = "OFXHEADER:100\nDATA:OFXSGML\n\n\
        <OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\n\
        <CURDEF>USD\n\
        <BANKTRANLIST>\n\
        <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260105<TRNAMT>-10.00<NAME>A &lt; B REPAIRS</STMTTRN>\n\
        </BANKTRANLIST>\n\
        </STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>\n";
    let table = ofx::parse(ofx.as_bytes()).expect("parses");

    assert_eq!(cell(&table.rows[0], "name"), "A < B REPAIRS");
}
