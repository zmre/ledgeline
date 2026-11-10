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
use crate::location::JournalLocation;
use crate::reports_api::{self, parse_date, parse_opt_date, today_utc};
use crate::review_api::{self, ReviewRow, ReviewWindow};
use crate::{AppState, Snapshot, generation_of, if_none_match_hits};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::Interval;
use ledgeline_core::review::ReviewIndex;
use ledgeline_core::view::Classes;
use ledgeline_core::view::cash::cash_balances_view;
use ledgeline_core::view::checks::{
    Diagnostics, EngineChecks, WireFlag, diagnostics_body, engine_checks, local_findings,
};
use ledgeline_core::view::filter::{JournalFilter, display_order, filter_positions};
use ledgeline_core::view::rows::{RowContext, TransferEnd, Transfers, haystack};
use ledgeline_core::view::running::{
    AssertionMark, BalanceWindow, PostingRef, RunningBalance, WireBalanceSeries, WireMark,
    WireRunning, WireRunningSummary, posting_order,
};
use ledgeline_core::view::series::{
    ChartMode, ChartSpec, JournalView, SignTable, chart, parse_category, sign_table, summary,
};
use ledgeline_core::view::wire::DeclaredStyles;
use ledgeline_core::{Journal, journals};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
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
    /// findings): whole-journal passes that do not depend on the date. Keeps
    /// the raw assertion failures from the same run for the running-balance
    /// marks, so the journal is never checked twice.
    engine: OnceLock<Arc<EngineChecks>>,
    /// Engine + local findings per client `today`, most recent last.
    diagnostics: Mutex<Vec<Arc<DiagnosticsEntry>>>,
    /// The declared commodity styles every row and summary renders with.
    declared: OnceLock<Arc<DeclaredStyles>>,
    /// Every transaction position in display order (date desc, tindex desc).
    order: OnceLock<Arc<Vec<u32>>>,
    /// Every transaction's lowercase search text, by position — built only
    /// when a filter first carries a query. A collapsing transfer half's is
    /// both halves' (plan 35 D11).
    haystacks: OnceLock<Arc<Vec<Box<str>>>>,
    /// The valid transfer links and which of them collapse into one row
    /// (plan 35 D11).
    transfers: OnceLock<Arc<Transfers>>,
    /// Every commodity's sign conventions over the whole journal.
    signs: OnceLock<Arc<SignTable>>,
    /// Recent filter results, most recently used last ([`FILTER_CACHE`]).
    filters: Mutex<Vec<(JournalFilter, Arc<Filtered>)>>,
    /// Every posting in hledger's evaluation order (plan 36). Lazy, NOT
    /// prewarmed: most sessions never pin an account.
    posting_order: OnceLock<Arc<Vec<PostingRef>>>,
    /// Recent pinned subtrees' running balances, most recently used last
    /// ([`RUNNING_CACHE`]). One build covers every commodity, so switching the
    /// commodity picker costs nothing.
    running: Mutex<Vec<(String, Arc<Running>)>>,
    /// Where the journal lives, for the review index's "can this file be
    /// written?" question. `None` for a journal with no backing file, whose
    /// transactions are then all "not editable".
    storage: Option<Arc<JournalLocation>>,
    /// The review index (plan 32 D14): every transaction's review state,
    /// whether it is unknown and editable, and the similarity-key buckets.
    review: OnceLock<Arc<ReviewIndex>>,
    /// The `GET /api/payees` body.
    pub(crate) payees: OnceLock<Bytes>,
    /// The `GET /api/tags` body.
    pub(crate) tags: OnceLock<Bytes>,
}

/// What one filter keeps, in display order.
pub(crate) struct Filtered {
    /// Every transaction the filter keeps: what totals and charts sum over.
    pub(crate) positions: Vec<u32>,
    /// The rows those transactions show as — `positions` with each linked
    /// transfer pair collapsed to its anchoring half (plan 35 D11): what
    /// `total`, offsets and positions count.
    pub(crate) rows: Vec<u32>,
}

