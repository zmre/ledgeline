//! **Bank sync** (plan 43, contract WP-S): SimpleFIN accounts linked to
//! journal accounts, fetched on click or — when the user opts in — in the
//! background, and imported through the existing import pipeline.
//!
//! # Where things are
//!
//! - [`client`] — the [`FeedProvider`] seam and the reqwest SimpleFIN client;
//! - [`secrets`] — the keychain behind [`SecretStore`];
//! - [`state`] — `sync-state.json`, the disposable machine memory;
//! - [`auto`] — the pure planner and the background loop;
//! - [`engine`] — one run: fetch → normalize → split → stage → dry run →
//!   commit, plus the held-row actions;
//! - [`link`] — the wizard: remote accounts with suggestions, previews, the
//!   link itself and rules sharing;
//! - [`wire`] — the wire shapes and the plan's §6 problems;
//! - [`cli`] — `ledgeline sync …`, the same engine without a window.
//!
//! # Trust
//!
//! `ledgeline.toml` is untrusted (it travels with a cloned repository), so
//! the `[sync]` links read from it are mapping data only. The credential and
//! the server's host come from the keychain; every path a link names is
//! resolved inside the journal's folder before use, and a link that fails
//! that is skipped with a `setup` problem.
//!
//! # Locks
//!
//! The network is never touched under a lock. A run fetches first, then takes
//! `AppState::import_writes` around the stage, dry run and commit only — the
//! same lock the import screen's commit takes — and `ledgeline.toml` is
//! written through `ui_prefs`' own pinned writer and lock.

// `WireProblem` is the error of most functions here, and it is ~230 bytes.
// Every one of them runs once per request or per account, never in a loop
// that matters, and the problem is what the user ends up reading, so it is
// passed by value rather than boxed at every `?`.
#![allow(clippy::result_large_err)]

pub(crate) mod auto;
pub(crate) mod cli;
pub(crate) mod client;
pub(crate) mod engine;
pub(crate) mod link;
pub(crate) mod secrets;
pub(crate) mod state;
pub(crate) mod wire;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Json;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::feeds::FeedBatch;
use ledgeline_core::feeds::config::{self as feed_config, ConfigError, SyncConfig};
use ledgeline_core::feeds::simplefin::AccessUrl;
use ledgeline_core::reports::periods::iso_from_days;
use serde::Deserialize;

use crate::AppState;
use crate::edit_api::json_body;
use crate::error::AppError;
use crate::reports_api::{compute, unix_now};
use crate::ui_prefs;

pub use client::{AccountsQuery, FeedError, FeedProvider, SimpleFinClient, filter_batch};
pub use secrets::{MemoryStore, SecretError, SecretStore, UnavailableStore};
use wire::{
    ConnectionState, Severity, WireConnection, WireLinkedAccount, WireProblem, WireProblemCount,
    WireQuota, WireSyncStatus,
};

/// The quota numbers every connection reports.
pub(crate) const QUOTA: WireQuota = WireQuota {
    auto_max: auto::AUTO_MAX,
    manual_warn: auto::MANUAL_WARN,
    manual_max: auto::MANUAL_MAX,
};

/// How long a balances-only fetch from connect/accounts may stand in for a
/// fresh one in the link preview.
const BATCH_FRESH_SECONDS: i64 = 10 * 60;

/// The wall clock, epoch seconds.
pub(crate) fn now_seconds() -> i64 {
    i64::try_from(unix_now()).unwrap_or(i64::MAX)
}

/// The UTC date of `epoch`, ISO.
pub(crate) fn iso_utc(epoch: i64) -> String {
    iso_from_days(epoch.div_euclid(86_400))
}

/// The local offset from UTC at `epoch`, seconds east.
pub(crate) fn local_offset(epoch: i64) -> i32 {
    use chrono::{Local, Offset, TimeZone};
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map_or(0, |time| time.offset().fix().local_minus_utc())
}

