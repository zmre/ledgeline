//! Whole-book imports: every source format that becomes the multi-posting
//! [`QbJournal`] shape the QuickBooks pipeline writes (plan 39).
//!
//! [`detect_and_parse`] is the one place an upload is asked "is this a book
//! export, and which?". It tries each format in turn and answers `None` for
//! anything else, which then goes on to the ordinary statement pipeline
//! (`convert` and a CSV rules file). Each detector is conservative for that
//! reason: a bank CSV mistaken for a book export would lose the user the rules
//! flow they needed.
//!
//! Order matters only where two detectors could both claim a file, and none
//! can today: QuickBooks needs a workbook, QIF needs a `!Type:`-style header.

use crate::qb_journal::{self, ImportSource, QbJournal, QbJournalError};
use crate::qif::{self, QifError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a file that was recognised as a book export could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum BookImportError {
    #[error("{0}")]
    QuickBooks(#[from] QbJournalError),
    #[error("{0}")]
    Qif(#[from] QifError),
}

/// Recognise and parse a book export with the QuickBooks reader run in this
/// process. See [`detect_and_parse_with`].
#[must_use]
pub fn detect_and_parse(name: &str, bytes: &[u8]) -> Option<Result<QbJournal, BookImportError>> {
    detect_and_parse_with(name, bytes, |bytes| {
        qb_journal::detect(bytes).then(|| qb_journal::parse(bytes))
    })
}

/// Recognise and parse a book export, or `None` when `bytes` are not one.
///
/// `quickbooks` answers the QuickBooks question: `None` when the bytes are not
/// a QuickBooks Journal report, else its parse. It is a parameter because
/// the server runs that reader (which opens workbooks with `calamine`) in a
/// resource-limited child process; [`detect_and_parse`] runs it in-process.
///
/// `Some(Err(..))` means the file is unmistakably that format but damaged, or
/// named `.qif` without being QIF. The user is owed that refusal by name rather
/// than a fall-through to the CSV rules screen.
pub fn detect_and_parse_with(
    name: &str,
    bytes: &[u8],
    quickbooks: impl FnOnce(&[u8]) -> Option<Result<QbJournal, QbJournalError>>,
) -> Option<Result<QbJournal, BookImportError>> {
    if let Some(parsed) = quickbooks(bytes) {
        return Some(parsed.map_err(BookImportError::from));
    }
    if qif::has_qif_extension(name) || qif::sniff(bytes) {
        return Some(
            crate::qif_import::parse_and_import(name, bytes).map_err(BookImportError::from),
        );
    }
    // MERGE SEAM (plan 39 grouped CSV): the posting-row CSV presets
    // (`grouped_csv.rs`: Xero, GnuCash, hledger `print -O csv`) and the
    // single-entry presets (`single_entry.rs`: YNAB, Monarch, Copilot) slot in
    // here, each as `if detected { return Some(parse(..).map_err(..)); }`
    // with its own `BookImportError` variant. Keep their sniffing
    // conservative: a CSV no preset claims must fall through to `None` and
    // the rules pipeline.
    None
}

/// Extensions only a book importer reads, which the statement pipeline's
/// format list (`convert::SourceFormat::ALL`) does not include. Published with
/// that list so the file picker offers them.
pub const EXTRA_EXTENSIONS: [&str; 2] = qif::EXTENSIONS;

impl ImportSource {
    /// The app or format name a person recognises, for labels and messages.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::QuickBooks => "QuickBooks",
            Self::Qif => "QIF",
            Self::Xero => "Xero",
            Self::GnuCash => "GnuCash",
            Self::HledgerCsv => "hledger CSV",
            Self::Ynab => "YNAB",
            Self::Monarch => "Monarch",
            Self::Copilot => "Copilot",
        }
    }

    /// The `format` a stage response reports for this source. QuickBooks keeps
    /// the `quickbooks-journal` it has always had.
    #[must_use]
    pub fn stage_format(self) -> &'static str {
        match self {
            Self::QuickBooks => "quickbooks-journal",
            Self::Qif => "qif",
            Self::Xero => "xero",
            Self::GnuCash => "gnucash",
            Self::HledgerCsv => "hledger-csv",
            Self::Ynab => "ynab",
            Self::Monarch => "monarch",
            Self::Copilot => "copilot",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn not_quickbooks(_: &[u8]) -> Option<Result<QbJournal, QbJournalError>> {
        None
    }

    #[test]
    fn a_qif_file_is_recognised_by_content_or_extension() {
        let qif = b"!Type:Bank\nD1/5/26\nT-1\nLFood\n^\n";
        let parsed = detect_and_parse_with("export.txt", qif, not_quickbooks)
            .expect("recognised by content")
            .expect("parses");
        assert_eq!(parsed.source, ImportSource::Qif);
        assert!(detect_and_parse_with("export.qmtf", qif, not_quickbooks).is_some());
    }

    #[test]
    fn a_qif_named_file_that_is_not_qif_is_refused_by_name() {
        let answer = detect_and_parse_with("x.qif", b"Date,Amount\n", not_quickbooks);
        assert_eq!(answer, Some(Err(BookImportError::Qif(QifError::NotQif))));
    }

    #[test]
    fn an_ordinary_csv_is_not_a_book_export() {
        let csv = b"Date,Description,Amount\n2026-01-05,Coffee,-3.00\n";
        assert_eq!(detect_and_parse_with("bank.csv", csv, not_quickbooks), None);
        assert_eq!(detect_and_parse("bank.csv", csv), None);
    }

    #[test]
    fn quickbooks_is_asked_first() {
        let answer = detect_and_parse_with("x.qif", b"!Type:Bank\n", |_| {
            Some(Err(QbJournalError::Unreadable))
        });
        assert_eq!(
            answer,
            Some(Err(BookImportError::QuickBooks(QbJournalError::Unreadable)))
        );
    }

    #[test]
    fn every_source_has_a_label_and_a_distinct_stage_format() {
        let all = [
            ImportSource::QuickBooks,
            ImportSource::Qif,
            ImportSource::Xero,
            ImportSource::GnuCash,
            ImportSource::HledgerCsv,
            ImportSource::Ynab,
            ImportSource::Monarch,
            ImportSource::Copilot,
        ];
        let formats: std::collections::HashSet<_> =
            all.iter().map(|source| source.stage_format()).collect();
        assert_eq!(formats.len(), all.len());
        assert!(all.iter().all(|source| !source.label().is_empty()));
    }
}
