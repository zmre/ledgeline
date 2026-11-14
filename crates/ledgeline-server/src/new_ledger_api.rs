//! File → New (plan 39 Phase 6): the routes the SPA's new-ledger wizard calls.
//!
//! # Who does what
//!
//! The wizard runs in the SPA, but two of its steps are things a web page must
//! not be able to do on its own: choose a folder anywhere on disk, and switch
//! which journal the app has open. Both belong to the native shell, which
//! plugs into this module through [`DesktopHost`] ([`AppState::with_desktop`]).
//!
//! 1. `POST /api/new-ledger/folder` asks the host to show the system folder
//!    picker and waits for the answer. The chosen path is kept HERE, under a
//!    random one-time id; the client gets the id and the folder's own name
//!    (`Books`), never the path. So the client cannot name a directory to write
//!    into — it can only point at the one the user just picked.
//! 2. `POST /api/new-ledger` renders the files ([`ledgeline_core::new_ledger`]),
//!    writes them into that folder (exclusive creates; a folder holding
//!    anything but dotfiles is refused), binds the editor to the new
//!    `main.journal` — so the very next request is served from it — and tells
//!    the host, which re-points its file watcher, records the journal in Open
//!    Recent and refreshes that menu. The SPA then navigates itself: to the
//!    journal, or into the import flow with a file it is still holding.
//! 3. `POST /api/new-ledger/sample` does the same with a copy of the sample
//!    journal, written into the app's cache folder.
//!
//! Without a host — the headless `--server`, a browser — every route that
//! writes or picks answers `501`, and the wizard says the desktop app is
//! needed. `GET /api/new-ledger` and the template listing work everywhere.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::response::Response;
use ledgeline_core::new_ledger::{
    self, ACCOUNTS_FILE, Currency, DecimalMark, LedgerKind, MAIN_FILE, NewLedger, NewLedgerError,
    SymbolSide, TemplateAccount,
};
use ledgeline_core::vault::LocalVault;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::edit_api::json_body;
use crate::error::AppError;
use crate::security::no_store_json;

/// The sample journal "Try with sample data" opens: `fixtures/sample.journal`,
/// carried in the binary because a packaged app has no `fixtures/`. A test
/// pins it to the fixture byte for byte.
pub const SAMPLE_JOURNAL: &str = include_str!("sample.journal");

/// [`SAMPLE_JOURNAL`] with every date moved by whole months so its newest
/// transaction falls in the month of `today` (`YYYY-MM-DD`): the default "last
/// 90 days" view opens on data, whenever the sample is opened. The desktop's
/// "Try with sample data" and the iOS app's sample ledger both write this.
#[must_use]
pub fn current_sample_journal(today: &str) -> String {
    ledgeline_core::date_shift::shift_into_month_of(SAMPLE_JOURNAL, today)
}

/// The folder under the cache directory the sample is written into, and the
/// file name it gets there.
const SAMPLE_DIR: &str = "sample";
const SAMPLE_FILE: &str = "sample.journal";

/// What the native shell does for the wizard. Implemented by the desktop GUI;
/// a fake in the tests.
pub trait DesktopHost: Send + Sync {
    /// Show the system folder picker and block until the user picks a folder
    /// (`Some`) or cancels (`None`). Called on a blocking thread.
    fn pick_folder(&self) -> Option<PathBuf>;

    /// The server has just bound its editor to the journal at `main` (a
    /// canonical path): watch it, remember it as the most recent journal, and
    /// update any menu that lists journals. Must not reload the page — the SPA
    /// navigates itself once the request that caused this returns.
    fn journal_opened(&self, main: &Path);
}

/// The host plus the folder it last picked.
pub(crate) struct Desktop {
    host: Arc<dyn DesktopHost>,
    /// The folder the user picked most recently and its one-time id. One
    /// slot: there is one wizard, and picking again replaces the choice.
    picked: Mutex<Option<PickedFolder>>,
}

impl Desktop {
    pub(crate) fn new(host: Arc<dyn DesktopHost>) -> Self {
        Self {
            host,
            picked: Mutex::new(None),
        }
    }

