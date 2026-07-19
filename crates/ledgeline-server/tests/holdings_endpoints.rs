//! End-to-end HTTP tests for the native `/api/holdings[/series]` endpoints.
//!
//! Drives the real axum `Router` through `tower`'s `oneshot` (no sockets) over
//! `fixtures/sample.journal`, asserting the wire JSON shape and the known
//! positions (AAPL/VTI priced with gains, GLD tainted+unpriced, NVDA absent,
//! TSLA negative-shares warning). The engine itself is verified in
//! `ledgeline-core`'s `tests/holdings.rs`; these tests pin the JSON contract.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline_core::Journal;
use ledgeline_server::app;
use serde_json::Value;
use tower::ServiceExt;

fn sample_journal() -> Journal {
    common::fixture_journal()
}

/// All stock activity + prices are ≤ 2026-06-30, so this `asOf` is stable.
const AS_OF: &str = "2026-07-16";

async fn get_on(journal: &Journal, uri: &str) -> (StatusCode, Option<String>, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::ORIGIN, "https://spa.example")
        .body(Body::empty())
        .expect("request builds");
    let response = app(journal)
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let allow_origin = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes();
    let body = if status == StatusCode::OK {
        serde_json::from_slice(&bytes).expect("body is JSON")
    } else {
        Value::Null
    };
    (status, allow_origin, body)
}

async fn body_ok(journal: &Journal, uri: &str) -> Value {
    let (status, _, body) = get_on(journal, uri).await;
    assert_eq!(status, StatusCode::OK, "GET {uri} should be 200 OK");
    body
}

/// Canonical `(mantissa, places)` (strip trailing zeros), so `{5282750,3}` and
/// `{528275,2}` compare equal.
fn canon(value: &Value) -> (i128, u64) {
    let mut mantissa: i128 = value["mantissa"]
        .as_str()
        .expect("mantissa string")
        .parse()
        .expect("mantissa");
    let mut places = value["places"].as_u64().expect("places");
    while places > 0 && mantissa % 10 == 0 {
        mantissa /= 10;
        places -= 1;
    }
    (mantissa, places)
}

fn holding<'a>(body: &'a Value, symbol: &str) -> &'a Value {
    body["holdings"]
        .as_array()
        .expect("holdings array")
        .iter()
        .find(|h| h["symbol"] == symbol)
        .unwrap_or_else(|| panic!("holding {symbol} in body"))
}

