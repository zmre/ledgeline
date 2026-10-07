//! `GET /api/holdings/benchmarks` — the Stocks tab's "what if the same money had
//! gone into an index fund" overlay.
//!
//! For each requested benchmark this answers one line, index-aligned to the
//! value-over-time series for the same scope and window: what the portfolio
//! would be worth had every contribution and withdrawal, on its own date, gone
//! into that benchmark instead. The arithmetic is the engine's
//! (`ledgeline_core::holdings::benchmark`); this module supplies its inputs —
//! the series, the dated flows, and the benchmark's dividend-adjusted history —
//! and keeps that history beside the journal so it is fetched once, not per
//! chart.
//!
//! # The price cache
//!
//! History lives in `benchmarks.prices.journal` BESIDE the main journal, as
//! ordinary hledger `P` directives (`P 2026-09-25 SPY 766.2900 USD`) under a
//! header saying what they are. It is deliberately NOT `include`d anywhere:
//! these are dividend-adjusted closes, which are not market prices and must
//! never value a real holding of SPY. Same format so any hledger tool can read
//! it; separate file so none ever does by accident. [`crate::sidecar`] has the
//! rules it shares with the profile cache: loaded once, kept in memory, written
//! through atomically only by a session that may write, safe to delete.
//!
//! Each symbol carries a coverage comment (`; ledgeline-benchmark SPY from
//! 2021-09-17 through 2026-09-25`) so a later request knows what has already
//! been asked for — including ranges where the fund had not yet traded, which
//! would otherwise be re-requested forever. `through` is the last close Yahoo
//! actually returned, and a symbol is recorded at all only when the answer had
//! a close in it, so an empty or refused answer advances nothing. Only sessions
//! before today are kept: today's candle is still forming.
//!
//! # Refreshing
//!
//! A symbol whose coverage falls short of the window is refetched WHOLE, in one
//! request, from the earlier of the window's start and its cached start: a few
//! thousand daily rows at most. Yahoo re-bases adjusted closes every time a
//! fund pays a dividend, so a fetched tail would be on a different scale from
//! the cached body; a whole refetch is always on one scale.
//!
//! # Failure
//!
//! A benchmark is decoration on a chart that is already drawn. A failed fetch
//! is reported on that benchmark (`error`, or `stale` when the cache could still
//! answer) with a `200`; only a malformed REQUEST is a `400`.

use axum::Json;
use axum::extract::{Query, State};
use axum::response::Response;
use futures::stream::{self, StreamExt};
use ledgeline_core::edit::render_dec;
use ledgeline_core::holdings::benchmark::{Close, simulate_benchmark};
use ledgeline_core::holdings::{DatedFlow, HoldingsPoint, holdings_series_and_flows, is_us_dollar};
use ledgeline_core::parse_journal;
use ledgeline_core::reports::periods::{add_days, weekday_on_or_before};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::AppState;
use crate::error::AppError;
use crate::reports_api::{HoldingsSeriesQuery, compute, stocks_series_request, today_utc};
use crate::security::no_store_json;
use crate::sidecar::{FETCH_CONCURRENCY, Sidecar, parse_symbol_list};
use crate::yahoo::{FetchedPrice, YahooError};
use crate::yahoo_history::{HistoryFeed, one_per_date};

/// Every benchmark the overlay can draw, in display order: total-return ETF
/// proxies for standard measures. The SPA keeps the same list, with each
/// symbol's label (`web/src/lib/holdings/benchmarks.ts`), because its order
/// fixes each line's colour; a symbol outside this list is a `400`, which also
/// keeps the server from being asked to fetch arbitrary tickers.
pub(crate) const BENCHMARKS: &[&str] = &["SPY", "DIA", "QQQ", "VTI", "BND", "VXUS", "IWM", "GLD"];

/// The commodity every cached close is quoted in.
const QUOTE: &str = "USD";

/// How far before the first chart point history is requested, so a point on a
/// weekend or after a long holiday still finds the previous session's close.
const LOOKBACK_DAYS: i64 = 10;

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

/// `?symbols=SPY,QQQ`, read beside `/api/holdings/series`' own query
/// ([`HoldingsSeriesQuery`]) from the same query string, so the overlay is
/// computed for exactly the window the base chart was.
#[derive(Deserialize)]
pub(crate) struct BenchmarksQuery {
    symbols: Option<String>,
}

