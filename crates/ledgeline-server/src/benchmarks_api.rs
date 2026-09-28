//! `GET /api/holdings/benchmarks` — the Stocks tab's "what if the same money had
//! gone into an index fund" overlay.
//!
//! For each requested benchmark this answers one line, index-aligned to the
//! value-over-time series for the same scope and window: what the portfolio
//! would be worth had every contribution and withdrawal, on its own date, gone
//! into that benchmark instead. The arithmetic is the engine's
//! (`ledgeline_core::holdings::benchmark`); this module supplies its inputs —
//! the series, the dated flows, and the benchmark's dividend-adjusted history —
//! and keeps that history on disk so it is fetched once, not per chart.
//!
//! # The price cache
//!
//! History lives in `benchmarks.prices.journal` BESIDE the main journal, as
//! ordinary hledger `P` directives (`P 2026-09-25 SPY 766.2900 USD`) under a
//! header saying what they are. It is deliberately NOT `include`d anywhere:
//! these are dividend-adjusted closes, which are not market prices and must
//! never value a real holding of SPY. Same format so any hledger tool can read
//! it; separate file so none ever does by accident. Deleting it loses nothing
//! but a re-download. It is written with `atomic_write` and never passed to the
//! git safety net (that only ever commits the paths an import names), and it is
//! outside the journal's include tree, so the file watcher ignores it too.
//!
//! A read-only session (no editor: `AppState::editing_enabled` is false) writes
//! nothing beside the journal. It keeps the same cache in memory instead, for
//! the life of the session, exactly as `profiles_api` does.
//!
//! Each symbol carries a coverage comment (`; ledgeline-benchmark SPY from
//! 2021-09-17 through 2026-09-27`) so a later request knows what has already
//! been asked for — including ranges where the fund had not yet traded, which
//! would otherwise be re-requested forever. Only the missing TAIL is fetched
//! once a symbol is covered, and only sessions before today are stored: today's
//! candle is still forming.
//!
//! Adjusted closes are re-based by Yahoo every time a fund pays a dividend —
//! the whole history is scaled by a factor. A tail fetched after that event is
//! on the new scale and the cached body on the old, so the tail always overlaps
//! the last cached session and the cached body is re-scaled by the ratio on
//! that day. Without it every dividend would leave a step in the line.
//!
//! # Failure
//!
//! A benchmark is decoration on a chart that is already drawn. A failed fetch
//! is reported on that benchmark (`error`, or `stale` when the cache could still
//! answer) with a `200`; only a malformed REQUEST is a `400`.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use futures::stream::{self, StreamExt};
use ledgeline_core::holdings::benchmark::{Close, simulate_benchmark};
use ledgeline_core::holdings::{DatedFlow, HoldingsPoint, holdings_flows, holdings_series};
use ledgeline_core::reports::periods::add_days;
use ledgeline_core::{Dec, parse_journal};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::AppState;
use crate::error::AppError;
use crate::reports_api::{HoldingsTab, compute, holdings_scope, series_window, today_utc};
use crate::yahoo::{FetchedPrice, YahooError};
use crate::yahoo_history::HistoryFeed;

/// One benchmark the overlay offers: a total-return ETF proxy for a standard
/// measure, labelled by the measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Benchmark {
    pub(crate) symbol: &'static str,
    pub(crate) label: &'static str,
}

/// Every benchmark the overlay can draw, in display order. The SPA keeps the
/// same list (`web/src/lib/holdings/benchmarks.ts`) because its order fixes
/// each line's colour; a symbol outside this list is a `400`, which also keeps
/// the server from being asked to fetch arbitrary tickers.
pub(crate) const BENCHMARKS: &[Benchmark] = &[
    Benchmark {
        symbol: "SPY",
        label: "S&P 500 (SPY)",
    },
    Benchmark {
        symbol: "DIA",
        label: "Dow Jones (DIA)",
    },
    Benchmark {
        symbol: "QQQ",
        label: "Nasdaq-100 (QQQ)",
    },
    Benchmark {
        symbol: "VTI",
        label: "US total market (VTI)",
    },
    Benchmark {
        symbol: "BND",
        label: "US bonds (BND)",
    },
    Benchmark {
        symbol: "VXUS",
        label: "International (VXUS)",
    },
    Benchmark {
        symbol: "IWM",
        label: "Small cap (IWM)",
    },
    Benchmark {
        symbol: "GLD",
        label: "Gold (GLD)",
    },
];

/// The cache file's name, beside the main journal.
pub(crate) const CACHE_FILE: &str = "benchmarks.prices.journal";

