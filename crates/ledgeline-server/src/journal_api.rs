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
use ledgeline_core::view::checks::{
    Diagnostics, Finding, diagnostics_body, engine_findings, local_findings,
};
use serde::Deserialize;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// How many distinct `today`s keep their problem list cached per snapshot. One
/// is the steady state (every client asks with its own local date); a few cover
/// clients either side of midnight or in other zones.
const DIAGNOSTICS_DAYS: usize = 4;

/// One `today`'s problem list, plus its full serialized body once asked for.
pub(crate) struct DiagnosticsEntry {
    today: String,
    pub(crate) diagnostics: Arc<Diagnostics>,
    full_body: OnceLock<Bytes>,
}

/// Lazily-built, per-snapshot derivations shared by the routes in this module.
#[derive(Default)]
pub(crate) struct ViewCache {
    /// Every account's engine classification.
    classes: OnceLock<Arc<Classes>>,
    /// The engine diagnostics (unbalanced, assertions, account tags, stock
    /// findings): whole-journal passes that do not depend on the date.
    engine: OnceLock<Arc<Vec<Finding>>>,
    /// Engine + local findings per client `today`, most recent last.
    diagnostics: Mutex<Vec<Arc<DiagnosticsEntry>>>,
}

impl ViewCache {
    fn classes(&self, snapshot: &Snapshot) -> Arc<Classes> {
        Arc::clone(
            self.classes
                .get_or_init(|| Arc::new(Classes::of(&snapshot.journal))),
        )
    }

    /// Every finding for `today`: the date-free engine half memoized for the
    /// snapshot, the local rules memoized per date.
    pub(crate) fn diagnostics(&self, snapshot: &Snapshot, today: &str) -> Arc<DiagnosticsEntry> {
        let cached = |entries: &Vec<Arc<DiagnosticsEntry>>| {
            entries.iter().find(|entry| entry.today == today).cloned()
        };
        if let Some(hit) = cached(&self.lock_diagnostics()) {
            return hit;
        }
        let engine = Arc::clone(
            self.engine
                .get_or_init(|| Arc::new(engine_findings(&snapshot.journal))),
        );
        let classes = self.classes(snapshot);
        let entry = Arc::new(DiagnosticsEntry {
            today: today.to_string(),
            diagnostics: Arc::new(Diagnostics::new(
                &engine,
                local_findings(&snapshot.journal, &classes, today),
            )),
            full_body: OnceLock::new(),
        });
        let mut entries = self.lock_diagnostics();
        // A concurrent request may have filled it while this one computed.
        if let Some(hit) = cached(&entries) {
            return hit;
        }
        if entries.len() == DIAGNOSTICS_DAYS {
            entries.remove(0);
        }
        entries.push(Arc::clone(&entry));
        entry
    }

    fn lock_diagnostics(&self) -> std::sync::MutexGuard<'_, Vec<Arc<DiagnosticsEntry>>> {
        // The list is only ever pushed to or trimmed whole; a panic elsewhere
        // cannot leave it half-written, so a poisoned lock is still sound.
        self.diagnostics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The snapshot `ETag` extended with the client's `today`, for a body that
/// depends on the date as well as the journal. A request that omits `today`
/// defaults to the server's date, which moves at midnight with the URL
/// unchanged — the suffix keeps a cached answer from outliving its day.
fn dated_etag(snapshot: &Snapshot, today: &str) -> HeaderValue {
    let opaque = snapshot.etag.to_str().unwrap_or_default().trim_matches('"');
    HeaderValue::from_str(&format!("\"{opaque}-{today}\""))
        .unwrap_or_else(|_| snapshot.etag.clone())
}

/// A `200` JSON body carrying `etag`.
fn ok_json(etag: HeaderValue, body: Bytes) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// The `304` for a client already holding the answer tagged `etag`.
fn not_modified(etag: HeaderValue) -> Response {
    Response::builder()
        .status(StatusCode::NOT_MODIFIED)
        .header(header::ETAG, etag)
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
    let etag = snapshot.etag.clone();
    conditional_tagged(snapshot, etag, headers, job).await
}

/// [`conditional`] with an explicit validator (see [`dated_etag`]).
async fn conditional_tagged<F>(
    snapshot: Arc<Snapshot>,
    etag: HeaderValue,
    headers: &HeaderMap,
    job: F,
) -> Result<Response, AppError>
where
    F: FnOnce(&Snapshot) -> Result<Bytes, AppError> + Send + 'static,
{
    if if_none_match_hits(headers, &etag) {
        return Ok(not_modified(etag));
    }
    let body = reports_api::compute(move || job(&snapshot)).await?.0;
    Ok(ok_json(etag, body))
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

/// `?today=YYYY-MM-DD&limit=N` — the client's local date (default: today,
/// UTC) and an optional cap on the entries returned.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsQuery {
    today: Option<String>,
    limit: Option<usize>,
}

/// `GET /api/diagnostics?today=&limit=` — every problem the badge counts and
/// the drawer lists ([`ledgeline_core::view::checks`]): the engine diagnostics,
/// then pending, uncategorized, negative-cash, missing-description and
/// future-date as of the client's `today`.
///
/// `{count, maxSeverity, diagnostics}`: `count` and `maxSeverity` describe the
/// whole list even when `limit` truncates it, so the always-mounted badge asks
/// for `limit=0` and pays for no entries; the drawer asks for all of them.
/// Each transaction finding carries its 1-based `tindex`, date and description
/// (and the legacy 0-based `txnIndex`), so nothing needs the journal to draw it.
pub(crate) async fn diagnostics(
    State(state): State<AppState>,
    Query(query): Query<DiagnosticsQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = parse_date("today", query.today, today_utc)?;
    let snapshot = state.snapshot();
    let etag = dated_etag(&snapshot, &today);
    conditional_tagged(snapshot, etag, &headers, move |snapshot| {
        let entry = snapshot.view.diagnostics(snapshot, &today);
        if let Some(limit) = query.limit {
            return to_bytes(&diagnostics_body(
                &snapshot.journal,
                &entry.diagnostics,
                Some(limit),
            ));
        }
        if let Some(body) = entry.full_body.get() {
            return Ok(body.clone());
        }
        let body = to_bytes(&diagnostics_body(
            &snapshot.journal,
            &entry.diagnostics,
            None,
        ))?;
        Ok(entry.full_body.get_or_init(|| body).clone())
    })
    .await
}
