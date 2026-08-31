//! The `/api/prices/{status,file,update}` HTTP surface — the Holdings tab's
//! "Update prices" button.
//!
//! Hermetic like `budget_endpoints.rs`: no real network call ever happens here.
//! `FakeFeed` stands in for Yahoo Finance, injected through
//! `AppState::with_price_source` — the one seam this route family adds on top
//! of the write-path pattern `budget_endpoints.rs` already pins.
//!
//! The properties this file exists to pin, in order of how much a regression
//! would cost:
//!
//! 1. **A fetched price lands as a `P` line in the journal's own style**, and
//!    nothing else in the file moves.
//! 2. **A price already on record for the fetched date is never duplicated.**
//! 3. **Creating `prices.journal` never overwrites anything**, same as budget's
//!    equivalent file.
//! 4. **The token guard covers all three routes**, two of which write to disk.

mod common;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{
    AccessToken, AppState, FetchedPrice, PriceFeed, Security, YahooError, router_with_security,
    router_with_state,
};
use ledgeline_core::Dec;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// The fake quote source
// ---------------------------------------------------------------------------

/// One canned answer [`FakeFeed`] gives for a ticker.
#[derive(Clone)]
enum FakeQuote {
    /// A usable close: `(date, quantity)`.
    Price(&'static str, Dec),
    /// Fetched fine, nothing usable — `Ok(None)`.
    NotFound,
    /// The request itself failed.
    Error,
}

/// A [`PriceFeed`] that never touches the network: every ticker's answer is
/// decided up front by the test.
struct FakeFeed {
    quotes: HashMap<&'static str, FakeQuote>,
}

impl FakeFeed {
    fn new(quotes: impl IntoIterator<Item = (&'static str, FakeQuote)>) -> Self {
        Self {
            quotes: quotes.into_iter().collect(),
        }
    }
}

#[async_trait]
impl PriceFeed for FakeFeed {
    async fn latest_close(
        &self,
        ticker: &str,
        _as_of: &str,
    ) -> Result<Option<FetchedPrice>, YahooError> {
        match self.quotes.get(ticker) {
            Some(FakeQuote::Price(date, quantity)) => Ok(Some(FetchedPrice {
                date: (*date).to_string(),
                quantity: *quantity,
            })),
            Some(FakeQuote::NotFound) | None => Ok(None),
            Some(FakeQuote::Error) => Err(YahooError::Http("fake network failure".to_string())),
        }
    }
}

fn dec(mantissa: i128, places: u32) -> Dec {
    Dec { mantissa, places }
}

// ---------------------------------------------------------------------------
// The scratch tree
// ---------------------------------------------------------------------------

/// A held stock with no existing price directive anywhere in the journal, so
/// `PriceDb::base_commodity()` has nothing to key off and the engine falls
/// back to `$` — matching what a brand new journal looks like before its
/// first price update.
const JOURNAL: &str = "\
commodity 1.0000 AAPL

2026-01-05 buy apple
    assets:broker    10 AAPL @ $200.00
    assets:cash
";

struct Tree {
    dir: TempDir,
    state: AppState,
}

impl Tree {
    fn with(text: &str, feed: FakeFeed) -> Self {
        Self::with_files(text, &[], feed)
    }

    /// `main.journal` plus any `include`d files it names, all written before the
    /// first parse — the only way a test can present a journal whose prices
    /// already live somewhere other than the root.
    fn with_files(text: &str, extra: &[(&str, &str)], feed: FakeFeed) -> Self {
        let dir = TempDir::new().expect("temp dir");
        for (name, body) in extra {
            std::fs::write(dir.path().join(name), body).expect("write included journal");
        }
        std::fs::write(dir.path().join("main.journal"), text).expect("write journal");
        let state = AppState::from_journal_path(dir.path().join("main.journal"))
            .expect("the scratch journal opens")
            .with_price_source(Arc::new(feed));
        Self { dir, state }
    }

    fn held() -> Self {
        Self::with(
            JOURNAL,
            FakeFeed::new([("AAPL", FakeQuote::Price("2026-06-30", dec(22800, 2)))]),
        )
    }

