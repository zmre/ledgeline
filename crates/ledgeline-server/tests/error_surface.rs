//! The `/api/*` ERROR surface, pinned: status + `Content-Type` + exact body.
//!
//! # Why this exists
//!
//! The success bodies of the native wire are byte-pinned by
//! `native_wire_golden.rs` against `fixtures/native/v1/`. The ERROR bodies had
//! no equivalent, and they are just as much a contract:
//!
//! `web/src/lib/api/native.ts` (`mutate`) reads the write path's error body with
//! `response.text()` and hands the **verbatim plain text** to
//! `ValidationError` / `NotFoundError` / `ConflictError` /
//! `NativeApiUnavailableError`, whose `.message` the setup modal and the edit
//! popup show to the user. So for these responses:
//!
//!   * the STATUS selects which error class the SPA throws,
//!   * the `Content-Type` must stay `text/plain` — a JSON body would be
//!     surfaced to the user as a raw `{"code":…}` blob, because nothing on the
//!     SPA side parses it,
//!   * the BODY TEXT *is* the user-facing sentence.
//!
//! Refactoring the Rust side's error plumbing (DRY-4: one `AppError` with an
//! `IntoResponse` impl, replacing two ad-hoc `type ApiError = (StatusCode,
//! String)` aliases) must therefore be invisible on the wire. This file is the
//! proof: it was written against the PRE-refactor server, passed unchanged
//! against the POST-refactor one, and fails loudly if anyone later changes a
//! status, a media type, or a word of the copy without meaning to.
//!
//! Every case below is reachable from the SPA with ordinary use.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{fixture_journal, fixture_journal_path};
use http_body_util::BodyExt;
use ledgeline_server::{AppState, app, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

/// What a client actually observes for a failed request.
#[derive(Debug, PartialEq, Eq)]
struct ErrorResponse {
    status: StatusCode,
    content_type: String,
    body: String,
}

/// Axum renders `(StatusCode, String)` — and so must anything that replaces it
/// — with exactly this media type. The SPA depends on it being text.
const PLAIN_TEXT: &str = "text/plain; charset=utf-8";

async fn send(router: axum::Router, method: &str, uri: &str, body: Option<Value>) -> ErrorResponse {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(value) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&value).expect("serialize")))
            .expect("request builds"),
        None => builder.body(Body::empty()).expect("request builds"),
    };
    let response = router.oneshot(request).await.expect("router responds");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("<absent>")
        .to_string();
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes()
            .to_vec(),
    )
    .expect("body is UTF-8");
    ErrorResponse {
        status,
        content_type,
        body,
    }
}

/// A read-only router (no editor bound) over `fixtures/sample.journal`.
async fn read_only(method: &str, uri: &str, body: Option<Value>) -> ErrorResponse {
    send(app(&fixture_journal()), method, uri, body).await
}