/// A file-name-safe slug: lowercase ASCII letters and digits, runs of
/// anything else as one `-`, at most 40 characters, never empty.
pub(crate) fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed: String = out.trim_matches('-').chars().take(40).collect();
    let trimmed = trimmed.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        "account".to_string()
    } else {
        trimmed
    }
}

// ===========================================================================
// The per-state session
// ===========================================================================

/// A batch with transactions a link preview fetched, kept briefly so that
/// stepping back and forth in the wizard doesn't spend the day's quota.
/// (A balances-only batch from connect/accounts has no transactions to
/// preview, so it is never used for one.)
#[derive(Clone)]
pub(crate) struct CachedBatch {
    pub(crate) at: i64,
    /// The window's start, epoch seconds.
    pub(crate) start: i64,
    pub(crate) batch: FeedBatch,
}

/// Bank sync's per-`AppState` parts: the provider and secret store (both
/// swappable for tests), and the run's in-memory state.
pub(crate) struct SyncSession {
    provider: Arc<dyn FeedProvider>,
    secrets: Arc<dyn SecretStore>,
    loopback_http: bool,
    auto: Arc<auto::SyncAuto>,
    /// One run at a time: two runs would count the quota twice and race on
    /// the same rows.
    run_lock: tokio::sync::Mutex<()>,
    running: AtomicBool,
    /// Read-modify-write of `sync-state.json`.
    state_lock: Mutex<()>,
    batches: Mutex<HashMap<String, CachedBatch>>,
    /// Where `sync-state.json` goes instead of the cache directory: the fake
    /// bridge's private folder, so an e2e or docs-screenshot run never writes
    /// fake connections into the developer's real sync state.
    state_dir: Option<std::path::PathBuf>,
    /// Test hook (`test-fakes`): commits to report as a conflict, so the
    /// "edited mid-run → skipped, retried next time" path can be driven.
    #[cfg(feature = "test-fakes")]
    pub(crate) inject_conflicts: std::sync::atomic::AtomicUsize,
}

impl SyncSession {
    /// The real client and the keychain.
    pub(crate) fn new() -> Self {
        Self::with(
            Arc::new(SimpleFinClient::new(false)),
            secrets::default_store(),
            false,
        )
    }

    fn with(
        provider: Arc<dyn FeedProvider>,
        secrets: Arc<dyn SecretStore>,
        loopback_http: bool,
    ) -> Self {
        Self {
            provider,
            secrets,
            loopback_http,
            auto: Arc::default(),
            run_lock: tokio::sync::Mutex::new(()),
            running: AtomicBool::new(false),
            state_lock: Mutex::new(()),
            batches: Mutex::default(),
            state_dir: None,
            #[cfg(feature = "test-fakes")]
            inject_conflicts: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// This session with another provider.
    pub(crate) fn with_provider(&self, provider: Arc<dyn FeedProvider>) -> Self {
        Self::with(provider, Arc::clone(&self.secrets), self.loopback_http)
    }

    /// This session with another secret store.
    pub(crate) fn with_secrets(&self, secrets: Arc<dyn SecretStore>) -> Self {
        Self::with(Arc::clone(&self.provider), secrets, self.loopback_http)
    }

    /// This session allowing plain http to a loopback host (tests): the real
    /// client rebuilt to allow it, and setup tokens decoded with it.
    #[cfg(feature = "test-fakes")]
    pub(crate) fn with_loopback_http(&self) -> Self {
        Self::with(
            Arc::new(SimpleFinClient::new(true)),
            Arc::clone(&self.secrets),
            true,
        )
    }

    /// This session keeping `sync-state.json` in `dir` (the fake bridge).
    #[cfg(feature = "test-fakes")]
    pub(crate) fn with_state_dir(&self, dir: std::path::PathBuf) -> Self {
        let mut session = Self::with(
            Arc::clone(&self.provider),
            Arc::clone(&self.secrets),
            self.loopback_http,
        );
        session.state_dir = Some(dir);
        session
    }

    pub(crate) fn provider(&self) -> &Arc<dyn FeedProvider> {
        &self.provider
    }

    pub(crate) fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }

    pub(crate) fn loopback_http(&self) -> bool {
        self.loopback_http
    }

    pub(crate) fn auto(&self) -> &auto::SyncAuto {
        &self.auto
    }

    pub(crate) fn auto_arc(&self) -> Arc<auto::SyncAuto> {
        Arc::clone(&self.auto)
    }

    pub(crate) fn run_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.run_lock
    }

