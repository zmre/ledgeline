//! The iOS app's core: an open journal, served in-process to the app's web
//! view.
//!
//! The app renders the same SvelteKit SPA the desktop embeds, in a
//! `WKWebView` whose `ledgeline://localhost/…` requests go to a Swift
//! `WKURLSchemeHandler`. That handler passes each request to
//! [`LedgelineApp::handle`], which runs it through Ledgeline's own axum router
//! — the desktop server minus its listener — so every wire route, every
//! `/api` route and the SPA shell itself work unchanged.
//!
//! A [`LedgelineApp`] is one open journal:
//!
//! - **Storage**: a [`CoordinatedVault`] over the granted folder — reads in
//!   Rust on `std::fs`, writes and download requests through the app's
//!   [`FileCoordinator`], not-downloaded files reported as such — or a
//!   [`SingleFileVault`] for one file opened in place
//!   ([`LedgelineApp::open_file`]).
//! - **App state**: [`AppDirs`] under the app's `Caches/` directory — the
//!   staging area for dropped statements, and room for a cache. No config
//!   directory: nothing in the preferences store applies to a build with no
//!   external tools, and recents are the app's own bookmarks
//!   ([`ledgeline::recents`] stores a bookmark id as an opaque token if the
//!   app wants Ledgeline to keep them).
//! - **Threads**: its own small tokio runtime, every thread failing fast on
//!   dataless files ([`crate::iopolicy`]).
//! - **Access**: the bearer token is still required
//!   ([`ledgeline::Security::in_process`]). The app hands it to its own pages
//!   with [`LedgelineApp::token_init_script`] as a document-start
//!   `WKUserScript`, exactly as the desktop WebView gets it.
//! - **Freshness**: no watcher. [`LedgelineApp::refresh`] re-checks the
//!   journal's files and reloads only when one changed; the app calls it on
//!   foreground and pull-to-refresh. The SPA's own 30 s poll then picks the
//!   new snapshot up (or the app reloads the page).
//!
//! Everything compiles on every platform with the `mobile` feature; the
//! UniFFI exports and the dataless-file handling are Apple-only.

mod runtime;
mod tracking;
mod transport;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use ledgeline::{AccessToken, AppDirs, AppState, Refreshed, Security, router_with_security};
use ledgeline_core::EditError;
use ledgeline_core::vault::{Availability, Vault, VaultPath};

pub use transport::{Header, LedgelineResponse};

use crate::coordinated_vault::CoordinatedVault;
use crate::coordinator::FileCoordinator;
use crate::single_file::SingleFileVault;
use runtime::BackgroundRuntime;
use tracking::TrackingVault;

/// The origin the app must load the SPA from. Any origin works for routing,
/// but [`LedgelineApp::token_init_script`] hands the token only to this one
/// unless told otherwise.
pub const MOBILE_ORIGIN: &str = "ledgeline://localhost";

/// Errors crossing the FFI boundary.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Error))]
pub enum MobileError {
    /// The journal could not be opened (bad path, unreadable, does not
    /// parse, runtime).
    #[error("could not open: {message}")]
    Open { message: String },
    /// Files the journal needs are listed but not on this device. Their
    /// download has been requested; open (or refresh) again once they land.
    /// `paths` are relative to the vault root.
    #[error("not downloaded yet: {}", paths.join(", "))]
    NotDownloaded { paths: Vec<String> },
    /// A file opened on its own `include`s other files, which are outside the
    /// grant. Ask the user for the folder and [`LedgelineApp::open`] it.
    #[error("{message}")]
    NeedsFolder { message: String },
    /// The request could not be turned into an HTTP request.
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },
    /// A background task or a response body failed.
    #[error("{message}")]
    Internal { message: String },
}

fn open_error(message: impl std::fmt::Display) -> MobileError {
    MobileError::Open {
        message: message.to_string(),
    }
}

fn internal(message: impl std::fmt::Display) -> MobileError {
    MobileError::Internal {
        message: message.to_string(),
    }
}

/// What a [`LedgelineApp::refresh`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Enum))]
pub enum RefreshOutcome {
    /// Every file still holds the bytes last read; nothing was reparsed.
    Unchanged,
    /// The journal was re-read and the snapshot republished: reload the page
    /// (or let the SPA's poll notice).
    Reloaded,
    /// Some of the journal's files are not on the device; their download was
    /// requested and nothing was reloaded. Refresh again shortly.
    Pending,
    /// The journal no longer reads cleanly (see
    /// [`RefreshSummary::message`]); the last good snapshot is still served.
    Failed,
}

