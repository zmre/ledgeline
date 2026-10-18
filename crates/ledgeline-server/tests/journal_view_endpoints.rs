//! The journal-view routes (plan 26 B3–B5): shapes, validation and transport.
//! The ARITHMETIC behind them is pinned in `ledgeline-core`'s golden suites
//! (`cash_balances_golden.rs`, …); these tests pin what the HTTP layer adds —
//! parameter parsing, the snapshot `ETag`, and that each route answers what its
//! core function computes.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{fixture_journal, fixtures_dir};
use http_body_util::BodyExt;
use ledgeline::{AppState, router_with_state};
use ledgeline_core::parse_journal;
use serde_json::Value;
use tower::ServiceExt;

async fn get_with(
    state: &AppState,
    uri: &str,
    headers: &[(header::HeaderName, &str)],
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method("GET").uri(uri);
    for (name, value) in headers {
        builder = builder.header(name, *value);
    }
    let response = router_with_state(state.clone())
        .oneshot(builder.body(Body::empty()).expect("request builds"))
        .await
        .expect("router responds");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec();
    (status, headers, body)
}

async fn json(state: &AppState, uri: &str) -> Value {
    let (status, _, body) = get_with(state, uri, &[]).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "GET {uri}: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).unwrap_or_else(|e| panic!("GET {uri} is JSON: {e}"))
}

fn edge_state() -> AppState {
    let path = fixtures_dir().join("journal-view/edge-cases.journal");
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    AppState::from_journal(&parse_journal(&text, &path.to_string_lossy()).expect("parses"))
}

/// Every journal-view route carries the snapshot `ETag` and answers a matching
/// `If-None-Match` with an empty `304`.
#[tokio::test]
async fn journal_view_routes_revalidate_against_the_snapshot_etag() {
    let state = AppState::from_journal(&fixture_journal());
    for uri in [
        "/api/balances/cash?asOf=2026-01-01",
        "/api/diagnostics?today=2026-01-01",
        "/api/diagnostics?today=2026-01-01&limit=0",
    ] {
        let (status, headers, _) = get_with(&state, uri, &[]).await;
        assert_eq!(status, StatusCode::OK, "GET {uri}");
        let etag = headers
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .expect("an ETag")
            .to_string();
        let (again, _, body) = get_with(&state, uri, &[(header::IF_NONE_MATCH, &etag)]).await;
        assert_eq!(again, StatusCode::NOT_MODIFIED, "GET {uri} revalidated");
        assert!(body.is_empty(), "a 304 has no body");
    }
}

#[tokio::test]
async fn cash_balances_serve_every_commodity_summary() {
    let state = edge_state();
    let body = json(&state, "/api/balances/cash?asOf=2025-01-12").await;
    assert_eq!(body["asOf"], "2025-01-12");
    assert_eq!(body["commodities"], serde_json::json!(["$", "EUR"]));
    let summaries = body["summaries"].as_array().expect("summaries");
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0]["commodity"], "$");
    assert_eq!(
        summaries[0]["totalCash"],
        serde_json::json!({"m": "7344544", "p": 3})
    );
    assert_eq!(summaries[0]["style"]["side"], "L");
    assert_eq!(summaries[1]["style"]["decimalPoint"], ",");
    assert_eq!(
        body["guessedLongTerm"],
        serde_json::json!(["assets:bank:long-term-cd", "liabilities:mortgage"])
    );
}

#[tokio::test]
async fn cash_balances_refuse_a_malformed_date() {
    let state = edge_state();
    let (status, _, _) = get_with(&state, "/api/balances/cash?asOf=yesterday", &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The problem list is per client date: `today` moves the future-date rule,
/// every transaction finding names its transaction, and `limit` truncates the
/// entries without changing what the badge reads.
#[tokio::test]
async fn diagnostics_follow_the_clients_today() {
    let state = edge_state();
    let early = json(&state, "/api/diagnostics?today=2025-01-15").await;
    let late = json(&state, "/api/diagnostics?today=2031-01-01").await;
    let future = |body: &Value| {
        body["diagnostics"]
            .as_array()
            .expect("entries")
            .iter()
            .filter(|d| d["rule"] == "future-date")
            .count()
    };
    assert_eq!(future(&early), 5);
    assert_eq!(future(&late), 0);
    assert_eq!(early["maxSeverity"], "warning");

    let pending = early["diagnostics"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|d| d["rule"] == "pending")
        .expect("a pending finding");
    assert_eq!(pending["tindex"], 3);
    assert_eq!(pending["txnIndex"], 2);
    assert_eq!(pending["date"], "2025-01-05");
    assert_eq!(pending["description"], "Grocery Mart | weekly shop");

    let badge = json(&state, "/api/diagnostics?today=2025-01-15&limit=0").await;
    assert_eq!(badge["count"], early["count"]);
    assert_eq!(badge["maxSeverity"], "warning");
    assert_eq!(badge["diagnostics"], serde_json::json!([]));
}

/// Two dates are two answers, so they must not share a validator.
#[tokio::test]
async fn diagnostics_etag_carries_the_date() {
    let state = edge_state();
    let tag = |headers: &axum::http::HeaderMap| {
        headers
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .expect("an ETag")
            .to_string()
    };
    let (_, a, _) = get_with(&state, "/api/diagnostics?today=2025-01-15", &[]).await;
    let (_, b, _) = get_with(&state, "/api/diagnostics?today=2025-01-16", &[]).await;
    assert_ne!(tag(&a), tag(&b));
    let (status, _, _) = get_with(
        &state,
        "/api/diagnostics?today=2025-01-16",
        &[(header::IF_NONE_MATCH, &tag(&a))],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "another day's tag must not revalidate"
    );
}
