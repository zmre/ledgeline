//! **Intraday quotes** (plan 47 D10/D11): one delayed market quote per
//! symbol, held in this process's memory, so the Holdings page can answer
//! "the market is having a bad day — what does that do to me?" on its short
//! periods.
//!
//! # What it is not
//!
//! Not a price feed and not a monitor. There is no polling loop: the page asks
//! ([`refresh`], `POST /api/prices/intraday`) when it opens a 1wk/1mo view of
//! today, and the report GETs only ever READ the cache (`live=1`, see
//! `reports_api`). Nothing here is ever written to a journal or a prices file
//! — the quotes reach a report as same-day `P` directives appended in memory
//! for one computation (`ledgeline_core::holdings::live`), and are gone when
//! the process is.
//!
//! # Never hammer Yahoo
//!
//! - A ticker whose quote OR CLOSE was fetched in the last [`FRESH_FOR`] is
//!   not asked again. The close fetches (Update prices, the automatic
//!   updater) get the quote in the same reply's `meta` and record it here, so
//!   a page opened just after an update asks for nothing.
//! - Requests go through the automatic updater's pacing loop
//!   ([`auto_prices::paced`]): two at a time, starts 250 ms apart.
//! - The first `429`/`999` stops the round, and every round for the next
//!   [`BACKOFF`] sends nothing at all. A rate limit seen by the automatic
//!   updater sets the same backoff.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use ledgeline_core::holdings::LiveQuote;
use ledgeline_core::model::Journal;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auto_prices::{self, now_seconds};
use crate::error::AppError;
use crate::prices_api::{self, WirePriceSymbol};
use crate::reports_api::{WireDec, compute, today_utc};
use crate::security::no_store_json as no_store;
use crate::yahoo::{MarketQuote, YahooError};

/// How long a fetched quote (or close) keeps its ticker from being asked
/// again, in seconds.
pub(crate) const FRESH_FOR: i64 = 15 * 60;

/// How long a rate limit silences every intraday round, in seconds.
pub(crate) const BACKOFF: i64 = 15 * 60;

/// The most symbols one refresh may name. Far past any real portfolio; it
/// exists so a request cannot queue an unbounded number of outbound fetches.
const MAX_SYMBOLS: usize = 500;

/// The longest symbol a refresh may name, in bytes.
const MAX_SYMBOL_BYTES: usize = 64;

// ===========================================================================
// The cache
// ===========================================================================

/// One ticker's cache entry.
#[derive(Debug, Clone)]
struct Entry {
    /// The newest quote seen for the ticker, if any reply ever carried one.
    quote: Option<MarketQuote>,
    /// When the ticker was last asked about (a quote or a close), Unix
    /// seconds — what [`FRESH_FOR`] is measured from.
    fetched_at: i64,
}

#[derive(Debug, Default)]
struct CacheState {
    /// Keyed by the SOURCE's ticker (a commodity's `yahoo:` tag, else its
    /// symbol): a quote is a fact about the ticker, whichever journal symbol
    /// names it.
    entries: HashMap<String, Entry>,
    backoff_until: Option<i64>,
}

/// The per-process quote cache, shared by every `AppState` clone. A
/// `std::sync::Mutex`: held for a few map operations, never across an
/// `.await`.
#[derive(Debug, Default)]
pub(crate) struct QuoteCache {
    state: Mutex<CacheState>,
}

impl QuoteCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `ticker` was asked about at `now`. A reply with no quote keeps the one
    /// already cached (it is still the newest known) but still counts as
    /// asked, so a ticker Yahoo has nothing for is not re-asked every click.
    pub(crate) fn record(&self, ticker: &str, quote: Option<MarketQuote>, now: i64) {
        let mut state = self.lock();
        let entry = state.entries.entry(ticker.to_string()).or_insert(Entry {
            quote: None,
            fetched_at: now,
        });
        entry.fetched_at = entry.fetched_at.max(now);
        if let Some(quote) = quote {
            // Concurrent replies can land out of order; the newer trade wins.
            if entry
                .quote
                .as_ref()
                .is_none_or(|old| old.time <= quote.time)
            {
                entry.quote = Some(quote);
            }
        }
    }

    /// Yahoo rate-limited a round at `now`.
    pub(crate) fn back_off(&self, now: i64) {
        let mut state = self.lock();
        let until = now + BACKOFF;
        state.backoff_until = Some(state.backoff_until.map_or(until, |old| old.max(until)));
    }

    fn backing_off(&self, now: i64) -> bool {
        self.lock().backoff_until.is_some_and(|until| now < until)
    }

    fn is_fresh(&self, ticker: &str, now: i64) -> bool {
        self.lock()
            .entries
            .get(ticker)
            .is_some_and(|entry| now - entry.fetched_at < FRESH_FOR)
    }

    /// The cached quote for `ticker`, whatever its age.
    pub(crate) fn quote(&self, ticker: &str) -> Option<MarketQuote> {
        self.lock()
            .entries
            .get(ticker)
            .and_then(|entry| entry.quote.clone())
    }
}

