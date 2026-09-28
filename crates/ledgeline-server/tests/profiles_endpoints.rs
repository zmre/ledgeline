//! `GET /api/holdings/profiles` — the Holdings pie's "by category" data.
//!
//! Hermetic: `FakeFeed` stands in for Yahoo Finance through
//! `AppState::with_profile_source`, and counts what it is asked so the cache's
//! behaviour is observable.
//!
//! What this file pins, most expensive regression first:
//!
//! 1. **A commodity tag beats Yahoo**, for exactly the dimension it names.
//! 2. **Yahoo failing is never an error response** — the answer degrades to
//!    tags-only and says so in `yahoo`.
//! 3. **The cache is used**: a second visit asks Yahoo nothing, an expired
//!    entry is refetched, a stale one is served when the refetch fails.
//! 4. **Only the journal's own commodities are ever looked up.**
//! 5. **The cache file sits beside the journal and never touches it.**

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{
    AccessToken, AppState, AssetMix, ProfileError, ProfileFeed, Security, YahooProfile,
    router_with_security, router_with_state,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tower::ServiceExt;

const CACHE_FILE: &str = "commodity-profiles.json";

// ---------------------------------------------------------------------------
// The fake source
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Answer {
    Found(YahooProfile),
    NotFound,
    Fail,
}

struct FakeFeed {
    answers: HashMap<String, Answer>,
    asked: Mutex<Vec<String>>,
    calls: AtomicUsize,
}

impl FakeFeed {
    fn new(answers: impl IntoIterator<Item = (&'static str, Answer)>) -> Arc<Self> {
        Arc::new(Self {
            answers: answers
                .into_iter()
                .map(|(ticker, answer)| (ticker.to_string(), answer))
                .collect(),
            asked: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        })
    }

    fn asked(&self) -> Vec<String> {
        let mut asked = self.asked.lock().unwrap().clone();
        asked.sort();
        asked
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ProfileFeed for FakeFeed {
    async fn profile(&self, ticker: &str) -> Result<Option<YahooProfile>, ProfileError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.asked.lock().unwrap().push(ticker.to_string());
        match self.answers.get(ticker) {
            Some(Answer::Found(profile)) => Ok(Some(profile.clone())),
            Some(Answer::NotFound) | None => Ok(None),
            Some(Answer::Fail) => Err(ProfileError::RateLimited),
        }
    }
}

fn apple() -> YahooProfile {
    YahooProfile {
        quote_type: Some("EQUITY".to_string()),
        sector: Some("Technology".to_string()),
        industry: Some("Consumer Electronics".to_string()),
        ..YahooProfile::default()
    }
}

fn balanced() -> YahooProfile {
    YahooProfile {
        quote_type: Some("MUTUALFUND".to_string()),
        category: Some("Moderate Allocation".to_string()),
        asset_mix: Some(AssetMix {
            equity: 0.6,
            bond: 0.4,
            cash: 0.0,
            other: 0.0,
        }),
        sector_weights: vec![("Technology".to_string(), 0.5), ("Energy".to_string(), 0.5)],
        risk_rating: Some(2),
        ..YahooProfile::default()
    }
}

// ---------------------------------------------------------------------------
// The scratch tree
// ---------------------------------------------------------------------------

const JOURNAL: &str = "\
commodity 1.0000 AAPL
commodity 1.0000 BAL   ; yahoo: VBAL-X, sector: Diversified
commodity 1.0000 HOUSEFUND  ; assetclass: real estate, sector: Real Estate, industry: REIT, type: fund, category: Private, risk: High

2026-01-05 buy
    assets:broker    10 AAPL @ $200.00
    assets:broker    10 BAL @ $20.00
    assets:broker    1 HOUSEFUND @ $1000.00
    assets:cash
";

struct Tree {
    dir: TempDir,
}

impl Tree {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), JOURNAL).expect("write journal");
        Self { dir }
    }

    /// Fresh state over the same directory — a new server session, empty
    /// in-memory cache, same cache file.
    fn state(&self, feed: Arc<FakeFeed>) -> AppState {
        AppState::from_journal_path(self.dir.path().join("main.journal"))
            .expect("the scratch journal opens")
            .with_profile_source(feed)
    }

    fn cache(&self) -> Option<Value> {
        std::fs::read_to_string(self.dir.path().join(CACHE_FILE))
            .ok()
            .map(|text| serde_json::from_str(&text).expect("cache is JSON"))
    }
}

