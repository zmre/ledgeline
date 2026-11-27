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
use invoicer_core::edit;
use invoicer_core::reconcile::{self, ReconcileOptions, Reconciled};
use invoicer_core::session::{OpenError, Session};
use invoicer_core::settings::{Settings, SettingsError};
use invoicer_core::time::{TimeBooks, TimeOpError};
use ledgeline_core::accrual::books_config;
use ledgeline_core::time::book::TimePaths;
use ledgeline_core::time::model::TimeBook;
use ledgeline_core::time::timeclock::Moment;
use ledgeline_core::vault::{Vault, VaultPath};
use mbr_core::Config as MbrConfig;
use mbr_core::raw_html::RawHtmlPolicy;
use mbr_core::standalone::Renderer;
use mbr_server::{LiveServer, Server};
use serde::Serialize;
use tokio::sync::broadcast;

use super::APP;
use super::preview::DraftPreviews;
use crate::starters;

/// The settings file beside the main journal: `[books]` and `[invoicer]`.
const SETTINGS_FILE: &str = "ledgeline.toml";

/// How many change events a slow listener may fall behind by before it is
/// told only that something changed.
const CHANGE_BACKLOG: usize = 64;

/// One settled change on disk, after the books were re-read and reconciled:
/// what `GET /api/events` sends the SPA, so it reloads what it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Changed {
    /// The files that changed, relative to the books folder (a journal
    /// outside it by its file name): never a full local path. Empty when a
    /// listener fell behind and missed some.
    pub paths: Vec<String>,
}

/// An open books folder.
pub(crate) struct OpenBooks {
    /// The main journal, canonical.
    journal: PathBuf,
    vault: Arc<dyn Vault>,
    main: VaultPath,
    session: Mutex<Session>,
    /// MBR's router over the books folder.
    mbr: Router,
    /// MBR over the books folder. The last holder dropped (this, or a
    /// router still answering a request) stops its background tasks.
    live: Arc<LiveServer>,
    /// The preview's renderer (`POST /api/preview`), built on first use and
    /// again whenever MBR's server is rebuilt (a `.mbr/config.toml` change),
    /// paired with the server it was built for.
    renderer: Mutex<Option<(Arc<Server>, Arc<Renderer>)>>,
    /// Change events, sent once each settled change has been applied
    /// ([`OpenBooks::apply_changes`]). Dropped with the books, which ends
    /// every listener's stream (File → Open: the SPA reconnects to the new
    /// books).
    changes: broadcast::Sender<Changed>,
    /// The editor's unsaved previews (invoicer PLAN 4a).
    drafts: DraftPreviews,
    /// The time book (plan 46), read on first use and again after any
    /// change to a time file ([`Self::forget_time`]); and the tracker's last
    /// stop, which a timedot file doesn't record (snap, T-slots).
    time: Mutex<TimeCache>,
    /// Serializes this app's time writes: each re-reads and re-applies on a
    /// change on disk, but two of ours needn't race.
    time_writes: Mutex<()>,
}

/// [`OpenBooks`]' time state.
#[derive(Default)]
pub(crate) struct TimeCache {
    /// The book as last read, with the settings it was read under.
    book: Option<(Arc<TimeBook>, TimePaths)>,
    /// When this app last stopped a timer, and which, per person (`""`:
    /// none).
    pub(crate) last_stop: std::collections::BTreeMap<String, (Moment, String)>,
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
    pub(crate) fn open(journal: &Path, gui: bool) -> Result<Self, OpenFailure> {
        let journal = journal
            .canonicalize()
            .map_err(|e| OpenFailure::other(format!("{}: {e}", journal.display())))?;
        let session = Session::open_local(&journal).map_err(|e| OpenFailure::of(&e))?;
        let vault = session.shared_vault();
        let main = session.main().clone();
        let live = mbr_live(vault.root(), gui).map_err(OpenFailure::other)?;
        Ok(Self {
            journal,
            vault,
            main,
            session: Mutex::new(session),
            mbr: live.router(),
            live,
            renderer: Mutex::new(None),
            changes: broadcast::channel(CHANGE_BACKLOG).0,
            drafts: DraftPreviews::default(),
            time: Mutex::new(TimeCache::default()),
            time_writes: Mutex::new(()),
        })
    }