/// The result of a [`LedgelineApp::refresh`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Record))]
pub struct RefreshSummary {
    pub outcome: RefreshOutcome,
    /// Vault-relative paths found not downloaded (their download requested).
    pub not_downloaded: Vec<String>,
    /// Why the refresh failed, for [`RefreshOutcome::Failed`].
    pub message: Option<String>,
    /// Whether the write endpoints are live after this refresh.
    pub editable: bool,
    pub duration_ms: u64,
}

/// An open journal. See the [module docs](self).
#[cfg_attr(target_vendor = "apple", derive(uniffi::Object))]
pub struct LedgelineApp {
    state: AppState,
    router: Router,
    token: AccessToken,
    vault: Arc<TrackingVault>,
    main: VaultPath,
    single_file: bool,
    /// Declared last so it is dropped last: the router's tasks run on it.
    runtime: BackgroundRuntime,
}

#[cfg_attr(target_vendor = "apple", uniffi::export)]
impl LedgelineApp {
    /// Open the journal `main_file` inside the folder at `root_path` — the
    /// path of the security-scoped URL the folder picker returned, accessed
    /// for as long as this lives.
    ///
    /// - `main_file`: the journal, relative to `root_path` (`2026.journal`,
    ///   `books/main.journal`). Its `include`s resolve inside `root_path`.
    /// - `cache_dir`: the app's `Caches/` directory (or a subfolder); the
    ///   staging area for dropped statements lives under it. `None` uses the
    ///   process temp directory.
    /// - `coordinator`: performs writes and downloads; `None` writes with
    ///   plain `std::fs` (only right for the app's own container).
    ///
    /// # Errors
    /// [`MobileError::NotDownloaded`] when a file the journal needs is not on
    /// the device (its download is requested); [`MobileError::Open`] for any
    /// other failure.
    #[cfg_attr(target_vendor = "apple", uniffi::constructor)]
    pub fn open(
        root_path: String,
        main_file: String,
        cache_dir: Option<String>,
        coordinator: Option<Arc<dyn FileCoordinator>>,
    ) -> Result<Arc<Self>, MobileError> {
        let root = canonical_dir(Path::new(&root_path))?;
        let main = VaultPath::new(main_file.trim_start_matches('/'))
            .map_err(|e| open_error(format!("{main_file}: {e}")))?;
        let vault: Arc<dyn Vault> = Arc::new(CoordinatedVault::new(root, coordinator));
        Self::with_vault(vault, main, mobile_dirs(cache_dir.as_deref()), false)
    }

    /// Open one journal file in place — what iOS grants for a file opened
    /// from Files or the share sheet, and all a provider that cannot grant
    /// folders offers. Only that file is visible ([`SingleFileVault`]): its
    /// `include`s are unavailable, so a journal that has any fails with
    /// [`MobileError::NeedsFolder`]. Granting the folder later means opening
    /// it with [`LedgelineApp::open`] and dropping this.
    ///
    /// # Errors
    /// As [`LedgelineApp::open`], plus [`MobileError::NeedsFolder`].
    #[cfg_attr(target_vendor = "apple", uniffi::constructor)]
    pub fn open_file(
        path: String,
        coordinator: Option<Arc<dyn FileCoordinator>>,
    ) -> Result<Arc<Self>, MobileError> {
        let given = Path::new(&path);
        let file = given.canonicalize().unwrap_or_else(|_| given.to_path_buf());
        let (Some(parent), Some(name)) = (file.parent(), file.file_name().and_then(|n| n.to_str()))
        else {
            return Err(open_error(format!("no folder or name: {}", file.display())));
        };
        let single = SingleFileVault::new(CoordinatedVault::new(parent, coordinator), name)
            .map_err(open_error)?;
        let main = single.file().clone();
        Self::with_vault(Arc::new(single), main, mobile_dirs(None), true)
    }

    /// The vault's root folder (canonical).
    #[must_use]
    pub fn root_path(&self) -> String {
        self.vault.root().to_string_lossy().into_owned()
    }

    /// The journal's main file, relative to [`Self::root_path`].
    #[must_use]
    pub fn main_file(&self) -> String {
        self.main.to_string()
    }

    /// Whether this is a single file opened in place.
    #[must_use]
    pub fn is_single_file(&self) -> bool {
        self.single_file
    }

    /// Whether the write endpoints are live (an editor is bound).
    #[must_use]
    pub fn is_editable(&self) -> bool {
        self.state.editing_enabled()
    }

