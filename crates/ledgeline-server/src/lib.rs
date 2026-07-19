//! `ledgeline-server` library: the axum application that serves the Phase 2
//! read endpoints from a parsed journal.
//!
//! The binary ([`main`](../main.rs)) is a thin CLI wrapper around [`app`]; the
//! app is exposed here as a library so integration tests can drive the real
//! HTTP layer with `tower`'s `oneshot` (no sockets required).
//!
//! Each wire endpoint's JSON body is precomputed once from the journal and
//! stored in an immutable [`Snapshot`]; handlers hand back the cached value. The
//! native report/budget endpoints ([`reports_api`]) instead depend on request
//! query params, so they are computed per request from the parsed [`Journal`]
//! the same snapshot holds.
//!
//! The whole snapshot lives behind an [`ArcSwap`] so the parsed journal can be
//! HOT-SWAPPED at runtime (live-reload on file change; the desktop File→Open
//! action) without restarting the server or touching the router: handlers always
//! read the current snapshot, and a swap atomically publishes a fresh one.
//!
//! The WRITE path ([`edit_api`], Phase 5.2) is layered on top: a state built from
//! a journal *file* also holds an [`Arc`]-shared [`std::sync::Mutex`] over a
//! [`JournalEditor`]. Reads stay lock-free (they only touch the `ArcSwap`); an
//! edit serializes on the mutex, validates + saves through the editor, and then
//! rebuilds and republishes the snapshot so the read endpoints reflect the change
//! immediately. A state built without a path (the oneshot test helper [`app`])
//! has no editor, so the edit endpoints report that editing is disabled.

mod edit_api;
mod reports_api;
mod spa;

use arc_swap::ArcSwap;
use axum::{
    Json, Router,
    extract::State,
    routing::{delete, get, post},
};
use ledgeline_core::{EditError, Journal, JournalEditor, wire};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use tower_http::cors::CorsLayer;

/// An immutable, atomically-publishable view of one parsed journal: the parsed
/// [`Journal`] for the per-request report handlers, plus every wire endpoint's
/// payload precomputed once (so handler dispatch never re-serializes).
pub(crate) struct Snapshot {
    /// The parsed journal, shared with the per-request report handlers.
    pub(crate) journal: Arc<Journal>,
    pub(crate) version: Value,
    pub(crate) accountnames: Value,
    pub(crate) transactions: Value,
    pub(crate) prices: Value,
    pub(crate) commodities: Value,
    pub(crate) accounts: Value,
}

impl Snapshot {
    /// Precompute every endpoint payload from `journal`.
    ///
    /// The wire serializers cannot fail for finite, string-keyed journal data,
    /// so any (impossible) `serde_json` error collapses to JSON `null` — the
    /// same guarantee Phase 1 relies on in `parse_to_transactions_value`.
    fn from_journal(journal: &Journal) -> Self {
        Self {
            journal: Arc::new(journal.clone()),
            version: wire::version_value(),
            accountnames: value_or_null(wire::journal_to_accountnames_value(journal)),
            transactions: value_or_null(wire::journal_to_value(journal)),
            prices: value_or_null(wire::journal_to_prices_value(journal)),
            commodities: value_or_null(wire::journal_to_commodities_value(journal)),
            accounts: value_or_null(wire::journal_to_accounts_value(journal)),
        }
    }
}

/// Cheaply-cloneable application state: an atomically-swappable [`Snapshot`] for
/// the lock-free read path, plus an optional [`JournalEditor`] behind a mutex for
/// the write path.
///
/// Cloning shares both the swap cell and the editor mutex, so a clone handed to a
/// file watcher, the GUI, or an edit handler operates on the same journal: reads
/// stay lock-free, and the single editor mutex serializes all writers.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<ArcSwap<Snapshot>>,
    /// The write path's editor, or `None` when the state was built from an
    /// already-parsed [`Journal`] with no backing file (the edit endpoints then
    /// report editing disabled). Held behind an `Arc<Mutex<…>>` so every clone
    /// shares one editor and writers serialize on it.
    editor: Arc<Mutex<Option<JournalEditor>>>,
}

