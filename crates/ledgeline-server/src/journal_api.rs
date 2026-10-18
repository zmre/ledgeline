//! The journal-view routes (plan 26 B3–B5): the computations the SPA used to
//! run over the whole downloaded journal, answered here from the snapshot so a
//! client fetches only what it draws. The logic lives in
//! [`ledgeline_core::view`]; this module is routing, caching and transport.
//!
//! # Transport contract, shared by every route here
//!
//! - **Snapshot `ETag`.** Each body is a pure function of the snapshot and the
//!   request URL, so the snapshot's `ETag` is a valid validator for it (HTTP
//!   caches key on the URL; the query string is part of it). An
//!   `If-None-Match` hit answers `304` before any work is done.
//! - **The blocking pool.** CPU work runs through [`reports_api::compute`], the
//!   same semaphore-bounded `spawn_blocking` every report uses.
//! - **Token-guarded and gzip'd** like their peers: the routes are registered
//!   above the router's `route_layer`, inside the global compression layer.
//!
//! # Per-snapshot caches ([`ViewCache`])
//!
//! Whatever a route derives from the whole journal and could reuse is computed
//! lazily, at most once per snapshot, and dies with it — a hot-swap publishes a
//! new snapshot with an empty cache, so nothing here can go stale.

use crate::error::AppError;
use crate::reports_api::{self, parse_date, today_utc};
use crate::{AppState, Snapshot, if_none_match_hits};
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::view::Classes;
use ledgeline_core::view::cash::cash_balances_view;
use serde::Deserialize;
use std::sync::{Arc, OnceLock};

/// Lazily-built, per-snapshot derivations shared by the routes in this module.
#[derive(Default)]
pub(crate) struct ViewCache {
    /// Every account's engine classification.
    classes: OnceLock<Arc<Classes>>,
}

impl ViewCache {
    fn classes(&self, snapshot: &Snapshot) -> Arc<Classes> {
        Arc::clone(
            self.classes
                .get_or_init(|| Arc::new(Classes::of(&snapshot.journal))),
        )
    }
}

/// A `200` JSON body carrying the snapshot's `ETag`.
fn ok_json(snapshot: &Snapshot, body: Bytes) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::ETAG, snapshot.etag.clone())
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// The `304` for a client already holding this snapshot's answer.
fn not_modified(snapshot: &Snapshot) -> Response {
    Response::builder()
        .status(StatusCode::NOT_MODIFIED)
        .header(header::ETAG, snapshot.etag.clone())
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Answer from `job` on the blocking pool, or `304` without running it when
/// the client already holds this snapshot's body for this URL.
async fn conditional<F>(
    snapshot: Arc<Snapshot>,
    headers: &HeaderMap,
    job: F,
) -> Result<Response, AppError>
where
    F: FnOnce(&Snapshot) -> Result<Bytes, AppError> + Send + 'static,
{
    if if_none_match_hits(headers, &snapshot.etag) {
        return Ok(not_modified(&snapshot));
    }
    let worker = Arc::clone(&snapshot);
    let body = reports_api::compute(move || job(&worker)).await?.0;
    Ok(ok_json(&snapshot, body))
}

/// Serialize a body, mapping the (unreachable) serializer error to a `500`.
fn to_bytes<T: serde::Serialize>(value: &T) -> Result<Bytes, AppError> {
    serde_json::to_vec(value)
        .map(Bytes::from)
        .map_err(|error| AppError::Internal(format!("serializing the response failed: {error}")))
}

/// `?asOf=YYYY-MM-DD` — the cash view's date (default: today, UTC). The SPA
/// always sends its own local date, so the server's zone never decides it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CashQuery {
    as_of: Option<String>,
}

/// `GET /api/balances/cash?asOf=` — cash and short-term-liability balances
/// as of a date, for every commodity the view holds
/// ([`ledgeline_core::view::cash`]).
///
/// Named for what it is rather than nested under `/api/reports`: it is the
/// journal tab's Balances view, deliberately NARROWER than the balance sheet
/// (as-written amounts, no valuation), and it ignores the journal filter.
pub(crate) async fn cash_balances(
    State(state): State<AppState>,
    Query(query): Query<CashQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let as_of = parse_date("asOf", query.as_of, today_utc)?;
    conditional(state.snapshot(), &headers, move |snapshot| {
        let classes = snapshot.view.classes(snapshot);
        to_bytes(&cash_balances_view(&snapshot.journal, &classes, &as_of))
    })
    .await
}