    fn picked(&self) -> std::sync::MutexGuard<'_, Option<PickedFolder>> {
        self.picked.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct PickedFolder {
    id: String,
    path: PathBuf,
}

// ===========================================================================
// Wire types
// ===========================================================================

/// `GET /api/new-ledger`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireStatus {
    /// Whether this server can pick a folder and open a journal — the desktop
    /// app. `false` in a browser on the headless server.
    desktop: bool,
    /// Whether a journal is open, so the wizard can offer to go back to it.
    journal_open: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TemplateQuery {
    kind: WireKind,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WireKind {
    Individual,
    Business,
    Empty,
}

impl From<WireKind> for LedgerKind {
    fn from(kind: WireKind) -> Self {
        match kind {
            WireKind::Individual => Self::Individual,
            WireKind::Business => Self::Business,
            WireKind::Empty => Self::Empty,
        }
    }
}

/// One tag on one account, as `{name, value}`; `value` is `""` for a
/// value-less tag such as `clearing:`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireTag {
    name: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireAccount {
    name: String,
    tags: Vec<WireTag>,
}

impl From<TemplateAccount> for WireAccount {
    fn from(account: TemplateAccount) -> Self {
        Self {
            name: account.name,
            tags: account
                .tags
                .into_iter()
                .map(|(name, value)| WireTag { name, value })
                .collect(),
        }
    }
}

impl From<WireAccount> for TemplateAccount {
    fn from(account: WireAccount) -> Self {
        Self {
            name: account.name,
            tags: account
                .tags
                .into_iter()
                .map(|tag| (tag.name, tag.value))
                .collect(),
        }
    }
}

/// `GET /api/new-ledger/template?kind=…`.
#[derive(Serialize)]
struct WireTemplate {
    accounts: Vec<WireAccount>,
}

/// `POST /api/new-ledger/folder`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePicked {
    /// `null` when the user cancelled the picker.
    folder: Option<WireFolder>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireFolder {
    /// The one-time handle `POST /api/new-ledger` names the folder by.
    id: String,
    /// The folder's own name — never its path.
    name: String,
    /// Whether it holds nothing but dotfiles, so the ledger can be written.
    empty: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WireSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WireMark {
    Period,
    Comma,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCurrency {
    commodity: String,
    side: WireSide,
    spaced: bool,
    decimals: u8,
    decimal_mark: WireMark,
}

impl From<WireCurrency> for Currency {
    fn from(currency: WireCurrency) -> Self {
        Self {
            commodity: currency.commodity,
            side: match currency.side {
                WireSide::Left => SymbolSide::Left,
                WireSide::Right => SymbolSide::Right,
            },
            spaced: currency.spaced,
            decimals: currency.decimals,
            decimal_mark: match currency.decimal_mark {
                WireMark::Period => DecimalMark::Period,
                WireMark::Comma => DecimalMark::Comma,
            },
        }
    }
}

/// `POST /api/new-ledger`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireCreate {
    folder_id: String,
    title: String,
    currency: WireCurrency,
    accounts: Vec<WireAccount>,
}

/// What `POST /api/new-ledger` and `/sample` wrote and opened. File NAMES,
/// relative to the folder; never a path.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCreated {
    folder: String,
    files: Vec<&'static str>,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `GET /api/new-ledger` — can this server create a ledger, and is one open?
pub(crate) async fn status(State(state): State<AppState>) -> Response {
    no_store_json(WireStatus {
        desktop: state.desktop().is_some(),
        journal_open: state.snapshot().storage().is_some(),
    })
}

/// `GET /api/new-ledger/template?kind=individual|business|empty` — the chart
/// of accounts the review step lists.
pub(crate) async fn template(
    query: Result<Query<TemplateQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let Query(query) =
        query.map_err(|rejection| AppError::BadRequest(format!("invalid query: {rejection}")))?;
    Ok(no_store_json(WireTemplate {
        accounts: new_ledger::chart(query.kind.into())
            .into_iter()
            .map(WireAccount::from)
            .collect(),
    }))
}

/// `POST /api/new-ledger/folder` — show the native folder picker.
pub(crate) async fn pick_folder(State(state): State<AppState>) -> Result<Response, AppError> {
    let desktop = state.desktop().ok_or_else(needs_desktop)?;
    let host = Arc::clone(&desktop.host);
    // Its own blocking thread, not a report slot: the picker waits on a
    // person, and must not hold up anyone's report while it does.
    let chosen = tokio::task::spawn_blocking(move || host.pick_folder())
        .await
        .map_err(|error| AppError::Internal(format!("the folder picker failed: {error}")))?;
    let Some(path) = chosen else {
        *desktop.picked() = None;
        return Ok(no_store_json(WirePicked { folder: None }));
    };
    let path = path.canonicalize().unwrap_or(path);
    let name = folder_name(&path);
    let empty = new_ledger::folder_is_empty(&LocalVault::new(&path)).map_err(|error| {
        AppError::BadRequest(format!("that folder can't be read: {}", quiet(&error)))
    })?;
    let id = one_time_id()?;
    *desktop.picked() = Some(PickedFolder {
        id: id.clone(),
        path,
    });
    Ok(no_store_json(WirePicked {
        folder: Some(WireFolder { id, name, empty }),
    }))
}

/// `POST /api/new-ledger` — write a new ledger into the picked folder and open it.
pub(crate) async fn create(
    State(state): State<AppState>,
    payload: Result<Json<WireCreate>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    state.desktop().ok_or_else(needs_desktop)?;
    // The journal write mutex: no import or edit may land in the journal being
    // swapped out from under it.
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let body = tokio::task::spawn_blocking(move || create_ledger(&state, request))
        .await
        .map_err(|error| AppError::Internal(format!("creating the ledger failed: {error}")))??;
    Ok(no_store_json(body))
}

/// `POST /api/new-ledger/sample` — open a fresh copy of the sample journal.
pub(crate) async fn sample(State(state): State<AppState>) -> Result<Response, AppError> {
    state.desktop().ok_or_else(needs_desktop)?;
    let guard = state.clone();
    let _write = guard.import_writes().lock().await;
    let body = tokio::task::spawn_blocking(move || open_sample(&state))
        .await
        .map_err(|error| AppError::Internal(format!("opening the sample failed: {error}")))??;
    Ok(no_store_json(body))
}

// ===========================================================================
// The work
// ===========================================================================

fn create_ledger(state: &AppState, request: WireCreate) -> Result<WireCreated, AppError> {
    let desktop = state.desktop().ok_or_else(needs_desktop)?;
    // The folder the id names. Looked up, not taken: a refusal below (a bad
    // name, a folder that has filled up since) leaves the choice in place so
    // the user can fix the answer and try again. It is spent only once a
    // ledger is written, so an id is good for exactly one.
    let folder = desktop
        .picked()
        .as_ref()
        .filter(|folder| folder.id == request.folder_id)
        .map(|folder| folder.path.clone())
        .ok_or_else(|| {
            AppError::Conflict(
                "that folder choice has expired; choose the folder again".to_string(),
            )
        })?;
    let ledger = NewLedger {
        title: request.title,
        currency: request.currency.into(),
        accounts: request
            .accounts
            .into_iter()
            .map(TemplateAccount::from)
            .collect(),
    };
    let files = new_ledger::render(&ledger).map_err(refused)?;
    let vault = LocalVault::new(&folder);
    new_ledger::write_ledger(&vault, &files).map_err(refused)?;
    *desktop.picked() = None;
    open(state, desktop, &vault.native(&vault_main()?))?;
    Ok(WireCreated {
        folder: folder_name(&folder),
        files: vec![MAIN_FILE, ACCOUNTS_FILE],
    })
}

fn open_sample(state: &AppState) -> Result<WireCreated, AppError> {
    let desktop = state.desktop().ok_or_else(needs_desktop)?;
    let base = state
        .app_dirs()
        .cache_dir
        .clone()
        .unwrap_or_else(|| state.app_dirs().temp_dir.clone());
    let dir = base.join(SAMPLE_DIR);
    let path = dir.join(SAMPLE_FILE);
    // The app's own copy, so it is simply rewritten: "Try with sample data"
    // always starts from the pristine sample, moved up to this month.
    let text = current_sample_journal(&crate::reports_api::today_utc());
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, text))
        .map_err(|error| {
            AppError::Internal(format!("the sample could not be written: {}", error.kind()))
        })?;
    open(state, desktop, &path)?;
    Ok(WireCreated {
        folder: SAMPLE_DIR.to_string(),
        files: vec![SAMPLE_FILE],
    })
}

/// Bind the editor to `main` and tell the host. The editor first, so the
/// host is never told about a journal that failed to open.
fn open(state: &AppState, desktop: &Desktop, main: &Path) -> Result<(), AppError> {
    let main = main.canonicalize().unwrap_or_else(|_| main.to_path_buf());
    state.rebind_editor(&main).map_err(|error| {
        // The parse error names the file by its full path; say what happened
        // without it.
        eprintln!("ledgeline: could not open {}: {error}", main.display());
        AppError::Internal(
            "the new journal was written but could not be opened; open it with File \u{2192} Open"
                .to_string(),
        )
    })?;
    desktop.host.journal_opened(&main);
    Ok(())
}

fn vault_main() -> Result<ledgeline_core::vault::VaultPath, AppError> {
    ledgeline_core::vault::VaultPath::root()
        .child(MAIN_FILE)
        .map_err(|error| AppError::Internal(error.to_string()))
}

/// A user-facing refusal for a [`NewLedgerError`], with no path in it.
fn refused(error: NewLedgerError) -> AppError {
    match error {
        NewLedgerError::Io(_) => AppError::Internal(format!(
            "the new ledger could not be written ({}); nothing was left behind",
            quiet(&error)
        )),
        NewLedgerError::FolderNotEmpty => AppError::Conflict(error.to_string()),
        NewLedgerError::Unreadable(_) => AppError::Internal(error.to_string()),
        _ => AppError::BadRequest(error.to_string()),
    }
}

/// An I/O failure described by its kind alone: a [`ledgeline_core::vault::VaultError`]'s
/// text can carry the folder's path.
fn quiet(error: &NewLedgerError) -> String {
    match error {
        NewLedgerError::Io(io) => io.kind().to_string(),
        other => other.to_string(),
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "the folder".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn one_time_id() -> Result<String, AppError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| AppError::Internal(format!("no randomness for a folder id: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn needs_desktop() -> AppError {
    AppError::EditingDisabled(
        "creating a new ledger needs the Ledgeline desktop app, which can show a folder picker"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_name_is_the_last_component_only() {
        assert_eq!(folder_name(Path::new("/Users/me/Books")), "Books");
        assert_eq!(folder_name(Path::new("/")), "the folder");
    }

    #[test]
    fn one_time_ids_are_unique_hex() {
        let a = one_time_id().unwrap();
        let b = one_time_id().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
