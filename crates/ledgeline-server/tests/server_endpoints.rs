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
//! The final section covers the local-access controls (SEC-1/7/9): the bearer
//! token on every wire and `/api` route, the `Host` guard, the exact-origin CORS
//! allowlist, and the security headers.

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use common::{account_contract, compare, fixture_journal, read_snapshot};
use http_body_util::BodyExt;
use ledgeline_server::{AccessToken, AppState, Security, app, router_with_security};
use serde_json::{Value, json};
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

/// `GET /api/diagnostics` serves the `{"diagnostics": [...]}` contract: one
/// `Problem`-shaped element per unbalanced transaction (PARSE-1), per failed
/// balance assertion (PARSE-2), and per stock finding (DRY-1).
///
/// `fixtures/sample.journal` balances and asserts cleanly, so its payload is
/// exactly the three stock warnings it plants on purpose — the ones the SPA used
/// to recompute in TypeScript. The array is always present; a journal with none
/// of the three serves `[]`, never null and never absent (see
/// `diagnostics_is_empty_for_a_journal_with_nothing_to_report`).
#[tokio::test]
async fn diagnostics_serves_the_sample_journals_stock_findings() {
    let body = body_of("/api/diagnostics").await;
    let found = body["diagnostics"]
        .as_array()
        .expect("diagnostics is an array");
    let anchors: Vec<(&Value, &Value)> = found
        .iter()
        .map(|diagnostic| (&diagnostic["txnIndex"], &diagnostic["rule"]))
        .collect();
    assert_eq!(
        anchors,
        vec![
            (&json!(102), &json!("stock-missing-basis")),
            (&json!(182), &json!("stock-negative")),
            (&json!(102), &json!("stock-unpriced")),
        ],
        "{body}"
    );
    // Warnings, never errors: this journal is one hledger accepts.
    assert!(found.iter().all(|d| d["severity"] == "warning"), "{body}");
}

/// The empty case the contract still guarantees: present, `[]`, never null.
#[tokio::test]
async fn diagnostics_is_empty_for_a_journal_with_nothing_to_report() {
    let journal = ledgeline_core::parse_journal(
        "2024-01-01 t\n    expenses:x   $1.00\n    assets:bank\n",
        "/tmp/diagnostics-clean.journal",
    )
    .expect("journal parses");
    let request = Request::builder()
        .method("GET")
        .uri("/api/diagnostics")
        .body(Body::empty())
        .expect("request builds");
    let response = app(&journal)
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("body is JSON");
    assert_eq!(body, serde_json::json!({"diagnostics": []}));
}

/// The same route over a journal that IS broken, proving the payload carries the
/// contract's four fields and that a broken journal still serves everything else.
#[tokio::test]
async fn diagnostics_reports_a_broken_journal_without_refusing_to_serve_it() {
    let journal = ledgeline_core::parse_journal(
        "2024-01-01 unbalanced\n    a   $1.00\n    b   $-2.00\n",
        "/tmp/diagnostics-endpoint.journal",
    )
    .expect("an unbalanced transaction is a diagnostic, never a parse error");
    let router = app(&journal);

    let request = Request::builder()
        .method("GET")
        .uri("/api/diagnostics")
        .body(Body::empty())
        .expect("request builds");
    let response = router.oneshot(request).await.expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("body is JSON");

    let diagnostics = body["diagnostics"]
        .as_array()
        .expect("diagnostics is an array");
    assert_eq!(diagnostics.len(), 1, "{body}");
    assert_eq!(diagnostics[0]["txnIndex"], 0);
    assert_eq!(diagnostics[0]["rule"], "unbalanced");
    assert_eq!(diagnostics[0]["severity"], "error");
    assert_eq!(
        diagnostics[0]["message"],
        "This transaction is unbalanced.\n\
         The real postings' sum should be 0 but is: $-1.00"
    );
}

// ---------------------------------------------------------------------------
// `GET /api/journal` — which journal am I looking at
// ---------------------------------------------------------------------------

