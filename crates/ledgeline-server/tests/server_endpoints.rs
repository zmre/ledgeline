//! End-to-end HTTP tests for the Phase 2 read server.
//!
//! Builds the real axum `Router` over `fixtures/sample.journal` and drives each
//! route through the full HTTP stack with `tower`'s `oneshot` (no sockets), then
//! checks every body against its committed hledger-web 1.52 snapshot:
//!   - `/version`, `/accountnames`, `/commodities`, `/prices`, `/transactions`
//!     are compared in full (ignoring `floatingPoint` and `sourceName`);
//!   - `/accounts` is compared on the `(aname -> aditags)` contract only (its
//!     `adata` balances are Phase-3 work and are excluded), matching Part A.
//!
//! A final test asserts the permissive CORS header is present on a GET.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{account_contract, compare, fixture_journal, read_snapshot};
use http_body_util::BodyExt;
use ledgeline_server::app;
use serde_json::Value;
use tower::ServiceExt;

/// Issue `GET uri` (with an `Origin` header) against a fresh clone of the app and
/// return the status, the `access-control-allow-origin` header, and the parsed
/// JSON body.
async fn get(uri: &str) -> (StatusCode, Option<String>, Value) {
    let router = app(&fixture_journal());
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::ORIGIN, "https://spa.example")
        .body(Body::empty())
        .expect("request builds");

    let response = router.oneshot(request).await.expect("router responds");
    let status = response.status();
    let allow_origin = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let body = serde_json::from_slice(&bytes).expect("body is JSON");
    (status, allow_origin, body)
}

async fn body_of(uri: &str) -> Value {
    let (status, _, body) = get(uri).await;
    assert_eq!(status, StatusCode::OK, "GET {uri} should be 200 OK");
    body
}

/// Full-body parity for the endpoints whose snapshots we reproduce exactly.
#[tokio::test]
async fn full_body_endpoints_match_snapshots() {
    for (uri, snapshot) in [
        ("/version", "version.json"),
        ("/accountnames", "accountnames.json"),
        ("/commodities", "commodities.json"),
        ("/prices", "prices.json"),
        ("/transactions", "transactions.json"),
    ] {
        let expected = read_snapshot(snapshot);
        let actual = body_of(uri).await;
        if let Err(message) = compare("$", &expected, &actual) {
            panic!("{uri} parity mismatch at {message}");
        }
    }
}

/// `/accounts` is validated on the SPA contract only; `adata` is excluded.
#[tokio::test]
async fn accounts_contract_matches_snapshot() {
    let expected = read_snapshot("accounts.json");
    let actual = body_of("/accounts").await;
    assert_eq!(
        account_contract(&actual),
        account_contract(&expected),
        "the /accounts (aname -> aditags) contract must match the snapshot (adata excluded)"
    );
}

/// The permissive CORS layer must echo an allow-origin header on a plain GET so
/// the browser SPA can read the response cross-origin.
#[tokio::test]
async fn get_carries_permissive_cors_header() {
    let (status, allow_origin, _) = get("/version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        allow_origin.as_deref(),
        Some("*"),
        "permissive CORS should return access-control-allow-origin: *"
    );
}