    /// The bearer token every wire and `/api` request must carry. Per open
    /// journal, never persisted.
    #[must_use]
    pub fn access_token(&self) -> String {
        self.token.as_str().to_owned()
    }

    /// The document-start script that hands the token to pages on `origin`
    /// (default [`MOBILE_ORIGIN`]) and to no other — install it as a
    /// `WKUserScript` (`.atDocumentStart`, main frame only).
    #[must_use]
    pub fn token_init_script(&self, origin: Option<String>) -> String {
        ledgeline::token_init_script(origin.as_deref().unwrap_or(MOBILE_ORIGIN), &self.token)
    }

    /// Handle one request from the web view.
    ///
    /// `url` is the full `ledgeline://localhost/path?query` URL; `body` is
    /// empty for requests without one. The status and headers are final when
    /// this returns; read the body with [`LedgelineResponse::next_chunk`].
    ///
    /// # Errors
    /// [`MobileError::InvalidRequest`] when the request cannot be built;
    /// [`MobileError::Internal`] if the runtime is gone. Every HTTP-level
    /// failure (401, 409, 503 …) is a response, not an error.
    pub async fn handle(
        &self,
        method: String,
        url: String,
        headers: Vec<Header>,
        body: Vec<u8>,
    ) -> Result<Arc<LedgelineResponse>, MobileError> {
        let request = transport::build_request(&method, &url, &headers, body)?;
        let router = self.router.clone();
        let runtime = self.runtime.handle().clone();
        // Spawned rather than awaited in place: the caller's executor is
        // UniFFI's (driven from Swift), not tokio's, and the router needs
        // tokio. A `JoinHandle` can be awaited from any executor.
        self.on_runtime(async move {
            let response = transport::dispatch(router, request).await;
            Arc::new(LedgelineResponse::from_response(response, &runtime))
        })
        .await
    }

    /// Pick up changes made outside the app — on foreground, pull-to-refresh,
    /// or after a provider finished a download. Cheap when nothing changed:
    /// each file is read and compared by content, nothing is reparsed.
    ///
    /// Not-downloaded files are checked for FIRST (one stat each), so an
    /// evicted file is reported as [`RefreshOutcome::Pending`] — and its
    /// download requested — without disturbing the open editor. When an
    /// earlier failure left the journal read-only, a clean refresh binds a
    /// fresh editor again.
    ///
    /// # Errors
    /// [`MobileError::Internal`] only if the runtime is gone; a journal that
    /// fails to reload is [`RefreshOutcome::Failed`], with the last good
    /// snapshot still served.
    pub async fn refresh(&self) -> Result<RefreshSummary, MobileError> {
        let state = self.state.clone();
        let vault = Arc::clone(&self.vault);
        let main = self.main.clone();
        self.on_runtime(async move {
            tokio::task::spawn_blocking(move || refresh_now(&state, &vault, &main)).await
        })
        .await?
        .map_err(internal)
    }
}

impl LedgelineApp {
    /// Open `main` in an arbitrary `vault` — the constructor every other one
    /// funnels into, and how the tests drive the app over a
    /// [`MemVault`](ledgeline_core::vault::MemVault). Not exported: Swift opens
    /// folders and files, never a Rust vault.
    ///
    /// # Errors
    /// As [`LedgelineApp::open`]; `single_file` additionally turns a missing
    /// `include` into [`MobileError::NeedsFolder`].
    pub fn with_vault(
        vault: Arc<dyn Vault>,
        main: VaultPath,
        app_dirs: AppDirs,
        single_file: bool,
    ) -> Result<Arc<Self>, MobileError> {
        let runtime = runtime::build().map_err(|e| open_error(format!("tokio runtime: {e}")))?;
        let tracking = Arc::new(TrackingVault::new(vault));
        // The parse reads the whole journal, so it runs on a runtime thread:
        // those fail fast on a dataless file instead of blocking on its
        // download (the caller's thread is Swift's, with no such policy).
        let opened = {
            let vault: Arc<dyn Vault> = Arc::clone(&tracking) as Arc<dyn Vault>;
            let main = main.clone();
            runtime
                .block_on(runtime.spawn_blocking(move || AppState::from_vault(vault, main)))
                .map_err(|e| open_error(format!("open task: {e}")))?
        };
        let state = match opened {
            Ok(state) => state.with_app_dirs(app_dirs),
            Err(error) => return Err(open_failure(&tracking, &main, &error, single_file)),
        };
        tracking.reset();
        let token = AccessToken::generate().map_err(open_error)?;
        let router = router_with_security(state.clone(), Security::in_process(token.clone()));
        Ok(Arc::new(Self {
            state,
            router,
            token,
            vault: tracking,
            main,
            single_file,
            runtime: BackgroundRuntime::new(runtime),
        }))
    }

