//! `GET /api/holdings/benchmarks` — the Stocks chart's benchmark overlay.
//!
//! Hermetic: `FakeHistory` stands in for Yahoo Finance through
//! `AppState::with_history_source`, and counts its calls so the cache's
//! "fetch once" promise is observable.
//!
//! Pinned here:
//!
//! 1. **The line is the same money into the benchmark**, seeded at the chart's
//!    left edge and moved by in-window flows only.
//! 2. **History is cached beside the journal** as hledger `P` lines, never
//!    `include`d, and a covered request does not refetch.
//! 3. **A fetch failure is the benchmark's problem, not the request's**: `200`
//!    with an `error` (or `stale` when a cache answers), never a `5xx`.
//! 4. **The request is validated like the series it overlays**, and sits behind
//!    the token guard.

mod common;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode};
use http_body_util::BodyExt;
use ledgeline::{
    AccessToken, AppState, FetchedPrice, HistoryFeed, Security, YahooError, router_with_security,
    router_with_state,
};
use ledgeline_core::{Dec, parse_journal};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// The fake history source
// ---------------------------------------------------------------------------

struct FakeHistory {
    closes: HashMap<&'static str, Vec<(&'static str, &'static str)>>,
    fail: bool,
    calls: AtomicUsize,
}

impl FakeHistory {
    fn new(
        closes: impl IntoIterator<Item = (&'static str, Vec<(&'static str, &'static str)>)>,
    ) -> Self {
        Self {
            closes: closes.into_iter().collect(),
            fail: false,
            calls: AtomicUsize::new(0),
        }
    }

    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::new([])
        }
    }
}

#[async_trait]
impl HistoryFeed for FakeHistory {
    async fn adjusted_history(
        &self,
        ticker: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<FetchedPrice>, YahooError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(YahooError::Http("fake network failure".to_string()));
        }
        Ok(self
            .closes
            .get(ticker)
            .into_iter()
            .flatten()
            .filter(|(date, _)| *date >= from && *date <= to)
            .map(|(date, close)| FetchedPrice {
                date: (*date).to_string(),
                quantity: Dec::parse(close, '.').expect("fixture close"),
            })
            .collect())
    }
}

/// SPY closes around the fixture's three month-ends and its second buy.
fn spy() -> Vec<(&'static str, &'static str)> {
    vec![
        ("2025-01-02", "95"),
        ("2025-01-31", "100"),
        ("2025-02-28", "110"),
        ("2025-03-03", "120"),
        ("2025-03-31", "132"),
    ]
}

// ---------------------------------------------------------------------------
// The scratch tree
// ---------------------------------------------------------------------------

/// Ten VTI in January, five more on 2025-03-03 for $600. Month-end values:
/// $1,100, $1,150, $1,875.
const JOURNAL: &str = "\
commodity 1.0 VTI

P 2025-01-31 VTI $110.00
P 2025-02-28 VTI $115.00
P 2025-03-31 VTI $125.00

2025-01-02 buy
    assets:broker    10 VTI @ $100.00
    assets:cash

2025-03-03 buy more
    assets:broker    5 VTI @ $120.00
    assets:cash
";

const WINDOW: &str = "asOf=2025-03-31&interval=monthly&count=3";

struct Tree {
    dir: TempDir,
    state: AppState,
    feed: Arc<FakeHistory>,
}

impl Tree {
    fn with(journal: &str, feed: FakeHistory) -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), journal).expect("write journal");
        let feed = Arc::new(feed);
        let state = AppState::from_journal_path(dir.path().join("main.journal"))
            .expect("the scratch journal opens")
            .with_history_source(feed.clone());
        Self { dir, state, feed }
    }

    fn calls(&self) -> usize {
        self.feed.calls.load(Ordering::SeqCst)
    }

    fn cache(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join("benchmarks.prices.journal")).ok()
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("request builds");
        let response = router_with_state(self.state.clone())
            .oneshot(request)
            .await
            .expect("router responds");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, body)
    }
}

