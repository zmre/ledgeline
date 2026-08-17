//! `convert` — normalize an imported statement file to one tabular extract.
//!
//! Every input format the New Transactions tab accepts collapses to [`Tabular`]
//! before anything downstream sees it, so rules matching, preview and CSV
//! emission each have exactly one shape to handle. See `plans/11-enhanced-import.md`.
//!
//! The module boundary is deliberately narrow: [`detect`] decides *what* a byte
//! slice is, [`convert`] turns it into a [`Tabular`], and [`to_csv`] renders
//! that back out — with [`align_to_skip`] as the one adjustment made to the copy
//! hledger reads, because stripping a preamble moves the header out from under
//! the `skip` the user's rules file already says. Nothing here touches the
//! filesystem, spawns a process, or
//! knows a path — matching the no-disclosure rule the rules API already holds
//! to (`docs/imports.md` § Security). Callers hand us bytes and a bare file
//! name; errors quote neither.
//!
//! Submodules land per lane:
//! - `delimited` — CSV/TSV/SSV, encoding detection and delimiter sniffing
//! - `ofx`       — OFX 1.x (SGML) and 2.x (XML), plus QFX
//! - `spreadsheet` — xls/xlsx/xlsm/xlsb/ods via `calamine`

pub mod delimited;
mod encoding;
pub mod ofx;
pub mod spreadsheet;

use thiserror::Error;

/// Upper bound on an accepted input, mirroring the server's upload cap. Applied
/// inside `convert` too, so a caller that skips the HTTP layer cannot blow past it.
pub const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

/// A normalised tabular extract. Never contains a path.
///
/// `header` is `None` only when a delimited file genuinely had no header row to
/// find — which is not the same as an empty header row, and rules matching
/// treats the two differently when scoring a `skip` value. Structured formats
/// (OFX, spreadsheets) always supply `Some`, because their field names come
/// from the format itself rather than from a guess about the first row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tabular {
    pub header: Option<Vec<String>>,
    pub rows: Vec<Vec<String>>,
    /// Set when a row cap was hit; the UI says so rather than implying the file was short.
    pub truncated: bool,
    /// Statement metadata a format volunteered. OFX gives a closing balance for
    /// free, which pre-fills the balance-assertion field so the user does not
    /// retype their statement.
    pub statement: Option<StatementMeta>,
    pub notes: Vec<ConvertNote>,
}

/// What a format told us about the statement as a whole, as opposed to its rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatementMeta {
    /// Masked to the last four characters before it leaves this crate.
    pub account_hint: Option<String>,
    pub currency: Option<String>,
    /// Verbatim decimal text — never parsed to a float, never re-rendered.
    pub ledger_balance: Option<String>,
    /// Normalised to `YYYY-MM-DD` in the statement's own local calendar. See
    /// the plan's OFX notes: converting to UTC moves transactions across days.
    pub balance_as_of: Option<String>,
}

/// Something the conversion decided that the user should know about, because it
/// was a judgement call rather than a fact. Rendered verbatim under the preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertNote {
    /// A workbook had more than one candidate sheet and we picked this one.
    SheetChosen { name: String, of: usize },
    /// Cells were stored as spreadsheet date serials and were rendered as dates.
    DatesFromSerial { count: usize },
    /// The text encoding was guessed rather than declared.
    EncodingGuessed { label: String },
    /// The delimiter was sniffed rather than declared.
    DelimiterSniffed { delimiter: char },
    /// Leading non-tabular lines were skipped to reach the header.
    PreambleSkipped { lines: usize },
    /// Trailing non-tabular lines below the last record were dropped — the
    /// disclaimer block a bank or brokerage puts under the transactions.
    ///
    /// As loud as [`Self::PreambleSkipped`] on purpose. "We ignored the last 26
    /// rows of your file" is exactly the kind of silent helpfulness that loses
    /// data, and the rows are ones the user can see in their own spreadsheet.
    TrailerSkipped { lines: usize },
    /// Rows holding nothing at all were dropped from the body.
    ///
    /// Distinct from [`Self::TrailerSkipped`] because it says something
    /// different: a blank row *inside* the transactions is usually a section
    /// break, and knowing one was there can explain a row count that looks
    /// short.
    BlankRowsDropped { count: usize },
    /// Rows did not all have the same field count.
    RaggedRows { count: usize },
    /// A running-balance or opening/closing check did not add up. This is the
    /// loud-failure signal from the plan: a silent misparse becomes visible here.
    BalanceMismatch { expected: String, computed: String },
}