async fn get(state: AppState, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    let response = router_with_state(state)
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
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

fn profile<'a>(body: &'a Value, symbol: &str) -> &'a Value {
    body["profiles"]
        .as_array()
        .expect("profiles is an array")
        .iter()
        .find(|p| p["symbol"] == json!(symbol))
        .unwrap_or_else(|| panic!("no profile for {symbol} in {body}"))
}

fn labels(weights: &Value) -> Vec<(String, f64)> {
    weights
        .as_array()
        .expect("weights array")
        .iter()
        .map(|w| {
            (
                w["label"].as_str().expect("label").to_string(),
                (w["weight"].as_f64().expect("weight") * 1000.0).round() / 1000.0,
            )
        })
        .collect()
}

fn l(label: &str, weight: f64) -> (String, f64) {
    (label.to_string(), weight)
}

const ALL: &str = "/api/holdings/profiles?symbols=AAPL,BAL,HOUSEFUND";

// ---------------------------------------------------------------------------
// Merging
// ---------------------------------------------------------------------------

#[tokio::test]
async fn yahoo_fills_what_tags_leave_open_and_a_tag_wins_its_dimension() {
    let tree = Tree::new();
    let feed = FakeFeed::new([
        ("AAPL", Answer::Found(apple())),
        ("VBAL-X", Answer::Found(balanced())),
    ]);
    let (status, body) = get(tree.state(feed.clone()), ALL).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["yahoo"], json!("ok"));

    let aapl = profile(&body, "AAPL");
    assert_eq!(aapl["source"], json!("yahoo"));
    assert_eq!(
        labels(&aapl["breakdown"]["sector"]),
        vec![l("Technology", 1.0)]
    );
    assert_eq!(
        labels(&aapl["breakdown"]["assetClass"]),
        vec![l("Equity", 1.0)]
    );
    assert!(aapl["fetchedAt"].is_string());

    // Looked up under its `yahoo:` tag; its `sector:` tag replaces Yahoo's
    // sector split, and nothing else.
    let bal = profile(&body, "BAL");
    assert_eq!(bal["yahooTicker"], json!("VBAL-X"));
    assert_eq!(bal["source"], json!("mixed"));
    assert_eq!(
        labels(&bal["breakdown"]["sector"]),
        vec![l("Diversified", 1.0)]
    );
    assert_eq!(
        labels(&bal["breakdown"]["assetClass"]),
        vec![l("Equity", 0.6), l("Bonds", 0.4)]
    );
    assert_eq!(
        labels(&bal["breakdown"]["risk"]),
        vec![l("Below average", 1.0)]
    );

    // Every dimension tagged: Yahoo is never asked about it.
    let house = profile(&body, "HOUSEFUND");
    assert_eq!(house["source"], json!("tags"));
    assert!(house.get("fetchedAt").is_none());
    assert_eq!(
        labels(&house["breakdown"]["securityType"]),
        vec![l("Mutual fund", 1.0)]
    );
    assert_eq!(
        labels(&house["breakdown"]["assetClass"]),
        vec![l("real estate", 1.0)],
        "kept as written"
    );
    assert_eq!(feed.asked(), vec!["AAPL".to_string(), "VBAL-X".to_string()]);
}

#[tokio::test]
async fn a_symbol_the_journal_does_not_know_is_omitted_and_never_looked_up() {
    let tree = Tree::new();
    let feed = FakeFeed::new([("AAPL", Answer::Found(apple()))]);
    let (status, body) = get(
        tree.state(feed.clone()),
        "/api/holdings/profiles?symbols=AAPL,NOTMINE",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(feed.asked(), vec!["AAPL".to_string()]);
}

#[tokio::test]
async fn no_symbols_is_an_empty_answer() {
    let tree = Tree::new();
    let feed = FakeFeed::new([]);
    let (status, body) = get(tree.state(feed.clone()), "/api/holdings/profiles").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"yahoo": "ok", "profiles": []}));
    assert_eq!(feed.calls(), 0);
}