/// The response: one line per requested benchmark, in catalog order.
#[derive(Debug, Serialize)]
pub(crate) struct WireBenchmarks {
    benchmarks: Vec<WireBenchmark>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WireBenchmark {
    symbol: String,
    /// Index-aligned to the series' points; `value` is `null` where the
    /// benchmark has no price yet (a gap, not a zero).
    points: Vec<WireBenchmarkPoint>,
    /// A refresh was needed and did not land, so the line is drawn from an
    /// older cache and its last points may be flat.
    stale: bool,
    /// Why there is no line at all, when there is not.
    error: Option<String>,
}

#[derive(Debug, Serialize)]
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
    /// The last close's date: the history is complete through it.
    through: String,
    /// Ascending, one per session.
    closes: Vec<FetchedPrice>,
}

impl History {
    /// What an answer to a request from `from` proves: its closes up to
    /// `completed` (the last session that has closed), covered from `from`
    /// through the last of them. `None` when it has no such close — an empty
    /// or live-candle-only answer proves no coverage at all.
    fn from_answer(from: String, mut prices: Vec<FetchedPrice>, completed: &str) -> Option<Self> {
        prices.retain(|price| price.date.as_str() <= completed);
        let through = prices.last()?.date.clone();
        Some(Self {
            from,
            through,
            closes: prices,
        })
    }

    /// Where a refresh must start for this history to cover
    /// `need_from..=need_through`, or `None` when it already does. Behind or
    /// short, the whole range is refetched (see the module docs).
    fn refetch_from(history: Option<&Self>, need_from: &str, need_through: &str) -> Option<String> {
        let Some(history) = history.filter(|history| !history.from.is_empty()) else {
            return Some(need_from.to_string());
        };
        if history.from.as_str() <= need_from && history.through.as_str() >= need_through {
            return None;
        }
        Some(need_from.min(history.from.as_str()).to_string())
    }

    fn closes(&self) -> Vec<Close> {
        self.closes
            .iter()
            .map(|price| (price.date.clone(), price.quantity.floating_point()))
            .collect()
    }
}

/// `benchmarks.prices.journal`: every cached symbol's history.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BenchmarkCache(BTreeMap<String, History>);

impl Sidecar for BenchmarkCache {
    const FILE: &'static str = "benchmarks.prices.journal";

    fn decode(bytes: &[u8]) -> Self {
        parse_cache(&String::from_utf8_lossy(bytes)).unwrap_or_else(|error| {
            eprintln!("ledgeline: ignoring an unreadable {}: {error}", Self::FILE);
            Self::default()
        })
    }

    fn encode(&self) -> std::io::Result<Vec<u8>> {
        Ok(render_cache(self).into_bytes())
    }
}

fn render_cache(cache: &BenchmarkCache) -> String {
    let mut out = String::from(CACHE_HEADER);
    for (symbol, history) in &cache.0 {
        out.push('\n');
        out.push_str(&format!(
            "{COVERAGE}{symbol} from {} through {}\n",
            history.from, history.through
        ));
        for close in &history.closes {
            out.push_str(&format!(
                "P {} {symbol} {} {QUOTE}\n",
                close.date,
                render_dec(close.quantity, '.')
            ));
        }
    }
    out
}

/// Read a cache file's text back: the `P` lines through the project's own
/// journal parser (so the file is provably something hledger reads), the
/// coverage comments by hand. A symbol with prices but no coverage line is
/// covered exactly as far as its prices reach.
fn parse_cache(text: &str) -> Result<BenchmarkCache, String> {
    let journal = parse_journal(text, BenchmarkCache::FILE).map_err(|error| error.to_string())?;
    let mut prices: BTreeMap<String, Vec<FetchedPrice>> = BTreeMap::new();
    for price in journal.prices {
        if price.price.commodity.0 == QUOTE {
            prices
                .entry(price.commodity.0)
                .or_default()
                .push(FetchedPrice {
                    date: price.date,
                    quantity: price.price.quantity,
                });
        }
    }
    let mut cache: BTreeMap<String, History> = prices
        .into_iter()
        .map(|(symbol, prices)| {
            let closes = one_per_date(prices);
            let span =
                |price: Option<&FetchedPrice>| price.map(|p| p.date.clone()).unwrap_or_default();
            let history = History {
                from: span(closes.first()),
                through: span(closes.last()),
                closes,
            };
            (symbol, history)
        })
        .collect();
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
    Ok(BenchmarkCache(cache))
}