/// The cached quotes for every commodity the journal prices, keyed back to
/// the JOURNAL's symbol and paired with their trade time — the input
/// `live_price_directives` wants, plus what `liveAt` is computed from. No
/// network: this only reads the cache.
pub(crate) fn cached_quotes(cache: &QuoteCache, journal: &Journal) -> Vec<(LiveQuote, i64)> {
    let symbols: BTreeSet<&str> = journal
        .prices
        .iter()
        .map(|price| price.commodity.0.as_str())
        .collect();
    symbols
        .into_iter()
        .filter_map(|symbol| {
            let ticker = prices_api::yahoo_ticker(&journal.commodity_tags, symbol);
            cache.quote(&ticker).map(|quote| {
                (
                    LiveQuote {
                        symbol: symbol.to_string(),
                        price: quote.price,
                        date: quote.date,
                        currency: quote.currency,
                    },
                    quote.time,
                )
            })
        })
        .collect()
}

// ===========================================================================
// POST /api/prices/intraday
// ===========================================================================

/// `{symbols?: string[]}` — the journal symbols to refresh. Absent = every
/// currently-held stock symbol (the same set Update prices fetches). An
/// empty body is the same as `{}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireIntradayRequest {
    symbols: Option<Vec<String>>,
}

/// The refresh's answer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireIntradayResponse {
    /// Every requested symbol the cache now holds a quote for — fetched this
    /// round or still fresh from an earlier one — in request order.
    quotes: Vec<WireQuote>,
    /// Requested symbols NOT asked about this round: still fresh in the cache,
    /// or left unsent because Yahoo is rate-limiting.
    skipped: Vec<String>,
    /// Yahoo rate-limited this round, or an earlier one's backoff is still in
    /// force.
    rate_limited: bool,
}

/// One cached quote on the wire.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireQuote {
    /// The journal's symbol.
    symbol: String,
    /// Decimal string, like every other money value on this wire.
    price: WireDec,
    /// When it traded, Unix seconds.
    time: i64,
    /// ISO currency, `null` when Yahoo did not say.
    currency: Option<String>,
}

/// `POST /api/prices/intraday`. See the module docs.
pub(crate) async fn refresh(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, AppError> {
    let request = parse_request(&body)?;
    let reader = state.clone();
    let Json(symbols) = compute(move || requested_symbols(&reader, request)).await?;
    let body = refresh_symbols(&state, symbols, now_seconds()).await;
    Ok(no_store(body))
}

fn parse_request(body: &[u8]) -> Result<WireIntradayRequest, AppError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(WireIntradayRequest::default());
    }
    let request: WireIntradayRequest = serde_json::from_slice(body)
        .map_err(|error| AppError::BadRequest(format!("invalid intraday request: {error}")))?;
    if let Some(symbols) = &request.symbols {
        if symbols.len() > MAX_SYMBOLS {
            return Err(AppError::BadRequest(format!(
                "at most {MAX_SYMBOLS} symbols per refresh, got {}",
                symbols.len()
            )));
        }
        if let Some(bad) = symbols
            .iter()
            .find(|symbol| symbol.trim().is_empty() || symbol.len() > MAX_SYMBOL_BYTES)
        {
            return Err(AppError::BadRequest(format!(
                "not a commodity symbol: {bad:?}"
            )));
        }
    }
    Ok(request)
}

/// The request's symbols with their Yahoo tickers, deduped in request order;
/// every held stock symbol when it named none.
fn requested_symbols(
    state: &AppState,
    request: WireIntradayRequest,
) -> Result<Vec<WirePriceSymbol>, AppError> {
    let snapshot = state.snapshot();
    let journal = &snapshot.journal;
    let Some(symbols) = request.symbols else {
        return Ok(prices_api::symbols_to_price(journal, &today_utc())?.0);
    };
    let mut seen = BTreeSet::new();
    Ok(symbols
        .into_iter()
        .map(|symbol| symbol.trim().to_string())
        .filter(|symbol| seen.insert(symbol.clone()))
        .map(|symbol| WirePriceSymbol {
            yahoo_ticker: prices_api::yahoo_ticker(&journal.commodity_tags, &symbol),
            symbol,
        })
        .collect())
}

