//! A minimal client for Yahoo Finance's public chart JSON endpoint — the same
//! data source `pricehist`'s `yahoo` backend reads from (what the user's own
//! `update-prices.sh`/`get-historical-prices.sh` scripts used before this was a
//! button).
//!
//! Split into a network call ([`YahooClient::latest_close`]) and a pure parser
//! ([`parse_chart_response`]) so the part that decides what "the price" is —
//! which candle wins, how a null close and a timezone offset are handled — is
//! unit-tested against canned JSON with no network involved.
//!
//! [`PriceFeed`] is the seam `prices_api` fetches through: production wires up
//! [`YahooClient`], and the integration tests wire up a fake that never leaves
//! the process. (Named `PriceFeed`, not `PriceSource` — `ledgeline_core::holdings`
//! already exports a `PriceSource` for a holding's price provenance, and reusing
//! the name here would read as the same concept when it is not.)

use async_trait::async_trait;
use ledgeline_core::reports::periods::iso_from_days;
use ledgeline_core::{Dec, DecError};
use reqwest::Url;
use serde::Deserialize;
use thiserror::Error;

/// One fetched close: a calendar date and a per-unit quantity, still unstyled —
/// the caller renders it in the journal's own `AmountStyle`
/// (`ledgeline_core::edit::render_amount`).
///
/// `pub` (not `pub(crate)`): a fake [`PriceFeed`] built by the integration
/// tests, an external crate, has to be able to construct one.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchedPrice {
    pub date: String,
    pub quantity: Dec,
}

/// A failure fetching or decoding a quote. Distinct from "fetched fine but no
/// usable candle" (that is `Ok(None)`) — this is a transport/shape failure, and
/// the caller reports it as `fetch-error` rather than `not-found`. `pub` for
/// the same reason as [`FetchedPrice`].
#[derive(Debug, Error)]
pub enum YahooError {
    #[error("request to Yahoo Finance failed: {0}")]
    Http(String),
    #[error("Yahoo Finance response could not be parsed: {0}")]
    Shape(String),
    #[error("price out of range: {0}")]
    Decimal(#[from] DecError),
}

/// Where `prices_api` fetches a quote from — the seam that lets the
/// integration tests substitute a fake for the real Yahoo client. `ticker` is
/// the SOURCE's own symbol (already resolved from the journal's `yahoo:` tag,
/// or the hledger symbol itself); `as_of` is `YYYY-MM-DD`, inclusive.
///
/// `pub` and re-exported from the crate root (see `lib.rs`) so
/// `AppState::with_price_source` — used only by the integration tests — can
/// take one from outside this crate.
#[async_trait]
pub trait PriceFeed: Send + Sync {
    async fn latest_close(
        &self,
        ticker: &str,
        as_of: &str,
    ) -> Result<Option<FetchedPrice>, YahooError>;
}

/// The real Yahoo Finance chart endpoint.
pub(crate) struct YahooClient {
    client: reqwest::Client,
}

impl YahooClient {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl PriceFeed for YahooClient {
    async fn latest_close(
        &self,
        ticker: &str,
        as_of: &str,
    ) -> Result<Option<FetchedPrice>, YahooError> {
        fetch_latest_close(&self.client, ticker, as_of).await
    }
}

const CHART_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
/// A month of daily candles is far more than the 7-day lookback the bash
/// scripts used, and costs nothing extra: it is one request either way, and the
/// wider window is strictly more forgiving of a long weekend or a mutual fund
/// that only publishes its NAV a few times a month.
///
/// Relative to *Yahoo's* clock, not `as_of` — correct because every caller in
/// this codebase passes today's date (there is no historical-backfill feature
/// yet; see `TODO.md`). If a future caller ever passes a date in the past, this
/// still returns candles up to today and [`parse_chart_response`] correctly
/// filters to `date <= as_of`, but it would be paying for a request whose upper
/// end it throws away — switch to explicit `period1`/`period2` query params if
/// that becomes a real call pattern.
const RANGE: &str = "1mo";

fn chart_url(ticker: &str) -> Result<Url, YahooError> {
    let mut url = Url::parse(CHART_BASE).map_err(|error| YahooError::Shape(error.to_string()))?;
    url.path_segments_mut()
        .map_err(|()| YahooError::Shape("the chart endpoint URL cannot take a path".to_string()))?
        .push(ticker);
    Ok(url)
}

/// GET the chart endpoint for `ticker` and resolve the latest close dated
/// `<= as_of`. `Ok(None)` means the request succeeded but no usable candle was
/// found (an empty range, every close null, or every date after `as_of`); an
/// `Err` means the request itself, or the response's shape, could not be
/// trusted.
async fn fetch_latest_close(
    client: &reqwest::Client,
    ticker: &str,
    as_of: &str,
) -> Result<Option<FetchedPrice>, YahooError> {
    let url = chart_url(ticker)?;
    let response = client
        .get(url)
        .query(&[("interval", "1d"), ("range", RANGE)])
        // Yahoo's chart endpoint 999s a client with no User-Agent at all.
        .header(
            reqwest::header::USER_AGENT,
            "ledgeline/0.1 (+https://github.com/zmre/ledgeline)",
        )
        .send()
        .await
        .map_err(|error| YahooError::Http(error.to_string()))?
        .error_for_status()
        .map_err(|error| YahooError::Http(error.to_string()))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| YahooError::Http(error.to_string()))?;
    parse_chart_response(&bytes, as_of)
}

#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: Chart,
}