/// How many filter results each snapshot keeps. Scrolling one view asks for
/// the same filter block after block, and the summary, chart and position
/// routes ask for it again; a handful covers flipping between recent filters.
const FILTER_CACHE: usize = 8;

/// How many pinned roots each snapshot keeps a running balance for.
const RUNNING_CACHE: usize = 4;

/// A pinned subtree's running balance and its assertion marks, built together.
pub(crate) struct Running {
    balance: RunningBalance,
    marks: BTreeMap<Commodity, Vec<AssertionMark>>,
}

impl ViewCache {
    /// A fresh cache for `journal`, warmed on a background thread so the
    /// first requests after a load or an edit find the expensive derivations
    /// already built (see [`ViewCache::prewarm`]).
    pub(crate) fn warmed(
        journal: &Arc<Journal>,
        storage: Option<Arc<JournalLocation>>,
    ) -> Arc<Self> {
        let cache = Arc::new(Self {
            storage,
            ..Self::default()
        });
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
    /// counts them), then the summary's inputs (its review counts among them),
    /// then the search text.
    ///
    /// Each step re-checks that the snapshot still exists: an edit publishes a
    /// new snapshot, and warming a superseded one is wasted work. Everything is
    /// a `OnceLock`, so a request racing a step simply shares its answer.
    fn prewarm(cache: &std::sync::Weak<Self>, journal: &Journal) {
        let today = today_utc();
        let steps: [&dyn Fn(&Self); 7] = [
            &|view| drop(view.order(journal)),
            &|view| drop(view.transfers(journal)),
            &|view| drop(view.diagnostics(journal, &today)),
            &|view| drop(view.signs(journal)),
            &|view| drop(view.declared(journal)),
            &|view| drop(view.review(journal)),
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
            let mut haystacks: Vec<Box<str>> = journal
                .transactions
                .iter()
                .map(|txn| haystack(txn, &declared).into_boxed_str())
                .collect();
            self.transfers(journal).join_haystacks(&mut haystacks);
            Arc::new(haystacks)
        }))
    }

    /// The transfer links of `journal`, built on first use.
    pub(crate) fn transfers(&self, journal: &Journal) -> Arc<Transfers> {
        Arc::clone(
            self.transfers
                .get_or_init(|| Arc::new(Transfers::of(journal))),
        )
    }

    pub(crate) fn classes(&self, journal: &Journal) -> Arc<Classes> {
        Arc::clone(self.classes.get_or_init(|| Arc::new(Classes::of(journal))))
    }

    /// The review index for `journal`, built on first use. A transaction is
    /// editable when its file is one a write may target
    /// ([`journals::writable_sources`] — the same judgement as every journal
    /// file API's `writable` flag).
    pub(crate) fn review(&self, journal: &Journal) -> Arc<ReviewIndex> {
        Arc::clone(self.review.get_or_init(|| {
            let writable: HashSet<PathBuf> = self
                .storage
                .as_deref()
                .map_or_else(HashSet::new, |location| {
                    journals::writable_sources(&**location.vault(), journal)
                });
            Arc::new(ReviewIndex::build(
                journal,
                &self.classes(journal),
                |file| writable.contains(file),
            ))
        }))
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
            let engine = scope.spawn(|| self.engine(journal));
            let local = local_findings(journal, &self.classes(journal), today);
            let engine = engine
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
            (engine, local)
        });
        let entry = Arc::new(DiagnosticsEntry {
            today: today.to_string(),
            diagnostics: Arc::new(Diagnostics::new(&engine.findings, local)),
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

    fn engine(&self, journal: &Journal) -> Arc<EngineChecks> {
        Arc::clone(self.engine.get_or_init(|| Arc::new(engine_checks(journal))))
    }

    fn posting_order(&self, journal: &Journal) -> Arc<Vec<PostingRef>> {
        Arc::clone(
            self.posting_order
                .get_or_init(|| Arc::new(posting_order(journal))),
        )
    }

    /// The running balance of `root`'s subtree — from the cache when a recent
    /// request pinned the same root. `400` when no posting is in the subtree:
    /// an empty column would read as a zero balance, which it is not.
    fn running(&self, journal: &Journal, root: &str) -> Result<Arc<Running>, AppError> {
        {
            let mut cache = self.lock_running();
            if let Some(at) = cache.iter().position(|(key, _)| key == root) {
                let hit = cache.remove(at);
                let running = Arc::clone(&hit.1);
                cache.push(hit);
                return Ok(running);
            }
        }
        let balance = RunningBalance::build(journal, &self.posting_order(journal), root).map_err(
            |error| {
                AppError::Internal(format!(
                    "the running balance of {root:?} overflowed: {error}"
                ))
            },
        )?;
        if balance.is_empty() {
            return Err(AppError::BadRequest(format!(
                "no posting is in the account {root:?} or under it"
            )));
        }
        let marks = balance.assertion_marks(journal, &self.engine(journal).assertion_failures);
        let running = Arc::new(Running { balance, marks });
        let mut cache = self.lock_running();
        if cache.len() == RUNNING_CACHE {
            cache.remove(0);
        }
        cache.push((root.to_string(), Arc::clone(&running)));
        Ok(running)
    }

    fn lock_running(&self) -> std::sync::MutexGuard<'_, Vec<(String, Arc<Running>)>> {
        // Entries are pushed and removed whole; a poisoned lock is still sound.
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
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

    /// The positions `filter` keeps and the rows they show as, in display
    /// order — from the cache when a recent request asked the same question.
    fn filtered(&self, journal: &Journal, filter: &JournalFilter) -> Arc<Filtered> {
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
        let positions = filter_positions(
            journal,
            &order,
            haystacks.as_deref().map(Vec::as_slice),
            filter,
        );
        let rows = self.transfers(journal).collapse(&positions);
        let positions = Arc::new(Filtered { positions, rows });
        let mut cache = self.lock_filters();
        if cache.len() == FILTER_CACHE {
            cache.remove(0);
        }
        cache.push((filter.clone(), Arc::clone(&positions)));
        positions
    }

    fn lock_filters(&self) -> std::sync::MutexGuard<'_, Vec<(JournalFilter, Arc<Filtered>)>> {
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
pub(crate) fn dated_etag(snapshot: &Snapshot, today: &str) -> HeaderValue {
    let opaque = snapshot.etag.to_str().unwrap_or_default().trim_matches('"');
    HeaderValue::from_str(&format!("\"{opaque}-{today}\""))
        .unwrap_or_else(|_| snapshot.etag.clone())
}

/// A `200` JSON body carrying `etag`.
pub(crate) fn ok_json(etag: HeaderValue, body: Bytes) -> Response {
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
pub(crate) fn not_modified(etag: HeaderValue) -> Response {
    Response::builder()
        .status(StatusCode::NOT_MODIFIED)
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Answer from `job` on the blocking pool, or `304` without running it when
/// the client already holds this snapshot's body for this URL.
pub(crate) async fn conditional<F>(
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
pub(crate) async fn conditional_tagged<F>(
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
pub(crate) fn to_bytes<T: serde::Serialize>(value: &T) -> Result<Bytes, AppError> {
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

/// `?from&to&accounts&q&offset&limit&order&today&balance&balanceCommodity&collapse`.
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
    /// The pinned account (plan 36): every row gains a `running` object.
    balance: Option<String>,
    /// The pinned commodity; absent or not posted in the subtree = the
    /// subtree's default.
    balance_commodity: Option<String>,
    /// `false`: one row per transaction, no transfer rows — what an export
    /// that writes transactions reads. Default `true`.
    collapse: Option<bool>,
}

/// `{generation, total, offset, rows}`.
#[derive(Serialize)]
struct RowsBody<'a> {
    generation: &'a str,
    total: usize,
    offset: usize,
    rows: Vec<JournalRow<'a>>,
}

/// One `/api/journal/rows` row: a [`ReviewRow`], plus (plan 35 D11)
///
/// - `linked: true` when the transaction has a half in any valid transfer
///   link — the ⇄ icon — including one in two links, which does not
///   collapse; absent otherwise;
/// - `transfer` when the row IS a collapsed linked pair: the row itself is
///   the anchoring half, and `transfer` carries both halves (earliest
///   first) as full rows, the `match` id, and the two ends;
/// - `running` when an account is pinned (plan 36): the cell of the row's
///   own transaction — for a transfer row, the anchoring half's, which is
///   the half the pin's account filter keeps.
#[derive(Serialize)]
pub(crate) struct JournalRow<'a> {
    #[serde(flatten)]
    row: ReviewRow<'a>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    linked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    transfer: Option<WireTransfer<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    running: Option<WireRunning<'a>>,
}

/// `{matchId, halves: [row, row], from, to}`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireTransfer<'a> {
    match_id: &'a str,
    halves: [ReviewRow<'a>; 2],
    from: WireTransferEnd<'a>,
    to: WireTransferEnd<'a>,
}

/// `{index, account, label}`: one end of a transfer — its transaction's
/// `tindex`, its real account and that account's short label ("checking"),
/// either null when the half has no real posting.
#[derive(Serialize)]
pub(crate) struct WireTransferEnd<'a> {
    pub(crate) index: u32,
    pub(crate) account: Option<&'a str>,
    pub(crate) label: Option<&'a str>,
}

impl<'a> WireTransferEnd<'a> {
    fn of(journal: &Journal, end: &'a TransferEnd) -> Self {
        Self {
            index: journal.transactions[end.position as usize].index.0,
            account: end.account.as_deref(),
            label: end.label.as_deref(),
        }
    }
}

/// The pinned column for one request: the cached balance, the commodity
/// drawn, and its marks by transaction position.
struct Pin<'a> {
    running: &'a Running,
    commodity: &'a Commodity,
    /// Marks shown on a row: those in `commodity`, plus every zero-check
    /// failure whatever its commodity.
    marks: HashMap<u32, Vec<&'a AssertionMark>>,
}

impl<'a> Pin<'a> {
    fn new(running: &'a Running, requested: Option<&str>) -> Result<Self, AppError> {
        let commodity = running
            .balance
            .resolve_commodity(requested)
            .ok_or_else(|| AppError::Internal("an empty running balance was cached".into()))?;
        let mut marks: HashMap<u32, Vec<&AssertionMark>> = HashMap::new();
        for mark in running
            .marks
            .values()
            .flatten()
            .filter(|mark| mark.commodity == *commodity || mark.zero_check)
        {
            marks.entry(mark.at.position).or_default().push(mark);
        }
        Ok(Self {
            running,
            commodity,
            marks,
        })
    }

    fn cell(
        &self,
        journal: &'a Journal,
        context: &RowContext<'_>,
        position: u32,
    ) -> Result<WireRunning<'a>, AppError> {
        let balance = &self.running.balance;
        let cell = balance
            .cell(journal, &self.commodity.0, position)
            .ok_or_else(|| AppError::Internal(format!("no row at position {position}")))?;
        let style = balance
            .style(&self.commodity.0)
            .ok_or_else(|| AppError::Internal("the pinned commodity has no stream".into()))?;
        let marks = self
            .marks
            .get(&position)
            .into_iter()
            .flatten()
            .map(|mark| WireMark::new(mark, context))
            .collect();
        Ok(WireRunning::new(
            cell,
            self.commodity,
            style,
            context,
            marks,
        ))
    }
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
pub(crate) fn flags_of(diagnostics: &Diagnostics, position: usize) -> Vec<WireFlag<'_>> {
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
///
/// Each linked transfer pair is ONE row (plan 35 D11, see [`JournalRow`]),
/// and `total` and `offset` count those rows. `collapse=false` serves every
/// kept transaction as its own row instead (still marked `linked`), for a
/// consumer that needs transactions, not rows.
///
/// With `balance=<account>` (plan 36) every row also carries `running`: the
/// subtree's change and balance in `balanceCommodity` (or the default) and
/// the assertion marks on it. Neither the balance nor the marks depend on the
/// filter. An account with no posting in its subtree is a `400`. Without
/// `balance` (or with it empty) there is no `running`.
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
    let pin = query.balance.filter(|account| !account.is_empty());
    let pin_commodity = query.balance_commodity;
    let collapse = query.collapse.unwrap_or(true);
    let snapshot = state.snapshot();
    let etag = dated_etag(&snapshot, &today);
    conditional_tagged(snapshot, etag, &headers, move |snapshot| {
        let journal = &snapshot.journal;
        let view = &snapshot.view;
        let running = pin
            .as_deref()
            .map(|root| view.running(journal, root))
            .transpose()?;
        let pin = running
            .as_deref()
            .map(|running| Pin::new(running, pin_commodity.as_deref()))
            .transpose()?;
        let filtered = view.filtered(journal, &filter);
        let entry = view.diagnostics(journal, &today);
        let declared = view.declared(journal);
        let review = view.review(journal);
        let transfers = view.transfers(journal);
        let context = RowContext::new(journal, &declared);
        let review_row = |position: u32| {
            let at = position as usize;
            ReviewRow::at(
                &review,
                at,
                context.row(&journal.transactions[at], flags_of(&entry.diagnostics, at)),
            )
        };
        let shown = if collapse {
            &filtered.rows
        } else {
            &filtered.positions
        };
        let window: Box<dyn Iterator<Item = &u32>> = if ascending {
            Box::new(shown.iter().rev().skip(offset).take(limit))
        } else {
            Box::new(shown.iter().skip(offset).take(limit))
        };
        let rows = window
            .map(|&position| {
                Ok(JournalRow {
                    row: review_row(position),
                    linked: transfers.is_linked(position),
                    transfer: collapse
                        .then(|| transfers.get(position))
                        .flatten()
                        .map(|transfer| WireTransfer {
                            match_id: &transfer.id,
                            halves: transfer.halves.map(review_row),
                            from: WireTransferEnd::of(journal, &transfer.from),
                            to: WireTransferEnd::of(journal, &transfer.to),
                        }),
                    running: pin
                        .as_ref()
                        .map(|pin| pin.cell(journal, &context, position))
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        to_bytes(&RowsBody {
            generation: generation_of(&snapshot.etag),
            total: shown.len(),
            offset,
            rows,
        })
    })
    .await
}

/// `?account=<root>`.
#[derive(Deserialize)]
pub(crate) struct RunningQuery {
    account: String,
}

/// `GET /api/journal/running?account=<root>` (plan 36) — what the pinned
/// account's chip and commodity picker draw: `{generation, account,
/// commodities, assertions}`.
///
/// `commodities` is every commodity the subtree posts with its posting count,
/// most postings first (ties by commodity); the first is the default
/// `rows?balance=` uses. `assertions` maps each commodity to its assertion
/// marks `{tindex, date, ok}` in hledger's evaluation order — every asserting
/// posting in the subtree under its written commodity, plus each `==`
/// zero-check failure under the commodity it failed on. An account with no
/// posting in its subtree is a `400`.
pub(crate) async fn running_route(
    State(state): State<AppState>,
    Query(query): Query<RunningQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let root = query.account;
    conditional(state.snapshot(), &headers, move |snapshot| {
        let journal = &snapshot.journal;
        let running = snapshot.view.running(journal, &root)?;
        to_bytes(&WithGeneration {
            generation: generation_of(&snapshot.etag),
            body: WireRunningSummary::new(journal, &running.balance, &running.marks),
        })
    })
    .await
}

/// `?account&commodity&from&to&today&interval`.
#[derive(Deserialize)]
pub(crate) struct RunningSeriesQuery {
    account: String,
    commodity: Option<String>,
    from: Option<String>,
    to: Option<String>,
    today: Option<String>,
    interval: Option<String>,
}

/// `GET /api/journal/running/series?account&commodity&from&to&today&interval`
/// (plan 36 amendment A1) — the pinned subtree's balance at the end of every
/// period of the journal filter's date range: `{generation, account,
/// commodity, style, interval, from, to, points}`.
///
/// Built from the same cached running balance as `rows?balance=`, so pinning
/// computes it once for both. `commodity` resolves exactly as the rows
/// route's `balanceCommodity` does (absent or not posted in the subtree = the
/// default). An open `from` is the subtree's first posting, an open `to` the
/// client's `today`; `interval` (daily … yearly) defaults to the holdings
/// chart's automatic choice. Each point is `{bucket, label, end, balance,
/// tindex?, failures?}`, a failure `{tindex, date, account, op, asserted,
/// calculated, expected}`. An account with no posting in its subtree, a
/// backwards range or one with more than `MAX_BUCKETS` periods is a `400`.
/// The `ETag` is dated by `today`, which an open end depends on.
pub(crate) async fn running_series_route(
    State(state): State<AppState>,
    Query(query): Query<RunningSeriesQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let from = parse_opt_date("from", query.from)?;
    let to = parse_opt_date("to", query.to)?;
    let today = parse_date("today", query.today, today_utc)?;
    let interval = query
        .interval
        .filter(|interval| !interval.is_empty())
        .map(|interval| reports_api::parse_interval(Some(&interval)))
        .transpose()?;
    let root = query.account;
    let requested = query.commodity.filter(|commodity| !commodity.is_empty());
    let snapshot = state.snapshot();
    let etag = dated_etag(&snapshot, &today);
    conditional_tagged(snapshot, etag, &headers, move |snapshot| {
        let journal = &snapshot.journal;
        let running = snapshot.view.running(journal, &root)?;
        let balance = &running.balance;
        let (Some(commodity), Some(first)) = (
            balance.resolve_commodity(requested.as_deref()),
            balance.first_date(journal),
        ) else {
            return Err(AppError::Internal(
                "an empty running balance was cached".into(),
            ));
        };
        let window =
            BalanceWindow::resolve(first, from.as_deref(), to.as_deref(), &today, interval);
        reports_api::check_range(&window.from, &window.to, Some(window.interval))?;
        let marks = running.marks.get(commodity).map_or(&[][..], Vec::as_slice);
        let points = balance
            .series(journal, &commodity.0, &window, marks)
            .map_err(|error| {
                AppError::Internal(format!("the balance series of {root:?} failed: {error}"))
            })?;
        let declared = snapshot.view.declared(journal);
        to_bytes(&WithGeneration {
            generation: generation_of(&snapshot.etag),
            body: WireBalanceSeries::new(journal, balance, commodity, &window, &points, &declared),
        })
    })
    .await
}

/// `?from&to&accounts&q&today&reviewLimit&reviewMaxDays` — the filter, plus
/// the client's date and review settings for the (unfiltered) review counts.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryQuery {
    from: Option<String>,
    to: Option<String>,
    accounts: Option<String>,
    q: Option<String>,
    today: Option<String>,
    review_limit: Option<usize>,
    review_max_days: Option<u32>,
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

/// `{generation, …Summary, transfers, review}`.
#[derive(Serialize)]
struct SummaryBody<'a, T: Serialize> {
    generation: &'a str,
    #[serde(flatten)]
    body: T,
    /// How many of the `total` rows are collapsed transfer pairs (plan 35
    /// D11): nonzero means `total` counts rows, not transactions.
    transfers: usize,
    review: review_api::ReviewCounts,
}

/// `GET /api/journal/summary` — for the filtered view: the count, the
/// footer's visible net, every commodity's income/expenses/net with its style
/// and categories, the commodities ranked by use, and the deepest account
/// ([`ledgeline_core::view::series::summary`]). Sign conventions are detected
/// over the whole journal and memoized per snapshot.
///
/// Plus `review` (plan 32 D18): the two review queues' sizes as of the
/// client's `today`, which IGNORE the filter (D4 Q3) — the Journal toolbar's
/// button badges. Because they depend on the date, the `ETag` does too.
pub(crate) async fn summary_route(
    State(state): State<AppState>,
    Query(query): Query<SummaryQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let filter = journal_filter(
        query.from,
        query.to,
        query.accounts.as_deref(),
        query.q.as_deref(),
    )?;
    let review = ReviewWindow::resolve(query.today, query.review_limit, query.review_max_days)?;
    let snapshot = state.snapshot();
    let etag = dated_etag(&snapshot, &review.today);
    conditional_tagged(snapshot, etag, &headers, move |snapshot| {
        let cache = &snapshot.view;
        let filtered = cache.filtered(&snapshot.journal, &filter);
        let (classes, signs, declared) = (
            cache.classes(&snapshot.journal),
            cache.signs(&snapshot.journal),
            cache.declared(&snapshot.journal),
        );
        let transfers = cache.transfers(&snapshot.journal);
        to_bytes(&SummaryBody {
            generation: generation_of(&snapshot.etag),
            // The figures sum every kept transaction's postings; the count is
            // the rows the journal shows.
            body: summary(
                journal_view(snapshot, &classes, &signs, &declared),
                &filtered.positions,
                &filter,
            )
            .with_total(filtered.rows.len()),
            transfers: filtered
                .rows
                .iter()
                .filter(|&&position| transfers.get(position).is_some())
                .count(),
            review: review.counts(&snapshot.journal, &cache.review(&snapshot.journal)),
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
        let filtered = cache.filtered(&snapshot.journal, &filter);
        let (classes, signs, declared) = (
            cache.classes(&snapshot.journal),
            cache.signs(&snapshot.journal),
            cache.declared(&snapshot.journal),
        );
        let view = journal_view(snapshot, &classes, &signs, &declared);
        // Postings, never rows: a collapsed transfer still charts both halves.
        to_bytes(&WithGeneration {
            generation: generation_of(&snapshot.etag),
            body: chart(view, &filtered.positions, &filter, &spec, mode),
        })
    })
    .await
}

/// The position of the transaction with `tindex` in `journal.transactions`.
/// Tindexes are assigned in file order, so the fast path is one probe; the
/// scan is the fallback should that ever not hold.
pub(crate) fn position_of(snapshot: &Snapshot, tindex: u32) -> Option<usize> {
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
    /// when the filter hides it (or it does not exist). A transfer half that
    /// the filter keeps but whose pair is anchored at its partner is at its
    /// partner's row (plan 35 D11).
    position: Option<usize>,
    /// The transaction's date, so a client can widen a date filter to reveal
    /// it; null when no transaction has this index.
    date: Option<&'a str>,
    /// The filtered row count.
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
        let filtered = snapshot.view.filtered(&snapshot.journal, &filter);
        let transfers = snapshot.view.transfers(&snapshot.journal);
        let at = position_of(snapshot, tindex);
        let row = at.and_then(|at| {
            let at = u32::try_from(at).ok()?;
            let row_of = |position: u32| filtered.rows.iter().position(|p| *p == position);
            row_of(at).or_else(|| {
                let partner = transfers.get(at)?.partner(at);
                filtered.positions.contains(&at).then(|| row_of(partner))?
            })
        });
        to_bytes(&PositionBody {
            generation: generation_of(&snapshot.etag),
            position: row,
            date: at.map(|at| snapshot.journal.transactions[at].date.as_str()),
            total: filtered.rows.len(),
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
        let review = snapshot.view.review(&snapshot.journal);
        let context = RowContext::new(&snapshot.journal, &declared);
        to_bytes(&ReviewRow::at(
            &review,
            at,
            context.row(
                &snapshot.journal.transactions[at],
                flags_of(&entry.diagnostics, at),
            ),
        ))
    })
    .await
}