// ---------------------------------------------------------------------------
// Degrading
// ---------------------------------------------------------------------------

#[tokio::test]
async fn yahoo_down_degrades_to_tags_only_and_says_so() {
    let tree = Tree::new();
    let feed = FakeFeed::new([("AAPL", Answer::Fail), ("VBAL-X", Answer::Fail)]);
    let (status, body) = get(tree.state(feed), ALL).await;
    assert_eq!(status, StatusCode::OK, "never an error response: {body}");
    assert_eq!(body["yahoo"], json!("unavailable"));
    assert_eq!(profile(&body, "AAPL")["source"], json!("none"));
    let bal = profile(&body, "BAL");
    assert_eq!(bal["source"], json!("tags"));
    assert_eq!(
        labels(&bal["breakdown"]["sector"]),
        vec![l("Diversified", 1.0)]
    );
    assert!(tree.cache().is_none(), "a failure caches nothing");
}

#[tokio::test]
async fn one_failure_among_successes_is_partial() {
    let tree = Tree::new();
    let feed = FakeFeed::new([("AAPL", Answer::Found(apple())), ("VBAL-X", Answer::Fail)]);
    let (_, body) = get(tree.state(feed), ALL).await;
    assert_eq!(body["yahoo"], json!("partial"));
}

#[tokio::test]
async fn not_found_is_a_normal_answer_not_a_failure() {
    let tree = Tree::new();
    let feed = FakeFeed::new([("AAPL", Answer::NotFound), ("VBAL-X", Answer::NotFound)]);
    let (_, body) = get(tree.state(feed), ALL).await;
    assert_eq!(body["yahoo"], json!("ok"));
    let cache = tree.cache().expect("negative answers are cached");
    assert_eq!(cache["entries"]["AAPL"]["profile"], Value::Null);
}

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_second_session_reads_the_cache_file_instead_of_yahoo() {
    let tree = Tree::new();
    let first = FakeFeed::new([
        ("AAPL", Answer::Found(apple())),
        ("VBAL-X", Answer::Found(balanced())),
    ]);
    let (_, before) = get(tree.state(first.clone()), ALL).await;
    assert_eq!(first.calls(), 2);

    let cache = tree
        .cache()
        .expect("the cache file is written beside the journal");
    assert_eq!(cache["version"], json!(1));
    assert!(cache["entries"]["VBAL-X"]["profile"]["assetMix"].is_object());

    // Yahoo is now down, and it does not matter.
    let second = FakeFeed::new([("AAPL", Answer::Fail), ("VBAL-X", Answer::Fail)]);
    let (_, after) = get(tree.state(second.clone()), ALL).await;
    assert_eq!(second.calls(), 0);
    assert_eq!(before["profiles"], after["profiles"]);
    assert_eq!(after["yahoo"], json!("ok"));
}

#[tokio::test]
async fn the_same_session_does_not_ask_twice() {
    let tree = Tree::new();
    let feed = FakeFeed::new([("AAPL", Answer::Found(apple()))]);
    let state = tree.state(feed.clone());
    get(state.clone(), "/api/holdings/profiles?symbols=AAPL").await;
    get(state, "/api/holdings/profiles?symbols=AAPL").await;
    assert_eq!(feed.calls(), 1);
}

