//! A minimal client for Yahoo Finance's public chart JSON endpoint — the same
//! data source `pricehist`'s `yahoo` backend reads from (what the user's own
//! `update-prices.sh`/`get-historical-prices.sh` scripts used before this was a
//! button).
//!
//! Split into a network call ([`YahooClient::latest_close`],
//! [`YahooClient::daily_closes`]) and a pure parser ([`parse_chart_closes`]) so
//! the part that decides what "the price" is — which candle wins, how a null
//! close, a timezone offset and a still-trading session are handled — is
//! unit-tested against canned JSON (and a saved real response) with no network
//! involved.
//!
//! [`PriceFeed`] is the seam `prices_api` fetches through: production wires up
//! [`YahooClient`], and the integration tests wire up a fake that never leaves
//! the process. (Named `PriceFeed`, not `PriceSource` — `ledgeline_core::holdings`
//! already exports a `PriceSource` for a holding's price provenance, and reusing
//! the name here would read as the same concept when it is not.)

use async_trait::async_trait;
use ledgeline_core::reports::periods::{days_from_iso, iso_from_days};
use ledgeline_core::{Dec, DecError};
use reqwest::Url;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;
use thiserror::Error;

use crate::reports_api::unix_now;

/// The User-Agent every Yahoo request identifies itself with, unless it needs
/// to look like a browser (`yahoo_profile`'s crumb handshake). Yahoo's chart
/// endpoint 999s a client with no User-Agent at all.
pub(crate) const USER_AGENT: &str = "ledgeline/0.1 (+https://github.com/zmre/ledgeline)";

/// Every Yahoo request's default ceiling. reqwest's own default never times
/// out, and a hung connection must cost a failed fetch, not a request held
/// open indefinitely. A request may set a tighter one (`yahoo_profile` does).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// The one HTTP client every Yahoo feed shares, so they pool connections (a
/// fresh client per feed, or per request, re-negotiates TLS each time).
///
/// Trusts the OS certificate store first (reqwest 0.13's platform verifier, so
/// a corporate TLS-inspecting proxy's root works). Where there is no store to
/// load — a minimal Linux install, the nix build sandbox — that build FAILS,
/// and `Client::default()` would then panic the whole server at startup, so
/// fall back to the bundled Mozilla roots that reqwest 0.12 always used.
pub(crate) fn http_client() -> reqwest::Client {
    let builder = || {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
    };
    builder()
        .build()
        .or_else(|_| builder().tls_certs_only(bundled_roots()).build())
        .unwrap_or_default()
}

/// Mozilla's root set, as compiled into `webpki-root-certs`.
fn bundled_roots() -> Vec<reqwest::Certificate> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|der| reqwest::Certificate::from_der(der).ok())
        .collect()
}

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
    /// Yahoo said "slow down": a `429`, or the `999` it answers a client it has
    /// decided is a bot. Its own variant, not folded into [`Self::Http`],
    /// because it means something different to a caller that runs unattended
    /// (`auto_prices`): any other failure is about ONE ticker, while this one
    /// is about US, and the right response is to stop asking for anything at
    /// all for a while rather than to carry on down the list.
    #[error("Yahoo Finance is limiting requests: {0}")]
    RateLimited(String),
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

    /// Every COMPLETED daily close for `ticker` from `from` (`YYYY-MM-DD`,
    /// inclusive) up to now, ascending, one per date. Empty when the request
    /// succeeded but had nothing usable (an unknown or delisted ticker).
    ///
    /// The automatic updater's one request per symbol (`auto_prices`): the
    /// latest close and every backfilled week's close come out of the same
    /// answer, so how many weeks are wanted changes the size of the response,
    /// never the number of requests.
    async fn daily_closes(&self, ticker: &str, from: &str)
    -> Result<Vec<FetchedPrice>, YahooError>;
}

/// The real Yahoo Finance chart endpoint.
pub(crate) struct YahooClient {
    pub(crate) client: reqwest::Client,
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

    async fn daily_closes(
        &self,
        ticker: &str,
        from: &str,
    ) -> Result<Vec<FetchedPrice>, YahooError> {
        fetch_daily_closes(&self.client, ticker, from).await
    }
}