fn values(benchmark: &Value) -> Vec<Option<f64>> {
    benchmark["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(|point| point["value"].as_f64())
        .collect()
}

// ---------------------------------------------------------------------------
// The line
// ---------------------------------------------------------------------------

/// Seeded with the portfolio's $1,100 at January's $100 close (11 units), then
/// the March $600 buys 5 units at that day's $120: February 11 × $110, March
/// 16 × $132.
#[tokio::test]
async fn the_line_is_the_same_money_into_the_benchmark() {
    let tree = Tree::with(JOURNAL, FakeHistory::new([("SPY", spy())]));
    let (status, body) = tree
        .get(&format!("/api/holdings/benchmarks?symbols=SPY&{WINDOW}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["base"], "$");
    let spy = &body["benchmarks"][0];
    assert_eq!(spy["symbol"], "SPY");
    assert_eq!(spy["label"], "S&P 500 (SPY)");
    assert_eq!(values(spy), vec![Some(1100.0), Some(1210.0), Some(2112.0)]);
    let dates: Vec<&str> = spy["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["date"].as_str().unwrap())
        .collect();
    assert_eq!(dates, ["2025-01-31", "2025-02-28", "2025-03-31"]);
    assert_eq!(spy["pricedThrough"], "2025-03-31");
    assert_eq!(spy["stale"], false);
    assert_eq!(spy["error"], Value::Null);
}

/// Requested out of order, answered in catalog order; a symbol the feed has no
/// history for is an all-null line with a reason, beside a good one.
#[tokio::test]
async fn several_benchmarks_come_back_in_catalog_order() {
    let tree = Tree::with(JOURNAL, FakeHistory::new([("SPY", spy())]));
    let (status, body) = tree
        .get(&format!(
            "/api/holdings/benchmarks?symbols=GLD,SPY&{WINDOW}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let symbols: Vec<&str> = body["benchmarks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["symbol"].as_str().unwrap())
        .collect();
    assert_eq!(symbols, ["SPY", "GLD"]);
    let gld = &body["benchmarks"][1];
    assert_eq!(values(gld), vec![None, None, None]);
    assert!(gld["error"].as_str().unwrap().contains("GLD"));
}

/// A batched request draws each line exactly as a request for that symbol
/// alone would, and fetches each symbol once.
#[tokio::test]
async fn a_batched_request_matches_its_single_symbol_requests() {
    let feed = || FakeHistory::new([("SPY", spy()), ("QQQ", spy())]);
    let batched = Tree::with(JOURNAL, feed());
    let (_, both) = batched
        .get(&format!(
            "/api/holdings/benchmarks?symbols=SPY,QQQ&{WINDOW}"
        ))
        .await;
    assert_eq!(batched.calls(), 2, "one fetch per symbol, in one request");
    for (index, symbol) in ["SPY", "QQQ"].into_iter().enumerate() {
        let alone = Tree::with(JOURNAL, feed());
        let (_, single) = alone
            .get(&format!(
                "/api/holdings/benchmarks?symbols={symbol}&{WINDOW}"
            ))
            .await;
        assert_eq!(
            both["benchmarks"][index], single["benchmarks"][0],
            "{symbol}"
        );
    }
}

/// An answer with no sessions in it is not "covered": nothing is recorded for
/// the symbol, so the next request asks again instead of waiting for tomorrow.
#[tokio::test]
async fn an_empty_answer_records_no_coverage_and_is_retried() {
    let tree = Tree::with(JOURNAL, FakeHistory::new([("SPY", spy())]));
    let uri = format!("/api/holdings/benchmarks?symbols=GLD&{WINDOW}");
    let (status, body) = tree.get(&uri).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["benchmarks"][0]["error"]
            .as_str()
            .unwrap()
            .contains("Could not fetch GLD")
    );
    assert!(
        !tree.cache().unwrap_or_default().contains("GLD"),
        "no coverage recorded for an empty answer"
    );
    let calls = tree.calls();
    tree.get(&uri).await;
    assert!(tree.calls() > calls, "the next request retries");
}

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

#[tokio::test]
async fn history_is_cached_beside_the_journal_and_not_refetched() {
    let tree = Tree::with(JOURNAL, FakeHistory::new([("SPY", spy())]));
    let journal_before = std::fs::read_to_string(tree.dir.path().join("main.journal")).unwrap();
    let uri = format!("/api/holdings/benchmarks?symbols=SPY&{WINDOW}");

    let (_, first) = tree.get(&uri).await;
    assert_eq!(tree.calls(), 1);
    let cache = tree.cache().expect("the cache file was written");
    assert!(cache.starts_with("; Ledgeline benchmark price cache."));
    assert!(cache.contains("NOT included in your journal"));
    assert!(cache.contains("P 2025-01-31 SPY 100 USD\n"));
    let parsed = parse_journal(&cache, "benchmarks.prices.journal").expect("cache is hledger");
    // Only what the window needs: from ten days before the first point.
    assert_eq!(parsed.prices.len(), 4);

    // The journal is untouched: no include, no edit.
    let journal_after = std::fs::read_to_string(tree.dir.path().join("main.journal")).unwrap();
    assert_eq!(journal_before, journal_after);

    let (_, second) = tree.get(&uri).await;
    assert_eq!(tree.calls(), 1, "a covered window is served from the cache");
    assert_eq!(first, second);
}

/// A read-only session may not write beside the journal: the history is kept
/// in memory — still drawn, still fetched only once — and no file appears.
#[tokio::test]
async fn a_read_only_session_caches_in_memory_only() {
    let tree = Tree::with(JOURNAL, FakeHistory::new([("SPY", spy())]));
    let path = tree.dir.path().join("main.journal");
    let journal = parse_journal(JOURNAL, &path.to_string_lossy()).expect("parses");
    let read_only = Tree {
        state: AppState::from_journal(&journal).with_history_source(tree.feed.clone()),
        ..tree
    };
    let uri = format!("/api/holdings/benchmarks?symbols=SPY&{WINDOW}");

    let (status, first) = read_only.get(&uri).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(
        values(&first["benchmarks"][0]),
        vec![Some(1100.0), Some(1210.0), Some(2112.0)]
    );
    assert!(
        read_only.cache().is_none(),
        "no file written without an editor"
    );

    let (_, second) = read_only.get(&uri).await;
    assert_eq!(read_only.calls(), 1, "cached in memory");
    assert_eq!(first, second);
    assert!(read_only.cache().is_none());
}

/// A cache that answers when the network does not: the line is drawn from it,
/// flagged stale.
#[tokio::test]
async fn a_failed_refresh_serves_the_cache_as_stale() {
    let tree = Tree::with(JOURNAL, FakeHistory::failing());
    std::fs::write(
        tree.dir.path().join("benchmarks.prices.journal"),
        "; ledgeline-benchmark SPY from 2024-12-01 through 2025-02-28\n\
         P 2025-01-31 SPY 100 USD\nP 2025-02-28 SPY 110 USD\n",
    )
    .unwrap();
    let (status, body) = tree
        .get(&format!("/api/holdings/benchmarks?symbols=SPY&{WINDOW}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let spy = &body["benchmarks"][0];
    assert_eq!(spy["stale"], true);
    assert_eq!(spy["error"], Value::Null);
    // March reads February's close: 11 + 600/110 units at $110.
    let march = values(spy)[2].unwrap();
    assert!((march - 1810.0).abs() < 0.01, "{march}");
    assert_eq!(tree.calls(), 1);
}

// ---------------------------------------------------------------------------
// Failure and validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unreachable_feed_is_a_per_benchmark_error_not_a_failed_request() {
    let tree = Tree::with(JOURNAL, FakeHistory::failing());
    let (status, body) = tree
        .get(&format!("/api/holdings/benchmarks?symbols=SPY&{WINDOW}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let spy = &body["benchmarks"][0];
    assert_eq!(values(spy), vec![None, None, None]);
    assert!(
        spy["error"]
            .as_str()
            .unwrap()
            .contains("Could not fetch SPY")
    );
    assert!(tree.cache().is_none(), "nothing fetched, nothing written");
}

#[tokio::test]
async fn holdings_not_valued_in_dollars_cannot_be_compared() {
    let journal = JOURNAL.replace('$', "EUR ");
    let tree = Tree::with(&journal, FakeHistory::new([("SPY", spy())]));
    let (status, body) = tree
        .get(&format!("/api/holdings/benchmarks?symbols=SPY&{WINDOW}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["base"], "EUR");
    assert!(
        body["benchmarks"][0]["error"]
            .as_str()
            .unwrap()
            .contains("US dollars")
    );
    assert_eq!(tree.calls(), 0, "no point fetching what cannot be used");
}

#[tokio::test]
async fn malformed_requests_are_rejected() {
    let tree = Tree::with(JOURNAL, FakeHistory::new([("SPY", spy())]));
    for query in [
        WINDOW.to_string(),
        format!("symbols=&{WINDOW}"),
        format!("symbols=SPY,AAPL&{WINDOW}"),
        "symbols=SPY&interval=fortnightly".to_string(),
        "symbols=SPY&since=2025-01-01&count=3&asOf=2025-03-31".to_string(),
    ] {
        let (status, _) = tree.get(&format!("/api/holdings/benchmarks?{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
    }
    assert_eq!(tree.calls(), 0);
}

/// The route can reach the network and write beside the journal, so it sits
/// behind the token like the price-update routes.
#[tokio::test]
async fn the_benchmarks_route_requires_the_token() {
    const PORT: u16 = 5098;
    let tree = Tree::with(JOURNAL, FakeHistory::new([("SPY", spy())]));
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/holdings/benchmarks?symbols=SPY&{WINDOW}"))
        .header(HeaderName::from_static("host"), "127.0.0.1:5098")
        .body(Body::empty())
        .expect("request builds");
    let response = router_with_security(tree.state.clone(), Security::local(token, PORT))
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(tree.calls(), 0);
}
