//! Plan 47 over HTTP: the holdings filter (`symbols=`, `rows=`), the series
//! `anchor`, and intraday quotes (`POST /api/prices/intraday`, `live=1`).
//!
//! Hermetic like `prices_endpoints.rs`: a fake [`PriceFeed`] stands in for
//! Yahoo and counts every request, so a test can prove a refresh asked for
//! nothing. The live tests date the journal's "today" from the system clock,
//! because `live=1` only ever applies when `asOf` IS today.

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{
    AccessToken, AppDirs, AppState, FetchedPrice, MarketQuote, PriceFeed, Security, YahooError,
    router_with_security, router_with_state,
};
use ledgeline_core::Dec;
use ledgeline_core::reports::periods::iso_from_days;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// The fake feed
// ---------------------------------------------------------------------------

/// What [`FakeFeed`] answers for one ticker.
#[derive(Clone)]
enum Answer {
    /// A quote at this price (cents), dated today.
    Quote(i128),
    /// Yahoo said "slow down".
    RateLimited,
}

struct FakeFeed {
    answers: HashMap<&'static str, Answer>,
    /// Quote requests (`market_quote`) answered.
    quote_requests: Arc<AtomicUsize>,
}

impl FakeFeed {
    fn new(answers: impl IntoIterator<Item = (&'static str, Answer)>) -> Self {
        Self {
            answers: answers.into_iter().collect(),
            quote_requests: Arc::default(),
        }
    }

    fn quote(&self, ticker: &str) -> Result<Option<MarketQuote>, YahooError> {
        match self.answers.get(ticker) {
            Some(Answer::Quote(cents)) => Ok(Some(MarketQuote {
                price: Dec::new(*cents, 2),
                time: now(),
                date: today(),
                currency: Some("USD".to_string()),
            })),
            Some(Answer::RateLimited) => Err(YahooError::RateLimited("fake 429".to_string())),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl PriceFeed for FakeFeed {
    async fn latest_close(
        &self,
        _ticker: &str,
        _as_of: &str,
    ) -> Result<Option<FetchedPrice>, YahooError> {
        Ok(None)
    }

    async fn daily_closes(
        &self,
        _ticker: &str,
        _from: &str,
    ) -> Result<Vec<FetchedPrice>, YahooError> {
        Ok(Vec::new())
    }

    /// The Update prices button's fetch: no close worth recording, but the
    /// reply's `meta` carries a quote — which must fill the cache.
    async fn latest_close_and_quote(
        &self,
        ticker: &str,
        _as_of: &str,
    ) -> Result<(Option<FetchedPrice>, Option<MarketQuote>), YahooError> {
        Ok((None, self.quote(ticker)?))
    }

    async fn market_quote(&self, ticker: &str) -> Result<Option<MarketQuote>, YahooError> {
        self.quote_requests.fetch_add(1, Ordering::SeqCst);
        self.quote(ticker)
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0))
}

/// Today in UTC, the server's own notion of it.
fn today() -> String {
    iso_from_days(now().div_euclid(86_400))
}

// ---------------------------------------------------------------------------
// The scratch tree
// ---------------------------------------------------------------------------

/// VTI 10 @ $200 and AAPL 4 @ $150, priced in `prices.journal`, plus a house
/// for the Other tab. Every date is long past, so "today" is whenever the
/// test runs.
const MAIN: &str = "\
account assets:broker  ; type: A
account assets:cash  ; type: C
account assets:property:house  ; type: A
account equity:opening  ; type: E

commodity 1.0000 VTI
commodity 1.0000 AAPL

include prices.journal

2026-01-05 buy vti
    assets:broker    10 VTI @ $200.00
    assets:cash

2026-03-02 buy apple
    assets:broker    4 AAPL @ $150.00
    assets:cash

2026-01-01 opening
    assets:property:house    $400,000.00
    equity:opening
";

const PRICES: &str = "\
P 2026-01-31 VTI $210.00
P 2026-03-31 VTI $230.00
P 2026-03-31 AAPL $160.00
";

struct Tree {
    dir: TempDir,
    state: AppState,
    quote_requests: Arc<AtomicUsize>,
}

impl Tree {
    fn new(feed: FakeFeed) -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), MAIN).expect("write journal");
        std::fs::write(dir.path().join("prices.journal"), PRICES).expect("write prices");
        let quote_requests = Arc::clone(&feed.quote_requests);
        let state = AppState::from_journal_path(dir.path().join("main.journal"))
            .expect("the scratch journal opens")
            .with_app_dirs(AppDirs::under(dir.path().join("app")))
            .with_price_source(Arc::new(feed));
        Self {
            dir,
            state,
            quote_requests,
        }
    }