    /// MBR's standalone renderer over the books folder, for previews of
    /// unsaved text (MBR-CHANGES M3). One renderer serves every preview: it
    /// loads MBR's templates once and never rescans the folder; its body
    /// wikilinks resolve against the live server's index, which MBR's
    /// watcher keeps current.
    ///
    /// # Errors
    /// When MBR's configuration or templates can't be read.
    pub(crate) fn renderer(&self) -> Result<Arc<Renderer>, String> {
        let server = self.live.current();
        let mut slot = self.renderer.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((built_for, renderer)) = slot.as_ref()
            && Arc::ptr_eq(built_for, &server)
        {
            return Ok(Arc::clone(renderer));
        }
        let config =
            mbr_config(self.vault.root()).map_err(|e| format!("MBR configuration: {e}"))?;
        let renderer = Arc::new(
            Renderer::new(&config)
                .map_err(|e| format!("MBR templates: {e}"))?
                .with_wikilink_index(Arc::clone(&server.repo().wikilink_index)),
        );
        *slot = Some((server, Arc::clone(&renderer)));
        Ok(renderer)
    }

    /// MBR's `[pdf]` settings for the books folder (paper, margins,
    /// timeout), read afresh: a template's own `@page` still wins.
    ///
    /// # Errors
    /// When MBR's configuration can't be read.
    pub(crate) fn pdf_config(&self) -> Result<mbr_core::pdf_config::PdfConfig, String> {
        mbr_config(self.vault.root())
            .map(|config| config.pdf)
            .map_err(|e| format!("MBR configuration: {e}"))
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

    /// The editor's unsaved previews.
    pub(crate) fn drafts(&self) -> &DraftPreviews {
        &self.drafts
    }

    /// MBR's router for this folder.
    pub(crate) fn mbr(&self) -> &Router {
        &self.mbr
    }

    /// MBR's note index for this folder, as its library façade: the same
    /// scan and backlink index its pages use (MBR-CHANGES M1, M2).
    pub(crate) fn mbr_site(&self) -> mbr_core::site::Site {
        self.live.current().site()
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

    /// Copy in the starter templates invoices name by `template:` but the
    /// books don't have yet (on open; [`crate::starters::install_referenced`]).
    pub(crate) fn install_starters(&self) -> Vec<VaultPath> {
        let mut session = self.session();
        install_starters_in(&mut session, None)
    }

    /// The time book's settings and paths, read now (`ledgeline.toml
    /// [time]` may have changed).
    ///
    /// # Errors
    /// When `[time]` or a configured path is unusable.
    pub(crate) fn time_books(&self) -> Result<TimeBooks, TimeOpError> {
        TimeBooks::open(Arc::clone(&self.vault), self.main.clone())
    }

    /// The time book, from the cache when it was read under the same
    /// paths; `None` before setup.
    ///
    /// # Errors
    /// When the roll-up doesn't parse.
    pub(crate) fn time_book(
        &self,
        books: &TimeBooks,
    ) -> Result<Option<Arc<TimeBook>>, TimeOpError> {
        let mut cache = self.time.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((book, paths)) = &cache.book
            && *paths == books.paths
        {
            return Ok(Some(Arc::clone(book)));
        }
        let book = books.load()?.map(Arc::new);
        cache.book = book
            .as_ref()
            .map(|book| (Arc::clone(book), books.paths.clone()));
        Ok(book)
    }

    /// Drop the cached time book: a time file changed (on disk, or by
    /// this app's own write).
    pub(crate) fn forget_time(&self) {
        self.time
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .book = None;
    }

    /// The time state, locked (the tracker's last stop).
    pub(crate) fn time_state(&self) -> MutexGuard<'_, TimeCache> {
        self.time.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hold while writing time files.
    pub(crate) fn time_writes(&self) -> MutexGuard<'_, ()> {
        self.time_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `path` (a local path the watcher reported) is part of the
    /// time book: a `.timedot`/`.timeclock` file anywhere under the folder,
    /// or the roll-up, the accounts file or any file the book read.
    fn is_time_file(&self, path: &Path) -> bool {
        if path
            .extension()
            .is_some_and(|ext| ext == "timedot" || ext == "timeclock")
        {
            return true;
        }
        let Ok(vault_path) = self.vault.vault_path(path) else {
            return false;
        };
        let cache = self.time.lock().unwrap_or_else(PoisonError::into_inner);
        cache.book.as_ref().is_some_and(|(book, paths)| {
            vault_path == paths.rollup
                || vault_path == paths.accounts
                || book.sources.iter().any(|s| s.path == vault_path)
        }) || TimeBooks::open(Arc::clone(&self.vault), self.main.clone()).is_ok_and(|books| {
            vault_path == books.paths.rollup || vault_path == books.paths.accounts
        })
    }

    /// Listen for change events ([`Changed`]).
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Changed> {
        self.changes.subscribe()
    }

    /// React to the files `changed` on disk (a settled watcher burst):
    /// [`Self::reload_changed`], then tell every listener (the SPA's
    /// `GET /api/events`) which of the files it shows changed. PDFs and
    /// other files the app doesn't read are no event.
    pub(crate) fn apply_changes(&self, changed: &[PathBuf]) -> Option<Reconciled> {
        // The time book follows the roll-up and every file it reads (plan
        // 46 Phase 2), as the recognition file is followed.
        let time: BTreeSet<&PathBuf> = changed.iter().filter(|p| self.is_time_file(p)).collect();
        let settings_file = self.vault.root().join(SETTINGS_FILE);
        if !time.is_empty() || changed.iter().any(|p| same_file(p, &settings_file)) {
            self.forget_time();
        }
        let report = self.reload_changed(changed);
        let journal_names: BTreeSet<_> = self
            .session()
            .journal_files()
            .iter()
            .filter_map(|f| f.file_name().map(ToOwned::to_owned))
            .collect();
        let settings = self.vault.root().join(SETTINGS_FILE);
        let mut paths: Vec<String> = changed
            .iter()
            .filter(|p| {
                is_markdown(p)
                    || is_html(p)
                    || same_file(p, &settings)
                    || p.file_name().is_some_and(|n| journal_names.contains(n))
                    || time.contains(p)
            })
            .filter_map(|p| match self.vault.vault_path(p) {
                Ok(path) => Some(path.to_string()),
                Err(_) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
            })
            .collect();
        paths.sort();
        paths.dedup();
        if !paths.is_empty() {
            // No listener is no error.
            let _ = self.changes.send(Changed { paths });
        }
        report
    }

    /// Re-read the files `changed` on disk, then reconcile the invoices
    /// they touched.
    ///
    /// - a markdown or `.html` file under the folder: the document index
    ///   refreshes it (an `.html` file may be a template); a draft whose
    ///   amounts no longer match its rows is recalculated
    ///   (`recalc-on-save`), and a starter template a document names is
    ///   copied in;
    /// - a journal file: the journal and `[books]` are read again, and the
    ///   invoices whose payment state moved are reconciled;
    /// - `ledgeline.toml`: when `[invoicer]` changed, the session is opened
    ///   afresh (the index and its caches with it) and every invoice is
    ///   reconciled; when only `[books]` did, the journal is read again and
    ///   every invoice reconciled; when neither did (the Settings page's own
    ///   Save echoing back, a theme pin, a bank link), nothing. A file that
    ///   no longer reads keeps the books as they were (the Settings page
    ///   says why).
    ///
    /// Returns the reconciler's report when it ran.
    fn reload_changed(&self, changed: &[PathBuf]) -> Option<Reconciled> {
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
            // Our own Save echoes back, and a theme pin or a bank link
            // changes another table: nothing to redo when `[invoicer]` and
            // `[books]` read as before.
            let fresh_settings = Settings::read(&*self.vault);
            let fresh_books = books_config::read(&*self.vault, &self.main);
            let same_settings = fresh_settings.as_ref().ok() == Some(session.settings());
            let same_books = fresh_books.as_ref().ok() == Some(session.books_config());
            if same_settings && same_books {
                return None;
            }
            if same_settings {
                // `[books]` alone (a basis, closed-through): the journal's
                // roles and the payment state, not the index.
                return match session.reload_books() {
                    Ok(_) => reconcile_in(&mut session, None),
                    Err(error) => {
                        eprintln!("{}: reload skipped: {error}", APP.log_prefix);
                        None
                    }
                };
            }
            return match reopen(&mut session, &self.vault, &self.main) {
                Ok(report) => report,
                Err(error) => {
                    // The page reads the file itself, so the change event
                    // (sent by the caller) is what brings it the problem.
                    eprintln!("{}: reload skipped: {error}", APP.log_prefix);
                    None
                }
            };
        }

        let mut numbers: BTreeSet<String> = BTreeSet::new();
        // Markdown notes, and `.html` files (templates: `/api/templates`
        // lists what the index's scan found).
        let markdown: Vec<VaultPath> = changed
            .iter()
            .filter(|p| is_markdown(p) || is_html(p))
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
            // An outside edit to a draft's rows: its amounts follow
            // (`[invoicer] recalc-on-save`, drafts only). Its own write comes
            // back as a change whose amounts agree, so it ends there.
            for path in &refreshed.updated {
                match edit::recalc_on_save(index, vault, path) {
                    Ok(Some(done)) => eprintln!(
                        "{}: {path}: recalculated {} amount(s) after an edit",
                        APP.log_prefix,
                        done.changes.len()
                    ),
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("{}: {path}: not recalculated: {error}", APP.log_prefix)
                    }
                }
            }
            // A `template:` naming a starter not copied in yet (a hand edit).
            install_starters_in(&mut session, Some(&refreshed.updated));
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

/// Why the books couldn't open.
#[derive(Debug, Clone)]
pub(crate) struct OpenFailure {
    /// The error, for the log and the open dialog.
    pub message: String,
    /// The problem in the books' `ledgeline.toml` (`[invoicer]` or
    /// `[books]`), in plain words, when that is why: the app then serves
    /// the Settings page over them anyway.
    pub in_settings: Option<String>,
}

impl OpenFailure {
    fn other(message: String) -> Self {
        Self {
            message,
            in_settings: None,
        }
    }

    fn of(error: &OpenError) -> Self {
        let in_settings = match error {
            OpenError::Settings(SettingsError::Invalid { message, .. }) => Some(format!(
                "the invoicer settings in ledgeline.toml have a problem. {message}"
            )),
            OpenError::Settings(SettingsError::Unreadable(_)) => {
                Some("ledgeline.toml can't be read.".to_string())
            }
            OpenError::Books(error) => Some(error.to_string()),
            OpenError::Path { .. } | OpenError::Journal { .. } => None,
        };
        Self {
            message: error.to_string(),
            in_settings,
        }
    }
}

impl std::fmt::Display for OpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<OpenFailure> for String {
    fn from(failure: OpenFailure) -> Self {
        failure.message
    }
}

/// Open `session`'s books afresh (`[invoicer]` changed: the index is built
/// with the new settings, its caches with it), then copy in the starters
/// documents name and reconcile every invoice. The session is unchanged
/// when they don't open.
///
/// # Errors
/// Why the books don't open with the file as it is now.
pub(crate) fn reopen(
    session: &mut Session,
    vault: &Arc<dyn Vault>,
    main: &VaultPath,
) -> Result<Option<Reconciled>, OpenError> {
    *session = Session::open(Arc::clone(vault), main.clone())?;
    let _ = session.book_states();
    install_starters_in(session, None);
    Ok(reconcile_in(session, None))
}

/// Read the journal again after this app wrote it (Send, Void, …), so the
/// next request (the Books panel, the invoice's checks) sees the booking.
///
/// The books bridge writes through its own copy of the session's journal
/// editor, so the session still holds the journal as it was; the watcher
/// would catch up after its debounce, but the SPA asks again at once. A
/// no-op when the journal is as the session read it.
pub(crate) fn reload_after_write(session: &mut Session) {
    crate::flows::reload_logged(session);
}

/// Log what a books write did to `number`'s paid mirror ([`crate::flows`]
/// refreshed it), as [`reconcile_in`] logs.
pub(crate) fn log_mirror(report: Option<&Reconciled>) {
    if let Some(report) = report {
        log_reconciled(report);
    }
}

/// [`starters::install_referenced`] over `only` (or every document),
/// logging what it copied. Returns the files copied.
fn install_starters_in(session: &mut Session, only: Option<&[VaultPath]>) -> Vec<VaultPath> {
    let (copied, failed) = starters::install_referenced(session, only);
    for path in &copied {
        eprintln!(
            "{}: copied the starter template to {path} (an invoice names it)",
            APP.log_prefix
        );
    }
    for (file, error) in &failed {
        eprintln!(
            "{}: the starter template {file} wasn't copied: {error}",
            APP.log_prefix
        );
    }
    copied
}

/// Reconcile `only` (or every sent invoice), logging the outcome.
pub(crate) fn reconcile_in(session: &mut Session, only: Option<Vec<String>>) -> Option<Reconciled> {
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
    log_reconciled(&report);
    Some(report)
}

/// Each mirror a reconcile changed, or couldn't write, on stderr.
fn log_reconciled(report: &Reconciled) {
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
}

/// Whether `path` is a markdown note (by extension, as MBR's default).
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

/// Whether `path` is an HTML file (a template, perhaps).
fn is_html(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
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
/// `.mbr/config.toml` and `MBR_*` read at that root (no search upward), then
/// what the invoicer decides whatever those say:
///
/// - MBR's in-browser editing is off. The invoicer writes the files through
///   its own API, where every write is token-gated and hash-checked.
/// - Raw HTML in a note is at least sanitized ([`RAW_HTML_FLOOR`]).
/// - MBR's `Content-Security-Policy` is the invoicer's
///   ([`super::mbr::content_security_policy`]).
fn mbr_config(root: &Path) -> Result<MbrConfig, mbr_core::errors::MbrError> {
    let mut config = MbrConfig::read_at_root(root)?;
    config.edit_enabled = false;
    config.raw_html = config.raw_html.strictest(RAW_HTML_FLOOR);
    config.content_security_policy = super::mbr::content_security_policy();
    Ok(config)
}

/// The least MBR does with raw HTML in a note (SEC-1): a pasted timesheet
/// cell, a shared customer note or an invoice body can hold `<script>` or
/// `<img onerror=…>`. `sanitize` keeps formatting (`<br>` in a table cell,
/// `<details>`) and drops scripts, handlers, `style`, frames, forms and
/// script URLs; a books folder can ask for `escape` in its
/// `.mbr/config.toml` (`raw_html = "escape"`), never for less.
pub(crate) const RAW_HTML_FLOOR: RawHtmlPolicy = RawHtmlPolicy::Sanitize;

/// MBR over `root`: a [`LiveServer`], so a change to the folder's
/// `.mbr/config.toml` applies without a restart (read with the same
/// overrides).
fn mbr_live(root: &Path, gui: bool) -> Result<Arc<LiveServer>, String> {
    let config = mbr_config(root).map_err(|e| format!("MBR configuration: {e}"))?;
    let root = root.to_path_buf();
    LiveServer::init(config, gui, Arc::new(move || mbr_config(&root)))
        .map_err(|e| format!("starting MBR: {e}"))
}