/// The formats the New Transactions tab accepts. `Pdf` is present precisely so
/// it can be refused by name — see [`ConvertError::PdfNotSupported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Csv,
    Tsv,
    Ssv,
    Ofx,
    Qfx,
    Qbo,
    Xls,
    Xlsx,
    Xlsm,
    Xlsb,
    Ods,
}

impl SourceFormat {
    /// Every format, in the order the New Transactions tab lists them.
    ///
    /// `/api/import/capabilities` publishes this, and the SPA refuses any
    /// extension it does not contain, so a variant missing here is a format the
    /// engine reads but the file picker will not offer. Derived from the enum
    /// rather than hand-listed at the call site for that reason.
    pub const ALL: [Self; 11] = [
        Self::Csv,
        Self::Tsv,
        Self::Ssv,
        Self::Ofx,
        Self::Qfx,
        Self::Qbo,
        Self::Xls,
        Self::Xlsx,
        Self::Xlsm,
        Self::Xlsb,
        Self::Ods,
    ];

    /// Lowercase display name, used in messages and in the wire types.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Ssv => "ssv",
            Self::Ofx => "ofx",
            Self::Qfx => "qfx",
            Self::Qbo => "qbo",
            Self::Xls => "xls",
            Self::Xlsx => "xlsx",
            Self::Xlsm => "xlsm",
            Self::Xlsb => "xlsb",
            Self::Ods => "ods",
        }
    }

    /// Whether this format is one of the delimited-text family.
    #[must_use]
    pub fn is_delimited(self) -> bool {
        matches!(self, Self::Csv | Self::Tsv | Self::Ssv)
    }

    /// Whether this format is read by the OFX backend.
    ///
    /// All three dialects parse identically — QFX adds Quicken's `INTU.BID` to
    /// `SONRS` and QuickBooks Web Connect (`.qbo`) is the same file under a
    /// third name — so the variants differ only in what we call them. They must
    /// share a predicate rather than a hand-written `|` chain: [`convert`]
    /// dispatches with a catch-all arm, so a dialect missing from that arm
    /// reaches the delimited parser and fails as "malformed" instead of failing
    /// to compile.
    #[must_use]
    pub fn is_ofx(self) -> bool {
        matches!(self, Self::Ofx | Self::Qfx | Self::Qbo)
    }

    /// Whether this format is read by the spreadsheet backend.
    #[must_use]
    pub fn is_spreadsheet(self) -> bool {
        matches!(
            self,
            Self::Xls | Self::Xlsx | Self::Xlsm | Self::Xlsb | Self::Ods
        )
    }
}

impl std::fmt::Display for SourceFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a conversion could not produce a [`Tabular`].
///
/// No variant carries a path, a raw cell value, or a byte offset into user data.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConvertError {
    #[error("unsupported file type '{ext}'")]
    Unsupported { ext: String },
    #[error("PDF statements are not supported yet")]
    PdfNotSupported,
    #[error("investment statements are not supported yet")]
    InvestmentStatement,
    #[error("the file is empty")]
    Empty,
    #[error("the file is larger than the {limit} byte limit")]
    TooLarge { limit: usize },
    #[error("malformed {format} file: {detail}")]
    Malformed {
        format: SourceFormat,
        detail: String,
    },
    #[error("no worksheet contained tabular data")]
    NoTable,
}

pub use delimited::{align_to_skip, to_csv};

