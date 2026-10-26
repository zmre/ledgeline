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
use crate::reports_api::{self, parse_date, parse_opt_date, today_utc};
use crate::{AppState, Snapshot, generation_of, if_none_match_hits};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::Journal;
use ledgeline_core::reports::Interval;
use ledgeline_core::view::Classes;
use ledgeline_core::view::cash::cash_balances_view;
use ledgeline_core::view::checks::{
    Diagnostics, Finding, WireFlag, diagnostics_body, engine_findings, local_findings,
};
use ledgeline_core::view::filter::{JournalFilter, display_order, filter_positions};
use ledgeline_core::view::rows::{Row, RowContext, haystack};
use ledgeline_core::view::series::{
    ChartMode, ChartSpec, JournalView, SignTable, chart, parse_category, sign_table, summary,
};
use ledgeline_core::view::wire::DeclaredStyles;
use serde::{Deserialize, Serialize};
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
    /// The declared commodity styles every row and summary renders with.
    declared: OnceLock<Arc<DeclaredStyles>>,
    /// Every transaction position in display order (date desc, tindex desc).
    order: OnceLock<Arc<Vec<u32>>>,
    /// Every transaction's lowercase search text, by position — built only
    /// when a filter first carries a query.
    haystacks: OnceLock<Arc<Vec<Box<str>>>>,
    /// Every commodity's sign conventions over the whole journal.
    signs: OnceLock<Arc<SignTable>>,
    /// Recent filter results, most recently used last ([`FILTER_CACHE`]).
    filters: Mutex<Vec<(JournalFilter, Arc<Vec<u32>>)>>,
}

/// How many filter results each snapshot keeps. Scrolling one view asks for
/// the same filter block after block, and the summary, chart and position
/// routes ask for it again; a handful covers flipping between recent filters.
const FILTER_CACHE: usize = 8;

impl ViewCache {
    /// A fresh cache for `journal`, warmed on a background thread so the
    /// first requests after a load or an edit find the expensive derivations
    /// already built (see [`ViewCache::prewarm`]).
    pub(crate) fn warmed(journal: &Arc<Journal>) -> Arc<Self> {
        let cache = Arc::new(Self::default());
        let weak = Arc::downgrade(&cache);
        let journal = Arc::clone(journal);
        // A failure to spawn only means the first requests build these
        // themselves, which is exactly what they would do without the warm-up.
        let _ = std::thread::Builder::new()
            .name("ledgeline-view-warm".to_string())
            .spawn(move || Self::prewarm(&weak, &journal));
        cache
    }

    /// Build, in the order the SPA's first screen asks for them: the display
    /// order and the problem list (every row embeds its flags, and the badge
    /// counts them), then the summary's inputs, then the search text.
    ///
    /// Each step re-checks that the snapshot still exists: an edit publishes a
    /// new snapshot, and warming a superseded one is wasted work. Everything is
    /// a `OnceLock`, so a request racing a step simply shares its answer.
    fn prewarm(cache: &std::sync::Weak<Self>, journal: &Journal) {
        let today = today_utc();
        let steps: [&dyn Fn(&Self); 5] = [
            &|view| drop(view.order(journal)),
            &|view| drop(view.diagnostics(journal, &today)),
            &|view| drop(view.signs(journal)),
            &|view| drop(view.declared(journal)),
            &|view| drop(view.haystacks(journal)),
        ];
        for step in steps {
            let Some(view) = cache.upgrade() else { return };
            step(&view);
        }
    }

    fn order(&self, journal: &Journal) -> Arc<Vec<u32>> {
        Arc::clone(self.order.get_or_init(|| Arc::new(display_order(journal))))
    }

    fn haystacks(&self, journal: &Journal) -> Arc<Vec<Box<str>>> {
        Arc::clone(self.haystacks.get_or_init(|| {
            let declared = self.declared(journal);
            Arc::new(
                journal
                    .transactions
                    .iter()
                    .map(|txn| haystack(txn, &declared).into_boxed_str())
                    .collect(),
            )
        }))
    }