/// The commodity every cached close is quoted in.
const QUOTE: &str = "USD";

/// The base commodities a USD-quoted benchmark can be compared against without
/// a currency conversion.
const USD_BASES: &[&str] = &["$", "USD", "US$"];

/// How far before the first chart point history is requested, so a point on a
/// weekend or after a long holiday still finds the previous session's close.
const LOOKBACK_DAYS: i64 = 10;

/// Concurrent fetches for one request (at most the catalog's eight).
const FETCH_CONCURRENCY: usize = 4;

/// The coverage comment's prefix. hledger reads it as a comment.
const COVERAGE: &str = "; ledgeline-benchmark ";

const CACHE_HEADER: &str = "\
; Ledgeline benchmark price cache.
;
; Daily closing prices for the index funds the Holdings tab can compare your
; portfolio against, ADJUSTED FOR DIVIDENDS AND SPLITS (Yahoo Finance's
; \"adjusted close\"), so a line drawn from them includes reinvested income.
;
; These are not market prices. This file is NOT included in your journal and
; must not be: an adjusted close would misvalue a real holding of the same
; fund. Ledgeline reads and rewrites it on its own; deleting it is safe and
; only costs a re-download.
";

// ===========================================================================
// Wire types
// ===========================================================================

/// `?symbols=SPY,QQQ&asOf=&accounts=&mode=&interval=&count=&since=&valueIn=`.
///
/// Everything but `symbols` is `/api/holdings/series`' query, field for field
/// and validated by the same functions, so the overlay is computed for exactly
/// the window the base chart was.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchmarksQuery {
    symbols: Option<String>,
    as_of: Option<String>,
    accounts: Option<String>,
    mode: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    since: Option<String>,
    value_in: Option<String>,
}

/// The response: one line per requested benchmark, in catalog order.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBenchmarks {
    base: String,
    benchmarks: Vec<WireBenchmark>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBenchmark {
    symbol: String,
    label: String,
    /// Index-aligned to the series' points; `value` is `null` where the
    /// benchmark has no price yet (a gap, not a zero).
    points: Vec<WireBenchmarkPoint>,
    /// The last session the cached history reaches, if any.
    priced_through: Option<String>,
    /// A refresh was needed and failed, so the line is drawn from an older
    /// cache and its last points may be flat.
    stale: bool,
    /// Portfolio flows that could not be valued and so are missing from the
    /// simulation (see `HoldingsFlows::unvalued`).
    unvalued_flows: usize,
    /// Why there is no line at all, when there is not.
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireBenchmarkPoint {
    date: String,
    /// Display-boundary number, rounded to cents: a hypothetical line, never a
    /// figure anyone reconciles.
    value: Option<f64>,
}

// ===========================================================================
// The cache file
// ===========================================================================

/// One symbol's cached history.
#[derive(Debug, Clone, PartialEq, Default)]
struct History {
    /// The earliest date this history has been REQUESTED from — which may be
    /// before its first close, for a fund younger than the request.
    from: String,
    /// The last date the history is complete through.
    through: String,
    /// Ascending, one per session.
    closes: Vec<(String, Dec)>,
}

type Cache = BTreeMap<String, History>;

/// A read-only session's benchmark history: fetched and served exactly as the
/// file's would be, but kept in memory and never written beside the journal —
/// the same rule `profiles_api` follows for its cache. Unused (empty) in a
/// session that may write.
#[derive(Debug, Default)]
pub(crate) struct SessionCache(Cache);

/// Where this request's cache lives.
#[derive(Debug, Clone)]
enum Store {
    /// `benchmarks.prices.journal` beside the journal (an editing session).
    Disk(PathBuf),
    /// [`SessionCache`], in memory (a read-only session, or one with no
    /// journal file to sit beside).
    Memory,
}

impl Store {
    fn new(cache_path: Option<PathBuf>, editable: bool) -> Self {
        match cache_path {
            Some(path) if editable => Self::Disk(path),
            _ => Self::Memory,
        }
    }
}

/// A `Dec` as plain decimal text (`766.29`, `-0.5`), exact.
fn plain(value: Dec) -> String {
    let digits = value.mantissa.unsigned_abs().to_string();
    let places = value.places as usize;
    let sign = if value.mantissa < 0 { "-" } else { "" };
    if places == 0 {
        return format!("{sign}{digits}");
    }
    let padded = format!("{digits:0>width$}", width = places + 1);
    let (whole, fraction) = padded.split_at(padded.len() - places);
    format!("{sign}{whole}.{fraction}")
}

fn render_cache(cache: &Cache) -> String {
    let mut out = String::from(CACHE_HEADER);
    for (symbol, history) in cache {
        out.push('\n');
        out.push_str(&format!(
            "{COVERAGE}{symbol} from {} through {}\n",
            history.from, history.through
        ));
        for (date, close) in &history.closes {
            out.push_str(&format!("P {date} {symbol} {} {QUOTE}\n", plain(*close)));
        }
    }
    out
}

/// Read a cache file's text back: the `P` lines through the project's own
/// journal parser (so the file is provably something hledger reads), the
/// coverage comments by hand. A symbol with prices but no coverage line is
/// covered exactly as far as its prices reach.
fn parse_cache(text: &str) -> Result<Cache, String> {
    let journal = parse_journal(text, CACHE_FILE).map_err(|error| error.to_string())?;
    let mut cache: Cache = BTreeMap::new();
    for price in journal.prices {
        if price.price.commodity.0 != QUOTE {
            continue;
        }
        cache
            .entry(price.commodity.0)
            .or_default()
            .closes
            .push((price.date, price.price.quantity));
    }
    for history in cache.values_mut() {
        history.closes.sort_by(|a, b| a.0.cmp(&b.0));
        history.closes.dedup_by(|later, earlier| {
            let same = later.0 == earlier.0;
            if same {
                earlier.1 = later.1;
            }
            same
        });
        history.from = history
            .closes
            .first()
            .map(|c| c.0.clone())
            .unwrap_or_default();
        history.through = history
            .closes
            .last()
            .map(|c| c.0.clone())
            .unwrap_or_default();
    }
    for line in text.lines() {
        let Some(rest) = line.strip_prefix(COVERAGE) else {
            continue;
        };
        let words: Vec<&str> = rest.split_whitespace().collect();
        if let [symbol, "from", from, "through", through] = words.as_slice() {
            let history = cache.entry((*symbol).to_string()).or_default();
            history.from = (*from).to_string();
            history.through = (*through).to_string();
        }
    }
    Ok(cache)
}

/// The cache beside the journal, or empty when there is none — or when it
/// cannot be read or parsed: it is a cache, and the next write replaces it.
fn read_cache(path: &Path) -> Cache {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_cache(&text).unwrap_or_else(|error| {
            eprintln!("ledgeline: ignoring an unreadable {CACHE_FILE}: {error}");
            Cache::new()
        }),
        Err(_) => Cache::new(),
    }
}