/// Refresh the stale ones among `symbols` at `now` and report the cache.
pub(crate) async fn refresh_symbols(
    state: &AppState,
    symbols: Vec<WirePriceSymbol>,
    now: i64,
) -> WireIntradayResponse {
    let cache = state.quotes_arc();
    let backing_off = cache.backing_off(now);
    // One request per TICKER, however many symbols share it.
    let stale: Vec<String> = if backing_off {
        Vec::new()
    } else {
        symbols
            .iter()
            .map(|symbol| symbol.yahoo_ticker.clone())
            .filter(|ticker| !cache.is_fresh(ticker, now))
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect()
    };

    let source = Arc::clone(state.price_source());
    let (answers, rate_limited) = auto_prices::paced(stale, |ticker: &String| {
        let (source, ticker) = (Arc::clone(&source), ticker.clone());
        async move { source.market_quote(&ticker).await }
    })
    .await;
    if rate_limited {
        cache.back_off(now);
    }
    let asked: BTreeMap<String, bool> = answers
        .into_iter()
        .map(|(ticker, answer)| {
            let sent = match answer {
                None | Some(Err(YahooError::RateLimited(_))) => false,
                Some(Ok(quote)) => {
                    cache.record(&ticker, quote, now);
                    true
                }
                // A failed request still counts as asked: the next click
                // inside the window must not retry a broken ticker at once.
                Some(Err(_)) => {
                    cache.record(&ticker, None, now);
                    true
                }
            };
            (ticker, sent)
        })
        .collect();

    let mut quotes = Vec::new();
    let mut skipped = Vec::new();
    for WirePriceSymbol {
        symbol,
        yahoo_ticker,
    } in symbols
    {
        if !asked.get(&yahoo_ticker).copied().unwrap_or(false) {
            skipped.push(symbol.clone());
        }
        if let Some(quote) = cache.quote(&yahoo_ticker) {
            quotes.push(WireQuote {
                symbol,
                price: quote.price.into(),
                time: quote.time,
                currency: quote.currency,
            });
        }
    }
    WireIntradayResponse {
        quotes,
        skipped,
        rate_limited: rate_limited || backing_off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::Dec;

    fn quote(time: i64) -> MarketQuote {
        MarketQuote {
            price: Dec::new(10_000, 2),
            time,
            date: "2026-11-26".to_string(),
            currency: Some("USD".to_string()),
        }
    }

    #[test]
    fn an_entry_is_fresh_for_fifteen_minutes() {
        let cache = QuoteCache::default();
        cache.record("VTI", Some(quote(1)), 1_000);
        assert!(cache.is_fresh("VTI", 1_000 + FRESH_FOR - 1));
        assert!(!cache.is_fresh("VTI", 1_000 + FRESH_FOR));
        assert!(!cache.is_fresh("AAPL", 1_000));
    }

    #[test]
    fn an_empty_reply_keeps_the_last_quote_and_an_older_one_never_replaces_it() {
        let cache = QuoteCache::default();
        cache.record("VTI", Some(quote(200)), 1_000);
        cache.record("VTI", None, 2_000);
        cache.record("VTI", Some(quote(100)), 2_000);
        assert_eq!(cache.quote("VTI").map(|q| q.time), Some(200));
        assert!(cache.is_fresh("VTI", 2_000 + FRESH_FOR - 1));
    }

    #[test]
    fn a_backoff_lasts_fifteen_minutes_and_never_shortens() {
        let cache = QuoteCache::default();
        cache.back_off(1_000);
        cache.back_off(500);
        assert!(cache.backing_off(1_000 + BACKOFF - 1));
        assert!(!cache.backing_off(1_000 + BACKOFF));
    }

    #[test]
    fn a_blank_body_is_an_empty_request_and_a_bad_symbol_is_refused() {
        assert!(parse_request(b"").unwrap().symbols.is_none());
        assert!(parse_request(b"  \n").unwrap().symbols.is_none());
        assert!(parse_request(br#"{"symbols":["VTI"]}"#).is_ok());
        assert!(parse_request(br#"{"symbols":[""]}"#).is_err());
        assert!(parse_request(br#"{"tickers":["VTI"]}"#).is_err());
    }
}