    fn classes(&self, journal: &Journal) -> Arc<Classes> {
        Arc::clone(self.classes.get_or_init(|| Arc::new(Classes::of(journal))))
    }

    /// Every finding for `today`: the date-free engine half memoized for the
    /// snapshot, the local rules memoized per date.
    pub(crate) fn diagnostics(&self, journal: &Journal, today: &str) -> Arc<DiagnosticsEntry> {
        let cached = |entries: &Vec<Arc<DiagnosticsEntry>>| {
            entries.iter().find(|entry| entry.today == today).cloned()
        };
        if let Some(hit) = cached(&self.lock_diagnostics()) {
            return hit;
        }
        // The engine half and the local rules are independent passes (the
        // local ones need the account classes first): run them side by side.
        let (engine, local) = std::thread::scope(|scope| {
            let engine = scope.spawn(|| {
                Arc::clone(
                    self.engine
                        .get_or_init(|| Arc::new(engine_findings(journal))),
                )
            });
            let local = local_findings(journal, &self.classes(journal), today);
            let engine = engine
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
            (engine, local)
        });
        let entry = Arc::new(DiagnosticsEntry {
            today: today.to_string(),
            diagnostics: Arc::new(Diagnostics::new(&engine, local)),
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

    pub(crate) fn declared(&self, journal: &Journal) -> Arc<DeclaredStyles> {
        Arc::clone(
            self.declared
                .get_or_init(|| Arc::new(DeclaredStyles::of(journal))),
        )
    }

    fn signs(&self, journal: &Journal) -> Arc<SignTable> {
        Arc::clone(
            self.signs
                .get_or_init(|| Arc::new(sign_table(journal, &self.classes(journal)))),
        )
    }

    /// The positions `filter` keeps, in display order — from the cache when a
    /// recent request asked the same question.
    fn filtered(&self, journal: &Journal, filter: &JournalFilter) -> Arc<Vec<u32>> {
        {
            let mut cache = self.lock_filters();
            if let Some(at) = cache.iter().position(|(key, _)| key == filter) {
                let hit = cache.remove(at);
                let positions = Arc::clone(&hit.1);
                cache.push(hit);
                return positions;
            }
        }
        let order = self.order(journal);
        let haystacks = (!filter.query().is_empty()).then(|| self.haystacks(journal));
        let positions = Arc::new(filter_positions(
            journal,
            &order,
            haystacks.as_deref().map(Vec::as_slice),
            filter,
        ));
        let mut cache = self.lock_filters();
        if cache.len() == FILTER_CACHE {
            cache.remove(0);
        }
        cache.push((filter.clone(), Arc::clone(&positions)));
        positions
    }

    fn lock_filters(&self) -> std::sync::MutexGuard<'_, Vec<(JournalFilter, Arc<Vec<u32>>)>> {
        // Entries are pushed and removed whole; a poisoned lock is still sound.
        self.filters.lock().unwrap_or_else(PoisonError::into_inner)
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
        let classes = snapshot.view.classes(&snapshot.journal);
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
        let entry = snapshot.view.diagnostics(&snapshot.journal, &today);
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

// ---------------------------------------------------------------------------
// The windowed journal (plan 26 B5)
// ---------------------------------------------------------------------------

/// The journal filter every windowed route shares, spelled as the SPA's URL
/// already spells it: `from`/`to` inclusive ISO dates, `accounts` a
/// comma-separated list of subtree roots, `q` the free text.
fn journal_filter(
    from: Option<String>,
    to: Option<String>,
    accounts: Option<&str>,
    q: Option<&str>,
) -> Result<JournalFilter, AppError> {
    let accounts = accounts
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .map(str::to_string);
    Ok(JournalFilter::new(
        parse_opt_date("from", from)?,
        parse_opt_date("to", to)?,
        accounts,
        q.unwrap_or_default(),
    ))
}

/// The most rows one `rows` request returns.
const MAX_ROWS: usize = 1000;
/// Rows per request when `limit` is absent: one client block.
const DEFAULT_ROWS: usize = 200;

/// `?from&to&accounts&q&offset&limit&order&today`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RowsQuery {
    from: Option<String>,
    to: Option<String>,
    accounts: Option<String>,
    q: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    order: Option<String>,
    today: Option<String>,
}

/// `{generation, total, offset, rows}`.
#[derive(Serialize)]
struct RowsBody<'a> {
    generation: &'a str,
    total: usize,
    offset: usize,
    rows: Vec<Row<'a>>,
}

/// Whether a request asked for ascending (oldest-first) order.
fn ascending(order: Option<&str>) -> Result<bool, AppError> {
    match order {
        None | Some("desc") => Ok(false),
        Some("asc") => Ok(true),
        Some(other) => Err(AppError::BadRequest(format!(
            "order must be asc or desc, got {other:?}"
        ))),
    }
}

/// The flags for the transaction at `position`, as a row embeds them.
fn flags_of(diagnostics: &Diagnostics, position: usize) -> Vec<WireFlag<'_>> {
    diagnostics
        .for_position(position)
        .map(WireFlag::from)
        .collect()
}

/// `GET /api/journal/rows` — one window of the filtered journal, in display
/// order (`desc`, the default: newest first) or its exact reverse (`asc`, for
/// an export reading the journal oldest-first).
///
/// Each row is the domain transaction the SPA's row components render, with
/// its check findings embedded as `flags` (as of the client's `today`).
/// `total` is the filtered count, so a client can size a scroll area before it
/// has any rows; `limit` is capped at [`MAX_ROWS`].
pub(crate) async fn rows(
    State(state): State<AppState>,
    Query(query): Query<RowsQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let filter = journal_filter(
        query.from,
        query.to,
        query.accounts.as_deref(),
        query.q.as_deref(),
    )?;
    let ascending = ascending(query.order.as_deref())?;
    let today = parse_date("today", query.today, today_utc)?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(DEFAULT_ROWS).min(MAX_ROWS);
    let snapshot = state.snapshot();
    let etag = dated_etag(&snapshot, &today);
    conditional_tagged(snapshot, etag, &headers, move |snapshot| {
        let view = &snapshot.view;
        let positions = view.filtered(&snapshot.journal, &filter);
        let entry = view.diagnostics(&snapshot.journal, &today);
        let declared = view.declared(&snapshot.journal);
        let context = RowContext::new(&snapshot.journal, &declared);
        let window: Box<dyn Iterator<Item = &u32>> = if ascending {
            Box::new(positions.iter().rev().skip(offset).take(limit))
        } else {
            Box::new(positions.iter().skip(offset).take(limit))
        };
        let rows = window
            .map(|position| {
                let at = *position as usize;
                context.row(
                    &snapshot.journal.transactions[at],
                    flags_of(&entry.diagnostics, at),
                )
            })
            .collect();
        to_bytes(&RowsBody {
            generation: generation_of(&snapshot.etag),
            total: positions.len(),
            offset,
            rows,
        })
    })
    .await
}

/// `?from&to&accounts&q` — the filter alone.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilterQuery {
    from: Option<String>,
    to: Option<String>,
    accounts: Option<String>,
    q: Option<String>,
}

