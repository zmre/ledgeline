//! Dividend-adjusted price HISTORY from Yahoo Finance's chart endpoint — the
//! input to the Holdings benchmark overlay (`benchmarks_api`).
//!
//! A sibling of [`crate::yahoo`] rather than a second method on its
//! [`PriceFeed`](crate::yahoo::PriceFeed): that trait answers "what is this
//! security's latest close" for a journal write, and this one answers "what
//! did an index fund return, day by day, including its dividends" for a chart.
//! Different questions, different price columns (`close` vs `adjclose`), and a
//! test double for one should not have to fake the other.
//!
//! Same split as the latest-close client: a network call and a pure parser
//! ([`parse_adjusted_history`]) that is unit-tested against real, saved
//! responses (`fixtures/yahoo/`), never the network.

use async_trait::async_trait;
use ledgeline_core::Dec;
use ledgeline_core::reports::periods::days_between;

use crate::yahoo::{
    ChartError, ChartResponse, FetchedPrice, YahooClient, YahooError, date_from_timestamp,
    send_chart,
};

/// How many decimal places an adjusted close is kept to. Yahoo answers in
/// binary floats (`656.59619140625`); four places is a hundredth of a cent,
/// far below anything a chart line can show, and keeps the cache file honest
/// about the precision it actually has.
const ADJUSTED_PLACES: usize = 4;