// ===========================================================================
// Planning and merging a fetch
// ===========================================================================

/// What one symbol needs fetched to cover `need_from..=need_through`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fetch {
    /// Already covered.
    Nothing,
    /// The whole range, replacing whatever is cached.
    Full { from: String },
    /// From the last cached session on, merged onto the cached body.
    Tail { from: String },
}

fn plan_fetch(cached: Option<&History>, need_from: &str, need_through: &str) -> Fetch {
    let full = || Fetch::Full {
        from: need_from.to_string(),
    };
    let Some(history) = cached else {
        return full();
    };
    if history.from.is_empty() || need_from < history.from.as_str() {
        // Extend the start by refetching everything: the head and the cached
        // body would otherwise be on different dividend scales.
        return full();
    }
    if history.through.as_str() >= need_through {
        return Fetch::Nothing;
    }
    match history.closes.last() {
        Some((last, _)) => Fetch::Tail { from: last.clone() },
        // Requested before but nothing traded yet: ask again from the start.
        None => Fetch::Full {
            from: history.from.clone(),
        },
    }
}

impl Fetch {
    /// The first date this fetch asks for; `None` when nothing is fetched.
    fn start(&self) -> Option<&str> {
        match self {
            Self::Full { from } | Self::Tail { from } => Some(from),
            Self::Nothing => None,
        }
    }
}