#[tokio::test]
async fn holdings_report_shape_and_positions() {
    let journal = sample_journal();
    let body = body_ok(&journal, &format!("/api/holdings?asOf={AS_OF}")).await;

    assert_eq!(body["asOf"], AS_OF);
    assert_eq!(body["base"], "$");

    // Sorted market value desc, unpriced (GLD) last.
    let symbols: Vec<&str> = body["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["symbol"].as_str().unwrap())
        .collect();
    assert_eq!(symbols, ["VTI", "AAPL", "GLD"]);

    // AAPL: priced by directive, average-cost basis, positive gain.
    let aapl = holding(&body, "AAPL");
    assert_eq!(aapl["name"], "Apple Inc.");
    assert_eq!(canon(&aapl["shares"]), (195, 1)); // 19.5
    assert_eq!(canon(&aapl["basis"]), (43461, 1)); // $4346.10 (canonical)
    assert_eq!(aapl["firstBasisDate"], "2024-09-16");
    assert_eq!(aapl["price"]["source"], "directive");
    assert_eq!(aapl["price"]["date"], "2026-06-30");
    assert_eq!(canon(&aapl["price"]["qty"]), (27025, 2)); // $270.25
    assert_eq!(canon(&aapl["marketValue"]), (5_269_875, 3)); // $5269.875
    assert_eq!(canon(&aapl["gain"]), (923_775, 3)); // $923.775
    assert!(aapl["gainPct"].as_f64().unwrap() > 0.0);

    // VTI: partial sell reduced basis at average cost.
    let vti = holding(&body, "VTI");
    assert_eq!(canon(&vti["shares"]), (17, 0));
    assert_eq!(canon(&vti["basis"]), (469_336, 2)); // $4693.36
    assert_eq!(canon(&vti["marketValue"]), (528_275, 2)); // $5282.75

    // GLD: tainted (basis null) + unpriced (price/marketValue null).
    let gld = holding(&body, "GLD");
    assert!(gld["basis"].is_null(), "GLD basis is null");
    assert!(gld["price"].is_null(), "GLD price is null");
    assert!(gld["marketValue"].is_null());
    assert!(gld["gain"].is_null());
    assert!(gld["gainPct"].is_null());

    // NVDA fully sold → absent.
    assert!(
        body["holdings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["symbol"] != "NVDA")
    );

    // Partial totals: GLD is tainted+unpriced (excluded), but AAPL + VTI still
    // count. basis = $4346.10 + $4693.36 = $9039.46; gain = $923.775 + $589.39 =
    // $1513.165. Market value stays the whole priced portfolio.
    assert_eq!(canon(&body["totals"]["marketValue"]), (10_552_625, 3)); // $10552.625
    assert_eq!(canon(&body["totals"]["basis"]), (903_946, 2)); // $9039.46
    assert_eq!(canon(&body["totals"]["gain"]), (1_513_165, 3)); // $1513.165

    // Gainers AAPL then VTI; no losers.
    let gainers: Vec<&str> = body["topGainers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["symbol"].as_str().unwrap())
        .collect();
    assert_eq!(gainers, ["AAPL", "VTI"]);
    assert!(body["topLosers"].as_array().unwrap().is_empty());

    // Warnings: GLD unpriced + missing-basis, then TSLA negative-shares.
    let warnings: Vec<(&str, &str)> = body["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["symbol"].as_str().unwrap(), w["kind"].as_str().unwrap()))
        .collect();
    assert_eq!(
        warnings,
        [
            ("GLD", "unpriced"),
            ("GLD", "missing-basis"),
            ("TSLA", "negative-shares"),
        ]
    );
}

#[tokio::test]
async fn holdings_gain_since_windows_the_gain() {
    let journal = sample_journal();
    // Default (all-time): AAPL gain $923.775 (mv − all-time basis).
    let base = body_ok(&journal, &format!("/api/holdings?asOf={AS_OF}")).await;
    assert_eq!(canon(&holding(&base, "AAPL")["gain"]), (923_775, 3));

    // Windowed since 2026-01-01: value_at_start = 15 sh × $255 = $3825, so
    // gain = $5269.875 − $3825 = $1444.875; basis stays the all-time $4346.10.
    let windowed = body_ok(
        &journal,
        &format!("/api/holdings?asOf={AS_OF}&gainSince=2026-01-01"),
    )
    .await;
    let aapl = holding(&windowed, "AAPL");
    assert_eq!(canon(&aapl["gain"]), (1_444_875, 3), "windowed gain");
    assert_eq!(canon(&aapl["basis"]), (43461, 1), "basis stays all-time");
}

#[tokio::test]
async fn holdings_exclude_mode_scopes_out_a_subtree() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        &format!("/api/holdings?asOf={AS_OF}&mode=exclude&accounts=assets:broker:taxable:gld,assets:broker:taxable:tsla"),
    )
    .await;

    let symbols: Vec<&str> = body["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["symbol"].as_str().unwrap())
        .collect();
    assert_eq!(symbols, ["VTI", "AAPL"]);
    // With GLD/TSLA scoped out, the basis total is now honest (non-null).
    assert!(body["warnings"].as_array().unwrap().is_empty());
    assert!(
        !body["totals"]["basis"].is_null(),
        "basis total no longer refused"
    );
}

#[tokio::test]
async fn holdings_series_shape() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        &format!("/api/holdings/series?asOf={AS_OF}&interval=monthly&count=3"),
    )
    .await;

    assert_eq!(body["base"], "$");
    let points = body["points"].as_array().unwrap();
    assert_eq!(points.len(), 3);
    let buckets: Vec<&str> = points
        .iter()
        .map(|p| p["bucket"].as_str().unwrap())
        .collect();
    assert_eq!(buckets, ["2026-05", "2026-06", "2026-07"]);
    // Final point clamps to asOf.
    assert_eq!(points.last().unwrap()["date"], AS_OF);
    // GLD is tainted/unpriced, but AAPL + VTI carry a known basis → the PARTIAL
    // basis is present at every point; no stock buy/sell moves it across these
    // three months, so it is $9039.46 throughout.
    assert!(points.iter().all(|p| canon(&p["basis"]) == (903_946, 2)));
    assert_eq!(body["hasBasis"], true);
}

#[tokio::test]
async fn holdings_defaults_and_bad_mode() {
    let journal = sample_journal();
    // No query at all → 200 (asOf defaults to today).
    let (status, allow_origin, _) = get_on(&journal, "/api/holdings").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        allow_origin.as_deref(),
        Some("*"),
        "permissive CORS covers holdings"
    );

    let (status, _, _) = get_on(&journal, "/api/holdings?mode=neither").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _, _) = get_on(&journal, "/api/holdings/series?interval=fortnightly").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
