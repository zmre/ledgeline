//! HTTP transport behaviour added by Phase 6 (PERF-1, PERF-2).
//!
//! The wire endpoints serve bytes that were serialized once into the
//! [`Snapshot`](ledgeline_server) rather than a `serde_json::Value` re-rendered
//! per request, and each response carries an `ETag` so the SPA's 30-second poll
//! costs a `304` until the journal actually changes.
//!
//! What these tests pin down:
//!   * every wire route offers an `ETag` and `Cache-Control: no-cache`;
//!   * `If-None-Match` with that tag is a `304` with an EMPTY body — the whole
//!     point, since the body is otherwise the entire journal;
//!   * a stale, weak, `*`, or list-valued `If-None-Match` all behave per RFC 9110;
//!   * republishing the journal MINTS A NEW TAG, so a 304 can never pin stale
//!     data on screen;
//!   * `Accept-Encoding: gzip` gets a gzip-encoded body that inflates back to the
//!     identity bytes, and a client that asks for no encoding still gets those
//!     identity bytes.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::fixture_journal;
use http_body_util::BodyExt;
use ledgeline_server::{AppState, app, router_with_state};
use tower::ServiceExt;

/// Every route whose body comes out of the snapshot.
const WIRE_ROUTES: [&str; 7] = [
    "/version",
    "/accountnames",
    "/transactions",
    "/prices",
    "/commodities",
    "/accounts",
    "/api/diagnostics",
];