/// An EDITING-ENABLED router over a temp copy of the sample journal, so the
/// write path reaches the editor instead of short-circuiting on "not enabled".
fn editing_state() -> AppState {
    let dir = std::env::temp_dir().join("ledgeline-error-surface-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("errors-{}.journal", std::process::id()));
    std::fs::copy(fixture_journal_path(), &path).expect("copy sample journal");
    AppState::from_journal_path(&path).expect("editor opens")
}

async fn editing(method: &str, uri: &str, body: Option<Value>) -> ErrorResponse {
    send(router_with_state(editing_state()), method, uri, body).await
}

fn expect(status: StatusCode, body: &str) -> ErrorResponse {
    ErrorResponse {
        status,
        content_type: PLAIN_TEXT.to_string(),
        body: body.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Read path — query-parameter validation (RPT-4 / SEC-2 / HOLD-3)
// ---------------------------------------------------------------------------

/// A date that is SHAPED like a date but is not one. Must name the field, echo
/// the value, and say why — the message is the whole diagnostic.
#[tokio::test]
async fn bad_date_is_a_400_naming_the_field_and_the_reason() {
    assert_eq!(
        read_only("GET", "/api/reports/balancesheet?asOf=2026-02-30", None).await,
        expect(
            StatusCode::BAD_REQUEST,
            "invalid asOf date '2026-02-30': month 02 of 2026 has no day 30 (it has 28)",
        )
    );
}

/// An explicit EMPTY date used to sort below every real date and serve an empty
/// report with a `200`.
#[tokio::test]
async fn empty_date_is_a_400_not_an_empty_report() {
    assert_eq!(
        read_only("GET", "/api/reports/balancesheet?asOf=", None).await,
        expect(
            StatusCode::BAD_REQUEST,
            "invalid asOf date '': expected YYYY-MM-DD (a four-digit year, then month and day, \
             separated by `-`, `/` or `.`)",
        )
    );
}

#[tokio::test]
async fn bad_interval_is_a_400_listing_the_accepted_values() {
    assert_eq!(
        read_only(
            "GET",
            "/api/reports/cashflow?end=2026-06-30&interval=hourly",
            None
        )
        .await,
        expect(
            StatusCode::BAD_REQUEST,
            "unknown interval 'hourly' (expected daily|weekly|monthly|quarterly|yearly)",
        )
    );
}

/// `count=0` reached `Vec::with_capacity` and aborted the request with a
/// `capacity overflow` panic (SEC-2). It rejects rather than clamps.
#[tokio::test]
async fn bad_count_is_a_400_stating_the_accepted_range() {
    assert_eq!(
        read_only("GET", "/api/reports/cashflow?end=2026-06-30&count=0", None).await,
        expect(
            StatusCode::BAD_REQUEST,
            "count 0 is out of range (expected 1..=1200)",
        )
    );
}

/// A `count` serde itself refuses (negative) is a 400 from the extractor, with
/// a differently-worded body. Pinned because it is the same user-visible class.
#[tokio::test]
async fn negative_count_is_rejected_by_the_extractor() {
    let response = read_only("GET", "/api/reports/cashflow?end=2026-06-30&count=-1", None).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.content_type, PLAIN_TEXT);
    assert!(
        response.body.contains("invalid digit"),
        "unexpected body: {}",
        response.body
    );
}

/// HOLD-3: a `valueIn` that prices nothing in scope would otherwise answer with
/// an all-zero portfolio and one `unpriced` warning per row.
#[tokio::test]
async fn unpriceable_value_in_is_a_400_explaining_the_missing_route() {
    assert_eq!(
        read_only("GET", "/api/holdings?asOf=2026-06-30&valueIn=ZZZ", None).await,
        expect(
            StatusCode::BAD_REQUEST,
            "cannot value these holdings in 'ZZZ': no price directive or cost annotation connects \
             any holding in scope to it",
        )
    );
}

#[tokio::test]
async fn bad_holdings_mode_is_a_400() {
    assert_eq!(
        read_only("GET", "/api/holdings?asOf=2026-06-30&mode=sideways", None).await,
        expect(
            StatusCode::BAD_REQUEST,
            "unknown mode 'sideways' (expected include|exclude)",
        )
    );
}

/// `changeMin` decides which rows appear in the "biggest change" boxes, so an
/// unreadable value is refused rather than silently replaced by the $10 default.
#[tokio::test]
async fn bad_change_min_is_a_400_not_a_silent_default() {
    let response = read_only("GET", "/api/insights?end=2026-06-30&changeMin=%2410", None).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.content_type, PLAIN_TEXT);
    assert!(
        response.body.starts_with("invalid changeMin '$10': "),
        "unexpected body: {}",
        response.body
    );
}

// ---------------------------------------------------------------------------
// Write path — the bodies the SPA shows the user verbatim
// ---------------------------------------------------------------------------

/// A state with no backing file answers `501`, which the SPA maps to
/// `NativeApiUnavailableError` and the setup modal surfaces.
#[tokio::test]
async fn editing_disabled_is_a_501_with_the_reason() {
    let body = json!({
        "date": "2026-01-01",
        "postings": [{"account": "expenses:a", "amount": {"commodity": "$", "quantity": {"mantissa": "100", "places": 2}}},
                     {"account": "assets:bank"}],
    });
    assert_eq!(
        read_only("POST", "/api/transactions", Some(body)).await,
        expect(
            StatusCode::NOT_IMPLEMENTED,
            "editing is not enabled: this server was started without a journal file bound to an \
             editor",
        )
    );
}

/// DL-2: an unrecognized posting `type` must be a 400, never a silent fallback
/// to `regular` (which rewrote `[balanced-virtual]` postings as real ones).
#[tokio::test]
async fn unknown_posting_type_is_a_400_not_a_silent_regular() {
    let body = json!({
        "date": "2026-01-01",
        "postings": [
            {"account": "expenses:a", "type": "bogus",
             "amount": {"commodity": "$", "quantity": {"mantissa": "100", "places": 2}}},
            {"account": "assets:bank"},
        ],
    });
    let response = editing("POST", "/api/transactions", Some(body)).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.content_type, PLAIN_TEXT);
    assert!(
        response.body.starts_with("invalid request body: ")
            && response.body.contains("unknown variant `bogus`"),
        "unexpected body: {}",
        response.body
    );
}

/// A bare asserted number re-reads as whatever commodity the journal defaults
/// to, so a commodity-less assertion is refused at the boundary (DL-2).
#[tokio::test]
async fn malformed_balance_assertion_is_a_400_explaining_the_risk() {
    let body = json!({
        "date": "2026-01-01",
        "postings": [
            {"account": "assets:bank",
             "amount": {"commodity": "$", "quantity": {"mantissa": "100", "places": 2}},
             "balanceAssertion": {"amount": {"commodity": "  ", "quantity": {"mantissa": "9900", "places": 2}}}},
            {"account": "expenses:a"},
        ],
    });
    assert_eq!(
        editing("POST", "/api/transactions", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "the balance assertion on 'assets:bank' needs a commodity: a bare asserted number \
             would re-read as whatever commodity the journal defaults to",
        )
    );
}

