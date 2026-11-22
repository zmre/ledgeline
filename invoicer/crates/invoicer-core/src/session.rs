//! An open books folder (ARCHITECTURE §6.4): the vault rooted at the main
//! journal's folder, `[books]`, the journal's money styles and `invoice:`
//! numbers, and the document index. The CLI opens one per run; the GUI and
//! its API will hold one.
//!
//! Opening reads the journal through ledgeline-core but never writes it:
//! nothing a [`Session`] does touches the books (I2).

use crate::doc::{InvoiceNumber, Money};
use crate::index::{DocumentIndex, IndexedDoc};
use crate::numbering::{self, NumberingError};
use crate::settings::{Settings, SettingsError};
use ledgeline_core::accrual::books_config::{self, BooksConfig, BooksError};
use ledgeline_core::edit::{EditError, JournalEditor};
use ledgeline_core::vault::{LocalVault, Vault, VaultError, VaultPath};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

/// Why the books can't be opened.
#[derive(Debug, Error)]
pub enum OpenError {
    /// The journal path can't be a vault's main journal.
    #[error("{path} can't be opened as a journal: {source}")]
    Path {
        /// The path given.
        path: String,
        /// Why.
        source: VaultError,
    },
    /// The journal doesn't read or parse.
    #[error("{path}: {source}")]
    Journal {
        /// The main journal.
        path: String,
        /// Why.
        source: Box<EditError>,
    },
    /// `ledgeline.toml [invoicer]` is unusable.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// `ledgeline.toml [books]` is unusable.
    #[error(transparent)]
    Books(#[from] BooksError),
}

/// Why a document argument names no single document.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FindError {
    /// Nothing matches.
    #[error("no invoice is numbered or at {0:?}")]
    NotFound(String),
    /// Several documents share the number.
    #[error("{} documents are numbered {number}: {}; name the file instead", paths.len(), paths.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))]
    Ambiguous {
        /// The number.
        number: String,
        /// The documents.
        paths: Vec<VaultPath>,
    },
}

/// An open books folder.
pub struct Session {
    vault: Arc<dyn Vault>,
    main: VaultPath,
    books: BooksConfig,
    booked: Vec<String>,
    index: DocumentIndex,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("main", &self.main)
            .field("documents", &self.index.docs().count())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Open the books whose main journal is `main` in `vault`.
    ///
    /// # Errors
    /// [`OpenError`].
    pub fn open(vault: Arc<dyn Vault>, main: VaultPath) -> Result<Self, OpenError> {
        let settings = Settings::read(&*vault)?;
        let books = books_config::read(&*vault, &main)?;
        let editor = JournalEditor::open_in(Arc::clone(&vault), main.clone()).map_err(|e| {
            OpenError::Journal {
                path: main.to_string(),
                source: Box::new(e),
            }
        })?;
        let journal = editor.journal();
        let money = Money::of_journal(journal);
        let booked = numbering::booked_numbers(journal);
        let index = DocumentIndex::open(&*vault, settings, money);
        Ok(Self {
            vault,
            main,
            books,
            booked,
            index,
        })
    }

    /// Open the books whose main journal is the local file `journal`; the
    /// books folder is its (canonical) folder.
    ///
    /// # Errors
    /// [`OpenError`].
    pub fn open_local(journal: &Path) -> Result<Self, OpenError> {
        let (vault, main) = LocalVault::for_journal(journal).map_err(|source| OpenError::Path {
            path: journal.display().to_string(),
            source,
        })?;
        Self::open(Arc::new(vault), main)
    }

    /// The vault (the books folder).
    #[must_use]
    pub fn vault(&self) -> &dyn Vault {
        &*self.vault
    }

    /// The main journal.
    #[must_use]
    pub fn main(&self) -> &VaultPath {
        &self.main
    }

    /// `[books]`.
    #[must_use]
    pub fn books_config(&self) -> &BooksConfig {
        &self.books
    }

    /// `[invoicer]`.
    #[must_use]
    pub fn settings(&self) -> &Settings {
        self.index.settings()
    }

    /// The document index.
    #[must_use]
    pub fn index(&self) -> &DocumentIndex {
        &self.index
    }

    /// The index and the vault, for operations that write documents.
    pub fn parts_mut(&mut self) -> (&mut DocumentIndex, &dyn Vault) {
        (&mut self.index, &*self.vault)
    }

    /// Every `invoice:` number in the books.
    #[must_use]
    pub fn booked_numbers(&self) -> &[String] {
        &self.booked
    }

    /// The next number over the files and the books (§5.1).
    ///
    /// # Errors
    /// [`NumberingError`].
    pub fn next_number(&self) -> Result<InvoiceNumber, NumberingError> {
        self.index
            .next_number(self.booked.iter().map(String::as_str))
    }

    /// A vault path as a local file path, for messages.
    #[must_use]
    pub fn local_path(&self, path: &VaultPath) -> PathBuf {
        self.vault.key(path)
    }

    /// The document `target` names: a number (`105`), else a path in the
    /// books folder (`invoices/105.md`, `/`-rooted or not, `.md` optional).
    ///
    /// # Errors
    /// [`FindError`].
    pub fn find(&self, target: &str) -> Result<&IndexedDoc, FindError> {
        let wanted = target.trim();
        let numbered = self.index.by_number(wanted);
        match numbered.as_slice() {
            [one] => return Ok(one),
            [] => {}
            many => {
                return Err(FindError::Ambiguous {
                    number: wanted.to_string(),
                    paths: many.iter().map(|d| d.path.clone()).collect(),
                });
            }
        }
        let relative = wanted.trim_start_matches('/');
        [relative.to_string(), format!("{relative}.md")]
            .iter()
            .filter_map(|p| VaultPath::new(p).ok())
            .find_map(|p| self.index.doc(&p))
            .ok_or_else(|| FindError::NotFound(wanted.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
            .join("books.journal")
    }

    #[test]
    fn opens_a_fixture_and_numbers_over_files_and_books() {
        let session = Session::open_local(&fixture("accrual")).expect("opens");
        assert_eq!(session.index().docs().count(), 8);
        assert!(session.booked_numbers().iter().any(|n| n == "2004"));
        // Plan 45's invoices 2001–2004 have no files, but their numbers are
        // used (fixtures/README.md).
        assert_eq!(session.next_number().expect("next").as_str(), "2005");
        let cash = Session::open_local(&fixture("cash")).expect("opens");
        assert_eq!(cash.next_number().expect("next").as_str(), "208");
    }

    #[test]
    fn finds_by_number_or_path() {
        let session = Session::open_local(&fixture("accrual")).expect("opens");
        let by_number = session.find("105").expect("105");
        assert_eq!(by_number.number(), Some("105"));
        let path = by_number.path.to_string();
        assert_eq!(session.find(&path).expect("path").path, by_number.path);
        assert_eq!(
            session
                .find(&format!("/{}", path.trim_end_matches(".md")))
                .expect("no .md")
                .path,
            by_number.path
        );
        assert_eq!(session.find("999"), Err(FindError::NotFound("999".into())));
        let local = session.local_path(&by_number.path);
        assert_eq!(
            session.vault().vault_path(&local).ok(),
            Some(by_number.path.clone())
        );
    }

    #[test]
    fn a_missing_journal_is_an_error() {
        let err = Session::open_local(Path::new("/nonexistent/books.journal")).expect_err("no");
        assert!(matches!(err, OpenError::Journal { .. }), "{err}");
    }
}