/// `{generation, …Summary}`.
#[derive(Serialize)]
struct WithGeneration<'a, T: Serialize> {
    generation: &'a str,
    #[serde(flatten)]
    body: T,
}

fn journal_view<'a>(
    snapshot: &'a Snapshot,
    classes: &'a Classes,
    signs: &'a SignTable,
    declared: &'a DeclaredStyles,
) -> JournalView<'a> {
    JournalView {
        journal: &snapshot.journal,
        classes,
        signs,
        declared,
    }
}

/// `GET /api/journal/summary` — for the filtered view: the count, the
/// footer's visible net, every commodity's income/expenses/net with its style
/// and categories, the commodities ranked by use, and the deepest account
/// ([`ledgeline_core::view::series::summary`]). Sign conventions are detected
/// over the whole journal and memoized per snapshot.
pub(crate) async fn summary_route(
    State(state): State<AppState>,
    Query(query): Query<FilterQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let filter = journal_filter(
        query.from,
        query.to,
        query.accounts.as_deref(),
        query.q.as_deref(),
    )?;
    conditional(state.snapshot(), &headers, move |snapshot| {
        let cache = &snapshot.view;
        let positions = cache.filtered(&snapshot.journal, &filter);
        let (classes, signs, declared) = (
            cache.classes(&snapshot.journal),
            cache.signs(&snapshot.journal),
            cache.declared(&snapshot.journal),
        );
        to_bytes(&WithGeneration {
            generation: generation_of(&snapshot.etag),
            body: summary(
                journal_view(snapshot, &classes, &signs, &declared),
                &positions,
                &filter,
            ),
        })
    })
    .await
}