fn write_cache(tree: &Tree, fetched_at: u64) {
    let cache = json!({
        "version": 1,
        "entries": {
            "AAPL": {"fetchedAt": fetched_at, "profile": {"quoteType": "EQUITY", "sector": "Old Sector"}}
        }
    });
    std::fs::write(
        tree.dir.path().join(CACHE_FILE),
        serde_json::to_vec(&cache).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn an_expired_entry_is_refetched_and_rewritten() {
    let tree = Tree::new();
    write_cache(&tree, 1_000); // 1970: long expired
    let feed = FakeFeed::new([("AAPL", Answer::Found(apple()))]);
    let (_, body) = get(
        tree.state(feed.clone()),
        "/api/holdings/profiles?symbols=AAPL",
    )
    .await;
    assert_eq!(feed.calls(), 1);
    assert_eq!(
        labels(&profile(&body, "AAPL")["breakdown"]["sector"]),
        vec![l("Technology", 1.0)]
    );
    let cache = tree.cache().unwrap();
    assert_eq!(
        cache["entries"]["AAPL"]["profile"]["sector"],
        json!("Technology")
    );
}

#[tokio::test]
async fn an_expired_entry_is_still_served_when_the_refetch_fails() {
    let tree = Tree::new();
    write_cache(&tree, 1_000);
    let feed = FakeFeed::new([("AAPL", Answer::Fail)]);
    let (_, body) = get(tree.state(feed), "/api/holdings/profiles?symbols=AAPL").await;
    assert_eq!(body["yahoo"], json!("ok"), "stale data is data");
    let aapl = profile(&body, "AAPL");
    assert_eq!(
        labels(&aapl["breakdown"]["sector"]),
        vec![l("Old Sector", 1.0)]
    );
    assert_eq!(aapl["fetchedAt"], json!("1970-01-01"));
}

#[tokio::test]
async fn a_corrupt_cache_file_is_replaced_not_fatal() {
    let tree = Tree::new();
    std::fs::write(tree.dir.path().join(CACHE_FILE), "{ nope").unwrap();
    let feed = FakeFeed::new([("AAPL", Answer::Found(apple()))]);
    let (status, _) = get(tree.state(feed), "/api/holdings/profiles?symbols=AAPL").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tree.cache().unwrap()["version"], json!(1));
}

#[tokio::test]
async fn the_journal_itself_is_never_touched() {
    let tree = Tree::new();
    let feed = FakeFeed::new([("AAPL", Answer::Found(apple()))]);
    get(tree.state(feed), ALL).await;
    let journal = std::fs::read_to_string(tree.dir.path().join("main.journal")).unwrap();
    assert_eq!(journal, JOURNAL);
    assert!(!journal.contains(CACHE_FILE));
}

#[tokio::test]
async fn a_read_only_session_caches_in_memory_only() {
    let tree = Tree::new();
    let path = tree.dir.path().join("main.journal");
    let journal =
        ledgeline_core::parse::parse_journal(JOURNAL, &path.to_string_lossy()).expect("parses");
    let feed = FakeFeed::new([("AAPL", Answer::Found(apple()))]);
    let state = AppState::from_journal(&journal).with_profile_source(feed.clone());
    get(state.clone(), "/api/holdings/profiles?symbols=AAPL").await;
    get(state, "/api/holdings/profiles?symbols=AAPL").await;
    assert_eq!(feed.calls(), 1, "cached in memory");
    assert!(tree.cache().is_none(), "no file written without an editor");
}

// ---------------------------------------------------------------------------
// Refusals and the guard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_absurdly_long_symbol_is_a_400() {
    let tree = Tree::new();
    let long = "X".repeat(200);
    let (status, _) = get(
        tree.state(FakeFeed::new([])),
        &format!("/api/holdings/profiles?symbols={long}"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// It makes outbound requests and writes a file beside the journal, so it
/// sits above the token guard like every other route.
#[tokio::test]
async fn the_route_requires_the_token() {
    const PORT: u16 = 5098;
    let tree = Tree::new();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");
    for auth in [None, Some("Bearer wrong-token")] {
        let mut builder = Request::builder()
            .method("GET")
            .uri(ALL)
            .header(HeaderName::from_static("host"), "127.0.0.1:5098");
        if let Some(value) = auth {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        let response = router_with_security(
            tree.state(FakeFeed::new([])),
            Security::local(token.clone(), PORT),
        )
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{auth:?}");
    }
}