/// `GET /api/journal` over a journal written to `<temp>/<folder>/<file>`.
///
/// The journal file goes in a subdirectory WE name rather than straight into the
/// `TempDir`, whose own name is random — the folder-name fallback is half of
/// what these tests assert, so the folder has to be something we can name.
async fn journal_info_for(folder: &str, file: &str, text: &str) -> Value {
    let root = tempfile::TempDir::new().expect("temp dir");
    let dir = root.path().join(folder);
    std::fs::create_dir(&dir).expect("journal directory");
    let path = dir.join(file);
    std::fs::write(&path, text).expect("journal written");
    let journal =
        ledgeline_core::parse_journal(text, &path.to_string_lossy()).expect("journal parses");

    let request = Request::builder()
        .method("GET")
        .uri("/api/journal")
        .body(Body::empty())
        .expect("request builds");
    let response = app(&journal)
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("body is JSON")
}

/// The whole point of the route: a journal that names itself in its first line
/// is called what it says it is called.
#[tokio::test]
async fn journal_serves_the_leading_comment_as_the_title() {
    let body = journal_info_for(
        "acme",
        "2026.journal",
        "; Acme Books\n\n2026-01-01 t\n    expenses:x   $1.00\n    assets:bank\n",
    )
    .await;
    assert_eq!(body, json!({"title": "Acme Books", "file": "2026.journal"}));
}

/// A journal that says nothing about itself is named for the folder it lives in.
/// `file` is the BARE name in both cases — never the path we just wrote to.
#[tokio::test]
async fn journal_falls_back_to_the_containing_folders_name() {
    let body = journal_info_for(
        "household-books",
        "main.journal",
        "2026-01-01 t\n    expenses:x   $1.00\n    assets:bank\n",
    )
    .await;
    assert_eq!(
        body,
        json!({"title": "household-books", "file": "main.journal"})
    );
}

/// The rejection path, end to end and over the real fixture:
/// `fixtures/sample.journal` opens with an eight-word sentence describing the
/// file. That is a description, not a name, so the folder answers instead.
#[tokio::test]
async fn journal_rejects_a_leading_comment_that_is_a_description() {
    let body = body_of("/api/journal").await;
    assert_eq!(
        body,
        json!({"title": "fixtures", "file": "sample.journal"}),
        "sample.journal's header is prose about the file and must not become a title"
    );
}

/// SEC-1: the server is same-origin ONLY by default. A cross-origin `Origin`
/// gets no `access-control-allow-origin`, so a browser refuses to hand the
/// response to the page that asked for it.
#[tokio::test]
async fn get_carries_no_cors_header_by_default() {
    let (status, allow_origin, _) = get("/version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        allow_origin, None,
        "the default path must install no CORS layer at all — a wildcard \
         access-control-allow-origin is what let any website read the journal"
    );
}

// ---------------------------------------------------------------------------
// Local-access controls (SEC-1, SEC-7, SEC-9)
//
// These drive `router_with_security`, the constructor a socket-bound server
// uses, rather than the unauthenticated `app` the parity tests above use.
// ---------------------------------------------------------------------------

const TEST_PORT: u16 = 5099;
const GOOD_HOST: &str = "127.0.0.1:5099";

fn test_token() -> AccessToken {
    AccessToken::parse("integration-test-token").expect("well-formed token")
}

fn secured_router(security: Security) -> axum::Router {
    router_with_security(AppState::from_journal(&fixture_journal()), security)
}

/// One request against a freshly built secured router. `headers` are applied
/// verbatim so a test can omit `Host` or `Authorization` entirely.
async fn probe(
    security: Security,
    method: &str,
    uri: &str,
    headers: &[(HeaderName, &str)],
) -> (StatusCode, Vec<(String, String)>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(name, *value);
    }
    let request = builder.body(Body::empty()).expect("request builds");
    let response = secured_router(security)
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let observed = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect();
    (status, observed)
}

fn header_of<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn local_security() -> Security {
    Security::local(test_token(), TEST_PORT)
}