/// `?<filter>&mode=pie|line&depth&commodity&category&interval&max`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChartQuery {
    from: Option<String>,
    to: Option<String>,
    accounts: Option<String>,
    q: Option<String>,
    mode: Option<String>,
    depth: Option<usize>,
    commodity: String,
    category: Option<String>,
    interval: Option<String>,
    max: Option<usize>,
}

/// Default and ceiling for a chart's group count.
const DEFAULT_GROUPS: usize = 6;
const MAX_GROUPS: usize = 50;
/// Deepest clamp a chart honours (account names are not deeper in practice).
const MAX_DEPTH: usize = 100;

fn chart_mode(mode: Option<&str>, interval: Option<&str>) -> Result<ChartMode, AppError> {
    match mode.unwrap_or("pie") {
        "pie" => Ok(ChartMode::Pie),
        "line" => match interval.unwrap_or("monthly") {
            "daily" => Ok(ChartMode::Line(Interval::Daily)),
            "weekly" => Ok(ChartMode::Line(Interval::Weekly)),
            "monthly" => Ok(ChartMode::Line(Interval::Monthly)),
            other => Err(AppError::BadRequest(format!(
                "interval must be daily, weekly or monthly, got {other:?}"
            ))),
        },
        other => Err(AppError::BadRequest(format!(
            "mode must be pie or line, got {other:?}"
        ))),
    }
}

/// `GET /api/journal/chart` — the insights pie or line for the filtered view,
/// one commodity, clamped to `depth` (default 2), scoped to `category` (absent
/// or `all` = every category), the tail past `max` groups (default 6) folded
/// into `(other)`. `groups` is the rank order colours are assigned from, so an
/// account keeps its colour whichever mode is drawn.
pub(crate) async fn chart_route(
    State(state): State<AppState>,
    Query(query): Query<ChartQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let filter = journal_filter(
        query.from,
        query.to,
        query.accounts.as_deref(),
        query.q.as_deref(),
    )?;
    let mode = chart_mode(query.mode.as_deref(), query.interval.as_deref())?;
    let category = match query.category.as_deref() {
        None | Some("" | "all") => None,
        Some(code) => Some(
            parse_category(code)
                .ok_or_else(|| AppError::BadRequest(format!("unknown category {code:?}")))?,
        ),
    };
    let spec = ChartSpec {
        depth: query.depth.unwrap_or(2).min(MAX_DEPTH),
        commodity: query.commodity,
        category,
        max: query.max.unwrap_or(DEFAULT_GROUPS).clamp(1, MAX_GROUPS),
    };
    conditional(state.snapshot(), &headers, move |snapshot| {
        let cache = &snapshot.view;
        let positions = cache.filtered(&snapshot.journal, &filter);
        let (classes, signs, declared) = (
            cache.classes(&snapshot.journal),
            cache.signs(&snapshot.journal),
            cache.declared(&snapshot.journal),
        );
        let view = journal_view(snapshot, &classes, &signs, &declared);
        to_bytes(&WithGeneration {
            generation: generation_of(&snapshot.etag),
            body: chart(view, &positions, &filter, &spec, mode),
        })
    })
    .await
}