    fn router(&self) -> axum::Router {
        router_with_state(self.state.clone())
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(relative)).expect("read back")
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }
}

// ---------------------------------------------------------------------------
// HTTP helpers (same shape as budget_endpoints.rs)
// ---------------------------------------------------------------------------

async fn send(router: axum::Router, request: Request<Body>) -> (StatusCode, String) {
    let response = router.oneshot(request).await.expect("router responds");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn json_or_text(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_string()))
}

async fn get(tree: &Tree, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    let (status, body) = send(tree.router(), request).await;
    (status, json_or_text(&body))
}

async fn post_json(tree: &Tree, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("request builds");
    let (status, text) = send(tree.router(), request).await;
    (status, json_or_text(&text))
}

/// One symbol's result out of an update response, by symbol.
fn result_for<'a>(body: &'a Value, symbol: &str) -> &'a Value {
    body["results"]
        .as_array()
        .expect("results is an array")
        .iter()
        .find(|result| result["symbol"] == json!(symbol))
        .unwrap_or_else(|| panic!("no result for {symbol} in {body}"))
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// A journal with a held stock and no price directives anywhere: the symbol is
/// listed with no `yahoo:` override, and the only home for a first price is
/// the root journal itself (mirrors budget's "no rules, no empty file" case).
#[tokio::test]
async fn status_lists_the_held_symbol_and_falls_back_to_the_root_journal() {
    let tree = Tree::held();
    let (status, body) = get(&tree, "/api/prices/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["quoteCommodity"], json!("$"));
    assert_eq!(
        body["symbols"],
        json!([{"symbol": "AAPL", "yahooTicker": "AAPL"}])
    );
    assert_eq!(body["canCreateFile"], json!(true));
    assert_eq!(body["defaultTarget"], json!("main.journal"));
}

/// A commodity's `yahoo:` tag overrides the ticker a quote is fetched as —
/// the case this whole feature exists for: an hledger symbol (like the real
/// `BRK'B`) that Yahoo Finance knows by a different one.
#[tokio::test]
async fn status_reads_the_commoditys_yahoo_tag() {
    let journal = "\
commodity 1.0000 BRKB ; yahoo: BRK-B

2026-01-05 buy berkshire
    assets:broker    5 BRKB @ $400.00
    assets:cash
";
    let tree = Tree::with(journal, FakeFeed::new([]));
    let (status, body) = get(&tree, "/api/prices/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["symbols"],
        json!([{"symbol": "BRKB", "yahooTicker": "BRK-B"}])
    );
}

/// Prices already on record in a file that is NOT called `prices.journal` —
/// `history.journal`, `kurse.journal`, whatever this user named it. That file
/// is the default target and no new one may be offered, because "where do
/// prices live" is answered by `P` directives, never by a filename.
#[tokio::test]
async fn status_targets_an_existing_price_file_whatever_it_is_called() {
    let main = "\
commodity 1.0000 AAPL

include history.journal

2026-01-05 buy apple
    assets:broker    10 AAPL @ $200.00
    assets:cash
";
    let tree = Tree::with_files(
        main,
        &[("history.journal", "P 2026-06-01 AAPL $220.00\n")],
        FakeFeed::new([("AAPL", FakeQuote::Price("2026-06-30", dec(22800, 2)))]),
    );

    let (status, body) = get(&tree, "/api/prices/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["defaultTarget"], json!("history.journal"));
    assert_eq!(
        body["canCreateFile"],
        json!(false),
        "a journal that already prices things must never be offered a second prices file"
    );

    // …and an update lands there, leaving the root journal untouched.
    let (status, body) = post_json(
        &tree,
        "/api/prices/update",
        json!({"journalId": "history.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(result_for(&body, "AAPL")["outcome"], json!("updated"));
    assert_eq!(
        tree.read("history.journal"),
        "P 2026-06-01 AAPL $220.00\nP 2026-06-30 AAPL $228.00\n"
    );
    assert_eq!(
        tree.read("main.journal"),
        main,
        "the root journal is untouched"
    );
    assert!(!tree.path("prices.journal").exists());
}

/// A `duplicate` reports the SAME rounded price an `updated` would have
/// written, not the raw `f64` tail the feed answered with.
#[tokio::test]
async fn a_duplicate_reports_the_rounded_price_an_update_would_have_written() {
    let tree = Tree::with(
        JOURNAL,
        FakeFeed::new([(
            "AAPL",
            FakeQuote::Price("2026-06-30", dec(3_668_500_061_035, 10)),
        )]),
    );
    post_json(&tree, "/api/prices/file", json!({})).await;
    std::fs::write(
        tree.path("prices.journal"),
        "; Market prices.\nP 2026-06-30 AAPL $366.85\n",
    )
    .expect("seed an existing price");

    let (status, body) = post_json(
        &tree,
        "/api/prices/update",
        json!({"journalId": "prices.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = result_for(&body, "AAPL");
    assert_eq!(result["outcome"], json!("duplicate"));
    assert_eq!(result["price"], json!({"mantissa": "36685", "places": 2}));
}

// ---------------------------------------------------------------------------
// Creating a prices file
// ---------------------------------------------------------------------------

#[tokio::test]
async fn creating_a_prices_file_writes_it_and_includes_it() {
    let tree = Tree::held();
    let (status, body) = post_json(&tree, "/api/prices/file", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["journalId"], json!("prices.journal"));
    assert_eq!(body["includedAs"], json!("include prices.journal"));
    assert_eq!(body["mainJournalId"], json!("main.journal"));

    assert!(tree.read("prices.journal").starts_with("; Market prices."));
    let main = tree.read("main.journal");
    assert!(main.ends_with("include prices.journal\n"), "{main}");
    assert!(main.starts_with("commodity 1.0000 AAPL"), "{main}");

    let (status, body) = get(&tree, "/api/prices/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["canCreateFile"], json!(false));
    assert_eq!(body["defaultTarget"], json!("prices.journal"));
}

/// An existing `prices.journal` is NEVER written over, even though the journal
/// has no price directives and the file is not included — same guarantee
/// `budget_endpoints.rs` pins for `budget.journal`.
#[tokio::test]
async fn an_existing_prices_file_is_never_overwritten() {
    let tree = Tree::held();
    std::fs::write(tree.path("prices.journal"), "; someone else's notes\n").expect("write");
    let before = std::fs::read(tree.path("prices.journal")).expect("read back");

    let (status, body) = get(&tree, "/api/prices/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["canCreateFile"],
        json!(false),
        "the button must not be offered when it would fail"
    );

    let (status, body) = post_json(&tree, "/api/prices/file", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        std::fs::read(tree.path("prices.journal")).expect("read back"),
        before
    );
}

// ---------------------------------------------------------------------------
// Updating
// ---------------------------------------------------------------------------

/// The whole happy path: fetch, format in the journal's own style, append.
#[tokio::test]
async fn an_update_appends_a_fetched_price_in_the_journals_own_style() {
    let tree = Tree::held();
    let (status, _) = post_json(&tree, "/api/prices/file", json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = post_json(
        &tree,
        "/api/prices/update",
        json!({"journalId": "prices.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["file"]["journalId"], json!("prices.journal"));
    assert_eq!(body["file"]["priceCount"], json!(1));

    let result = result_for(&body, "AAPL");
    assert_eq!(result["outcome"], json!("updated"));
    assert_eq!(result["date"], json!("2026-06-30"));

    assert_eq!(
        tree.read("prices.journal"),
        "; Market prices.\n;\n; Each `P` line below records one commodity's price on a date. \
         Ledgeline\n; writes to this file from the Holdings tab's \"Update prices\" button; \
         hledger\n; reads it for valuation.\nP 2026-06-30 AAPL $228.00\n"
    );
}

/// A quote source that round-trips through `f64` (the real Yahoo client does)
/// routinely answers with a binary-float artifact instead of a clean price —
/// this journal's own established `$` style (two decimals, from the `$200.00`
/// cost in `JOURNAL`) is what an update must round a fetched quote DOWN to,
/// not the noise Yahoo's JSON happened to carry.
#[tokio::test]
async fn an_update_rounds_a_fetched_price_to_the_journals_own_precision() {
    let tree = Tree::with(
        JOURNAL,
        FakeFeed::new([(
            "AAPL",
            FakeQuote::Price("2026-06-30", dec(3_668_500_061_035, 10)),
        )]),
    );
    post_json(&tree, "/api/prices/file", json!({})).await;

    let (status, body) = post_json(
        &tree,
        "/api/prices/update",
        json!({"journalId": "prices.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(result_for(&body, "AAPL")["outcome"], json!("updated"));

    let written = tree.read("prices.journal");
    assert!(
        written.ends_with("P 2026-06-30 AAPL $366.85\n"),
        "expected a clean two-decimal price, got: {written}"
    );
}

/// A price already on record for the fetched date is a no-op, not a second
/// line — running the button twice in a row must not grow the file.
#[tokio::test]
async fn an_update_skips_a_symbol_already_priced_for_that_date() {
    let tree = Tree::held();
    post_json(&tree, "/api/prices/file", json!({})).await;
    std::fs::write(
        tree.path("prices.journal"),
        "; Market prices.\nP 2026-06-30 AAPL $220.00\n",
    )
    .expect("seed an existing price");

    let (status, body) = post_json(
        &tree,
        "/api/prices/update",
        json!({"journalId": "prices.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = result_for(&body, "AAPL");
    assert_eq!(result["outcome"], json!("duplicate"));
    assert_eq!(
        tree.read("prices.journal"),
        "; Market prices.\nP 2026-06-30 AAPL $220.00\n",
        "a duplicate must write nothing"
    );
}

/// Yahoo has nothing for this ticker: reported, not treated as an error.
#[tokio::test]
async fn an_update_reports_not_found_without_failing_the_request() {
    let tree = Tree::with(JOURNAL, FakeFeed::new([("AAPL", FakeQuote::NotFound)]));
    post_json(&tree, "/api/prices/file", json!({})).await;

    let (status, body) = post_json(
        &tree,
        "/api/prices/update",
        json!({"journalId": "prices.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(result_for(&body, "AAPL")["outcome"], json!("not-found"));
    assert_eq!(
        tree.read("prices.journal"),
        "; Market prices.\n;\n; Each `P` line below records one commodity's price on a date. \
         Ledgeline\n; writes to this file from the Holdings tab's \"Update prices\" button; \
         hledger\n; reads it for valuation.\n",
        "nothing to append means nothing written"
    );
}

/// The fetch itself failed: reported as `fetch-error`, and nothing else about
/// the request fails because of it.
#[tokio::test]
async fn an_update_reports_a_fetch_error_without_failing_the_request() {
    let tree = Tree::with(JOURNAL, FakeFeed::new([("AAPL", FakeQuote::Error)]));
    post_json(&tree, "/api/prices/file", json!({})).await;

    let (status, body) = post_json(
        &tree,
        "/api/prices/update",
        json!({"journalId": "prices.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(result_for(&body, "AAPL")["outcome"], json!("fetch-error"));
}

/// An unknown `journalId` is a `404`, and nothing on disk moves.
#[tokio::test]
async fn an_update_against_an_unknown_journal_id_is_not_found() {
    let tree = Tree::held();
    let (status, body) = post_json(
        &tree,
        "/api/prices/update",
        json!({"journalId": "nope.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

// ---------------------------------------------------------------------------
// Access control
// ---------------------------------------------------------------------------

/// All three routes sit above the token guard. Two of them write to the user's
/// journal directory, so this is the test that fails rather than shipping if
/// anyone moves them below it.
#[tokio::test]
async fn every_prices_route_requires_the_token() {
    const PORT: u16 = 5099;
    const HOST: &str = "127.0.0.1:5099";
    let tree = Tree::held();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");

    let probe = |method: &'static str, uri: &'static str, auth: Option<&'static str>| {
        let state = tree.state.clone();
        let security = Security::local(token.clone(), PORT);
        async move {
            let mut builder = Request::builder()
                .method(method)
                .uri(uri)
                .header(HeaderName::from_static("host"), HOST);
            if let Some(value) = auth {
                builder = builder.header(header::AUTHORIZATION, value);
            }
            let request = builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"journalId":"main.journal"}"#))
                .expect("request builds");
            router_with_security(state, security)
                .oneshot(request)
                .await
                .expect("router responds")
        }
    };

    for (method, uri) in [
        ("GET", "/api/prices/status"),
        ("POST", "/api/prices/file"),
        ("POST", "/api/prices/update"),
    ] {
        let response = probe(method, uri, None).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with no token"
        );
        let response = probe(method, uri, Some("Bearer wrong-token")).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with the wrong token"
        );
    }
}