/// An assertion on the elided leg would be dropped by the formatter — the
/// silent loss DL-2 is about.
#[tokio::test]
async fn assertion_on_the_elided_leg_is_a_400() {
    let body = json!({
        "date": "2026-01-01",
        "postings": [
            {"account": "expenses:a",
             "amount": {"commodity": "$", "quantity": {"mantissa": "100", "places": 2}}},
            {"account": "assets:bank",
             "balanceAssertion": {"amount": {"commodity": "$", "quantity": {"mantissa": "9900", "places": 2}}}},
        ],
    });
    assert_eq!(
        editing("POST", "/api/transactions", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "the balance assertion on 'assets:bank' needs an amount on the same posting: an \
             assertion cannot be written on the inferred (elided) leg",
        )
    );
}

/// SEC-5: `places` beyond what the PARSER stores would commit an amount that
/// re-reading the journal could never reproduce.
#[tokio::test]
async fn out_of_range_places_is_a_400() {
    let body = json!({
        "date": "2026-01-01",
        "postings": [
            {"account": "expenses:a",
             "amount": {"commodity": "$", "quantity": {"mantissa": "0", "places": 65534}}},
            {"account": "assets:bank"},
        ],
    });
    let response = editing("POST", "/api/transactions", Some(body)).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.content_type, PLAIN_TEXT);
    assert!(
        response
            .body
            .starts_with("amount places 65534 is out of range"),
        "unexpected body: {}",
        response.body
    );
}

#[tokio::test]
async fn non_integer_mantissa_is_a_400() {
    let body = json!({
        "date": "2026-01-01",
        "postings": [
            {"account": "expenses:a",
             "amount": {"commodity": "$", "quantity": {"mantissa": "1.5", "places": 2}}},
            {"account": "assets:bank"},
        ],
    });
    assert_eq!(
        editing("POST", "/api/transactions", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "invalid amount mantissa '1.5': expected a base-10 integer string",
        )
    );
}

#[tokio::test]
async fn a_transaction_with_no_postings_is_a_400() {
    let body = json!({"date": "2026-01-01", "postings": []});
    assert_eq!(
        editing("POST", "/api/transactions", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "a transaction needs at least one posting",
        )
    );
}

/// The SPA maps this status to `NotFoundError` ("that transaction no longer
/// exists — refresh the journal").
#[tokio::test]
async fn deleting_a_missing_transaction_is_a_404() {
    assert_eq!(
        editing("DELETE", "/api/transactions/99999", None).await,
        expect(
            StatusCode::NOT_FOUND,
            "transaction #99999 not found in the journal",
        )
    );
}

#[tokio::test]
async fn patching_a_missing_transaction_is_a_404() {
    let body = json!({"description": "nope"});
    assert_eq!(
        editing("PATCH", "/api/transactions/99999", Some(body)).await,
        expect(
            StatusCode::NOT_FOUND,
            "transaction #99999 not found in the journal",
        )
    );
}

/// An unbalanced replace is rejected with the editor's own sentence, which the
/// edit popup shows.
#[tokio::test]
async fn an_unbalanced_transaction_is_a_400_from_the_editor() {
    let body = json!({
        "date": "2026-01-01",
        "postings": [
            {"account": "expenses:a", "amount": {"commodity": "$", "quantity": {"mantissa": "100", "places": 2}}},
            {"account": "assets:bank", "amount": {"commodity": "$", "quantity": {"mantissa": "-999", "places": 2}}},
        ],
    });
    assert_eq!(
        editing("POST", "/api/transactions", Some(body)).await,
        expect(StatusCode::BAD_REQUEST, "the transaction does not balance")
    );
}

/// A malformed JSON body reaches the `JsonRejection` arm, which all three
/// body-taking handlers render identically.
#[tokio::test]
async fn a_malformed_json_body_is_a_400_on_every_body_taking_route() {
    for (method, uri) in [
        ("POST", "/api/transactions"),
        ("PUT", "/api/transactions/1"),
        ("PATCH", "/api/transactions/1"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{not json"))
            .expect("request builds");
        let response = router_with_state(editing_state())
            .oneshot(request)
            .await
            .expect("router responds");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("<absent>")
            .to_string();
        let body = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .expect("collects")
                .to_bytes()
                .to_vec(),
        )
        .expect("UTF-8");
        assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {uri}");
        assert_eq!(content_type, PLAIN_TEXT, "{method} {uri}");
        assert!(
            body.starts_with("invalid request body: "),
            "{method} {uri}: unexpected body: {body}"
        );
    }
}
