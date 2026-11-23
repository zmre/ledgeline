//! One open books folder, as the app holds it: the invoicer [`Session`]
//! (journal, `[books]`, document index), and the MBR server over the same
//! folder. File → Open builds a new one and swaps it in whole.
//!
//! The reconciler (ARCHITECTURE §5.4) runs here: once over every sent
//! invoice when the books open, then on every settled change the watcher
//! reports, over only the invoices the change touched (§5.6).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Router;
use invoicer_core::reconcile::{self, ReconcileOptions, Reconciled};
use invoicer_core::session::Session;
use ledgeline_core::vault::{Vault, VaultPath};
use mbr_core::Config as MbrConfig;
use mbr_server::LiveServer;

use super::APP;

/// The settings file beside the main journal: `[books]` and `[invoicer]`.
const SETTINGS_FILE: &str = "ledgeline.toml";

/// An open books folder.
pub(crate) struct OpenBooks {
    /// The main journal, canonical.
    journal: PathBuf,
    vault: Arc<dyn Vault>,
    main: VaultPath,
    session: Mutex<Session>,
    /// MBR over the books folder. Its router holds the server; the last
    /// router dropped stops the server's background tasks.
    mbr: Router,
}

impl std::fmt::Debug for OpenBooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenBooks")
            .field("journal", &self.journal)
            .finish_non_exhaustive()
    }
}

impl OpenBooks {
    /// Open the books whose main journal is `journal`, and MBR over its
    /// folder. Must run inside the tokio runtime (MBR's server spawns its
    /// tasks on it).
    ///
    /// # Errors
    /// Why the books or MBR's configuration can't be opened.
    pub(crate) fn open(journal: &Path, gui: bool) -> Result<Self, String> {
        let journal = journal
            .canonicalize()
            .map_err(|e| format!("{}: {e}", journal.display()))?;
        let session = Session::open_local(&journal).map_err(|e| e.to_string())?;
        let vault = session.shared_vault();
        let main = session.main().clone();
        let mbr = mbr_router(vault.root(), gui)?;
        Ok(Self {
            journal,
            vault,
            main,
            session: Mutex::new(session),
            mbr,
        })
    }

    /// The main journal, canonical.
    pub(crate) fn journal(&self) -> &Path {
        &self.journal
    }

    /// The books folder's vault.
    pub(crate) fn vault(&self) -> &Arc<dyn Vault> {
        &self.vault
    }

    /// The main journal in the vault.
    pub(crate) fn main(&self) -> &VaultPath {
        &self.main
    }

    /// MBR's router for this folder.
    pub(crate) fn mbr(&self) -> &Router {
        &self.mbr
    }

    /// The session, locked. A panic while it was held leaves it usable: the
    /// session reads its files afresh before every write.
    pub(crate) fn session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mirror the books into every sent invoice's `paid:` (the reconciler,
    /// on open), logging what changed.
    pub(crate) fn reconcile_all(&self) -> Option<Reconciled> {
        let mut session = self.session();
        // Read the payment state now, so the first change diffs against it.
        let _ = session.book_states();
        reconcile_in(&mut session, None)
    }

