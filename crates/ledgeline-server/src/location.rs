//! Where the open journal lives: the [`Vault`] every journal-tree read and
//! write in this crate goes through, and the main file's place in it.
//!
//! Every snapshot built from an editor carries one, so a handler reads the
//! journal it is serving and the storage it came from out of the same atomic
//! load — a File→Open between the two can never pair a journal with the wrong
//! directory.
//!
//! # Keys and paths
//!
//! The parsed journal records each source file by its [`Vault::key`]
//! (`Transaction::source_file`, `Journal::source_files`). Handlers keep those
//! keys as they always did and convert at the edge — [`JournalLocation::path_of`]
//! turns a key back into a [`VaultPath`] for a read or a write. On the desktop a
//! key is the file's canonical path, exactly as before the seam.
//!
//! # Subprocesses
//!
//! `hledger` and `git` need a real path. [`JournalLocation::local_path`] is the
//! one place that asks for one, and the one place that says why there is none.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ledgeline_core::parse::{self, ParseError};
use ledgeline_core::vault::{Vault, VaultPath};
use ledgeline_core::{Journal, journals};

use crate::error::AppError;

/// The open journal's storage: its vault and its main file.
#[derive(Debug, Clone)]
pub struct JournalLocation {
    vault: Arc<dyn Vault>,
    main: VaultPath,
}

impl JournalLocation {
    #[must_use]
    pub fn new(vault: Arc<dyn Vault>, main: VaultPath) -> Self {
        Self { vault, main }
    }

    /// The desktop location of a journal parsed from disk with no vault in hand
    /// — its main file is `source_files[0]`, an absolute path — or `None` for a
    /// journal with no such file. What a read-only state (built from an
    /// already-parsed journal, as the test harness builds one) falls back to,
    /// so it reads the same files it always read.
    #[must_use]
    pub(crate) fn of_parsed(journal: &Journal) -> Option<Self> {
        let main = journal.source_files.first()?;
        if !main.is_absolute() {
            return None;
        }
        let (vault, main) = ledgeline_core::vault::LocalVault::for_journal(main).ok()?;
        Some(Self::new(Arc::new(vault), main))
    }

    /// The vault every journal-tree read and write goes through.
    #[must_use]
    pub fn vault(&self) -> &Arc<dyn Vault> {
        &self.vault
    }

    /// The main journal file's place in [`Self::vault`].
    #[must_use]
    pub fn main(&self) -> &VaultPath {
        &self.main
    }

    /// The main journal's own directory — the include root every feature here
    /// is confined to.
    #[must_use]
    pub fn root_dir(&self) -> VaultPath {
        self.main.parent().unwrap_or_default()
    }

    /// The vault path for a key the parse recorded, or `None` for a key that is
    /// not inside this vault (which no parse through it produces).
    #[must_use]
    pub fn path_of(&self, key: &Path) -> Option<VaultPath> {
        self.vault.vault_path(key).ok()
    }

    /// The key the parse records for `path` ([`Vault::key`]).
    #[must_use]
    pub fn key(&self, path: &VaultPath) -> PathBuf {
        self.vault.key(path)
    }

    /// Reparse the whole journal through the vault with `overrides` standing in
    /// for those files — the "is it still a journal?" proof every write makes
    /// before it writes.
    ///
    /// # Errors
    /// The [`ParseError`] the edited journal would raise.
    pub fn reparse(&self, overrides: &HashMap<VaultPath, String>) -> Result<Journal, ParseError> {
        parse::parse_journal_in(&*self.vault, &self.main, overrides).map(|parsed| parsed.journal)
    }

    /// [`journals::targets_in`] for `journal`, judged in this vault.
    #[must_use]
    pub fn targets(&self, journal: &Journal) -> Vec<journals::JournalTarget> {
        journals::targets_in(&*self.vault, journal)
    }

    /// The vault path a `journalId` names, taken from the files the parse
    /// actually read: the id joined to the include root, and then only if that
    /// is one of `journal`'s source files. Security layer 2 of every
    /// journal-file API — an id that is not a file the journal already reads
    /// resolves to nothing.
    #[must_use]
    pub fn source_path(&self, journal: &Journal, id: &str) -> Option<VaultPath> {
        // Only an id that is ALREADY in normal form can match, as only an exact
        // `root.join(id)` could before: `a/../b` is not a spelling of `b` here.
        let relative = VaultPath::new(id).ok().filter(|path| path.as_str() == id)?;
        let candidate = self.root_dir().join_path(&relative);
        journal
            .source_files
            .iter()
            .any(|key| self.path_of(key).as_ref() == Some(&candidate))
            .then_some(candidate)
    }

    /// A real filesystem path for `path`, for handing to `hledger` or `git`.
    ///
    /// # Errors
    /// [`AppError::BadRequest`] naming what needs one, when the journal is not
    /// on a local filesystem (an in-memory or sandboxed vault).
    pub(crate) fn local_path(
        &self,
        path: &VaultPath,
        needed_by: &str,
    ) -> Result<PathBuf, AppError> {
        self.vault
            .local_path(path)
            .ok_or_else(|| no_local_path(needed_by))
    }
}

/// Why an operation that runs an external program cannot run here.
pub(crate) fn no_local_path(needed_by: &str) -> AppError {
    AppError::BadRequest(format!(
        "{needed_by} needs the journal on a local filesystem, and this one is not on one"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::vault::MemVault;

    fn vp(path: &str) -> VaultPath {
        VaultPath::new(path).unwrap()
    }

    #[test]
    fn source_paths_resolve_only_files_the_journal_reads() {
        let vault = Arc::new(
            MemVault::new()
                .with_file("books/main.journal", "include 2026/a.journal\n")
                .with_file("books/2026/a.journal", "")
                .with_file("books/other.journal", ""),
        );
        let location = JournalLocation::new(vault, vp("books/main.journal"));
        let journal = location.reparse(&HashMap::new()).unwrap();
        assert_eq!(
            location.source_path(&journal, "2026/a.journal"),
            Some(vp("books/2026/a.journal"))
        );
        assert_eq!(
            location.source_path(&journal, "main.journal"),
            Some(vp("books/main.journal"))
        );
        assert_eq!(location.source_path(&journal, "other.journal"), None);
        assert_eq!(
            location.source_path(&journal, "../books/main.journal"),
            None
        );
        assert!(location.local_path(&vp("books/main.journal"), "x").is_err());
        assert_eq!(location.root_dir(), vp("books"));
    }
}