/// Where `benchmarks_api` fetches benchmark history from — the seam the
/// integration tests replace with a fake that never leaves the process.
///
/// `pub` and re-exported from the crate root for the same reason as
/// [`crate::yahoo::PriceFeed`]: `AppState::with_history_source` takes one from
/// outside this crate.
#[async_trait]
pub trait HistoryFeed: Send + Sync {
    /// Daily dividend-adjusted closes for `ticker` dated `from..=to`
    /// (`YYYY-MM-DD`), ascending, one per trading day. Empty when the source
    /// answered but had no candles in the range; an `Err` when it refused or
    /// failed. The cache never reads either as coverage (`benchmarks_api`).
    async fn adjusted_history(
        &self,
        ticker: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<FetchedPrice>, YahooError>;
}

#[async_trait]
impl HistoryFeed for YahooClient {
    async fn adjusted_history(
        &self,
        ticker: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<FetchedPrice>, YahooError> {
        fetch_adjusted_history(&self.client, ticker, from, to).await
    }
}

/// Seconds since the Unix epoch at midnight UTC on `date`.
fn epoch_seconds(date: &str) -> i64 {
    days_between("1970-01-01", date) * 86_400
}

async fn fetch_adjusted_history(
    client: &reqwest::Client,
    ticker: &str,
    from: &str,
    to: &str,
) -> Result<Vec<FetchedPrice>, YahooError> {
    // `period2` is exclusive, so the day after `to` — plus a day's slack for an
    // exchange west of Greenwich, whose last candle carries a timestamp that is
    // already "tomorrow" in UTC. The parser filters back to `to`.
    let period1 = epoch_seconds(from).to_string();
    let period2 = (epoch_seconds(to) + 2 * 86_400).to_string();
    let bytes = send_chart(
        client,
        ticker,
        &[
            ("interval", "1d"),
            ("period1", period1.as_str()),
            ("period2", period2.as_str()),
            ("events", "div,splits"),
            ("includeAdjustedClose", "true"),
        ],
    )
    .await?;
    parse_adjusted_history(bytes.as_ref(), to)
}

/// The pure half of a history fetch: every dividend-adjusted close in a chart
/// response dated on or before `to`, ascending, one per date.
///
/// A result with no candles is an empty success; a null result is an error
/// (as is a non-success status: [`crate::yahoo::check_chart_status`]).
///
/// A null (a holiday, or today's still-forming candle) is skipped rather than
/// read as zero. Two candles on one date — Yahoo appends a live candle that can
/// share the last session's date — keep the later one. A response with no
/// `adjclose` column at all is a `Shape` error: silently falling back to the
/// raw close would hand the benchmark line a return without its dividends,
/// which is precisely the understatement the adjusted column exists to avoid.
///
/// # Errors
/// [`YahooError::Shape`] for a body that is not a chart response or lacks the
/// adjusted column; [`YahooError::Decimal`] for a close outside `Dec`'s range.
pub(crate) fn parse_adjusted_history(
    bytes: &[u8],
    to: &str,
) -> Result<Vec<FetchedPrice>, YahooError> {
    let parsed: ChartResponse =
        serde_json::from_slice(bytes).map_err(|error| YahooError::Shape(error.to_string()))?;
    let Some(result) = parsed.chart.result.into_iter().flatten().next() else {
        // No result is never "no history": either Yahoo said why (an error
        // object), or the body is not the answer it claims to be.
        return Err(match parsed.chart.error.and_then(ChartError::describe) {
            Some(description) => YahooError::Http(description),
            None => YahooError::Shape("the chart response has no result".to_string()),
        });
    };
    if result.timestamp.is_empty() {
        return Ok(Vec::new());
    }
    let Some(column) = result.indicators.adjclose.into_iter().next() else {
        return Err(YahooError::Shape(
            "the chart response has no adjusted-close column".to_string(),
        ));
    };
    let offset = result.meta.gmtoffset.unwrap_or(0);

    let mut prices: Vec<FetchedPrice> = Vec::with_capacity(result.timestamp.len());
    for (&timestamp, close) in result.timestamp.iter().zip(column.adjclose.iter()) {
        let Some(close) = close.filter(|close| close.is_finite() && *close > 0.0) else {
            continue;
        };
        let date = date_from_timestamp(timestamp, offset);
        if date.as_str() > to {
            continue;
        }
        let quantity = Dec::parse(&format!("{close:.ADJUSTED_PLACES$}"), '.')?;
        prices.push(FetchedPrice { date, quantity });
    }
    Ok(one_per_date(prices))
}

/// `prices` in date order, one per date: the LAST of any that share a date (in
/// their input order) wins.
pub(crate) fn one_per_date(mut prices: Vec<FetchedPrice>) -> Vec<FetchedPrice> {
    // Stable, so same-date prices keep their input order for `dedup_by` to
    // resolve: it drops the later element, so copy its value back first.
    prices.sort_by(|a, b| a.date.cmp(&b.date));
    prices.dedup_by(|later, earlier| {
        let same = later.date == earlier.date;
        if same {
            std::mem::swap(&mut earlier.quantity, &mut later.quantity);
        }
        same
    });
    prices
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPY_1Y: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/yahoo/chart_SPY_1y_1d.json"
    ));
    const BND_5Y_WEEKLY: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/yahoo/chart_BND_5y_1wk.json"
    ));

    fn dec(text: &str) -> Dec {
        Dec::parse(text, '.').unwrap()
    }

    /// A real SPY year: 251 sessions, the first on 2025-09-29 at an adjusted
    /// $656.60 (the raw close that day was $663.68 — the dividends since are
    /// what the difference is), the last on 2026-09-28 — the live candle of the
    /// day it was saved, which the cache layer declines to store.
    #[test]
    fn reads_the_adjusted_column_of_a_real_response() {
        let prices = parse_adjusted_history(SPY_1Y, "2026-12-31").unwrap();
        assert_eq!(prices.len(), 251);
        assert_eq!(prices[0].date, "2025-09-29");
        assert_eq!(prices[0].quantity, dec("656.5962"));
        let last = prices.last().unwrap();
        assert_eq!(last.date, "2026-09-28");
        assert_eq!(last.quantity, dec("766.2900"));
        assert!(prices.windows(2).all(|pair| pair[0].date < pair[1].date));
    }

    #[test]
    fn stops_at_the_requested_end_date() {
        let prices = parse_adjusted_history(SPY_1Y, "2025-10-01").unwrap();
        let dates: Vec<&str> = prices.iter().map(|price| price.date.as_str()).collect();
        assert_eq!(dates, ["2025-09-29", "2025-09-30", "2025-10-01"]);
    }

    /// Weekly candles parse the same way — the column is the same column — and
    /// the exchange's UTC offset keeps a Monday candle on Monday.
    #[test]
    fn reads_weekly_candles_on_their_exchange_local_dates() {
        let prices = parse_adjusted_history(BND_5Y_WEEKLY, "2030-01-01").unwrap();
        assert!(prices.len() > 250);
        // 1632715200 = 2021-09-27T04:00:00Z, midnight in New York.
        assert_eq!(prices[0].date, "2021-09-27");
    }

    const REFUSED: &[u8] = br#"{"chart":{"result":null,"error":{"code":"Bad Request","description":"Data doesn't exist for startDate = 946684800, endDate = 1296000000"}}}"#;

    #[test]
    fn a_null_result_with_an_error_is_an_error_not_an_empty_history() {
        match parse_adjusted_history(REFUSED, "2026-01-01") {
            Err(YahooError::Http(message)) => assert!(message.contains("Data doesn't exist")),
            other => panic!("expected an error, got {other:?}"),
        }
        let bare = br#"{"chart":{"result":null}}"#;
        assert!(matches!(
            parse_adjusted_history(bare, "2026-01-01"),
            Err(YahooError::Shape(_))
        ));
    }

    /// A result with no candles parses as an empty success; the cache layer
    /// decides what that means (it never advances coverage on it).
    #[test]
    fn a_result_without_candles_is_an_empty_success() {
        let body = br#"{"chart":{"result":[{"meta":{"gmtoffset":0},"indicators":{"adjclose":[{}]}}],"error":null}}"#;
        assert!(
            parse_adjusted_history(body, "2026-01-01")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn skips_null_candles_and_keeps_the_later_of_two_on_one_date() {
        // Two stamps on 2026-06-30 (a live candle after the session's own) and
        // a null on 07-01.
        let body = br#"{"chart":{"result":[{"meta":{"gmtoffset":0},
            "timestamp":[1782820800,1782846000,1782907200],
            "indicators":{"quote":[{"close":[1,2,3]}],"adjclose":[{"adjclose":[10.5,10.75,null]}]}}]}}"#;
        let prices = parse_adjusted_history(body, "2026-12-31").unwrap();
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].date, "2026-06-30");
        assert_eq!(prices[0].quantity, dec("10.75"));
    }

    #[test]
    fn a_response_without_the_adjusted_column_is_refused() {
        let body = br#"{"chart":{"result":[{"meta":{"gmtoffset":0},"timestamp":[1782820800],
            "indicators":{"quote":[{"close":[1]}]}}]}}"#;
        assert!(matches!(
            parse_adjusted_history(body, "2026-12-31"),
            Err(YahooError::Shape(_))
        ));
    }

    #[test]
    fn one_per_date_sorts_and_keeps_the_last_of_a_date() {
        let price = |date: &str, close: &str| FetchedPrice {
            date: date.to_string(),
            quantity: dec(close),
        };
        let kept = one_per_date(vec![
            price("2026-01-03", "3"),
            price("2026-01-02", "1"),
            price("2026-01-02", "2"),
        ]);
        assert_eq!(kept, [price("2026-01-02", "2"), price("2026-01-03", "3")]);
    }

    #[test]
    fn epoch_seconds_is_midnight_utc() {
        assert_eq!(epoch_seconds("1970-01-01"), 0);
        assert_eq!(epoch_seconds("2000-01-01"), 946_684_800);
    }
}
