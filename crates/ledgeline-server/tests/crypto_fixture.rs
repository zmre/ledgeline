//! Plan 48 WP6: the crypto fixture ledgers (`fixtures/crypto/`) through the
//! real HTTP surface.
//!
//! Each test copies the fixture into a scratch directory first, so nothing
//! here can write a price into `fixtures/` (an update, or the automatic
//! updater, would). No network: the price feed is a fake that knows nothing.

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ledgeline::{AppDirs, AppState, FetchedPrice, PriceFeed, YahooError, router_with_state};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

/// A price feed with nothing to say: these tests read the journal only.
struct Silent;

#[async_trait]
impl PriceFeed for Silent {
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
}

/// `fixtures/crypto/<files>` copied into a scratch dir, opened at `main`.
fn open(main: &str, files: &[&str]) -> (TempDir, AppState) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/crypto");
    let dir = TempDir::new().expect("temp dir");
    for file in files {
        std::fs::copy(fixtures.join(file), dir.path().join(file)).expect("copy fixture");
    }
    let state = AppState::from_journal_path(dir.path().join(main))
        .expect("the fixture opens")
        .with_app_dirs(AppDirs::under(dir.path().join("app")))
        .with_price_source(Arc::new(Silent));
    (dir, state)
}

async fn get(state: &AppState, uri: &str) -> Value {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK, "{uri}");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("JSON")
}

fn holding<'a>(report: &'a Value, symbol: &str) -> &'a Value {
    report["holdings"]
        .as_array()
        .expect("holdings")
        .iter()
        .find(|h| h["symbol"] == json!(symbol))
        .unwrap_or_else(|| panic!("no {symbol} in {report}"))
}

/// `{mantissa, places}` → its decimal text.
fn dec(value: &Value) -> String {
    let mantissa: i128 = value["mantissa"]
        .as_str()
        .expect("mantissa")
        .parse()
        .expect("an integer");
    let places = value["places"].as_u64().expect("places") as u32;
    let text = ledgeline_core::Dec::new(mantissa, places).normalized();
    let divisor = 10i128.pow(text.places);
    let (whole, frac) = (text.mantissa / divisor, (text.mantissa % divisor).abs());
    if text.places == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{frac:0width$}", width = text.places as usize)
    }
}

/// Every coin values exactly — eight-place BTC, eighteen-place ETH (read to
/// hledger's ten), a sub-cent token — with no overflow, no cost-basis
/// warning from the fee paid in BTC, and the staking reward in the basis.
#[tokio::test]
async fn the_crypto_fixture_values_every_coin_exactly() {
    let (_dir, state) = open("crypto.journal", &["crypto.journal", "prices.journal"]);
    let report = get(&state, "/api/holdings?asOf=2026-11-23").await;
    assert_eq!(report["warnings"], json!([]), "{report}");

    let btc = holding(&report, "BTC");
    assert_eq!(dec(&btc["shares"]), "0.0499");
    assert_eq!(dec(&btc["marketValue"]), "3113.76499");
    assert_eq!(
        dec(&btc["basis"]),
        "3053.88",
        "the fee's share of basis left with it"
    );

    let eth = holding(&report, "ETH");
    assert_eq!(dec(&eth["shares"]), "1.26");
    assert_eq!(dec(&eth["basis"]), "3025", "the reward's @ value is basis");
    assert_eq!(dec(&eth["marketValue"]), "2218.104");

    assert_eq!(dec(&holding(&report, "USDT")["marketValue"]), "998.74");
    assert_eq!(dec(&holding(&report, "USDC")["marketValue"]), "499.93125");
    assert_eq!(dec(&holding(&report, "PEPE24478")["marketValue"]), "89.5");
    assert_eq!(dec(&holding(&report, "UNI")["marketValue"]), "204.936");
}

/// Every coin's ticker: its dollar pair, or the `yahoo:` tag's.
#[tokio::test]
async fn the_crypto_fixture_asks_yahoo_for_each_coins_pair() {
    let (_dir, state) = open("crypto.journal", &["crypto.journal", "prices.journal"]);
    let status = get(&state, "/api/prices/status").await;
    let mut tickers: Vec<(String, String)> = status["symbols"]
        .as_array()
        .expect("symbols")
        .iter()
        .map(|s| {
            (
                s["symbol"].as_str().unwrap().to_string(),
                s["yahooTicker"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    tickers.sort();
    let expected = [
        ("BTC", "BTC-USD"),
        ("ETH", "ETH-USD"),
        ("PEPE24478", "PEPE24478-USD"),
        ("UNI", "UNI7083-USD"),
        ("USDC", "USDC-USD"),
        ("USDT", "USDT-USD"),
    ]
    .map(|(a, b)| (a.to_string(), b.to_string()));
    assert_eq!(tickers, expected);
}

/// The untagged ledger's `BTC` is looked up as itself — maybe the ETF.
#[tokio::test]
async fn the_untagged_fixture_looks_btc_up_as_itself() {
    let (_dir, state) = open("untagged.journal", &["untagged.journal"]);
    let status = get(&state, "/api/prices/status").await;
    assert_eq!(
        status["symbols"],
        json!([{"symbol": "BTC", "yahooTicker": "BTC"}])
    );
}

/// An eighteen-place quantity (read to hledger's ten) times a ten-place
/// price carries twenty places in an `i128`: no overflow, and the value is
/// exact. A million coins at that precision still fits.
#[tokio::test]
async fn wei_precision_times_a_ten_place_price_values_exactly() {
    let dir = TempDir::new().expect("temp dir");
    std::fs::write(
        dir.path().join("main.journal"),
        "\
commodity 1.000000000000000000 ETH  ; type: crypto

2026-01-05 buy
    assets:crypto    1000000.123456789012345678 ETH @ $2400.00
    assets:cash

P 2026-11-22 ETH $1733.0000000001
",
    )
    .expect("write journal");
    let state = AppState::from_journal_path(dir.path().join("main.journal"))
        .expect("opens")
        .with_app_dirs(AppDirs::under(dir.path().join("app")))
        .with_price_source(Arc::new(Silent));
    let report = get(&state, "/api/holdings?asOf=2026-11-23").await;
    assert_eq!(report["warnings"], json!([]), "{report}");
    let eth = holding(&report, "ETH");
    // 1000000.123456789012345678 → 1000000.1234567890 (half-even at ten).
    assert_eq!(
        dec(&eth["shares"]),
        "1000000.123456789",
        "read to ten places"
    );
    // 1000000.123456789 × 1733.0000000001, every digit kept.
    assert_eq!(dec(&eth["marketValue"]), "1733000213.9507153370123456789");
}