const CHART_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
/// A month of daily candles is far more than the 7-day lookback the bash
/// scripts used, and costs nothing extra: it is one request either way, and the
/// wider window is strictly more forgiving of a long weekend or a mutual fund
/// that only publishes its NAV a few times a month.
///
/// Relative to *Yahoo's* clock, not `as_of` — correct because every caller of
/// [`PriceFeed::latest_close`] passes today's date. The weekly backfill, which
/// does want a window of its own choosing, goes through
/// [`PriceFeed::daily_closes`] and explicit `period1`/`period2` instead.
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
    let bytes = send_chart(client, ticker, &[("interval", "1d"), ("range", RANGE)]).await?;
    parse_chart_response(bytes.as_ref(), as_of, now_seconds())
}

/// GET the chart endpoint for `ticker`'s daily candles from `from` until now,
/// keeping the completed ones ([`parse_chart_closes`]).
///
/// `period2` is a day past now rather than now itself: it is exclusive, and
/// Yahoo snaps it to its own candle boundaries, so the slack costs nothing and
/// can never hide the session that just closed. The session that has NOT
/// closed is the parser's job to drop, not the URL's.
async fn fetch_daily_closes(
    client: &reqwest::Client,
    ticker: &str,
    from: &str,
) -> Result<Vec<FetchedPrice>, YahooError> {
    let now = now_seconds();
    let period1 = (days_from_iso(from) * SECONDS_PER_DAY).to_string();
    let period2 = (now + SECONDS_PER_DAY).to_string();
    let bytes = send_chart(
        client,
        ticker,
        &[
            ("interval", "1d"),
            ("period1", period1.as_str()),
            ("period2", period2.as_str()),
        ],
    )
    .await?;
    parse_chart_closes(bytes.as_ref(), now)
}

const SECONDS_PER_DAY: i64 = 86_400;

/// The wall clock as Unix seconds, for the parser's "has this session closed
/// yet" question. Read once per request, here at the network edge, so the
/// parser itself stays a pure function of its inputs.
fn now_seconds() -> i64 {
    i64::try_from(unix_now()).unwrap_or(i64::MAX)
}

/// GET the chart endpoint for `ticker` with `query`, returning the body of a
/// SUCCESSFUL reply ([`check_chart_status`]).
pub(crate) async fn send_chart(
    client: &reqwest::Client,
    ticker: &str,
    query: &[(&str, &str)],
) -> Result<impl AsRef<[u8]> + use<>, YahooError> {
    let response = client
        .get(chart_url(ticker)?)
        .query(query)
        .send()
        .await
        .map_err(|error| YahooError::Http(error.to_string()))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| YahooError::Http(error.to_string()))?;
    check_chart_status(status, &bytes)?;
    Ok(bytes)
}

/// A non-success status is an error whatever the body says. A `400` with a
/// well-formed `{"chart":{"result":null,"error":…}}` body is Yahoo REFUSING the
/// request, not answering "no data" — reading it as an empty success would let
/// a caller record coverage it never received. Yahoo's own description, when
/// the body carries one, is kept for the message.
pub(crate) fn check_chart_status(
    status: reqwest::StatusCode,
    bytes: &[u8],
) -> Result<(), YahooError> {
    if status.is_success() {
        return Ok(());
    }
    let detail = serde_json::from_slice::<ChartResponse>(bytes)
        .ok()
        .and_then(|parsed| parsed.chart.error)
        .and_then(ChartError::describe)
        .map(|description| format!(": {description}"))
        .unwrap_or_default();
    let message = format!("Yahoo Finance answered {status}{detail}");
    if is_rate_limit(status) {
        return Err(YahooError::RateLimited(message));
    }
    Err(YahooError::Http(message))
}

/// `429 Too Many Requests`, or Yahoo's own non-standard `999`, which it sends
/// a client it has decided is scraping too hard. Both mean "back off", not
/// "this ticker is bad".
fn is_rate_limit(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.as_u16() == 999
}

