//! Exporting a journal to formats other apps can import (plan 39).
//!
//! - [`qif`] — Quicken Interchange Format, one file per commodity, with a
//!   [`LossReport`] counting everything QIF cannot carry. This is the path to
//!   Banktivity, Quicken, Moneydance and GnuCash.
//! - [`csv`] — one row per posting, byte-compatible with hledger 1.52's
//!   `print -O csv`. The lossless-enough path to anything that reads a
//!   posting-row CSV, including Ledgeline's own grouped-CSV importer.
//!
//! Both are pure: they take a parsed [`Journal`] and return text. Neither runs
//! hledger. Both apply the journal's `=` auto-posting rules first
//! ([`crate::auto::apply_journal`], which mirrors `hledger print --auto`),
//! because neither format can carry the rules themselves and a book exported
//! without their postings would not balance to the same totals.

pub mod csv;
mod loss;
pub mod qif;

pub use loss::LossReport;

use crate::model::{Journal, Transaction};
use crate::parse::{ParseError, parse_journal_in, tag_in_segment};
use crate::vault::LocalVault;
use std::collections::HashMap;
use std::path::Path;

/// Parse the journal whose main file is `path`, following its `include`s, the
/// same way the desktop app opens it (a [`LocalVault`] rooted at the journal's
/// real directory).
///
/// # Errors
/// [`ParseError`] when the journal cannot be read or parsed.
pub fn load_journal(path: &Path) -> Result<Journal, ParseError> {
    let (vault, main) = LocalVault::for_journal(path).map_err(|error| {
        ParseError::Include(format!("could not open {}: {error}", path.display()))
    })?;
    parse_journal_in(&vault, &main, &HashMap::new()).map(|parsed| parsed.journal)
}

/// The transactions in the order hledger's `print` writes them: by primary
/// date, ties kept in file order. Both exporters walk this order, so a QIF
/// register and the CSV list transactions the same way.
pub(crate) fn print_order(journal: &Journal) -> Vec<&Transaction> {
    let mut ordered: Vec<&Transaction> = journal.transactions.iter().collect();
    // `sort_by` is stable, so equal dates keep their file order.
    ordered.sort_by(|a, b| a.date.cmp(&b.date));
    ordered
}

/// A comment's free text with its `name:value` tags removed, on one line.
///
/// Each line keeps its non-tag comma segments (hledger's tag syntax: a tag
/// runs from its name to the next comma), and the lines are joined with
/// `"; "`. Empty when the comment holds nothing but tags.
pub(crate) fn free_text(comment: &str) -> String {
    comment
        .trim()
        .split('\n')
        .map(|line| {
            line.split(',')
                .filter(|segment| tag_in_segment(segment).is_none())
                .map(str::trim)
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Tags the engine or hledger adds on its own (auto-posting markers and
/// hidden `_`-prefixed tags). They describe how a posting came to be, not
/// anything the user wrote, so they are never reported as a lost tag.
pub(crate) fn is_internal_tag(name: &str) -> bool {
    name.starts_with('_')
        || name == crate::auto::GENERATED_POSTING_TAG
        || name == crate::auto::MODIFIED_TRANSACTION_TAG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_text_drops_tags_and_joins_lines() {
        assert_eq!(free_text("txn note, trip:paris\n"), "txn note");
        assert_eq!(free_text("first\nsecond, a:b\n"), "first; second");
        assert_eq!(free_text("a:1, b:2\n"), "");
        assert_eq!(free_text(""), "");
    }
}