/// Every wire and `/api` route demands the token: absent is a 401, wrong is a
/// 401, right is a 200. This is what makes an ephemeral port irrelevant.
#[tokio::test]
async fn wire_and_api_routes_require_the_token() {
    for uri in [
        "/version",
        "/transactions",
        "/accountnames",
        "/prices",
        "/commodities",
        "/accounts",
        "/api/reports/balancesheet",
        "/api/holdings",
        "/api/diagnostics",
        "/api/journal",
    ] {
        let (missing, missing_headers) =
            probe(local_security(), "GET", uri, &[(header::HOST, GOOD_HOST)]).await;
        assert_eq!(
            missing,
            StatusCode::UNAUTHORIZED,
            "GET {uri} without a token must be 401"
        );
        assert_eq!(
            header_of(&missing_headers, "www-authenticate"),
            Some("Bearer"),
            "a 401 must say how to authenticate"
        );

        let (wrong, _) = probe(
            local_security(),
            "GET",
            uri,
            &[
                (header::HOST, GOOD_HOST),
                (header::AUTHORIZATION, "Bearer integration-test-tokeN"),
            ],
        )
        .await;
        assert_eq!(
            wrong,
            StatusCode::UNAUTHORIZED,
            "GET {uri} with a wrong token must be 401"
        );

        let (right, _) = probe(
            local_security(),
            "GET",
            uri,
            &[
                (header::HOST, GOOD_HOST),
                (header::AUTHORIZATION, "Bearer integration-test-token"),
            ],
        )
        .await;
        assert_eq!(
            right,
            StatusCode::OK,
            "GET {uri} with the token must be 200"
        );
    }
}

/// The write path is gated too — SEC-1's proof-of-concept was a cross-origin
/// `POST`/`DELETE` on `/api/transactions`.
#[tokio::test]
async fn write_routes_require_the_token() {
    for (method, uri) in [
        ("POST", "/api/transactions"),
        ("DELETE", "/api/transactions/1"),
        ("PUT", "/api/transactions/1"),
        ("PATCH", "/api/transactions/1"),
    ] {
        let (status, _) = probe(
            local_security(),
            method,
            uri,
            &[
                (header::HOST, GOOD_HOST),
                (header::ORIGIN, "https://evil.example"),
            ],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} without a token must be 401"
        );
    }
}