/// Fold a fetch into a symbol's history. `None` for a tail that does not
/// overlap the cached body — nothing to re-scale by — so the caller refetches
/// the whole range instead; and `None` for a fetch with no completed session
/// in it, which proves no coverage at all.
///
/// `through` is the last complete session date (yesterday); fetched candles
/// after it are dropped.
fn merge(
    cached: Option<&History>,
    fetch: &Fetch,
    fetched: &[FetchedPrice],
    through: &str,
) -> Option<History> {
    let fresh: Vec<(String, Dec)> = fetched
        .iter()
        .filter(|price| price.date.as_str() <= through)
        .map(|price| (price.date.clone(), price.quantity))
        .collect();
    if fresh.is_empty() && *fetch != Fetch::Nothing {
        // No completed session came back — not even a tail's anchor, which a
        // real answer always repeats. Nothing here proves the range is
        // covered, so coverage must not move (the handler never gets here:
        // [`completed`] already turned such an answer into a failure).
        return None;
    }
    match fetch {
        Fetch::Nothing => cached.cloned(),
        Fetch::Full { from } => Some(History {
            from: from.clone(),
            through: through.to_string(),
            closes: fresh,
        }),
        Fetch::Tail { .. } => {
            let history = cached?;
            let (first_fresh, _) = fresh.first()?;
            let fresh_at: BTreeMap<&str, Dec> = fresh
                .iter()
                .map(|(date, close)| (date.as_str(), *close))
                .collect();
            let (anchor_old, anchor_new) = history
                .closes
                .iter()
                .rev()
                .find_map(|(date, old)| fresh_at.get(date.as_str()).map(|new| (*old, *new)))?;
            let ratio = anchor_new.floating_point() / anchor_old.floating_point();
            let rescale = (ratio - 1.0).abs() > 1e-9 && ratio.is_finite() && ratio > 0.0;
            let mut closes: Vec<(String, Dec)> = history
                .closes
                .iter()
                .filter(|(date, _)| date < first_fresh)
                .map(|(date, close)| {
                    let close = if rescale {
                        Dec::parse(&format!("{:.4}", close.floating_point() * ratio), '.')
                            .unwrap_or(*close)
                    } else {
                        *close
                    };
                    (date.clone(), close)
                })
                .collect();
            closes.extend(fresh);
            Some(History {
                from: history.from.clone(),
                through: through.to_string(),
                closes,
            })
        }
    }
}

// ===========================================================================
// Handler
// ===========================================================================

/// Everything the blocking pool computes before any network call.
struct Prepared {
    base: String,
    points: Vec<HoldingsPoint>,
    flows: Vec<DatedFlow>,
    unvalued: usize,
    wanted: Vec<Benchmark>,
    cache_path: Option<PathBuf>,
}

fn parse_symbols(raw: Option<&str>) -> Result<Vec<Benchmark>, AppError> {
    let requested: Vec<&str> = raw
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .collect();
    if requested.is_empty() {
        return Err(AppError::BadRequest(
            "symbols is required (a comma-separated list of benchmarks)".to_string(),
        ));
    }
    if let Some(unknown) = requested
        .iter()
        .find(|symbol| !BENCHMARKS.iter().any(|b| b.symbol == **symbol))
    {
        let known: Vec<&str> = BENCHMARKS.iter().map(|b| b.symbol).collect();
        return Err(AppError::BadRequest(format!(
            "unknown benchmark '{unknown}' (expected one of {})",
            known.join(", ")
        )));
    }
    Ok(BENCHMARKS
        .iter()
        .filter(|b| requested.contains(&b.symbol))
        .copied()
        .collect())
}

fn prepare(state: &AppState, query: BenchmarksQuery) -> Result<Prepared, AppError> {
    let wanted = parse_symbols(query.symbols.as_deref())?;
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let scope = holdings_scope(
        journal,
        HoldingsTab::Stocks,
        query.accounts.as_deref(),
        query.mode.as_deref(),
        query.as_of,
        None,
        query.value_in.as_deref(),
    )?;
    let (interval, count) = series_window(
        journal,
        HoldingsTab::Stocks,
        &scope,
        query.interval.as_deref(),
        query.count,
        query.since.as_deref(),
    )?;
    let series = holdings_series(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
        interval,
        count,
    )?;
    let after = series.points.first().map(|point| point.date.as_str());
    let flows = holdings_flows(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
        after,
    )?;
    let cache_path = journal
        .source_files
        .first()
        .and_then(|main| main.parent())
        .map(|dir| dir.join(CACHE_FILE));
    Ok(Prepared {
        base: series.base,
        points: series.points,
        flows: flows.flows,
        unvalued: flows.unvalued,
        wanted,
        cache_path,
    })
}

/// One symbol's planned fetch.
struct Job {
    symbol: &'static str,
    fetch: Fetch,
}

/// Run one [`Job`] against `source`, through `today`, keeping sessions
/// completed by `through`.
async fn fetch_one(
    source: Arc<dyn HistoryFeed>,
    job: Job,
    today: String,
    through: String,
) -> Fetched {
    let from = job.fetch.start().unwrap_or(&today).to_string();
    let result = source.adjusted_history(job.symbol, &from, &today).await;
    Fetched {
        symbol: job.symbol,
        fetch: job.fetch,
        result: completed(result, job.symbol, &through),
    }
}