    fn quotes() -> Self {
        Self::new(FakeFeed::new([
            ("VTI", Answer::Quote(25_000)),
            ("AAPL", Answer::Quote(17_500)),
        ]))
    }

    fn requests(&self) -> usize {
        self.quote_requests.load(Ordering::SeqCst)
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Value) {
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

    async fn get(&self, uri: &str) -> Value {
        let request = Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request builds");
        let (status, body) = self.send(request).await;
        assert_eq!(status, StatusCode::OK, "GET {uri}: {body}");
        body
    }

    async fn post(&self, uri: &str, body: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request builds");
        self.send(request).await
    }

    async fn refresh(&self, body: &str) -> Value {
        let (status, body) = self.post("/api/prices/intraday", body).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// Every file under the tree (app dir included) and its bytes.
    fn files(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    let name = path.strip_prefix(root).unwrap().display().to_string();
                    out.insert(name, std::fs::read(&path).expect("read file"));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.dir.path(), self.dir.path(), &mut out);
        out
    }
}

/// A `{mantissa, places}` wire decimal as a float, for readable assertions.
fn dec(value: &Value) -> f64 {
    let mantissa: f64 = value["mantissa"]
        .as_str()
        .unwrap_or_else(|| panic!("not a wire decimal: {value}"))
        .parse()
        .expect("mantissa");
    let places = i32::try_from(value["places"].as_u64().expect("places")).expect("places");
    mantissa / 10f64.powi(places)
}

fn symbols(report: &Value) -> Vec<&str> {
    report["holdings"]
        .as_array()
        .expect("holdings")
        .iter()
        .map(|row| row["symbol"].as_str().expect("symbol"))
        .collect()
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[tokio::test]
async fn symbols_narrow_the_stocks_report_and_series() {
    let tree = Tree::quotes();
    let report = tree.get("/api/holdings?asOf=2026-04-30&symbols=AAPL").await;
    assert_eq!(symbols(&report), ["AAPL"]);
    assert_eq!(dec(&report["totals"]["marketValue"]), 640.0);
    assert_eq!(report["liveAt"], Value::Null);

    let all = tree.get("/api/holdings?asOf=2026-04-30&symbols=").await;
    assert_eq!(symbols(&all), ["VTI", "AAPL"], "an empty list is no filter");

    let series = tree
        .get("/api/holdings/series?asOf=2026-04-30&interval=monthly&count=2&symbols=VTI")
        .await;
    let values: Vec<f64> = series["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| dec(&p["marketValue"]))
        .collect();
    assert_eq!(values, [2300.0, 2300.0]);
    assert_eq!(series["anchor"], Value::Null, "no anchor asked for");
    assert_eq!(series["liveAt"], Value::Null);
}

#[tokio::test]
async fn rows_narrow_the_other_tab() {
    let tree = Tree::quotes();
    let report = tree
        .get("/api/holdings/other?asOf=2026-04-30&rows=assets:nothing")
        .await;
    assert_eq!(report["holdings"], json!([]));
    let report = tree
        .get("/api/holdings/other?asOf=2026-04-30&rows=assets:property:house")
        .await;
    assert_eq!(report["holdings"][0]["account"], "assets:property:house");
}

// ---------------------------------------------------------------------------
// Anchor
// ---------------------------------------------------------------------------

/// A one-week window in April anchors on the year behind it: the leading
/// zero months before the January buy are skipped, so the range runs from
/// January's $2,100 up to April's $2,940 — not from $0.
#[tokio::test]
async fn a_short_window_is_anchored_on_the_year_behind_it() {
    let tree = Tree::quotes();
    let series = tree
        .get("/api/holdings/series?asOf=2026-04-30&since=2026-04-23&interval=daily&anchor=1yr")
        .await;
    assert_eq!(dec(&series["anchor"]["min"]), 2100.0);
    assert_eq!(dec(&series["anchor"]["max"]), 2940.0);

    let only_aapl = tree
        .get("/api/holdings/series?asOf=2026-04-30&since=2026-04-23&interval=daily&anchor=all&symbols=AAPL")
        .await;
    assert_eq!(dec(&only_aapl["anchor"]["min"]), 600.0);
    assert_eq!(dec(&only_aapl["anchor"]["max"]), 640.0);

    let other = tree
        .get("/api/holdings/other/series?asOf=2026-04-30&interval=monthly&count=1&anchor=5yr")
        .await;
    assert_eq!(dec(&other["anchor"]["min"]), 400_000.0);
    assert_eq!(dec(&other["anchor"]["max"]), 400_000.0);
}

#[tokio::test]
async fn nothing_held_has_no_anchor_and_a_bad_anchor_is_refused() {
    let tree = Tree::quotes();
    let series = tree
        .get("/api/holdings/series?asOf=2025-06-30&interval=monthly&count=3&anchor=all")
        .await;
    assert_eq!(series["anchor"], Value::Null);
    let request = Request::builder()
        .uri("/api/holdings/series?asOf=2026-04-30&anchor=2yr")
        .body(Body::empty())
        .unwrap();
    let (status, _) = tree.send(request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Flows and returns (amendment A1)
// ---------------------------------------------------------------------------

fn column<T>(series: &Value, read: impl Fn(&Value) -> T) -> Vec<T> {
    series["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(read)
        .collect()
}

/// January–April month-ends: VTI $2,100, $2,100, then AAPL's $600 buy on
/// 2026-03-02 lifts March to $2,940 — of which $240 is gain. Modified Dietz:
/// 240 / (2,100 + 600 / 2) = +10 %, and April holds it.
#[tokio::test]
async fn series_points_carry_their_buckets_flow_and_the_chained_return() {
    let tree = Tree::quotes();
    let series = tree
        .get("/api/holdings/series?asOf=2026-04-30&interval=monthly&count=4")
        .await;
    let point = &series["points"][0];
    let mut keys: Vec<&str> = point
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "basis",
            "bucket",
            "date",
            "flow",
            "label",
            "marketValue",
            "returnPct"
        ]
    );
    // The buy is money IN: positive, at its cost.
    assert_eq!(column(&series, |p| dec(&p["flow"])), [0.0, 0.0, 600.0, 0.0]);
    let returns = column(&series, |p| p["returnPct"].as_f64().expect("a number"));
    assert!(returns[0].abs() < 1e-12 && returns[1].abs() < 1e-12);
    assert!((returns[2] - 0.1).abs() < 1e-12, "{returns:?}");
    assert!((returns[3] - 0.1).abs() < 1e-12, "{returns:?}");
    assert_eq!(series["returnAnchor"], Value::Null, "no anchor asked for");

    // Nothing held yet: no return to report.
    let empty = tree
        .get("/api/holdings/series?asOf=2025-12-31&interval=monthly&count=2")
        .await;
    assert!(
        column(&empty, |p| p["returnPct"].is_null())
            .into_iter()
            .all(|null| null)
    );
    assert_eq!(column(&empty, |p| dec(&p["flow"])), [0.0, 0.0]);
}

/// The last April week is flat, but the year behind it is not: rebased on
/// March's index (1.05 × 1.10), January's 1.05 sits 1/1.1 − 1 ≈ −9.1 % below
/// the window's start. The January buy is the bucket that opens from nothing:
/// $2,000 in, $2,100 at month end = +5 %.
#[tokio::test]
async fn the_return_anchor_spans_the_horizon_rebased_to_the_window() {
    let tree = Tree::quotes();
    let series = tree
        .get("/api/holdings/series?asOf=2026-04-30&since=2026-04-23&interval=daily&anchor=1yr")
        .await;
    assert!(
        column(&series, |p| p["returnPct"].as_f64().expect("a number"))
            .iter()
            .all(|r| r.abs() < 1e-12)
    );
    let min = series["returnAnchor"]["min"].as_f64().expect("min");
    let max = series["returnAnchor"]["max"].as_f64().expect("max");
    assert!((min - (1.0 / 1.1 - 1.0)).abs() < 1e-9, "{min}");
    assert!(max.abs() < 1e-12, "{max}");

    // The Other tab carries the same keys. An opening balance is no flow, so
    // the house's return stays flat at zero.
    let other = tree
        .get("/api/holdings/other/series?asOf=2026-04-30&interval=monthly&count=2&anchor=5yr")
        .await;
    assert_eq!(column(&other, |p| dec(&p["flow"])), [0.0, 0.0]);
    assert_eq!(other["returnAnchor"], json!({"min": 0.0, "max": 0.0}));

    // Nothing ever held: no return anchor either.
    let none = tree
        .get("/api/holdings/series?asOf=2025-06-30&interval=monthly&count=3&anchor=all")
        .await;
    assert_eq!(none["returnAnchor"], Value::Null);
}

// ---------------------------------------------------------------------------
// Intraday
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_refresh_fetches_every_held_symbol_once_and_then_serves_the_cache() {
    let tree = Tree::quotes();
    let body = tree.refresh("{}").await;
    assert_eq!(tree.requests(), 2);
    assert_eq!(body["rateLimited"], false);
    assert_eq!(body["skipped"], json!([]));
    let quotes = body["quotes"].as_array().expect("quotes");
    assert_eq!(quotes.len(), 2);
    let vti = quotes.iter().find(|q| q["symbol"] == "VTI").expect("VTI");
    assert_eq!(dec(&vti["price"]), 250.0);
    assert_eq!(vti["currency"], "USD");
    assert!(vti["time"].as_i64().is_some_and(|time| time > 0));

    // Inside fifteen minutes: nothing is asked again, but the quotes are
    // still answered from the cache.
    let again = tree.refresh(r#"{"symbols":["VTI","AAPL"]}"#).await;
    assert_eq!(tree.requests(), 2, "fresh: no request");
    assert_eq!(again["skipped"], json!(["VTI", "AAPL"]));
    assert_eq!(again["quotes"].as_array().map(Vec::len), Some(2));

    // An empty body is `{}`.
    let (status, _) = tree.post("/api/prices/intraday", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tree.requests(), 2);
}

#[tokio::test]
async fn a_rate_limit_stops_the_round_and_backs_off() {
    let tree = Tree::new(FakeFeed::new([
        ("VTI", Answer::RateLimited),
        ("AAPL", Answer::RateLimited),
    ]));
    let body = tree.refresh("{}").await;
    let asked = tree.requests();
    assert!((1..=2).contains(&asked), "stopped at the limit: {asked}");
    assert_eq!(body["rateLimited"], true);
    assert_eq!(body["quotes"], json!([]));

    // A NEW symbol, never asked about, is still not asked: the backoff is
    // global, not per symbol.
    let body = tree.refresh(r#"{"symbols":["MSFT"]}"#).await;
    assert_eq!(tree.requests(), asked, "backing off: no request");
    assert_eq!(body["rateLimited"], true);
    assert_eq!(body["skipped"], json!(["MSFT"]));
}

#[tokio::test]
async fn a_close_fetch_fills_the_cache_so_the_refresh_asks_nothing() {
    let tree = Tree::quotes();
    let (status, body) = tree
        .post("/api/prices/update", r#"{"journalId":"prices.journal"}"#)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body = tree.refresh("{}").await;
    assert_eq!(
        tree.requests(),
        0,
        "the update's replies already carried quotes"
    );
    assert_eq!(body["skipped"], json!(["VTI", "AAPL"]));
    assert_eq!(body["quotes"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn live_values_today_with_the_cached_quotes_and_writes_nothing() {
    let tree = Tree::quotes();
    let today = today();
    let before = tree.files();

    // Nothing cached yet: `live=1` is the plain report.
    let plain = tree
        .get(&format!("/api/holdings?asOf={today}&live=1"))
        .await;
    assert_eq!(dec(&plain["totals"]["marketValue"]), 2300.0 + 640.0);
    assert_eq!(plain["liveAt"], Value::Null);

    tree.refresh("{}").await;
    let live = tree
        .get(&format!("/api/holdings?asOf={today}&live=1"))
        .await;
    assert_eq!(dec(&live["totals"]["marketValue"]), 2500.0 + 700.0);
    let live_at = live["liveAt"].as_i64().expect("liveAt");
    assert!((now() - live_at).abs() < 600, "the quote's own time");
    let vti = &live["holdings"][0];
    assert_eq!(vti["symbol"], "VTI");
    assert_eq!(dec(&vti["price"]["qty"]), 250.0);
    assert_eq!(vti["price"]["date"], today.as_str());

    // Without `live`, or on any other date, the journal's prices alone.
    let off = tree.get(&format!("/api/holdings?asOf={today}")).await;
    assert_eq!(dec(&off["totals"]["marketValue"]), 2940.0);
    assert_eq!(off["liveAt"], Value::Null);
    let past = tree.get("/api/holdings?asOf=2026-04-30&live=1").await;
    assert_eq!(dec(&past["totals"]["marketValue"]), 2940.0);
    assert_eq!(past["liveAt"], Value::Null);

    // The series: only today's point moves.
    let series = tree
        .get(&format!(
            "/api/holdings/series?asOf={today}&interval=daily&count=2&live=1&symbols=VTI"
        ))
        .await;
    let values: Vec<f64> = series["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| dec(&p["marketValue"]))
        .collect();
    assert_eq!(values, [2300.0, 2500.0]);
    assert_eq!(series["liveAt"].as_i64(), Some(live_at));

    // The Other tab holds no quoted symbol: nothing live about it.
    let other = tree
        .get(&format!("/api/holdings/other?asOf={today}&live=1"))
        .await;
    assert_eq!(other["liveAt"], Value::Null);
    let other_series = tree
        .get(&format!(
            "/api/holdings/other/series?asOf={today}&interval=daily&count=1&live=1"
        ))
        .await;
    assert_eq!(other_series["liveAt"], Value::Null);

    assert_eq!(tree.files(), before, "no quote ever reaches a file");
}

#[tokio::test]
async fn the_intraday_route_requires_the_token() {
    const PORT: u16 = 5099;
    let tree = Tree::quotes();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");
    for auth in [None, Some("Bearer wrong-token")] {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/prices/intraday")
            .header(HeaderName::from_static("host"), "127.0.0.1:5099")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(value) = auth {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        let response =
            router_with_security(tree.state.clone(), Security::local(token.clone(), PORT))
                .oneshot(builder.body(Body::from("{}")).unwrap())
                .await
                .expect("router responds");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "auth {auth:?}");
    }
    assert_eq!(tree.requests(), 0);
}
