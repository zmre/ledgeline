//! The `[books]` table of `ledgeline.toml` (plan 45 D-settings).
//!
//! ```toml
//! [books]
//! basis = "accrual"                          # or "cash"; overrides the gate
//! closed-through = "2026-09-30"              # written by "Close books through…"
//! recognition-file = "books/recognition.journal"   # hand-written only
//! ```
//!
//! Values are validated like `[ui]`'s: `basis` is a closed set, the date is
//! parsed, the path is confined when it is resolved
//! ([`super::book::recognition_path`]). [`parse`] never fails as a whole: an
//! unreadable value is dropped with a warning. Only `closed-through` is ever
//! written, by [`with_closed_through`], through `toml_edit` so everything else
//! comes back byte for byte.

use super::{Basis, is_iso_date};
use crate::ledger_file::{self, LEDGER_FILE, LedgerEditError, LedgerFile};
use crate::vault::{Vault, VaultPath};
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, value};

/// The table.
pub const BOOKS_TABLE: &str = "books";
/// `basis = "cash" | "accrual"`.
pub const BASIS_KEY: &str = "basis";
/// `closed-through = "YYYY-MM-DD"`.
pub const CLOSED_THROUGH_KEY: &str = "closed-through";
/// `recognition-file = "relative/path.journal"`.
pub const RECOGNITION_FILE_KEY: &str = "recognition-file";

/// What `[books]` says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BooksConfig {
    /// The gate override.
    pub basis: Option<Basis>,
    /// Books are closed through this date, inclusive.
    pub closed_through: Option<String>,
    /// Where the recognition file is, relative to the journal's folder.
    pub recognition_file: Option<String>,
    /// What was ignored and why.
    pub warnings: Vec<String>,
}

/// Why `[books]` can't be read or written.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BooksError {
    /// The value to write is not a date.
    #[error("`{0}` is not a date (YYYY-MM-DD)")]
    BadDate(String),
    /// The file is there but unusable.
    #[error("{LEDGER_FILE} {0}, so Ledgeline will not write to it")]
    Unusable(&'static str),
    /// The file isn't TOML, or `[books]` isn't a table.
    #[error("{LEDGER_FILE}: {0}")]
    Malformed(String),
    /// Another writer kept changing it.
    #[error("{LEDGER_FILE} kept changing while it was being saved; nothing was written")]
    KeptChanging,
    /// The write failed.
    #[error("could not save {LEDGER_FILE} ({0:?}); nothing was written")]
    Write(std::io::ErrorKind),
}

/// The header a `ledgeline.toml` created by "Close books through…" opens
/// with.
pub const BOOKS_HEADER: &str = concat!(
    "# Ledgeline settings for this ledger. [books] says how the books are kept;\n",
    "# [ui] keys override ui-prefs.toml while this ledger is open.\n",
    "# Docs: \"Settings files\".\n",
    "\n",
);

/// Read `[books]` out of `ledgeline.toml`'s text.
#[must_use]
pub fn parse(text: &str) -> BooksConfig {
    let Ok(document) = text.parse::<DocumentMut>() else {
        return BooksConfig {
            warnings: vec![format!("{LEDGER_FILE} is not valid TOML")],
            ..BooksConfig::default()
        };
    };
    of_document(&document)
}

fn of_document(document: &DocumentMut) -> BooksConfig {
    let mut config = BooksConfig::default();
    let Some(item) = document.get(BOOKS_TABLE) else {
        return config;
    };
    let Some(table) = item.as_table_like() else {
        config
            .warnings
            .push(format!("[{BOOKS_TABLE}] must be a table"));
        return config;
    };
    let text = |key: &str| -> Option<String> {
        let item = table.get(key)?;
        item.as_str().map(str::to_string).or_else(|| {
            item.as_value()
                .and_then(|v| v.as_datetime())
                .map(ToString::to_string)
        })
    };
    if let Some(raw) = table.get(BASIS_KEY) {
        match raw.as_str().and_then(Basis::parse) {
            Some(basis) => config.basis = Some(basis),
            None => config.warnings.push(format!(
                "books.{BASIS_KEY} must be \"cash\" or \"accrual\"; ignored"
            )),
        }
    }
    if table.get(CLOSED_THROUGH_KEY).is_some() {
        match text(CLOSED_THROUGH_KEY).filter(|date| is_iso_date(date)) {
            Some(date) => config.closed_through = Some(date),
            None => config.warnings.push(format!(
                "books.{CLOSED_THROUGH_KEY} must be a date (YYYY-MM-DD); ignored"
            )),
        }
    }
    if table.get(RECOGNITION_FILE_KEY).is_some() {
        match text(RECOGNITION_FILE_KEY).filter(|path| !path.trim().is_empty()) {
            Some(path) => config.recognition_file = Some(path),
            None => config.warnings.push(format!(
                "books.{RECOGNITION_FILE_KEY} must be a relative path; ignored"
            )),
        }
    }
    config
}