    pub(crate) fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::SeqCst);
    }

    pub(crate) fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Keep a preview's batch (transactions from `start`).
    pub(crate) fn remember_batch(&self, connection: &str, at: i64, start: i64, batch: FeedBatch) {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(connection.to_string(), CachedBatch { at, start, batch });
    }

    /// A preview batch fetched less than [`BATCH_FRESH_SECONDS`] before `now`
    /// that reaches back to `start` and holds every account in `accounts`.
    pub(crate) fn fresh_batch(
        &self,
        connection: &str,
        now: i64,
        start: i64,
        accounts: &[String],
    ) -> Option<FeedBatch> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(connection)
            .filter(|cached| now - cached.at < BATCH_FRESH_SECONDS && cached.start <= start)
            .filter(|cached| {
                accounts
                    .iter()
                    .all(|id| cached.batch.accounts.iter().any(|a| &a.id == id))
            })
            .map(|cached| cached.batch.clone())
    }

    /// Drop a connection's cached batch (its token changed or it went away).
    pub(crate) fn forget_batch(&self, connection: &str) {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(connection);
    }
}

impl Default for SyncSession {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// State file, config and secrets, for every part of the module
// ===========================================================================

/// `sync-state.json`'s path for `state`.
fn state_path(state: &AppState) -> Option<std::path::PathBuf> {
    let session = state.sync();
    let dir = session
        .state_dir
        .as_deref()
        .or(state.app_dirs().cache_dir.as_deref());
    state::path_in(dir)
}

/// Read `sync-state.json`.
pub(crate) fn read_state(state: &AppState) -> state::SyncState {
    let _guard = state
        .sync()
        .state_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    state::load(state_path(state).as_deref())
}

/// Change `sync-state.json` with `change`, under the session's lock.
pub(crate) fn update_state<T>(
    state: &AppState,
    change: impl FnOnce(&mut state::SyncState) -> T,
) -> T {
    let _guard = state
        .sync()
        .state_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let path = state_path(state);
    let mut current = state::load(path.as_deref());
    let answer = change(&mut current);
    state::save(path.as_deref(), &current);
    answer
}

/// The `[sync]` tables of the open ledger's `ledgeline.toml`, and the
/// warnings for entries skipped.
///
/// # Errors
/// The file exists but can't be read safely.
pub(crate) fn read_config(state: &AppState) -> Result<(SyncConfig, Vec<String>), AppError> {
    Ok(ui_prefs::ledger_text(state)?
        .map(|text| feed_config::parse(&text))
        .unwrap_or_default())
}

/// Apply a `feeds::config` writer to the open ledger's `ledgeline.toml`,
/// creating the file if needed.
///
/// # Errors
/// [`AppError::BadRequest`] for a writer's refusal; the file errors of
/// [`ui_prefs::edit_ledger_text`].
pub(crate) fn write_config(
    state: &AppState,
    edit: impl Fn(&str) -> Result<String, ConfigError>,
) -> Result<(), AppError> {
    ui_prefs::edit_ledger_text(state, |text| {
        edit(text).map_err(|error| AppError::BadRequest(error.to_string()))
    })
}

/// The access URL kept for `connection`, if this machine has one.
///
/// # Errors
/// A `setup` problem when the store can't be read or holds something that
/// isn't an access URL.
pub(crate) fn access_for(
    state: &AppState,
    connection: &str,
) -> Result<Option<AccessUrl>, WireProblem> {
    let session = state.sync();
    match session.secrets().get(connection) {
        Ok(None) => Ok(None),
        Ok(Some(secret)) => AccessUrl::parse(&secret, session.loopback_http())
            .map(Some)
            .map_err(|_| WireProblem::damaged_key(connection)),
        Err(error) => Err(WireProblem::setup(error.to_string())),
    }
}

// ===========================================================================
// Replies
// ===========================================================================

/// A sync route's failure: an ordinary [`AppError`], a problem the wizard
/// shows (`422 {problem}`), or a write on a read-only server (`403`).
pub(crate) enum SyncReply {
    App(AppError),
    Problem(Box<WireProblem>),
    Forbidden,
}

impl From<AppError> for SyncReply {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl From<WireProblem> for SyncReply {
    fn from(problem: WireProblem) -> Self {
        Self::Problem(Box::new(problem))
    }
}

impl IntoResponse for SyncReply {
    fn into_response(self) -> Response {
        match self {
            Self::App(error) => error.into_response(),
            Self::Problem(problem) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "problem": problem })),
            )
                .into_response(),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "this ledger is open read-only, so bank sync can't change it",
            )
                .into_response(),
        }
    }
}