// ===========================================================================
// Handler
// ===========================================================================

/// Everything the blocking pool computes before any network call.
struct Prepared {
    base: String,
    points: Vec<HoldingsPoint>,
    flows: Vec<DatedFlow>,
    wanted: Vec<&'static str>,
}

/// The requested benchmarks, in catalog order. None, or one outside the
/// catalog, is a `400`.
fn parse_benchmarks(raw: Option<&str>) -> Result<Vec<&'static str>, AppError> {
    let requested = parse_symbol_list(raw, BENCHMARKS.len())?;
    if requested.is_empty() {
        return Err(AppError::BadRequest(
            "symbols is required (a comma-separated list of benchmarks)".to_string(),
        ));
    }
    if let Some(unknown) = requested.iter().find(|symbol| !BENCHMARKS.contains(symbol)) {
        return Err(AppError::BadRequest(format!(
            "unknown benchmark '{unknown}' (expected one of {})",
            BENCHMARKS.join(", ")
        )));
    }
    Ok(BENCHMARKS
        .iter()
        .copied()
        .filter(|symbol| requested.contains(symbol))
        .collect())
}

/// The request's shared inputs: the series and the portfolio's flows are
/// computed ONCE per request, by one replay, and every requested symbol's line
/// is simulated from them, which is why the SPA batches its ticked benchmarks
/// into one request rather than asking per symbol. The series is recomputed
/// here even though the page already has it: the seed must be the engine's own
/// values, not numbers a client sends back.
fn prepare(
    state: &AppState,
    symbols: BenchmarksQuery,
    series_query: HoldingsSeriesQuery,
) -> Result<Prepared, AppError> {
    let wanted = parse_benchmarks(symbols.symbols.as_deref())?;
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let (scope, window) = stocks_series_request(journal, series_query)?;
    let (series, flows) = holdings_series_and_flows(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
        &window,
    )?;
    Ok(Prepared {
        base: series.base,
        points: series.points,
        flows: flows.flows,
        wanted,
    })
}

/// `GET /api/holdings/benchmarks`. See the module docs.
pub(crate) async fn benchmarks(
    State(state): State<AppState>,
    Query(symbols): Query<BenchmarksQuery>,
    Query(series_query): Query<HoldingsSeriesQuery>,
) -> Result<Response, AppError> {
    let prep_state = state.clone();
    let Json(prepared) = compute(move || prepare(&prep_state, symbols, series_query)).await?;

    let (Some(first), Some(last)) = (prepared.points.first(), prepared.points.last()) else {
        return Ok(no_store_json(WireBenchmarks {
            benchmarks: Vec::new(),
        }));
    };
    if !is_us_dollar(&prepared.base) {
        let message = format!(
            "Benchmarks are priced in US dollars, and these holdings are valued in {}.",
            prepared.base
        );
        let benchmarks = prepared
            .wanted
            .iter()
            .map(|symbol| failed(symbol, &prepared.points, message.clone()))
            .collect();
        return Ok(no_store_json(WireBenchmarks { benchmarks }));
    }

    let today = today_utc();
    let yesterday = add_days(&today, -1);
    let need_from = add_days(&first.date, -LOOKBACK_DAYS);
    // No session closes on a weekend, so one ending there is covered by Friday's.
    let need_through = weekday_on_or_before(last.date.as_str().min(yesterday.as_str()));

    let cache = state.benchmark_cache();
    let cached = cache.snapshot(&state).await?;
    // Owned symbols: a borrowed one in a stream item trips rustc's
    // higher-ranked `Send` check on the handler's future.
    let jobs: Vec<(String, String)> = prepared
        .wanted
        .iter()
        .filter_map(|&symbol| {
            History::refetch_from(cached.0.get(symbol), &need_from, &need_through)
                .map(|from| (symbol.to_string(), from))
        })
        .collect();

    // Fetched with no lock held; only the merge below takes it.
    let source = Arc::clone(state.history_source());
    let answers: Vec<(String, Result<Option<History>, YahooError>)> = stream::iter(jobs)
        .map(|(symbol, from)| {
            fetch_history(
                Arc::clone(&source),
                symbol,
                from,
                today.clone(),
                yesterday.clone(),
            )
        })
        .buffer_unordered(FETCH_CONCURRENCY)
        .collect()
        .await;

    let mut refreshes: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut fresh: Vec<(String, History)> = Vec::new();
    for (symbol, answer) in answers {
        let failure = match answer {
            Ok(Some(history)) => {
                fresh.push((symbol, history));
                continue;
            }
            Ok(None) => None,
            Err(error) => Some(error.to_string()),
        };
        refreshes.insert(symbol, failure);
    }
    let cache = if fresh.is_empty() {
        cached
    } else {
        drop(cached); // so the merge need not copy a snapshot nobody reads
        cache.update(&state, |cache| cache.0.extend(fresh)).await?
    };

    let benchmarks = prepared
        .wanted
        .iter()
        .map(|&symbol| {
            let missed = refreshes.get(symbol).map(Option::as_deref);
            line(symbol, &prepared, cache.0.get(symbol), missed)
        })
        .collect();
    Ok(no_store_json(WireBenchmarks { benchmarks }))
}