/// SEC-1 item 1: the anti-DNS-rebinding control. A rebound name reaches the
/// socket but never the handler — even with a valid token, and even for the SPA
/// shell that carries the token.
#[tokio::test]
async fn host_guard_rejects_anything_but_loopback_on_our_port() {
    for host in [
        "attacker.example.com",
        "attacker.example.com:5099",
        "127.0.0.1:5000",
        "192.168.1.9:5099",
    ] {
        let (status, _) = probe(
            local_security(),
            "GET",
            "/version",
            &[
                (header::HOST, host),
                (header::AUTHORIZATION, "Bearer integration-test-token"),
            ],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Host: {host} must be refused"
        );
    }

    // The shell is served without a token, so the Host guard is the only thing
    // standing between a rebound page and the token printed inside it.
    let (shell, _) = probe(
        local_security(),
        "GET",
        "/",
        &[(header::HOST, "attacker.example.com")],
    )
    .await;
    assert_eq!(shell, StatusCode::FORBIDDEN);

    for host in ["127.0.0.1:5099", "localhost:5099", "LOCALHOST:5099"] {
        let (status, _) = probe(
            local_security(),
            "GET",
            "/version",
            &[
                (header::HOST, host),
                (header::AUTHORIZATION, "Bearer integration-test-token"),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "Host: {host} must be accepted");
    }
}

/// A request with no `Host` at all cannot be attributed to us, so it is refused.
#[tokio::test]
async fn host_guard_rejects_a_missing_host() {
    let (status, _) = probe(
        local_security(),
        "GET",
        "/version",
        &[(header::AUTHORIZATION, "Bearer integration-test-token")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// The SPA shell and its assets must stay reachable WITHOUT a token — the
/// browser has nothing to present until it has loaded the page.
#[tokio::test]
async fn the_spa_shell_is_served_without_a_token_and_publishes_it() {
    let request = Request::builder()
        .method("GET")
        .uri("/")
        .header(header::HOST, GOOD_HOST)
        .body(Body::empty())
        .expect("request builds");
    let response = secured_router(local_security())
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    let csp = response
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .expect("the shell carries a CSP");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("window.__LEDGELINE_TOKEN__=\"integration-test-token\""),
        "the shell must hand the same-origin SPA its token"
    );
    // SEC-7: the shell's own policy must hash the inline scripts it just served,
    // or the browser silently refuses to boot the SPA.
    assert!(csp.contains("script-src 'self' 'sha256-"), "CSP: {csp}");
    assert!(csp.contains("connect-src 'self'"), "CSP: {csp}");
}

/// SEC-1 item 2: `--allow-origin` echoes the EXACT origin, never a wildcard, and
/// only for origins on the list.
#[tokio::test]
async fn allowlisted_origins_get_an_exact_allow_origin_header() {
    let security = local_security()
        .allow_origins(&["http://localhost:4173"])
        .expect("valid origin");
    let (status, headers) = probe(
        security,
        "GET",
        "/version",
        &[
            (header::HOST, GOOD_HOST),
            (header::ORIGIN, "http://localhost:4173"),
            (header::AUTHORIZATION, "Bearer integration-test-token"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_of(&headers, "access-control-allow-origin"),
        Some("http://localhost:4173"),
        "an allowlisted origin is echoed verbatim"
    );
    assert_eq!(
        header_of(&headers, "access-control-allow-credentials"),
        None,
        "the token rides in Authorization, so credentials are never allowed"
    );

    let security = local_security()
        .allow_origins(&["http://localhost:4173"])
        .expect("valid origin");
    let (_, headers) = probe(
        security,
        "GET",
        "/version",
        &[
            (header::HOST, GOOD_HOST),
            (header::ORIGIN, "https://evil.example"),
            (header::AUTHORIZATION, "Bearer integration-test-token"),
        ],
    )
    .await;
    assert_eq!(
        header_of(&headers, "access-control-allow-origin"),
        None,
        "an origin off the list gets nothing"
    );
}

/// SEC-7: every response carries the hardening headers, including error ones.
#[tokio::test]
async fn security_headers_are_on_every_response() {
    for (uri, auth) in [
        ("/", None),
        ("/version", Some("Bearer integration-test-token")),
        ("/version", None), // the 401
    ] {
        let mut headers = vec![(header::HOST, GOOD_HOST)];
        if let Some(auth) = auth {
            headers.push((header::AUTHORIZATION, auth));
        }
        let (_, observed) = probe(local_security(), "GET", uri, &headers).await;
        assert_eq!(
            header_of(&observed, "x-content-type-options"),
            Some("nosniff"),
            "GET {uri}"
        );
        assert_eq!(
            header_of(&observed, "referrer-policy"),
            Some("no-referrer"),
            "GET {uri}"
        );
        let csp = header_of(&observed, "content-security-policy").unwrap_or_default();
        assert!(csp.contains("connect-src 'self'"), "GET {uri} CSP: {csp}");
        assert!(
            csp.contains("frame-ancestors 'none'"),
            "GET {uri} CSP: {csp}"
        );
        assert!(csp.contains("object-src 'none'"), "GET {uri} CSP: {csp}");
    }
}

/// SEC-2 item 1: a panicking handler must still produce an HTTP response.
///
/// `?count=0` is the empty-bucket panic from CLEANUP.md; another agent is fixing
/// the panic itself, so this asserts only what the `CatchPanicLayer` guarantees —
/// that *some* response comes back rather than the connection being dropped (or,
/// here, the test task unwinding).
#[tokio::test]
async fn a_panicking_handler_still_answers() {
    let (status, headers) = probe(
        local_security(),
        "GET",
        "/api/budget?count=0",
        &[
            (header::HOST, GOOD_HOST),
            (header::AUTHORIZATION, "Bearer integration-test-token"),
        ],
    )
    .await;
    assert!(
        status.is_server_error() || status.is_client_error(),
        "a panic must surface as a status, not a dropped connection (got {status})"
    );
    // Layer order: the header layers are OUTSIDE the panic catcher, so even a
    // synthesised 500 is hardened.
    assert_eq!(
        header_of(&headers, "x-content-type-options"),
        Some("nosniff"),
        "a caught panic's response must still carry the security headers"
    );
}