impl AppState {
    /// Build read-only state serving an already-parsed `journal`, with no backing
    /// file — the edit endpoints are disabled. Used by the oneshot test harness
    /// ([`app`]) and by callers that hot-swap journals in place without editing.
    #[must_use]
    pub fn from_journal(journal: &Journal) -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(Snapshot::from_journal(journal))),
            editor: Arc::new(Mutex::new(None)),
        }
    }

    /// Build editing-enabled state bound to the journal file at `path`: open a
    /// [`JournalEditor`] over it and serve the snapshot built from its parsed
    /// journal. The edit endpoints (`POST`/`DELETE /api/transactions`) are then
    /// live and mutate this file through the editor's validation + atomic write.
    ///
    /// # Errors
    /// [`EditError::Io`] if the file cannot be read, or [`EditError::Parse`] if it
    /// does not parse.
    pub fn from_journal_path(path: impl AsRef<Path>) -> Result<Self, EditError> {
        let editor = JournalEditor::open(path.as_ref())?;
        let snapshot = Snapshot::from_journal(editor.journal());
        Ok(Self {
            inner: Arc::new(ArcSwap::from_pointee(snapshot)),
            editor: Arc::new(Mutex::new(Some(editor))),
        })
    }

    /// Atomically replace the served journal (and its precomputed payloads).
    /// In-flight requests keep their snapshot; subsequent ones see the new data.
    pub fn replace_journal(&self, journal: &Journal) {
        self.inner.store(Arc::new(Snapshot::from_journal(journal)));
    }

    /// Re-open the bound editor from disk after an *external* change, republishing
    /// the snapshot from the freshly-read file so its rope, parsed journal, and
    /// external-change fingerprint all track what is now on disk.
    ///
    /// Returns `None` when no editor is bound (read-only state), so the file
    /// watcher can fall back to a plain reparse + hot-swap; `Some(Ok(()))` on a
    /// successful re-open, or `Some(Err(_))` if the file could not be re-read or
    /// re-parsed (the previous state is then kept).
    pub fn reopen_editor(&self) -> Option<Result<(), EditError>> {
        let mut guard = self.editor.lock().unwrap_or_else(PoisonError::into_inner);
        let editor = guard.as_mut()?;
        match JournalEditor::open(editor.path().to_path_buf()) {
            Ok(reopened) => {
                self.inner
                    .store(Arc::new(Snapshot::from_journal(reopened.journal())));
                *editor = reopened;
                Some(Ok(()))
            }
            Err(error) => Some(Err(error)),
        }
    }

    /// The current snapshot (a cheap atomic load; the returned `Arc` is a stable
    /// view for the duration of one request even if a swap happens meanwhile).
    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.load_full()
    }

    /// The write-path editor mutex, shared by all clones. Used by [`edit_api`] to
    /// serialize edits; `None` inside means editing is disabled for this state.
    pub(crate) fn editor(&self) -> &Mutex<Option<JournalEditor>> {
        &self.editor
    }
}

fn value_or_null(result: Result<Value, serde_json::Error>) -> Value {
    result.unwrap_or(Value::Null)
}

/// Build the router for a parsed `journal`, ready to hand to `axum::serve`.
pub fn app(journal: &Journal) -> Router {
    router_with_state(AppState::from_journal(journal))
}

/// Build the router from precomputed [`AppState`] (handy for tests).
pub fn router_with_state(state: AppState) -> Router {
    // Mirror hledger-web's `--cors='*'`: allow any origin/method/header so the
    // browser SPA can call this local server cross-origin.
    let cors = CorsLayer::permissive();
    Router::new()
        .route("/version", get(version))
        .route("/accountnames", get(accountnames))
        .route("/transactions", get(transactions))
        .route("/prices", get(prices))
        .route("/commodities", get(commodities))
        .route("/accounts", get(accounts))
        .route("/api/reports/balancesheet", get(reports_api::balancesheet))
        .route(
            "/api/reports/incomestatement",
            get(reports_api::incomestatement),
        )
        .route("/api/reports/cashflow", get(reports_api::cashflow))
        .route("/api/reports/networth", get(reports_api::networth))
        .route("/api/budget", get(reports_api::budget))
        .route("/api/holdings", get(reports_api::holdings))
        .route(
            "/api/holdings/series",
            get(reports_api::holdings_series_report),
        )
        // Write path (Phase 5.2): add / delete a transaction through the editor.
        .route("/api/transactions", post(edit_api::add_transaction))
        .route(
            "/api/transactions/{index}",
            delete(edit_api::delete_transaction),
        )
        // Everything else (the SPA shell, its embedded assets, and client-side
        // deep links) is served same-origin; the explicit routes above win.
        .fallback(spa::fallback)
        .layer(cors)
        .with_state(state)
}

// Each handler serves its endpoint's precomputed value from the current
// snapshot. The journal is parsed and each value built once per snapshot (in
// `Snapshot::from_journal`); a request only clones the single value being served
// (serde does not serialize `Arc` without its opt-in `rc` feature, so we hand
// `Json` an owned `Value`).
async fn version(State(state): State<AppState>) -> Json<Value> {
    Json(state.snapshot().version.clone())
}

async fn accountnames(State(state): State<AppState>) -> Json<Value> {
    Json(state.snapshot().accountnames.clone())
}

async fn transactions(State(state): State<AppState>) -> Json<Value> {
    Json(state.snapshot().transactions.clone())
}

async fn prices(State(state): State<AppState>) -> Json<Value> {
    Json(state.snapshot().prices.clone())
}

async fn commodities(State(state): State<AppState>) -> Json<Value> {
    Json(state.snapshot().commodities.clone())
}

async fn accounts(State(state): State<AppState>) -> Json<Value> {
    Json(state.snapshot().accounts.clone())
}