/// `403` unless this server may write.
fn writable(state: &AppState) -> Result<(), SyncReply> {
    if state.editing_enabled() {
        Ok(())
    } else {
        Err(SyncReply::Forbidden)
    }
}

/// Run `job` on the blocking pool.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    compute(job).await.map(|Json(value)| value)
}

// ===========================================================================
// Status
// ===========================================================================

/// Everything `GET /api/sync/status` reports, at `now`.
pub(crate) fn status(state: &AppState, now: i64) -> Result<WireSyncStatus, AppError> {
    let session = state.sync();
    let settings = ui_prefs::sync_settings(state);
    let (config, warnings) = read_config(state)?;
    let memory = read_state(state);
    let available = session.secrets().check();
    let running = session.running();
    let snapshot = state.snapshot();
    let ctx = link::Context::of(state, &snapshot);

    let mut accounts: Vec<WireLinkedAccount> = config
        .accounts
        .iter()
        .map(|link| link::linked_account(state, &ctx, &config, &memory, link))
        .collect();
    let config_problems: Vec<WireProblem> = warnings
        .iter()
        .map(|warning| WireProblem::setup(format!("ledgeline.toml: {warning}")))
        .collect();
    let mut connections = Vec::new();
    for link in &config.connections {
        let remembered = memory.connections.get(&link.id);
        let access = access_for(state, &link.id);
        let connected = matches!(access, Ok(Some(_)));
        let host = match &access {
            Ok(Some(access)) => Some(access.host.clone()),
            _ => remembered.and_then(|c| c.host.clone()),
        };
        let mut problems: Vec<WireProblem> =
            remembered.map(|c| c.problems.clone()).unwrap_or_default();
        problems.extend(config_problems.iter().cloned());
        match access {
            Err(problem) => problems.push(problem),
            Ok(None) => problems.push(WireProblem::not_connected(&link.id)),
            Ok(Some(_)) => {}
        }
        let mine: Vec<&WireLinkedAccount> = accounts
            .iter()
            .filter(|account| account.connection_id == link.id)
            .collect();
        let worst = problems
            .iter()
            .chain(mine.iter().flat_map(|account| account.problems.iter()))
            .map(|problem| problem.severity)
            .max();
        let state_now = if !connected {
            ConnectionState::NotConnected
        } else if running {
            ConnectionState::Syncing
        } else if problems.iter().any(|p| p.severity == Severity::Error) {
            ConnectionState::Broken
        } else if worst >= Some(Severity::Warning) {
            ConnectionState::Attention
        } else {
            ConnectionState::Ok
        };
        connections.push(WireConnection {
            id: link.id.clone(),
            name: link.name.clone(),
            kind: link.kind.clone(),
            manage_url: wire::manage_url(host.as_deref()),
            host,
            connected,
            state: state_now,
            last_attempt: remembered.and_then(|c| c.last_attempt),
            last_success: remembered.and_then(|c| c.last_success),
            requests_today: remembered.map_or(0, |c| c.requests_on(&auto::utc_day(now))),
            quota: QUOTA,
            problems,
        });
    }
    // A link to a connection this file doesn't list reads as not connected.
    for account in &mut accounts {
        if config.connection(&account.connection_id).is_none() {
            account
                .problems
                .push(WireProblem::not_connected(&account.connection_id));
        }
    }
    let unlinked = config
        .connections
        .iter()
        .flat_map(|connection| {
            link::unlinked_accounts(state, &ctx, &config, &memory, &connection.id)
        })
        .collect();
    let mut problem_count = WireProblemCount::default();
    problem_count.add(connections.iter().flat_map(|c| c.problems.iter()));
    problem_count.add(accounts.iter().flat_map(|a| a.problems.iter()));
    let next_auto = settings
        .auto_sync
        .then(|| engine::next_auto(state, now, &config, &memory, &settings))
        .flatten();
    Ok(WireSyncStatus {
        auto_sync: settings.auto_sync,
        available: available.is_ok() && snapshot.storage().is_some(),
        unavailable_reason: available.err().map(|error| error.to_string()).or_else(|| {
            snapshot
                .storage()
                .is_none()
                .then(|| "No journal file is open.".to_string())
        }),
        running,
        last_run: session.auto().last_run(),
        next_auto,
        connections,
        accounts,
        unlinked,
        problem_count,
    })
}