/// A fetch that returned no session completed by `through` is a failure, not
/// an empty success. Every planned range reaches back at least to a session
/// that has traded (a tail starts ON the last cached close; a full fetch spans
/// the lookback), so such an answer means the source did not really answer,
/// and recording coverage for it would stop retries until the date rolls over.
fn completed(
    result: Result<Vec<FetchedPrice>, YahooError>,
    symbol: &str,
    through: &str,
) -> Result<Vec<FetchedPrice>, YahooError> {
    let prices = result?;
    if prices.iter().any(|price| price.date.as_str() <= through) {
        Ok(prices)
    } else {
        Err(YahooError::Shape(format!(
            "no completed {symbol} sessions in the answer"
        )))
    }
}

/// One symbol's fetch outcome.
struct Fetched {
    symbol: &'static str,
    fetch: Fetch,
    result: Result<Vec<FetchedPrice>, YahooError>,
}

fn no_store<T: Serialize>(body: T) -> Response {
    const NO_STORE: (HeaderName, HeaderValue) =
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    ([NO_STORE], Json(body)).into_response()
}

/// `GET /api/holdings/benchmarks`. See the module docs.
pub(crate) async fn benchmarks(
    State(state): State<AppState>,
    Query(query): Query<BenchmarksQuery>,
) -> Result<Response, AppError> {
    let prep_state = state.clone();
    let Json(prepared) = compute(move || prepare(&prep_state, query)).await?;

    let Some(last_point) = prepared.points.last().map(|p| p.date.clone()) else {
        return Ok(no_store(WireBenchmarks {
            base: prepared.base,
            benchmarks: Vec::new(),
        }));
    };
    if !USD_BASES.contains(&prepared.base.as_str()) {
        let message = format!(
            "Benchmarks are priced in US dollars, and these holdings are valued in {}.",
            prepared.base
        );
        let benchmarks = prepared
            .wanted
            .iter()
            .map(|b| failed(b, &prepared.points, message.clone(), prepared.unvalued))
            .collect();
        return Ok(no_store(WireBenchmarks {
            base: prepared.base,
            benchmarks,
        }));
    }

    let yesterday = add_days(&today_utc(), -1);
    let need_from = add_days(&prepared.points[0].date, -LOOKBACK_DAYS);
    let need_through = last_point.as_str().min(yesterday.as_str()).to_string();

    // Plan against the cache as it is now: read ONCE here, and again below
    // only when a fetch makes a write necessary. A disk read needs no lock:
    // every write is an atomic rename, so a reader sees the old file or the
    // new one.
    let store = Store::new(prepared.cache_path.clone(), state.editing_enabled());
    let cached = match &store {
        Store::Disk(path) => {
            let path = path.clone();
            let Json(cached) = compute(move || Ok(read_cache(&path))).await?;
            cached
        }
        Store::Memory => state.benchmark_cache().lock().await.0.clone(),
    };
    let jobs: Vec<Job> = prepared
        .wanted
        .iter()
        .map(|b| Job {
            symbol: b.symbol,
            fetch: plan_fetch(cached.get(b.symbol), &need_from, &need_through),
        })
        .filter(|job| job.fetch != Fetch::Nothing)
        .collect();

    let source = Arc::clone(state.history_source());
    let today = today_utc();
    let fetched: Vec<Fetched> = stream::iter(jobs)
        .map(|job| fetch_one(Arc::clone(&source), job, today.clone(), yesterday.clone()))
        .buffer_unordered(FETCH_CONCURRENCY)
        .collect()
        .await;

    // A tail that does not overlap the cache gets one full refetch, outside
    // the lock like every other network call.
    let mut refetched: Vec<Fetched> = Vec::with_capacity(fetched.len());
    for outcome in fetched {
        let unanchored = matches!(outcome.fetch, Fetch::Tail { .. })
            && matches!(&outcome.result, Ok(prices)
                if merge(cached.get(outcome.symbol), &outcome.fetch, prices, &yesterday).is_none());
        if unanchored {
            // A tail is only planned for a history whose start already covers
            // `need_from`, so its own start is the range to refetch.
            let from = cached
                .get(outcome.symbol)
                .map_or_else(|| need_from.clone(), |history| history.from.clone());
            let result = completed(
                source.adjusted_history(outcome.symbol, &from, &today).await,
                outcome.symbol,
                &yesterday,
            );
            let fetch = Fetch::Full { from };
            refetched.push(Fetched {
                symbol: outcome.symbol,
                fetch,
                result,
            });
        } else {
            refetched.push(outcome);
        }
    }

    let outcome = if refetched.is_empty() {
        // Everything was covered: the cache as read is the answer.
        MergeOutcome {
            cache: cached,
            failures: BTreeMap::new(),
        }
    } else {
        // Read-merge-write under the lock, re-reading the store so a
        // concurrent request's symbols survive this one's write.
        let mut session = state.benchmark_cache().lock().await;
        let through = yesterday.clone();
        match store {
            Store::Disk(path) => {
                let Json(outcome) = compute(move || {
                    let merged = merge_fetched(read_cache(&path), &refetched, &through);
                    if merged.changed {
                        write_cache(&path, &merged.outcome.cache);
                    }
                    Ok(merged.outcome)
                })
                .await?;
                outcome
            }
            Store::Memory => {
                let merged = merge_fetched(session.0.clone(), &refetched, &through);
                if merged.changed {
                    session.0.clone_from(&merged.outcome.cache);
                }
                merged.outcome
            }
        }
    };

    let benchmarks = prepared
        .wanted
        .iter()
        .map(|benchmark| {
            let history = outcome.cache.get(benchmark.symbol);
            let failure = outcome.failures.get(benchmark.symbol);
            line(benchmark, &prepared, history, failure)
        })
        .collect();
    Ok(no_store(WireBenchmarks {
        base: prepared.base.clone(),
        benchmarks,
    }))
}