    /// React to the files `changed` on disk (a settled watcher burst):
    /// re-read what changed, then reconcile the invoices it touched.
    ///
    /// - a markdown file under the folder: the document index refreshes it;
    /// - a journal file: the journal and `[books]` are read again, and the
    ///   invoices whose payment state moved are reconciled;
    /// - `ledgeline.toml`: the session is opened afresh (`[invoicer]` may
    ///   have changed), and every invoice is reconciled.
    ///
    /// Returns the reconciler's report when it ran.
    pub(crate) fn apply_changes(&self, changed: &[PathBuf]) -> Option<Reconciled> {
        let root = self.vault.root().to_path_buf();
        let mut session = self.session();
        let journal_names: BTreeSet<_> = session
            .journal_files()
            .iter()
            .filter_map(|f| f.file_name().map(ToOwned::to_owned))
            .collect();
        let settings = root.join(SETTINGS_FILE);
        let settings_changed = changed.iter().any(|p| same_file(p, &settings));
        if settings_changed {
            match Session::open(Arc::clone(&self.vault), self.main.clone()) {
                Ok(fresh) => *session = fresh,
                Err(error) => {
                    eprintln!("{}: reload skipped: {error}", APP.log_prefix);
                    return None;
                }
            }
            let _ = session.book_states();
            return reconcile_in(&mut session, None);
        }

        let mut numbers: BTreeSet<String> = BTreeSet::new();
        let markdown: Vec<VaultPath> = changed
            .iter()
            .filter(|p| is_markdown(p))
            .filter_map(|p| self.vault.vault_path(p).ok())
            .collect();
        if !markdown.is_empty() {
            let (index, vault) = session.parts_mut();
            let refreshed = index.refresh(vault, &markdown);
            numbers.extend(
                refreshed
                    .updated
                    .iter()
                    .filter_map(|p| index.doc(p).and_then(|d| d.number()))
                    .map(str::to_string),
            );
        }

        let journal_touched = changed.iter().any(|p| {
            p.file_name()
                .is_some_and(|name| journal_names.contains(name))
        });
        let mut everything = false;
        if journal_touched && !session.changed_journal_files().is_empty() {
            match session.reload_books() {
                Ok(Some(before)) => {
                    numbers.extend(session.book_states().changed_since(&before));
                }
                Ok(None) => everything = true,
                Err(error) => {
                    eprintln!("{}: reload skipped: {error}", APP.log_prefix);
                    return None;
                }
            }
            eprintln!("{}: reloaded {}", APP.log_prefix, self.journal.display());
        }
        if everything {
            reconcile_in(&mut session, None)
        } else if numbers.is_empty() {
            None
        } else {
            reconcile_in(&mut session, Some(numbers.into_iter().collect()))
        }
    }
}

/// Reconcile `only` (or every sent invoice), logging the outcome.
fn reconcile_in(session: &mut Session, only: Option<Vec<String>>) -> Option<Reconciled> {
    let books = session.invoice_books();
    let loaded = match books.load() {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("{}: reconcile skipped: {error}", APP.log_prefix);
            return None;
        }
    };
    let states = session.book_states().clone();
    let (index, _) = session.parts_mut();
    let report = reconcile::reconcile_with(
        &books,
        index,
        &loaded,
        &states,
        &ReconcileOptions {
            only,
            dry_run: false,
        },
    );
    for change in &report.changed {
        eprintln!(
            "{}: {}: paid: {} -> {}",
            APP.log_prefix,
            change.path,
            change.before.as_deref().unwrap_or("(none)"),
            change.after
        );
    }
    for failure in &report.failed {
        eprintln!(
            "{}: {}: paid: not written: {}",
            APP.log_prefix, failure.path, failure.error
        );
    }
    Some(report)
}

/// Whether `path` is a markdown note (by extension, as MBR's default).
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

/// Whether `a` and `b` name the same file, comparing canonical parents (the
/// file itself may be mid-rename).
fn same_file(a: &Path, b: &Path) -> bool {
    a.file_name() == b.file_name()
        && match (
            a.parent().and_then(|d| d.canonicalize().ok()),
            b.parent().and_then(|d| d.canonicalize().ok()),
        ) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        }
}

/// MBR's configuration for the books folder `root`: its own
/// `.mbr/config.toml` and `MBR_*` read at that root (no search upward), with
/// MBR's in-browser editing off. The invoicer writes the files through its
/// own API, where every write is token-gated and hash-checked.
fn mbr_config(root: &Path) -> Result<MbrConfig, mbr_core::errors::MbrError> {
    let mut config = MbrConfig::read_at_root(root)?;
    config.edit_enabled = false;
    Ok(config)
}

/// MBR's router over `root`: a [`LiveServer`], so a change to the folder's
/// `.mbr/config.toml` applies without a restart (read with the same
/// overrides).
fn mbr_router(root: &Path, gui: bool) -> Result<Router, String> {
    let config = mbr_config(root).map_err(|e| format!("MBR configuration: {e}"))?;
    let root = root.to_path_buf();
    let live = LiveServer::init(config, gui, Arc::new(move || mbr_config(&root)))
        .map_err(|e| format!("starting MBR: {e}"))?;
    Ok(live.router())
}