/// `text` with `[books] closed-through` set to `date` (or removed for
/// `None`), every other byte kept.
///
/// # Errors
/// [`BooksError::BadDate`] for a malformed date; [`BooksError::Malformed`]
/// when `text` isn't TOML or `books` isn't a table.
pub fn with_closed_through(text: &str, date: Option<&str>) -> Result<String, BooksError> {
    if let Some(date) = date
        && !is_iso_date(date)
    {
        return Err(BooksError::BadDate(date.to_string()));
    }
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|_| BooksError::Malformed("not valid TOML".into()))?;
    if document.get(BOOKS_TABLE).is_none() {
        if date.is_none() {
            return Ok(text.to_string());
        }
        document.insert(BOOKS_TABLE, Item::Table(Table::new()));
    }
    let table = document
        .get_mut(BOOKS_TABLE)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| BooksError::Malformed(format!("[{BOOKS_TABLE}] must be a table")))?;
    match date {
        Some(date) => {
            table.insert(CLOSED_THROUGH_KEY, value(date));
        }
        None => {
            table.remove(CLOSED_THROUGH_KEY);
        }
    }
    Ok(document.to_string())
}

/// The `[books]` of the ledger whose main journal is `main` in `vault`; the
/// default when there is no `ledgeline.toml`.
///
/// # Errors
/// [`BooksError::Unusable`] when the file is there but can't be read.
pub fn read(vault: &dyn Vault, main: &VaultPath) -> Result<BooksConfig, BooksError> {
    let path = ledger_file::ledger_path(main).map_err(|_| BooksError::Unusable("has no place"))?;
    match ledger_file::read(vault, &path) {
        LedgerFile::Missing => Ok(BooksConfig::default()),
        LedgerFile::Present { document, .. } => Ok(of_document(&document)),
        LedgerFile::Unusable(reason) => Err(BooksError::Unusable(reason)),
    }
}

/// Set (or clear) `closed-through` — the explicit **Close books through…**
/// action, which creates `ledgeline.toml` when it is missing (D-settings).
///
/// # Errors
/// As [`with_closed_through`], plus the write's own refusals.
pub fn set_closed_through(
    vault: &dyn Vault,
    main: &VaultPath,
    date: Option<&str>,
) -> Result<(), BooksError> {
    let path = ledger_file::ledger_path(main).map_err(|_| BooksError::Unusable("has no place"))?;
    ledger_file::edit(vault, &path, BOOKS_HEADER, |text| {
        with_closed_through(text, date)
    })
    .map(|_| ())
    .map_err(|error| match error {
        LedgerEditError::Edit(error) => error,
        LedgerEditError::Unusable(reason) => BooksError::Unusable(reason),
        LedgerEditError::KeptChanging => BooksError::KeptChanging,
        LedgerEditError::Write(kind) => BooksError::Write(kind),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_read_and_bad_ones_warned_about() {
        let config = parse(
            "[ui]\ntheme = \"dark\"\n\n[books]\nbasis = \"Accrual\"\nclosed-through = \"2026-09-30\"\nrecognition-file = \"r.journal\"\n",
        );
        assert_eq!(config.basis, Some(Basis::Accrual));
        assert_eq!(config.closed_through.as_deref(), Some("2026-09-30"));
        assert_eq!(config.recognition_file.as_deref(), Some("r.journal"));
        assert!(config.warnings.is_empty());
        let bad = parse("[books]\nbasis = \"modified\"\nclosed-through = \"Sept\"\n");
        assert_eq!((bad.basis, bad.closed_through), (None, None));
        assert_eq!(bad.warnings.len(), 2);
        // A TOML date works too.
        assert_eq!(
            parse("[books]\nclosed-through = 2026-09-30\n")
                .closed_through
                .as_deref(),
            Some("2026-09-30")
        );
    }

    #[test]
    fn writing_keeps_everything_else() {
        let text = "# mine\n[ui]\ntheme = \"dark\" # keep\n";
        let set = with_closed_through(text, Some("2026-10-31")).expect("writes");
        assert!(set.starts_with(text), "{set}");
        assert_eq!(parse(&set).closed_through.as_deref(), Some("2026-10-31"));
        let cleared = with_closed_through(&set, None).expect("clears");
        assert_eq!(parse(&cleared).closed_through, None);
        assert!(cleared.starts_with(text));
        assert!(with_closed_through(text, Some("2026-13-01")).is_err());
        assert_eq!(with_closed_through(text, None).expect("no-op"), text);
    }
}