/// The merged cache and any per-symbol fetch failure.
struct MergeOutcome {
    cache: Cache,
    failures: BTreeMap<&'static str, String>,
}

/// A [`MergeOutcome`], and whether the cache changed (and so needs storing).
struct Merged {
    outcome: MergeOutcome,
    changed: bool,
}

/// Fold every fetch outcome into `cache` — the store as it is NOW, read under
/// the lock: a concurrent request may have extended it meanwhile, and a tail
/// still anchors onto that. One that no longer can leaves the stored version
/// alone.
fn merge_fetched(mut cache: Cache, fetched: &[Fetched], through: &str) -> Merged {
    let mut failures: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut changed = false;
    for outcome in fetched {
        match &outcome.result {
            Err(error) => {
                failures.insert(outcome.symbol, error.to_string());
            }
            Ok(prices) => {
                if let Some(history) =
                    merge(cache.get(outcome.symbol), &outcome.fetch, prices, through)
                {
                    cache.insert(outcome.symbol.to_string(), history);
                    changed = true;
                }
            }
        }
    }
    Merged {
        outcome: MergeOutcome { cache, failures },
        changed,
    }
}

/// Write the cache file. A failure is logged, not raised: the line is served
/// from memory all the same, and the next request refetches.
fn write_cache(path: &Path, cache: &Cache) {
    if let Err(error) = ledgeline_core::edit::atomic_write(path, render_cache(cache).as_bytes()) {
        eprintln!("ledgeline: could not write {CACHE_FILE}: {}", error.kind());
    }
}

fn closes_of(history: &History) -> Vec<Close> {
    history
        .closes
        .iter()
        .map(|(date, close)| (date.clone(), close.floating_point()))
        .collect()
}

fn failed(
    benchmark: &Benchmark,
    points: &[HoldingsPoint],
    error: String,
    unvalued: usize,
) -> WireBenchmark {
    WireBenchmark {
        symbol: benchmark.symbol.to_string(),
        label: benchmark.label.to_string(),
        points: points
            .iter()
            .map(|point| WireBenchmarkPoint {
                date: point.date.clone(),
                value: None,
            })
            .collect(),
        priced_through: None,
        stale: false,
        unvalued_flows: unvalued,
        error: Some(error),
    }
}