/// Identify what `bytes` actually is.
///
/// Content is sniffed FIRST and the name is only consulted to break ties the
/// bytes cannot: a `.qfx` that is really OFX 2.x XML must not be parsed as
/// SGML, and a bank that ships `.xls` containing a ZIP is shipping `.xlsx`.
/// `name` is a bare file name, never a path.
///
/// # Errors
/// [`ConvertError::PdfNotSupported`] for a PDF — refused by name rather than
/// falling through to a confusing delimited-parse failure. [`ConvertError::Empty`],
/// [`ConvertError::TooLarge`], or [`ConvertError::Unsupported`] otherwise.
pub fn detect(name: &str, bytes: &[u8]) -> Result<SourceFormat, ConvertError> {
    if bytes.is_empty() {
        return Err(ConvertError::Empty);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ConvertError::TooLarge {
            limit: MAX_INPUT_BYTES,
        });
    }

    let ext = extension(name);

    // Magic numbers first — these are facts, where the extension is a claim.
    if bytes.starts_with(b"%PDF-") {
        return Err(ConvertError::PdfNotSupported);
    }
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        // OLE2 compound document: legacy .xls.
        return Ok(SourceFormat::Xls);
    }
    if bytes.starts_with(b"PK\x03\x04") {
        // A ZIP container: xlsx/xlsm/xlsb/ods all live here, and only the name
        // (or a much deeper look inside) tells them apart.
        return Ok(match ext.as_deref() {
            Some("ods") => SourceFormat::Ods,
            Some("xlsm") => SourceFormat::Xlsm,
            Some("xlsb") => SourceFormat::Xlsb,
            _ => SourceFormat::Xlsx,
        });
    }
    if ofx::looks_like_ofx(bytes) {
        // OFX, QFX and QBO parse identically; the distinction is only ever
        // cosmetic (QFX adds INTU.BID), so the name decides how we label it.
        // They are labelled apart rather than folded together because the label
        // is what `/api/import/capabilities` publishes, and the SPA refuses any
        // extension absent from that list -- folding `.qbo` into `Qfx` made a
        // format the engine reads unselectable in the file picker.
        return Ok(match ext.as_deref() {
            Some("qfx") => SourceFormat::Qfx,
            Some("qbo") => SourceFormat::Qbo,
            _ => SourceFormat::Ofx,
        });
    }

    match ext.as_deref() {
        Some("csv") => Ok(SourceFormat::Csv),
        Some("tsv") => Ok(SourceFormat::Tsv),
        Some("ssv") => Ok(SourceFormat::Ssv),
        Some("ofx") => Ok(SourceFormat::Ofx),
        Some("qfx") => Ok(SourceFormat::Qfx),
        Some("qbo") => Ok(SourceFormat::Qbo),
        Some("xls") => Ok(SourceFormat::Xls),
        Some("xlsx") => Ok(SourceFormat::Xlsx),
        Some("xlsm") => Ok(SourceFormat::Xlsm),
        Some("xlsb") => Ok(SourceFormat::Xlsb),
        Some("ods") => Ok(SourceFormat::Ods),
        Some("pdf") => Err(ConvertError::PdfNotSupported),
        other => Err(ConvertError::Unsupported {
            ext: other.unwrap_or_default().to_string(),
        }),
    }
}

/// Normalise `bytes` of a known `format` into one [`Tabular`].
///
/// # Errors
/// See [`ConvertError`]. Errors never carry a path or raw user data.
pub fn convert(format: SourceFormat, bytes: &[u8]) -> Result<Tabular, ConvertError> {
    if bytes.is_empty() {
        return Err(ConvertError::Empty);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ConvertError::TooLarge {
            limit: MAX_INPUT_BYTES,
        });
    }
    match format {
        f if f.is_ofx() => ofx::parse(bytes),
        f if f.is_spreadsheet() => spreadsheet::parse(bytes, f),
        f => delimited::parse(bytes, f),
    }
}