/// The position of the transaction with `tindex` in `journal.transactions`.
/// Tindexes are assigned in file order, so the fast path is one probe; the
/// scan is the fallback should that ever not hold.
fn position_of(snapshot: &Snapshot, tindex: u32) -> Option<usize> {
    let txns = &snapshot.journal.transactions;
    let guess = usize::try_from(tindex).ok()?.checked_sub(1)?;
    if txns.get(guess).is_some_and(|txn| txn.index.0 == tindex) {
        return Some(guess);
    }
    txns.iter().position(|txn| txn.index.0 == tindex)
}

/// `?index=N&<filter>`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PositionQuery {
    index: u32,
    from: Option<String>,
    to: Option<String>,
    accounts: Option<String>,
    q: Option<String>,
}

/// `{generation, position, date, total}`.
#[derive(Serialize)]
struct PositionBody<'a> {
    generation: &'a str,
    /// 0-based row of the transaction in the filtered display order, or null
    /// when the filter hides it (or it does not exist).
    position: Option<usize>,
    /// The transaction's date, so a client can widen a date filter to reveal
    /// it; null when no transaction has this index.
    date: Option<&'a str>,
    /// The filtered count.
    total: usize,
}

/// `GET /api/journal/position?index=N&<filter>` — where the transaction with
/// `tindex` N sits in the filtered view: what a jump (problems drawer, a
/// cursor re-anchor) scrolls to without the client holding the journal.
pub(crate) async fn position_route(
    State(state): State<AppState>,
    Query(query): Query<PositionQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let filter = journal_filter(
        query.from,
        query.to,
        query.accounts.as_deref(),
        query.q.as_deref(),
    )?;
    let tindex = query.index;
    conditional(state.snapshot(), &headers, move |snapshot| {
        let positions = snapshot.view.filtered(&snapshot.journal, &filter);
        let at = position_of(snapshot, tindex);
        let row = at.and_then(|at| {
            let at = u32::try_from(at).ok()?;
            positions.iter().position(|p| *p == at)
        });
        to_bytes(&PositionBody {
            generation: generation_of(&snapshot.etag),
            position: row,
            date: at.map(|at| snapshot.journal.transactions[at].date.as_str()),
            total: positions.len(),
        })
    })
    .await
}

/// `?today=` for a single transaction's flags.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransactionQuery {
    today: Option<String>,
}

/// `GET /api/transactions/{index}` — one transaction as a row, flags and all:
/// what the edit popup opens on. A GET on the COLLECTION path stays
/// unanswered (`405`): the SPA's "can I edit?" probe depends on it.
pub(crate) async fn transaction(
    State(state): State<AppState>,
    Path(index): Path<u32>,
    Query(query): Query<TransactionQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let today = parse_date("today", query.today, today_utc)?;
    let snapshot = state.snapshot();
    let Some(at) = position_of(&snapshot, index) else {
        return Err(AppError::NotFound(format!(
            "no transaction with index {index}"
        )));
    };
    let etag = dated_etag(&snapshot, &today);
    conditional_tagged(snapshot, etag, &headers, move |snapshot| {
        let entry = snapshot.view.diagnostics(&snapshot.journal, &today);
        let declared = snapshot.view.declared(&snapshot.journal);
        let context = RowContext::new(&snapshot.journal, &declared);
        to_bytes(&context.row(
            &snapshot.journal.transactions[at],
            flags_of(&entry.diagnostics, at),
        ))
    })
    .await
}