/// One request against a fresh clone of `state`'s router, with optional extra
/// headers. Returns the status, the headers, and the raw body bytes.
async fn get_with(
    state: &AppState,
    uri: &str,
    headers: &[(header::HeaderName, &str)],
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method("GET").uri(uri);
    for (name, value) in headers {
        builder = builder.header(name, *value);
    }
    let request = builder.body(Body::empty()).expect("request builds");
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

fn header_str(headers: &axum::http::HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[tokio::test]
async fn every_wire_route_offers_an_etag_and_revalidates() {
    let state = AppState::from_journal(&fixture_journal());
    for uri in WIRE_ROUTES {
        let (status, headers, body) = get_with(&state, uri, &[]).await;
        assert_eq!(status, StatusCode::OK, "GET {uri}");
        assert_eq!(
            header_str(&headers, header::CONTENT_TYPE).as_deref(),
            Some("application/json"),
            "GET {uri} content-type"
        );
        assert_eq!(
            header_str(&headers, header::CACHE_CONTROL).as_deref(),
            Some("no-cache"),
            "GET {uri} must be revalidated, never served from a cache blind"
        );
        let etag = header_str(&headers, header::ETAG).expect("an ETag on {uri}");
        assert!(
            etag.starts_with('"') && etag.ends_with('"'),
            "GET {uri} ETag must be a quoted entity-tag, got {etag}"
        );
        assert!(!body.is_empty(), "GET {uri} must have a body");
        serde_json::from_slice::<serde_json::Value>(&body)
            .unwrap_or_else(|err| panic!("GET {uri} body is not JSON: {err}"));
    }
}

#[tokio::test]
async fn if_none_match_with_the_current_tag_is_a_304_with_no_body() {
    let state = AppState::from_journal(&fixture_journal());
    for uri in WIRE_ROUTES {
        let (_, headers, full) = get_with(&state, uri, &[]).await;
        let etag = header_str(&headers, header::ETAG).expect("an ETag");

        let (status, headers, body) =
            get_with(&state, uri, &[(header::IF_NONE_MATCH, &etag)]).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED, "GET {uri} revalidated");
        assert!(
            body.is_empty(),
            "GET {uri} 304 must not carry the {} byte body",
            full.len()
        );
        assert_eq!(
            header_str(&headers, header::ETAG).as_deref(),
            Some(etag.as_str()),
            "GET {uri} 304 must repeat the tag so the client keeps revalidating"
        );
    }
}

/// RFC 9110 §13.1.2 spellings a proxy or a browser may legitimately send.
#[tokio::test]
async fn if_none_match_accepts_wildcard_weak_and_list_forms() {
    let state = AppState::from_journal(&fixture_journal());
    let (_, headers, _) = get_with(&state, "/transactions", &[]).await;
    let etag = header_str(&headers, header::ETAG).expect("an ETag");
    let weak = format!("W/{etag}");
    let in_a_list = format!("\"someone-elses\", {etag}, \"another\"");

    for candidate in ["*", etag.as_str(), weak.as_str(), in_a_list.as_str()] {
        let (status, _, _) = get_with(
            &state,
            "/transactions",
            &[(header::IF_NONE_MATCH, candidate)],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_MODIFIED,
            "If-None-Match: {candidate} should match"
        );
    }
}

#[tokio::test]
async fn a_stale_or_absent_if_none_match_still_gets_the_body() {
    let state = AppState::from_journal(&fixture_journal());
    for candidate in ["\"stale\"", "W/\"stale\"", ""] {
        let headers: Vec<(header::HeaderName, &str)> = if candidate.is_empty() {
            vec![]
        } else {
            vec![(header::IF_NONE_MATCH, candidate)]
        };
        let (status, _, body) = get_with(&state, "/transactions", &headers).await;
        assert_eq!(status, StatusCode::OK, "If-None-Match: {candidate:?}");
        assert!(!body.is_empty());
    }
}

/// The failure this guards against is the nastiest one a conditional GET can
/// have: a client holding a tag from BEFORE an edit being told "not modified"
/// and rendering the old journal forever.
#[tokio::test]
async fn republishing_the_journal_mints_a_new_etag() {
    let state = AppState::from_journal(&fixture_journal());
    let (_, headers, before) = get_with(&state, "/transactions", &[]).await;
    let old_etag = header_str(&headers, header::ETAG).expect("an ETag");

    let edited = ledgeline_core::parse_journal(
        "2024-01-01 a new transaction\n    expenses:x   $1.00\n    assets:bank\n",
        "/tmp/etag-rotation.journal",
    )
    .expect("journal parses");
    state.replace_journal(&std::sync::Arc::new(edited));

    let (status, headers, after) = get_with(
        &state,
        "/transactions",
        &[(header::IF_NONE_MATCH, &old_etag)],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the pre-edit tag must NOT match the post-edit snapshot"
    );
    assert_ne!(
        header_str(&headers, header::ETAG).as_deref(),
        Some(old_etag.as_str())
    );
    assert_ne!(before, after, "and the body really did change");
}

/// Two snapshots built from the SAME journal still differ, because the tag is a
/// generation counter, not a content hash. That is the safe direction: it costs
/// one redundant transfer, where the reverse would serve stale data.
#[tokio::test]
async fn identical_journals_do_not_share_a_tag() {
    let journal = fixture_journal();
    let one = AppState::from_journal(&journal);
    let two = AppState::from_journal(&journal);
    let (_, headers_one, body_one) = get_with(&one, "/transactions", &[]).await;
    let (_, headers_two, body_two) = get_with(&two, "/transactions", &[]).await;
    assert_eq!(body_one, body_two, "same journal, same bytes");
    assert_ne!(
        header_str(&headers_one, header::ETAG),
        header_str(&headers_two, header::ETAG)
    );
}

#[tokio::test]
async fn accept_encoding_gzip_gets_a_gzip_body_that_inflates_to_the_identity_bytes() {
    let journal = fixture_journal();
    let state = AppState::from_journal(&journal);
    let (_, _, identity) = get_with(&state, "/transactions", &[]).await;

    let (status, headers, compressed) = get_with(
        &state,
        "/transactions",
        &[(header::ACCEPT_ENCODING, "gzip")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_str(&headers, header::CONTENT_ENCODING).as_deref(),
        Some("gzip"),
        "a gzip-capable client should be sent gzip"
    );
    assert!(
        compressed.len() < identity.len(),
        "gzip should be smaller: {} vs {}",
        compressed.len(),
        identity.len()
    );
    assert_eq!(
        inflate(&compressed),
        identity,
        "the compressed body must inflate to exactly the identity bytes"
    );
}

#[tokio::test]
async fn a_client_that_advertises_no_encoding_gets_identity_bytes() {
    let state = AppState::from_journal(&fixture_journal());
    let (status, headers, body) = get_with(
        &state,
        "/transactions",
        &[(header::ACCEPT_ENCODING, "identity")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, header::CONTENT_ENCODING), None);
    serde_json::from_slice::<serde_json::Value>(&body).expect("plain JSON");
}

/// PERF-1's contract with the rest of the suite: the bytes a request gets back
/// are the SAME buffer the snapshot holds, so repeated requests are identical
/// and no re-serialization happens between them.
#[tokio::test]
async fn repeated_requests_serve_byte_identical_bodies() {
    let router_journal = fixture_journal();
    let first = {
        let request = Request::builder()
            .uri("/transactions")
            .body(Body::empty())
            .expect("request builds");
        app(&router_journal)
            .oneshot(request)
            .await
            .expect("responds")
            .into_body()
            .collect()
            .await
            .expect("collects")
            .to_bytes()
    };
    let state = AppState::from_journal(&router_journal);
    let (_, _, second) = get_with(&state, "/transactions", &[]).await;
    let (_, _, third) = get_with(&state, "/transactions", &[]).await;
    assert_eq!(first.as_ref(), second.as_slice());
    assert_eq!(second, third);
}

/// Minimal gzip inflate so the test does not need a decompression dependency:
/// shell out to the `gzip` the dev shell already provides.
fn inflate(compressed: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("gzip")
        .arg("-d")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("gzip is on PATH");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(compressed)
        .expect("write compressed body");
    let output = child.wait_with_output().expect("gzip runs");
    assert!(output.status.success(), "gzip -d failed");
    output.stdout
}