/// Ask `source` for `symbol` from `from` through `today`, keeping what the
/// answer proves through `completed` ([`History::from_answer`]).
async fn fetch_history(
    source: Arc<dyn HistoryFeed>,
    symbol: String,
    from: String,
    today: String,
    completed: String,
) -> (String, Result<Option<History>, YahooError>) {
    let answer = source
        .adjusted_history(&symbol, &from, &today)
        .await
        .map(|prices| History::from_answer(from, prices, &completed));
    (symbol, answer)
}

fn failed(symbol: &str, points: &[HoldingsPoint], error: String) -> WireBenchmark {
    WireBenchmark {
        symbol: symbol.to_string(),
        points: points
            .iter()
            .map(|point| WireBenchmarkPoint {
                date: point.date.clone(),
                value: None,
            })
            .collect(),
        stale: false,
        error: Some(error),
    }
}

/// One benchmark's line. `missed` is `Some` when a refresh was needed and did
/// not land — carrying the fetch error, if it was one.
fn line(
    symbol: &str,
    prepared: &Prepared,
    history: Option<&History>,
    missed: Option<Option<&str>>,
) -> WireBenchmark {
    let Some(history) = history.filter(|history| !history.closes.is_empty()) else {
        let error = match missed.flatten() {
            Some(error) => {
                format!("Could not fetch {symbol} history from Yahoo Finance ({error}).")
            }
            None => format!("No {symbol} price history covers this window."),
        };
        return failed(symbol, &prepared.points, error);
    };
    let simulated = simulate_benchmark(&prepared.points, &prepared.flows, &history.closes());
    WireBenchmark {
        symbol: symbol.to_string(),
        points: prepared
            .points
            .iter()
            .zip(simulated)
            .map(|(point, value)| WireBenchmarkPoint {
                date: point.date.clone(),
                value: value.map(|v| (v * 100.0).round() / 100.0),
            })
            .collect(),
        stale: missed.is_some(),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::Dec;

    fn prices(rows: &[(&str, &str)]) -> Vec<FetchedPrice> {
        rows.iter()
            .map(|(date, close)| FetchedPrice {
                date: (*date).to_string(),
                quantity: Dec::parse(close, '.').unwrap(),
            })
            .collect()
    }

    fn history(from: &str, through: &str, closes: &[(&str, &str)]) -> History {
        History {
            from: from.to_string(),
            through: through.to_string(),
            closes: prices(closes),
        }
    }

    /// The file is a real hledger journal — the project's own parser reads the
    /// `P` lines back — and the coverage survives the round trip.
    #[test]
    fn the_cache_round_trips_through_the_journal_parser() {
        let cache = BenchmarkCache(
            [
                (
                    "SPY".to_string(),
                    history(
                        "2025-01-01",
                        "2025-01-03",
                        &[("2025-01-02", "580.1234"), ("2025-01-03", "585.5")],
                    ),
                ),
                ("VXUS".to_string(), history("1990-01-01", "1990-12-31", &[])),
            ]
            .into(),
        );
        let text = render_cache(&cache);
        assert!(text.starts_with("; Ledgeline benchmark price cache."));
        assert!(text.contains("P 2025-01-02 SPY 580.1234 USD\n"));
        assert!(
            !text
                .lines()
                .any(|line| line.trim_start().starts_with("include"))
        );

        let journal =
            parse_journal(&text, BenchmarkCache::FILE).expect("hledger-format text parses");
        assert_eq!(journal.prices.len(), 2);
        assert_eq!(journal.transactions.len(), 0);

        assert_eq!(BenchmarkCache::decode(text.as_bytes()), cache);
    }

    #[test]
    fn prices_without_a_coverage_line_cover_exactly_their_own_span() {
        let text = "P 2025-01-03 SPY 2.00 USD\nP 2025-01-02 SPY 1.00 USD\nP 2025-01-02 EUR 1.10 USD\nP 2025-01-02 QQQ 9 EUR\n";
        let cache = parse_cache(text).unwrap();
        assert_eq!(
            cache.0.get("SPY"),
            Some(&history(
                "2025-01-02",
                "2025-01-03",
                &[("2025-01-02", "1.00"), ("2025-01-03", "2.00")]
            ))
        );
        assert!(!cache.0.contains_key("QQQ"), "not quoted in USD");
    }

    #[test]
    fn an_unreadable_cache_is_treated_as_empty() {
        let cache = BenchmarkCache::decode(b"this is not ::: a journal\n  bad indent\n");
        assert!(cache.0.values().all(|history| history.closes.is_empty()));
    }

    #[test]
    fn nothing_is_refetched_when_covered_and_the_whole_range_is_when_not() {
        let cached = history(
            "2025-01-01",
            "2025-06-30",
            &[("2025-01-02", "1"), ("2025-06-30", "2")],
        );
        let refetch =
            |need_from, need_through| History::refetch_from(Some(&cached), need_from, need_through);
        assert_eq!(refetch("2025-02-01", "2025-06-30"), None);
        // Behind: everything from the cached start, in one request.
        assert_eq!(
            refetch("2025-02-01", "2025-07-15").as_deref(),
            Some("2025-01-01")
        );
        // Short at the start: from the new start.
        assert_eq!(
            refetch("2024-06-01", "2025-06-30").as_deref(),
            Some("2024-06-01")
        );
        assert_eq!(
            History::refetch_from(None, "2024-06-01", "2025-06-30").as_deref(),
            Some("2024-06-01")
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
            History::refetch_from(Some(&cached), "2008-01-01", "2025-06-30"),
            None
        );
    }

    /// Coverage runs to the last close received, and today's still-forming
    /// candle is not kept.
    #[test]
    fn an_answer_covers_through_its_last_completed_close() {
        let answer = prices(&[
            ("2025-01-02", "10"),
            ("2025-01-03", "11"),
            ("2025-01-06", "12"),
        ]);
        assert_eq!(
            History::from_answer("2024-12-20".to_string(), answer, "2025-01-05"),
            Some(history(
                "2024-12-20",
                "2025-01-03",
                &[("2025-01-02", "10"), ("2025-01-03", "11")]
            ))
        );
    }

    /// An answer with no completed session proves nothing, so it records no
    /// coverage at all.
    #[test]
    fn an_answer_without_a_completed_close_proves_nothing() {
        assert_eq!(
            History::from_answer("2025-01-01".to_string(), Vec::new(), "2025-01-05"),
            None
        );
        let live = prices(&[("2025-01-06", "12")]);
        assert_eq!(
            History::from_answer("2025-01-01".to_string(), live, "2025-01-05"),
            None
        );
    }

    #[test]
    fn symbols_are_validated_against_the_catalog_and_returned_in_its_order() {
        assert_eq!(
            parse_benchmarks(Some("QQQ, SPY,QQQ")).unwrap(),
            ["SPY", "QQQ"]
        );
        assert!(parse_benchmarks(Some("SPY,EVIL/../x")).is_err());
        assert!(parse_benchmarks(None).is_err());
        assert!(parse_benchmarks(Some(" , ")).is_err());
    }
}