/// The lowercased final extension of a bare file name, if it has one.
fn extension(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.trim().to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pdf_is_refused_by_name_not_by_a_parse_failure() {
        let err = detect("statement.pdf", b"%PDF-1.7\n...").unwrap_err();
        assert_eq!(err, ConvertError::PdfNotSupported);
    }

    #[test]
    fn a_pdf_is_refused_even_when_the_extension_lies() {
        let err = detect("statement.csv", b"%PDF-1.7\n...").unwrap_err();
        assert_eq!(err, ConvertError::PdfNotSupported);
    }

    #[test]
    fn content_beats_the_extension_for_ofx() {
        // A .csv that is really OFX. The bytes win.
        let ofx = b"OFXHEADER:100\nDATA:OFXSGML\n\n<OFX><SIGNONMSGSRSV1></SIGNONMSGSRSV1></OFX>";
        assert_eq!(detect("export.csv", ofx), Ok(SourceFormat::Ofx));
    }

    #[test]
    fn every_published_format_is_detected_from_its_own_extension() {
        // The bug this pins: `.qbo` was readable -- the extension match sent it
        // to the OFX parser -- but it had no variant of its own, so it could
        // not appear in `/api/import/capabilities`, and the SPA refuses any
        // extension absent from that list. The result was a format the engine
        // reads and the file picker will not offer.
        //
        // Bytes that trip no magic number, so every name resolves through the
        // extension table and the whole of `ALL` is exercised.
        let plain = b"header\nrow\n";
        for format in SourceFormat::ALL {
            let name = format!("statement.{format}");
            assert_eq!(detect(&name, plain), Ok(format), "{name}");
        }
    }

    #[test]
    fn the_quickbooks_dialect_is_read_by_the_ofx_backend() {
        // `.qbo` is Web Connect: OFX 1.x SGML under a third name. `convert`
        // dispatches through a catch-all arm, so a dialect the predicate misses
        // reaches the DELIMITED parser and fails as "malformed" rather than
        // failing to compile -- which is exactly how this would regress.
        assert!(SourceFormat::Qbo.is_ofx());
        assert!(!SourceFormat::Qbo.is_delimited());
        assert!(!SourceFormat::Qbo.is_spreadsheet());

        let ofx = b"OFXHEADER:100\nDATA:OFXSGML\n\n<OFX><SIGNONMSGSRSV1></SIGNONMSGSRSV1></OFX>";
        assert_eq!(detect("webconnect.qbo", ofx), Ok(SourceFormat::Qbo));
    }

    #[test]
    fn a_zip_container_is_disambiguated_by_name() {
        let zip = b"PK\x03\x04rest-of-a-zip";
        assert_eq!(detect("book.ods", zip), Ok(SourceFormat::Ods));
        assert_eq!(detect("book.xlsm", zip), Ok(SourceFormat::Xlsm));
        // An .xls that is really a ZIP is really an .xlsx.
        assert_eq!(detect("book.xls", zip), Ok(SourceFormat::Xlsx));
    }

    #[test]
    fn a_legacy_xls_is_recognised_by_its_ole2_magic() {
        let ole = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        assert_eq!(detect("book.xls", &ole), Ok(SourceFormat::Xls));
    }

    #[test]
    fn delimited_formats_fall_back_to_the_extension() {
        assert_eq!(detect("a.csv", b"x,y\n1,2\n"), Ok(SourceFormat::Csv));
        assert_eq!(detect("a.tsv", b"x\ty\n1\t2\n"), Ok(SourceFormat::Tsv));
        assert_eq!(detect("a.ssv", b"x;y\n1;2\n"), Ok(SourceFormat::Ssv));
    }

    #[test]
    fn an_unknown_extension_names_itself_and_nothing_else() {
        let err = detect("statement.docx", b"some bytes").unwrap_err();
        assert_eq!(
            err,
            ConvertError::Unsupported {
                ext: "docx".to_string()
            }
        );
        // The message must not quote a path or the file's contents.
        let rendered = err.to_string();
        assert!(!rendered.contains('/'), "{rendered}");
        assert!(!rendered.contains("some bytes"), "{rendered}");
    }

    #[test]
    fn an_empty_input_is_empty_regardless_of_name() {
        assert_eq!(detect("a.csv", b""), Err(ConvertError::Empty));
        assert_eq!(convert(SourceFormat::Csv, b""), Err(ConvertError::Empty));
    }

    #[test]
    fn oversize_input_is_refused_before_any_parsing() {
        let big = vec![b'a'; MAX_INPUT_BYTES + 1];
        let limit = MAX_INPUT_BYTES;
        assert_eq!(detect("a.csv", &big), Err(ConvertError::TooLarge { limit }));
        assert_eq!(
            convert(SourceFormat::Csv, &big),
            Err(ConvertError::TooLarge { limit })
        );
    }

    #[test]
    fn extensions_are_case_and_whitespace_insensitive() {
        assert_eq!(extension("A.CSV"), Some("csv".to_string()));
        assert_eq!(extension("a.Csv "), Some("csv".to_string()));
        assert_eq!(extension("noextension"), None);
        assert_eq!(extension("trailing."), None);
    }
}
