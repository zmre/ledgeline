//! `ledgeline-server` library: the axum application that serves the Phase 2
//! read endpoints from a parsed journal.
//!
//! The binary ([`main`](../main.rs)) is a thin CLI wrapper around [`app`]; the
//! app is exposed here as a library so integration tests can drive the real
//! HTTP layer with `tower`'s `oneshot` (no sockets required).
//!
//! Each wire endpoint's JSON body is precomputed once from the journal at
//! startup and stored behind an `Arc` in [`AppState`]; handlers hand back the
//! cached value. The native report/budget endpoints ([`reports_api`]) instead
//! depend on request query params, so they are computed per request from the
//! parsed [`Journal`] that [`AppState`] also holds.

mod reports_api;

use axum::{Json, Router, extract::State, routing::get};
use ledgeline_core::{Journal, wire};
use serde_json::Value;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

/// Immutable, cheaply-cloneable application state: one precomputed JSON value per
/// wire endpoint (each shared via `Arc` so handler dispatch never re-serializes
/// the journal), plus the parsed [`Journal`] the report endpoints compute from.
#[derive(Clone)]
pub struct AppState {
    /// The parsed journal, shared with the per-request report handlers.
    journal: Arc<Journal>,
    version: Arc<Value>,
    accountnames: Arc<Value>,
    transactions: Arc<Value>,
    prices: Arc<Value>,
    commodities: Arc<Value>,
    accounts: Arc<Value>,
}

impl AppState {
    /// Precompute every endpoint payload from `journal`.
    ///
    /// The wire serializers cannot fail for finite, string-keyed journal data,
    /// so any (impossible) `serde_json` error collapses to JSON `null` — the
    /// same guarantee Phase 1 relies on in `parse_to_transactions_value`.
    #[must_use]
    pub fn from_journal(journal: &Journal) -> Self {
        Self {
            journal: Arc::new(journal.clone()),
            version: Arc::new(wire::version_value()),
            accountnames: Arc::new(value_or_null(wire::journal_to_accountnames_value(journal))),
            transactions: Arc::new(value_or_null(wire::journal_to_value(journal))),
            prices: Arc::new(value_or_null(wire::journal_to_prices_value(journal))),
            commodities: Arc::new(value_or_null(wire::journal_to_commodities_value(journal))),
            accounts: Arc::new(value_or_null(wire::journal_to_accounts_value(journal))),
        }
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
        .layer(cors)
        .with_state(state)
}

// Each handler serves its endpoint's precomputed value. The journal is parsed
// and each value built exactly once (in `AppState::from_journal`); a request
// only clones the single value being served (serde does not serialize `Arc`
// without its opt-in `rc` feature, so we hand `Json` an owned `Value`).
async fn version(State(state): State<AppState>) -> Json<Value> {
    Json((*state.version).clone())
}

async fn accountnames(State(state): State<AppState>) -> Json<Value> {
    Json((*state.accountnames).clone())
}

async fn transactions(State(state): State<AppState>) -> Json<Value> {
    Json((*state.transactions).clone())
}

async fn prices(State(state): State<AppState>) -> Json<Value> {
    Json((*state.prices).clone())
}

async fn commodities(State(state): State<AppState>) -> Json<Value> {
    Json((*state.commodities).clone())
}

async fn accounts(State(state): State<AppState>) -> Json<Value> {
    Json((*state.accounts).clone())
}