    /// The served state, for a Rust host that wants more than HTTP.
    #[must_use]
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Run `future` on this journal's runtime and wait for it from whatever
    /// executor the caller is on.
    async fn on_runtime<T: Send + 'static>(
        &self,
        future: impl std::future::Future<Output = T> + Send + 'static,
    ) -> Result<T, MobileError> {
        self.runtime
            .handle()
            .spawn(future)
            .await
            .map_err(|e| internal(format!("background task: {e}")))
    }
}

/// The app's directories on a phone: everything under `cache_dir` (or the
/// process temp directory), no config directory, no `hledger`.
fn mobile_dirs(cache_dir: Option<&str>) -> AppDirs {
    let cache = cache_dir.map(std::path::PathBuf::from);
    AppDirs {
        config_dir: None,
        temp_dir: cache
            .as_ref()
            .map_or_else(std::env::temp_dir, |dir| dir.join("tmp")),
        cache_dir: cache,
        hledger_bin: None,
        // A phone has no home directory that is the user's: no `~/.hledger.conf`.
        hledger_conf: ledgeline_core::hledger_conf::UserConfDirs::default(),
    }
}

/// Why an open failed, in the terms the app acts on.
fn open_failure(
    tracking: &TrackingVault,
    main: &VaultPath,
    error: &EditError,
    single_file: bool,
) -> MobileError {
    let not_downloaded = tracking.take_not_downloaded();
    if !not_downloaded.is_empty() {
        return MobileError::NotDownloaded {
            paths: not_downloaded.iter().map(ToString::to_string).collect(),
        };
    }
    let missing_includes = tracking
        .take_missing()
        .into_iter()
        .any(|path| path != *main);
    if single_file && missing_includes {
        return MobileError::NeedsFolder {
            message: format!(
                "{main} includes other files, which opening it on its own cannot reach. Open \
                 the folder that contains it instead."
            ),
        };
    }
    open_error(error)
}

/// The body of [`LedgelineApp::refresh`], on a blocking thread.
fn refresh_now(state: &AppState, vault: &TrackingVault, main: &VaultPath) -> RefreshSummary {
    let started = Instant::now();
    vault.reset();
    let summary = |outcome, not_downloaded: Vec<VaultPath>, message| RefreshSummary {
        outcome,
        not_downloaded: not_downloaded.iter().map(ToString::to_string).collect(),
        message,
        editable: state.editing_enabled(),
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    };

    // Evicted files first: a reload that tripped over one would unbind the
    // editor for nothing.
    let evicted: Vec<VaultPath> = state
        .source_files()
        .iter()
        .filter_map(|key| vault.vault_path(key).ok())
        .filter(|path| vault.availability(path) == Availability::NotDownloaded)
        .collect();
    if !evicted.is_empty() {
        vault.request_download(&evicted);
        return summary(RefreshOutcome::Pending, evicted, None);
    }

    let refreshed = match (state.editing_enabled(), state.location()) {
        // An earlier failure unbound the editor; a journal that reads cleanly
        // now gets one back (and is republished by it).
        (false, Some(location)) => state
            .rebind_vault(Arc::clone(location.vault()), main.clone())
            .map(|()| Refreshed::Reloaded),
        _ => state.refresh(),
    };
    let not_downloaded = vault.take_not_downloaded();
    match refreshed {
        Ok(Refreshed::Unchanged) => summary(RefreshOutcome::Unchanged, not_downloaded, None),
        Ok(Refreshed::Reloaded | Refreshed::NoStorage) => {
            summary(RefreshOutcome::Reloaded, not_downloaded, None)
        }
        Err(_) if !not_downloaded.is_empty() => {
            summary(RefreshOutcome::Pending, not_downloaded, None)
        }
        Err(error) => summary(
            RefreshOutcome::Failed,
            not_downloaded,
            Some(error.to_string()),
        ),
    }
}

/// `path`, canonical, provided it is a directory.
fn canonical_dir(path: &Path) -> Result<std::path::PathBuf, MobileError> {
    let root = path
        .canonicalize()
        .map_err(|e| open_error(format!("{}: {e}", path.display())))?;
    if !root.is_dir() {
        return Err(open_error(format!("not a folder: {}", root.display())));
    }
    Ok(root)
}

#[cfg(test)]
mod tests;