#[derive(Debug, Deserialize)]
struct Chart {
    #[serde(default)]
    result: Option<Vec<ChartResult>>,
}

#[derive(Debug, Deserialize)]
struct ChartResult {
    #[serde(default)]
    meta: ChartMeta,
    #[serde(default)]
    timestamp: Vec<i64>,
    indicators: Indicators,
}

#[derive(Debug, Default, Deserialize)]
struct ChartMeta {
    /// Seconds east of UTC for the exchange this symbol trades on. Shifting a
    /// candle's timestamp by this before taking its calendar date is what keeps
    /// a 4pm-close US stock from landing on tomorrow's date for a journal
    /// that's read in a timezone west of Greenwich (or the reverse, east of it).
    #[serde(default)]
    gmtoffset: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Indicators {
    #[serde(default)]
    quote: Vec<Quote>,
}

#[derive(Debug, Deserialize)]
struct Quote {
    #[serde(default)]
    close: Vec<Option<f64>>,
}

/// The pure half of a fetch: given a chart response body, the latest close
/// dated `<= as_of`, or `None` when there is none.
///
/// A `null` close (Yahoo's marker for a still-forming or holiday candle) is
/// skipped, matching the bash scripts' `awk '$1 == "P"' | tail -n1` — take the
/// most recent USABLE quote, not simply the most recent timestamp.
pub(crate) fn parse_chart_response(
    bytes: &[u8],
    as_of: &str,
) -> Result<Option<FetchedPrice>, YahooError> {
    let parsed: ChartResponse =
        serde_json::from_slice(bytes).map_err(|error| YahooError::Shape(error.to_string()))?;
    let Some(result) = parsed.chart.result.into_iter().flatten().next() else {
        return Ok(None);
    };
    let Some(quote) = result.indicators.quote.into_iter().next() else {
        return Ok(None);
    };
    let offset = result.meta.gmtoffset.unwrap_or(0);

    let latest = result
        .timestamp
        .iter()
        .zip(quote.close.iter())
        .filter_map(|(&timestamp, close)| {
            let close = (*close)?;
            let date = date_from_timestamp(timestamp, offset);
            (date.as_str() <= as_of).then_some((date, close))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b));

    let Some((date, close)) = latest else {
        return Ok(None);
    };
    let quantity = Dec::parse(&close.to_string(), '.')?;
    Ok(Some(FetchedPrice { date, quantity }))
}

/// A Unix timestamp, shifted by an exchange's UTC offset, as an ISO calendar
/// date — reusing the same civil-calendar math [`crate::reports_api::today_utc`]
/// is built on ([`iso_from_days`]), so a "today" computed there and a candle
/// date computed here can never disagree about what day it is.
fn date_from_timestamp(timestamp: i64, gmtoffset: i64) -> String {
    let days = (timestamp + gmtoffset).div_euclid(86_400);
    iso_from_days(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(timestamps: &str, closes: &str, gmtoffset: &str) -> String {
        format!(
            r#"{{"chart":{{"result":[{{"meta":{{"gmtoffset":{gmtoffset}}},
              "timestamp":[{timestamps}],
              "indicators":{{"quote":[{{"close":[{closes}]}}]}}}}],"error":null}}}}"#
        )
    }

    /// Days since the Unix epoch for a civil (y, m, d) — Howard Hinnant's
    /// `days_from_civil`, the standard algorithm and the forward direction of
    /// the one [`iso_from_days`] already implements. Kept as an INDEPENDENT
    /// implementation here (not calling into `iso_from_days`'s own crate) so
    /// these tests build genuinely-known timestamps rather than timestamps the
    /// function under test would agree with by construction.
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn unix_seconds(y: i64, m: i64, d: i64, hour: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 + hour * 3_600
    }

    /// Self-check: a well-known epoch constant, so a bug in [`days_from_civil`]
    /// itself fails loudly here rather than silently mis-dating every test below.
    #[test]
    fn days_from_civil_matches_a_known_epoch_constant() {
        assert_eq!(unix_seconds(2000, 1, 1, 0), 946_684_800);
        assert_eq!(unix_seconds(1970, 1, 1, 0), 0);
    }

    /// A run of daily candles ending after `as_of`: the latest one AT OR BEFORE
    /// `as_of` wins, not simply the last element.
    #[test]
    fn takes_the_latest_close_at_or_before_as_of() {
        let timestamps = format!(
            "{},{},{},{}",
            unix_seconds(2026, 6, 29, 12),
            unix_seconds(2026, 6, 30, 12),
            unix_seconds(2026, 7, 1, 12),
            unix_seconds(2026, 7, 2, 12),
        );
        let closes = "228.10,229.40,230.00,231.50";
        let parsed =
            parse_chart_response(body(&timestamps, closes, "0").as_bytes(), "2026-06-30").unwrap();
        let price = parsed.expect("a price");
        assert_eq!(price.date, "2026-06-30");
        assert_eq!(price.quantity, Dec::parse("229.4", '.').unwrap());
    }

    /// Yahoo nulls out a still-forming (or holiday) candle; the parser must
    /// skip it and fall back to the last REAL close, not report a gap as a
    /// price of zero.
    #[test]
    fn skips_a_null_close_and_falls_back_to_the_prior_real_one() {
        let timestamps = format!(
            "{},{}",
            unix_seconds(2026, 6, 29, 12),
            unix_seconds(2026, 6, 30, 12),
        );
        let closes = "228.10,null";
        let parsed =
            parse_chart_response(body(&timestamps, closes, "0").as_bytes(), "2026-06-30").unwrap();
        let price = parsed.expect("a price");
        assert_eq!(price.date, "2026-06-29");
        assert_eq!(price.quantity, Dec::parse("228.1", '.').unwrap());
    }

    /// A negative offset (west of Greenwich) can push a timestamp back a
    /// calendar day relative to raw UTC.
    #[test]
    fn applies_the_exchange_gmtoffset_before_taking_the_date() {
        // 2026-07-01T02:00:00Z, exchange at UTC-5 => local date is still 06-30.
        let timestamp = unix_seconds(2026, 7, 1, 2);
        let closes = "100.00";
        let parsed = parse_chart_response(
            body(&timestamp.to_string(), closes, "-18000").as_bytes(),
            "2026-07-01",
        )
        .unwrap();
        assert_eq!(parsed.unwrap().date, "2026-06-30");
    }

    /// Every candle postdates `as_of`: there is nothing usable yet.
    #[test]
    fn returns_none_when_every_candle_is_after_as_of() {
        let timestamp = unix_seconds(2026, 6, 30, 12);
        let closes = "229.40";
        let parsed = parse_chart_response(
            body(&timestamp.to_string(), closes, "0").as_bytes(),
            "2020-01-01",
        )
        .unwrap();
        assert!(parsed.is_none());
    }

    /// An unknown ticker: `result` is `null`, not an empty array.
    #[test]
    fn returns_none_for_a_null_result() {
        let parsed = parse_chart_response(
            br#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found"}}}"#,
            "2026-06-30",
        )
        .unwrap();
        assert!(parsed.is_none());
    }

    /// Garbage in the body is a `Shape` error, not a panic or a silent `None`.
    #[test]
    fn malformed_json_is_a_shape_error() {
        let error = parse_chart_response(b"not json", "2026-06-30").unwrap_err();
        assert!(matches!(error, YahooError::Shape(_)));
    }
}