fn line(
    benchmark: &Benchmark,
    prepared: &Prepared,
    history: Option<&History>,
    failure: Option<&String>,
) -> WireBenchmark {
    let usable = history.filter(|h| !h.closes.is_empty());
    let Some(history) = usable else {
        let error = match failure {
            Some(error) => format!(
                "Could not fetch {} history from Yahoo Finance ({error}).",
                benchmark.symbol
            ),
            None => format!("No {} price history covers this window.", benchmark.symbol),
        };
        return failed(benchmark, &prepared.points, error, prepared.unvalued);
    };
    let simulated = simulate_benchmark(&prepared.points, &prepared.flows, &closes_of(history));
    WireBenchmark {
        symbol: benchmark.symbol.to_string(),
        label: benchmark.label.to_string(),
        points: prepared
            .points
            .iter()
            .zip(simulated.values)
            .map(|(point, value)| WireBenchmarkPoint {
                date: point.date.clone(),
                value: value.map(|v| (v * 100.0).round() / 100.0),
            })
            .collect(),
        priced_through: history.closes.last().map(|(date, _)| date.clone()),
        stale: failure.is_some(),
        unvalued_flows: prepared.unvalued,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Dec {
        Dec::parse(text, '.').unwrap()
    }

    fn history(from: &str, through: &str, closes: &[(&str, &str)]) -> History {
        History {
            from: from.to_string(),
            through: through.to_string(),
            closes: closes
                .iter()
                .map(|(d, c)| ((*d).to_string(), dec(c)))
                .collect(),
        }
    }

    fn fetched(rows: &[(&str, &str)]) -> Vec<FetchedPrice> {
        rows.iter()
            .map(|(d, c)| FetchedPrice {
                date: (*d).to_string(),
                quantity: dec(c),
            })
            .collect()
    }

    #[test]
    fn plain_renders_decimals_exactly() {
        assert_eq!(plain(dec("766.2900")), "766.2900");
        assert_eq!(plain(dec("0.05")), "0.05");
        assert_eq!(plain(dec("-12.5")), "-12.5");
        assert_eq!(plain(dec("42")), "42");
    }

    /// The file is a real hledger journal — the project's own parser reads the
    /// `P` lines back — and the coverage survives the round trip.
    #[test]
    fn the_cache_round_trips_through_the_journal_parser() {
        let mut cache = Cache::new();
        cache.insert(
            "SPY".to_string(),
            history(
                "2025-01-01",
                "2025-01-03",
                &[("2025-01-02", "580.1234"), ("2025-01-03", "585.5")],
            ),
        );
        cache.insert("VXUS".to_string(), history("1990-01-01", "1990-12-31", &[]));
        let text = render_cache(&cache);
        assert!(text.starts_with("; Ledgeline benchmark price cache."));
        assert!(text.contains("P 2025-01-02 SPY 580.1234 USD\n"));
        assert!(
            !text
                .lines()
                .any(|line| line.trim_start().starts_with("include"))
        );

        let journal = parse_journal(&text, CACHE_FILE).expect("hledger-format text parses");
        assert_eq!(journal.prices.len(), 2);
        assert_eq!(journal.transactions.len(), 0);

        assert_eq!(parse_cache(&text).unwrap(), cache);
    }

    #[test]
    fn prices_without_a_coverage_line_cover_exactly_their_own_span() {
        let text = "P 2025-01-03 SPY 2.00 USD\nP 2025-01-02 SPY 1.00 USD\nP 2025-01-02 EUR 1.10 USD\nP 2025-01-02 QQQ 9 EUR\n";
        let cache = parse_cache(text).unwrap();
        assert_eq!(
            cache.get("SPY"),
            Some(&history(
                "2025-01-02",
                "2025-01-03",
                &[("2025-01-02", "1.00"), ("2025-01-03", "2.00")]
            ))
        );
        assert!(!cache.contains_key("QQQ"), "not quoted in USD");
    }

    #[test]
    fn an_unreadable_cache_is_treated_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE);
        std::fs::write(&path, "this is not ::: a journal\n  bad indent\n").unwrap();
        assert!(
            read_cache(&path).is_empty() || read_cache(&path).values().all(|h| h.closes.is_empty())
        );
        assert!(read_cache(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn plans_nothing_when_covered_a_tail_when_behind_and_a_full_fetch_to_extend_the_start() {
        let cached = history(
            "2025-01-01",
            "2025-06-30",
            &[("2025-01-02", "1"), ("2025-06-27", "2")],
        );
        assert_eq!(
            plan_fetch(Some(&cached), "2025-02-01", "2025-06-30"),
            Fetch::Nothing
        );
        assert_eq!(
            plan_fetch(Some(&cached), "2025-02-01", "2025-07-15"),
            Fetch::Tail {
                from: "2025-06-27".to_string()
            }
        );
        assert_eq!(
            plan_fetch(Some(&cached), "2024-06-01", "2025-06-30"),
            Fetch::Full {
                from: "2024-06-01".to_string()
            }
        );
        assert_eq!(
            plan_fetch(None, "2024-06-01", "2025-06-30"),
            Fetch::Full {
                from: "2024-06-01".to_string()
            }
        );
    }

    /// A fund younger than an earlier request: its coverage starts before its
    /// first close, and a later request inside that coverage is not refetched.
    #[test]
    fn coverage_before_a_funds_first_trade_is_remembered() {
        let cached = history(
            "2005-01-01",
            "2025-06-30",
            &[("2011-01-28", "31.0"), ("2025-06-30", "70")],
        );
        assert_eq!(
            plan_fetch(Some(&cached), "2008-01-01", "2025-06-30"),
            Fetch::Nothing
        );
    }

    #[test]
    fn a_tail_is_appended_and_today_is_not_stored() {
        let cached = history(
            "2025-01-01",
            "2025-01-03",
            &[("2025-01-02", "10"), ("2025-01-03", "11")],
        );
        let fetch = Fetch::Tail {
            from: "2025-01-03".to_string(),
        };
        let merged = merge(
            Some(&cached),
            &fetch,
            &fetched(&[
                ("2025-01-03", "11"),
                ("2025-01-06", "12"),
                ("2025-01-07", "13"),
            ]),
            "2025-01-06",
        )
        .unwrap();
        assert_eq!(
            merged,
            history(
                "2025-01-01",
                "2025-01-06",
                &[
                    ("2025-01-02", "10"),
                    ("2025-01-03", "11"),
                    ("2025-01-06", "12")
                ]
            )
        );
    }

    /// Yahoo re-bases adjusted closes after each dividend. The tail comes back
    /// on the new scale; the cached body is re-scaled by the ratio on the
    /// overlapping session so the line has no step.
    #[test]
    fn a_rebased_tail_rescales_the_cached_body() {
        let cached = history(
            "2025-01-01",
            "2025-01-03",
            &[("2025-01-02", "100"), ("2025-01-03", "110")],
        );
        let fetch = Fetch::Tail {
            from: "2025-01-03".to_string(),
        };
        let merged = merge(
            Some(&cached),
            &fetch,
            &fetched(&[("2025-01-03", "99"), ("2025-01-06", "108")]),
            "2025-01-06",
        )
        .unwrap();
        let closes: Vec<(&str, f64)> = merged
            .closes
            .iter()
            .map(|(d, c)| (d.as_str(), c.floating_point()))
            .collect();
        assert_eq!(
            closes,
            vec![
                ("2025-01-02", 90.0),
                ("2025-01-03", 99.0),
                ("2025-01-06", 108.0)
            ]
        );
    }

    #[test]
    fn a_tail_that_does_not_overlap_cannot_be_merged() {
        let cached = history("2025-01-01", "2025-01-03", &[("2025-01-03", "11")]);
        let fetch = Fetch::Tail {
            from: "2025-01-03".to_string(),
        };
        assert!(
            merge(
                Some(&cached),
                &fetch,
                &fetched(&[("2025-02-03", "12")]),
                "2025-02-03"
            )
            .is_none()
        );
    }

    /// An answer with no completed session proves nothing, so coverage stays
    /// where it was — for a tail and for a full fetch alike.
    #[test]
    fn an_empty_fetch_never_advances_coverage() {
        let cached = history("2025-01-01", "2025-01-03", &[("2025-01-03", "11")]);
        let tail = Fetch::Tail {
            from: "2025-01-03".to_string(),
        };
        assert!(merge(Some(&cached), &tail, &[], "2025-01-05").is_none());
        // Only today's still-forming candle: nothing completed either.
        let live = fetched(&[("2025-01-06", "12")]);
        assert!(merge(Some(&cached), &tail, &live, "2025-01-05").is_none());
        let full = Fetch::Full {
            from: "2024-12-01".to_string(),
        };
        assert!(merge(Some(&cached), &full, &[], "2025-01-05").is_none());
        assert!(merge(None, &full, &[], "2025-01-05").is_none());
    }

    #[test]
    fn an_answer_without_a_completed_session_is_a_failure() {
        assert!(completed(Ok(Vec::new()), "SPY", "2025-01-05").is_err());
        assert!(completed(Ok(fetched(&[("2025-01-06", "12")])), "SPY", "2025-01-05").is_err());
        assert_eq!(
            completed(Ok(fetched(&[("2025-01-03", "11")])), "SPY", "2025-01-05")
                .unwrap()
                .len(),
            1
        );
        assert!(
            completed(
                Err(YahooError::Http("down".to_string())),
                "SPY",
                "2025-01-05"
            )
            .is_err()
        );
    }

    #[test]
    fn symbols_are_validated_against_the_catalog_and_returned_in_its_order() {
        let picked = parse_symbols(Some("QQQ, SPY,QQQ")).unwrap();
        assert_eq!(
            picked.iter().map(|b| b.symbol).collect::<Vec<_>>(),
            ["SPY", "QQQ"]
        );
        assert!(parse_symbols(Some("SPY,EVIL/../x")).is_err());
        assert!(parse_symbols(None).is_err());
        assert!(parse_symbols(Some(" , ")).is_err());
    }
}
