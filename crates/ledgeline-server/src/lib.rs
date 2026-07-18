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

mod reports_api;
mod spa;

use arc_swap::ArcSwap;
use axum::{Json, Router, extract::State, routing::get};
use ledgeline_core::{Journal, wire};
use serde_json::Value;
use std::sync::Arc;
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

/// Cheaply-cloneable application state: an atomically-swappable [`Snapshot`].
/// Cloning shares the same swap cell, so a clone handed to a file watcher or the
/// GUI can [`replace_journal`](AppState::replace_journal) the data the router's
/// handlers serve.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<ArcSwap<Snapshot>>,
}

impl AppState {
    /// Build state serving `journal`.
    #[must_use]
    pub fn from_journal(journal: &Journal) -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(Snapshot::from_journal(journal))),
        }
    }

    /// Atomically replace the served journal (and its precomputed payloads).
    /// In-flight requests keep their snapshot; subsequent ones see the new data.
    pub fn replace_journal(&self, journal: &Journal) {
        self.inner.store(Arc::new(Snapshot::from_journal(journal)));
    }

    /// The current snapshot (a cheap atomic load; the returned `Arc` is a stable
    /// view for the duration of one request even if a swap happens meanwhile).
    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.load_full()
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