/// The chart endpoint's reply — one shape for both the latest close (`quote`)
/// and the adjusted history (`adjclose`).
#[derive(Debug, Deserialize)]
pub(crate) struct ChartResponse {
    pub(crate) chart: Chart,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Chart {
    #[serde(default)]
    pub(crate) result: Option<Vec<ChartResult>>,
    #[serde(default)]
    pub(crate) error: Option<ChartError>,
}

/// Yahoo's `chart.error` object: `{"code":"Not Found","description":"…"}`.
#[derive(Debug, Deserialize)]
pub(crate) struct ChartError {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

impl ChartError {
    /// The most specific text the error carries, if any.
    pub(crate) fn describe(self) -> Option<String> {
        self.description
            .or(self.code)
            .filter(|text| !text.trim().is_empty())
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChartResult {
    #[serde(default)]
    pub(crate) meta: ChartMeta,
    #[serde(default)]
    pub(crate) timestamp: Vec<i64>,
    pub(crate) indicators: Indicators,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ChartMeta {
    /// Seconds east of UTC for the exchange this symbol trades on. Shifting a
    /// candle's timestamp by this before taking its calendar date is what keeps
    /// a 4pm-close US stock from landing on tomorrow's date for a journal
    /// that's read in a timezone west of Greenwich (or the reverse, east of it).
    #[serde(default)]
    pub(crate) gmtoffset: Option<i64>,
    /// The exchange's current (or, out of hours, most recent or next) trading
    /// day, as Unix-second bounds. What tells a FINISHED day's candle from the
    /// one still forming: during market hours Yahoo's daily series ends with a
    /// candle for today whose `close` is NOT null — it is the last trade so
    /// far — and without this the button would record an intraday price as
    /// the day's close. See [`parse_chart_closes`].
    #[serde(default, rename = "currentTradingPeriod")]
    pub(crate) current_trading_period: Option<TradingPeriods>,
}

/// `meta.currentTradingPeriod`: `pre`, `regular` and `post` sessions. Only the
/// regular one decides anything — a close is the regular session's last trade.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct TradingPeriods {
    #[serde(default)]
    pub(crate) regular: Option<TradingPeriod>,
}

/// One session's bounds, in Unix seconds.
#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct TradingPeriod {
    pub(crate) start: i64,
    pub(crate) end: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Indicators {
    /// The raw closes (`fetch_latest_close`).
    #[serde(default)]
    pub(crate) quote: Vec<Quote>,
    /// The dividend-adjusted closes (`yahoo_history`), present only when asked
    /// for with `includeAdjustedClose`.
    #[serde(default)]
    pub(crate) adjclose: Vec<AdjClose>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Quote {
    #[serde(default)]
    close: Vec<Option<f64>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdjClose {
    #[serde(default)]
    pub(crate) adjclose: Vec<Option<f64>>,
}

/// The pure half of a fetch: given a chart response body, the latest
/// COMPLETED close dated `<= as_of`, or `None` when there is none.
///
/// A `null` close (Yahoo's marker for a holiday candle) is skipped, matching
/// the bash scripts' `awk '$1 == "P"' | tail -n1` — take the most recent
/// USABLE quote, not simply the most recent timestamp. So is the session still
/// trading at `now` ([`parse_chart_closes`]).
pub(crate) fn parse_chart_response(
    bytes: &[u8],
    as_of: &str,
    now: i64,
) -> Result<Option<FetchedPrice>, YahooError> {
    Ok(parse_chart_closes(bytes, now)?
        .into_iter()
        .rev()
        .find(|price| price.date.as_str() <= as_of))
}

/// Every completed daily close in a chart response, ascending, one per date
/// (`now` is Unix seconds).
///
/// "Completed" is decided by the EXCHANGE, through
/// `meta.currentTradingPeriod.regular`: a candle dated on that session's day
/// is dropped while `now` is before the session's end. Yahoo's daily series
/// carries today's candle all through the trading day with a live, non-null
/// close, and a price recorded from it is a mid-session last trade written
/// down as the day's close — wrong, and wrong silently, because the next
/// update dedupes on the date and never corrects it. The exchange's own
/// session bounds get this right for any market and any daylight-saving rule,
/// where a fixed "after 4pm New York" cut-off would not. A response with no
/// trading period at all (Yahoo omits it for some instruments) keeps every
/// candle: there is nothing to judge by, and that is how this read before.
///
/// A `null` close is skipped rather than read as zero. Two candles on one date
/// (Yahoo can append a live candle sharing the last session's date) keep the
/// later one. An unknown ticker — `result: null` — is an empty answer, not an
/// error: there is genuinely nothing to price it at.
pub(crate) fn parse_chart_closes(bytes: &[u8], now: i64) -> Result<Vec<FetchedPrice>, YahooError> {
    let parsed: ChartResponse =
        serde_json::from_slice(bytes).map_err(|error| YahooError::Shape(error.to_string()))?;
    let Some(result) = parsed.chart.result.into_iter().flatten().next() else {
        return Ok(Vec::new());
    };
    let Some(quote) = result.indicators.quote.into_iter().next() else {
        return Ok(Vec::new());
    };
    let offset = result.meta.gmtoffset.unwrap_or(0);
    // The day whose candle is still forming, if any: the regular session's
    // local date, while it has not yet ended.
    let open_session = result
        .meta
        .current_trading_period
        .and_then(|periods| periods.regular)
        .filter(|regular| now < regular.end)
        .map(|regular| date_from_timestamp(regular.start, offset));

    // A BTreeMap: ascending by date, and a later candle on the same date
    // overwrites the earlier one.
    let by_date: BTreeMap<String, f64> = result
        .timestamp
        .iter()
        .zip(quote.close.iter())
        .filter_map(|(&timestamp, close)| {
            let close = (*close).filter(|close| close.is_finite())?;
            let date = date_from_timestamp(timestamp, offset);
            (open_session.as_deref() != Some(date.as_str())).then_some((date, close))
        })
        .collect();

    by_date
        .into_iter()
        .map(|(date, close)| {
            let quantity = Dec::parse(&close.to_string(), '.')?;
            Ok(FetchedPrice { date, quantity })
        })
        .collect()
}

/// A Unix timestamp, shifted by an exchange's UTC offset, as an ISO calendar
/// date — reusing the same civil-calendar math [`crate::reports_api::today_utc`]
/// is built on ([`iso_from_days`]), so a "today" computed there and a candle
/// date computed here can never disagree about what day it is.
pub(crate) fn date_from_timestamp(timestamp: i64, gmtoffset: i64) -> String {
    let days = (timestamp + gmtoffset).div_euclid(86_400);
    iso_from_days(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback has to build on its own: it is what runs where the OS
    /// store cannot be loaded, and there the only alternative is a panic.
    #[test]
    fn the_bundled_roots_build_a_client_without_the_os_store() {
        let roots = bundled_roots();
        assert!(roots.len() > 100, "Mozilla's set, not a stub");
        reqwest::Client::builder()
            .tls_certs_only(roots)
            .build()
            .expect("a client from the bundled roots alone");
    }

    /// A `now` long after every candle in these tests, so the completed-session
    /// filter never applies unless a test sets a trading period on purpose.
    const NOW: i64 = 4_000_000_000;

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
            parse_chart_response(body(&timestamps, closes, "0").as_bytes(), "2026-06-30", NOW)
                .unwrap();
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
            parse_chart_response(body(&timestamps, closes, "0").as_bytes(), "2026-06-30", NOW)
                .unwrap();
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
            NOW,
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
            NOW,
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
            NOW,
        )
        .unwrap();
        assert!(parsed.is_none());
    }

    /// A refused request is an error even when its body is a well-formed chart
    /// reply, and Yahoo's own description is kept for the message.
    #[test]
    fn a_non_success_status_is_an_error_whatever_the_body() {
        let refused = br#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found"}}}"#;
        match check_chart_status(reqwest::StatusCode::NOT_FOUND, refused) {
            Err(YahooError::Http(message)) => {
                assert!(message.contains("404"), "{message}");
                assert!(message.contains("No data found"), "{message}");
            }
            other => panic!("expected an error, got {other:?}"),
        }
        assert!(check_chart_status(reqwest::StatusCode::BAD_GATEWAY, b"<html>").is_err());
        assert!(check_chart_status(reqwest::StatusCode::OK, b"{}").is_ok());
    }

    /// `429` and Yahoo's `999` are "slow down", a different thing from a bad
    /// ticker — the unattended updater stops on the first and carries on past
    /// the second.
    #[test]
    fn a_429_or_999_is_a_rate_limit_not_a_plain_failure() {
        for code in [429, 999] {
            let status = reqwest::StatusCode::from_u16(code).expect("a status");
            assert!(
                matches!(
                    check_chart_status(status, b""),
                    Err(YahooError::RateLimited(_))
                ),
                "{code}"
            );
        }
        assert!(matches!(
            check_chart_status(reqwest::StatusCode::NOT_FOUND, b""),
            Err(YahooError::Http(_))
        ));
    }

    /// A body with a regular session `start..end`, at UTC-4 (New York in
    /// summer), and one candle per `(timestamp, close)`.
    fn body_with_session(candles: &[(i64, &str)], start: i64, end: i64) -> String {
        let timestamps: Vec<String> = candles.iter().map(|(t, _)| t.to_string()).collect();
        let closes: Vec<&str> = candles.iter().map(|(_, c)| *c).collect();
        format!(
            r#"{{"chart":{{"result":[{{"meta":{{"gmtoffset":-14400,
              "currentTradingPeriod":{{"regular":{{"start":{start},"end":{end},"gmtoffset":-14400}}}}}},
              "timestamp":[{}],
              "indicators":{{"quote":[{{"close":[{}]}}]}}}}],"error":null}}}}"#,
            timestamps.join(","),
            closes.join(",")
        )
    }

    /// The bug the manual button had: mid-session, today's candle carries the
    /// last trade as a non-null close. Until the exchange's regular session
    /// ends it is not a close and must not be recorded as one; a second after
    /// the bell it is.
    #[test]
    fn todays_candle_counts_only_once_the_regular_session_has_ended() {
        // Thu 2026-07-02 and Fri 2026-07-03 sessions, 09:30-16:00 EDT.
        let thursday = unix_seconds(2026, 7, 2, 13) + 1_800;
        let friday_open = unix_seconds(2026, 7, 3, 13) + 1_800;
        let friday_close = unix_seconds(2026, 7, 3, 20);
        let body = body_with_session(
            &[(thursday, "100.5"), (friday_open, "101.25")],
            friday_open,
            friday_close,
        );

        let during = parse_chart_closes(body.as_bytes(), friday_close - 60).unwrap();
        assert_eq!(
            during.iter().map(|p| p.date.as_str()).collect::<Vec<_>>(),
            ["2026-07-02"],
            "the live candle is dropped"
        );
        let latest = parse_chart_response(body.as_bytes(), "2026-07-03", friday_close - 60)
            .unwrap()
            .expect("yesterday's close");
        assert_eq!(latest.date, "2026-07-02");

        let after = parse_chart_closes(body.as_bytes(), friday_close).unwrap();
        assert_eq!(
            after.iter().map(|p| p.date.as_str()).collect::<Vec<_>>(),
            ["2026-07-02", "2026-07-03"],
            "the session's end makes it a close"
        );
    }

    /// Over a weekend `currentTradingPeriod` names the last (ended) session, so
    /// nothing is dropped.
    #[test]
    fn an_ended_session_drops_nothing() {
        let friday_open = unix_seconds(2026, 7, 3, 13) + 1_800;
        let friday_close = unix_seconds(2026, 7, 3, 20);
        let body = body_with_session(&[(friday_open, "101.25")], friday_open, friday_close);
        let saturday = unix_seconds(2026, 7, 4, 12);
        assert_eq!(
            parse_chart_closes(body.as_bytes(), saturday).unwrap().len(),
            1
        );
    }

    /// Two candles sharing a date keep the later; a null is skipped; the
    /// result is ascending whatever order the candles came in.
    #[test]
    fn closes_are_one_per_date_ascending() {
        let a = unix_seconds(2026, 7, 1, 14);
        let b = unix_seconds(2026, 7, 1, 19);
        let c = unix_seconds(2026, 7, 2, 14);
        let body = body(&format!("{c},{a},{b}"), "null,1.5,2.5", "0");
        let closes = parse_chart_closes(body.as_bytes(), NOW).unwrap();
        assert_eq!(
            closes,
            vec![FetchedPrice {
                date: "2026-07-01".to_string(),
                quantity: Dec::parse("2.5", '.').unwrap(),
            }]
        );
    }

    /// A real saved response (`fixtures/yahoo/chart_SPY_1y_1d.json`, taken
    /// during the Monday 2026-09-28 session): its last candle is that live
    /// session, and only a `now` past `regular.end` lets it through.
    #[test]
    fn a_saved_mid_session_response_drops_its_live_candle() {
        const SPY_1Y: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/yahoo/chart_SPY_1y_1d.json"
        ));
        // `regularMarketTime` in that file: mid-session.
        let saved_at = 1_790_612_371;
        let closes = parse_chart_closes(SPY_1Y, saved_at).unwrap();
        assert_eq!(closes.last().map(|p| p.date.as_str()), Some("2026-09-25"));
        assert!(closes.len() > 200, "a year of closes");
        let closes = parse_chart_closes(SPY_1Y, 1_790_625_600).unwrap();
        assert_eq!(closes.last().map(|p| p.date.as_str()), Some("2026-09-28"));
    }

    /// Garbage in the body is a `Shape` error, not a panic or a silent `None`.
    #[test]
    fn malformed_json_is_a_shape_error() {
        let error = parse_chart_response(b"not json", "2026-06-30", NOW).unwrap_err();
        assert!(matches!(error, YahooError::Shape(_)));
    }
}