// ===========================================================================
// Routes
// ===========================================================================

/// `GET /api/sync/status`.
pub(crate) async fn status_route(State(state): State<AppState>) -> Result<Response, SyncReply> {
    let reader = state.clone();
    let body = blocking(move || status(&reader, now_seconds())).await?;
    Ok(Json(body).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConnectBody {
    setup_token: String,
    #[serde(default)]
    name: Option<String>,
}

/// `POST /api/sync/connect`.
pub(crate) async fn connect_route(
    State(state): State<AppState>,
    payload: Result<Json<ConnectBody>, JsonRejection>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let body = json_body(payload)?;
    let answer = link::connect(&state, &body.setup_token, body.name.as_deref(), None).await?;
    Ok(Json(answer).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TokenBody {
    setup_token: String,
}

/// `POST /api/sync/connections/{id}/token`.
pub(crate) async fn token_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TokenBody>, JsonRejection>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let body = json_body(payload)?;
    let answer = link::connect(&state, &body.setup_token, None, Some(&id)).await?;
    Ok(Json(answer).into_response())
}

/// `GET /api/sync/connections/{id}/accounts`.
pub(crate) async fn accounts_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, SyncReply> {
    let accounts = link::remote_accounts(&state, &id).await?;
    Ok(Json(serde_json::json!({ "accounts": accounts })).into_response())
}

/// `DELETE /api/sync/connections/{id}`: remove this computer's keychain item
/// (plan §4.4: "Removes the access key from this Mac"). `ledgeline.toml` is
/// shared by every computer that opens the ledger, so the
/// `[[sync.connection]]` entry stays while any account still links to it;
/// those accounts read as not connected until **Connect on this computer…**
/// stores a token again under the same id, which keeps their `id:` prefixes.
/// An entry with no linked accounts left is removed.
pub(crate) async fn disconnect_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let worker = state.clone();
    let body = blocking(move || {
        let (config, _) = read_config(&worker)?;
        if config.connection(&id).is_none() {
            return Err(AppError::NotFound(format!("no connection {id:?}")));
        }
        worker
            .sync()
            .secrets()
            .delete(&id)
            .map_err(|error| AppError::Conflict(error.to_string()))?;
        if !config
            .accounts
            .iter()
            .any(|account| account.connection == id)
        {
            write_config(&worker, |text| feed_config::remove_connection(text, &id))?;
        }
        worker.sync().forget_batch(&id);
        update_state(&worker, |memory| {
            memory.connections.remove(&id);
        });
        status(&worker, now_seconds())
    })
    .await?;
    Ok(Json(body).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RenameBody {
    name: String,
}

/// `PATCH /api/sync/connections/{id}` `{name}`: rename (WP-W).
pub(crate) async fn rename_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<RenameBody>, JsonRejection>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let body = json_body(payload)?;
    let worker = state.clone();
    let answer = blocking(move || {
        let (config, _) = read_config(&worker)?;
        let mut connection = config
            .connection(&id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("no connection {id:?}")))?;
        let name = ledgeline_core::feeds::sanitize(&body.name);
        if name.is_empty() {
            return Err(AppError::BadRequest(
                "a connection needs a name".to_string(),
            ));
        }
        connection.name = name;
        write_config(&worker, |text| {
            feed_config::upsert_connection(text, &connection)
        })?;
        status(&worker, now_seconds())
    })
    .await?;
    Ok(Json(answer).into_response())
}

/// `GET /api/sync/connections/{id}/diagnostics`: a redacted text report.
pub(crate) async fn diagnostics_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, SyncReply> {
    let worker = state.clone();
    let text = blocking(move || engine::diagnostics(&worker, &id)).await?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(text))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ShareBody {
    manual_rules_id: String,
    #[allow(dead_code)]
    #[serde(default)]
    journal_account: Option<String>,
}

/// `POST /api/sync/share/preview`.
pub(crate) async fn share_preview_route(
    State(state): State<AppState>,
    payload: Result<Json<ShareBody>, JsonRejection>,
) -> Result<Response, SyncReply> {
    let body = json_body(payload)?;
    let worker = state.clone();
    let preview = blocking(move || link::share_preview(&worker, &body.manual_rules_id)).await?;
    Ok(Json(preview).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LinksBody {
    links: Vec<link::LinkRequest>,
}

/// `POST /api/sync/link/preview`.
pub(crate) async fn link_preview_route(
    State(state): State<AppState>,
    payload: Result<Json<LinksBody>, JsonRejection>,
) -> Result<Response, SyncReply> {
    let body = json_body(payload)?;
    let accounts = link::preview(&state, body.links).await?;
    Ok(Json(serde_json::json!({ "accounts": accounts })).into_response())
}

/// `POST /api/sync/link`.
pub(crate) async fn link_route(
    State(state): State<AppState>,
    payload: Result<Json<LinksBody>, JsonRejection>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let body = json_body(payload)?;
    let run = link::link(&state, body.links).await?;
    Ok(Json(run).into_response())
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NowBody {
    #[serde(default)]
    connection_id: Option<String>,
    #[serde(default)]
    account_key: Option<String>,
}

/// `POST /api/sync/now`.
pub(crate) async fn now_route(
    State(state): State<AppState>,
    payload: Result<Json<NowBody>, JsonRejection>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let body = json_body(payload)?;
    let run = engine::run(
        &state,
        now_seconds(),
        &engine::Request {
            mode: auto::Mode::Manual,
            connection: body.connection_id,
            accounts: body.account_key.into_iter().collect(),
            reuse_preview: false,
        },
    )
    .await;
    Ok(Json(run).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AccountPatch {
    #[serde(default)]
    pub(crate) journal_account: Option<String>,
    #[serde(default)]
    pub(crate) target: Option<String>,
    #[serde(default)]
    pub(crate) rules: Option<String>,
    #[serde(default)]
    pub(crate) start_date: Option<String>,
    #[serde(default)]
    pub(crate) balance_sign: Option<i8>,
    #[serde(default)]
    pub(crate) enabled: Option<bool>,
}

/// `PATCH /api/sync/accounts/{key}`.
pub(crate) async fn account_patch_route(
    State(state): State<AppState>,
    Path(key): Path<String>,
    payload: Result<Json<AccountPatch>, JsonRejection>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let patch = json_body(payload)?;
    let worker = state.clone();
    let answer = blocking(move || {
        link::patch_account(&worker, &key, &patch)?;
        status(&worker, now_seconds())
    })
    .await?;
    Ok(Json(answer).into_response())
}

/// `DELETE /api/sync/accounts/{key}`: stop syncing. The journal and the
/// account's history stay.
pub(crate) async fn account_delete_route(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let worker = state.clone();
    let answer = blocking(move || {
        let (config, _) = read_config(&worker)?;
        let link = config
            .account(&key)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("no linked account {key:?}")))?;
        write_config(&worker, |text| {
            feed_config::remove_account(text, &link.connection, &link.remote_id)
        })?;
        status(&worker, now_seconds())
    })
    .await?;
    Ok(Json(answer).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HeldBody {
    action: engine::HeldAction,
}

/// `POST /api/sync/held/{key}/{heldId}`.
pub(crate) async fn held_route(
    State(state): State<AppState>,
    Path((key, held_id)): Path<(String, String)>,
    payload: Result<Json<HeldBody>, JsonRejection>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let body = json_body(payload)?;
    engine::resolve_held(&state, now_seconds(), &key, &held_id, body.action).await?;
    let worker = state.clone();
    let answer = blocking(move || status(&worker, now_seconds())).await?;
    Ok(Json(answer).into_response())
}

/// `GET /api/sync/manual`.
pub(crate) async fn manual_route(State(state): State<AppState>) -> Result<Response, SyncReply> {
    let worker = state.clone();
    let accounts = blocking(move || link::manual_accounts(&worker, now_seconds())).await?;
    Ok(Json(serde_json::json!({ "accounts": accounts })).into_response())
}

/// `PUT /api/sync/manual`.
pub(crate) async fn manual_put_route(
    State(state): State<AppState>,
    payload: Result<Json<wire::WireManualAccount>, JsonRejection>,
) -> Result<Response, SyncReply> {
    writable(&state)?;
    let body = json_body(payload)?;
    let worker = state.clone();
    let accounts = blocking(move || {
        let manual = feed_config::ManualAccount {
            account: body.account,
            download_url: body.download_url.filter(|url| !url.trim().is_empty()),
            nudge: body.nudge,
        };
        write_config(&worker, |text| feed_config::upsert_manual(text, &manual))?;
        link::manual_accounts(&worker, now_seconds())
    })
    .await?;
    Ok(Json(serde_json::json!({ "accounts": accounts })).into_response())
}

/// Every `/api/sync/*` route.
pub(crate) fn routes() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/sync/status", get(status_route))
        .route("/api/sync/connect", post(connect_route))
        .route("/api/sync/connections/{id}/token", post(token_route))
        .route("/api/sync/connections/{id}/accounts", get(accounts_route))
        .route(
            "/api/sync/connections/{id}",
            axum::routing::delete(disconnect_route).patch(rename_route),
        )
        .route(
            "/api/sync/connections/{id}/diagnostics",
            get(diagnostics_route),
        )
        .route("/api/sync/share/preview", post(share_preview_route))
        .route("/api/sync/link/preview", post(link_preview_route))
        .route("/api/sync/link", post(link_route))
        .route("/api/sync/now", post(now_route))
        .route(
            "/api/sync/accounts/{key}",
            axum::routing::patch(account_patch_route).delete(account_delete_route),
        )
        .route("/api/sync/held/{key}/{held_id}", post(held_route))
        .route("/api/sync/manual", get(manual_route).put(manual_put_route))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_folder_names() {
        assert_eq!(
            slug("First Tech · Checking ••1234"),
            "first-tech-checking-1234"
        );
        assert_eq!(slug("../../etc"), "etc");
        assert_eq!(slug("   "), "account");
        assert_eq!(slug(&"x".repeat(100)).len(), 40);
    }

    #[test]
    fn iso_utc_reads_epochs() {
        assert_eq!(iso_utc(1_794_268_800), "2026-11-10");
    }
}
