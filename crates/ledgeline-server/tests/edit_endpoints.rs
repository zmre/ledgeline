//! End-to-end HTTP tests for the native WRITE endpoints (`POST`/`DELETE
//! /api/transactions`, Phase 5.2).
//!
//! Each test drives the real axum `Router` through `tower`'s `oneshot` over an
//! editing-enabled [`AppState`] bound to a TEMP COPY of a journal, then asserts
//! all three of: the HTTP status/body, that `GET /transactions` (the snapshot)
//! reflects the change, and that the file ON DISK changed correctly. The editor
//! itself is unit-tested in `ledgeline-core`'s `tests/edit.rs`; these tests pin
//! the HTTP contract, the amount-style inference, and the `EditError` → HTTP map.

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline_server::{AppState, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

const THREE_TXNS: &str = "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank

2024-01-02 * B
    expenses:b  $2.00
    assets:bank

2024-01-03 * C
    expenses:c  $3.00
    assets:bank
";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Write `content` to a unique temp file and return its path.
fn temp_journal(content: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("ledgeline-edit-endpoint-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("edit-{}-{seq}.journal", std::process::id()));
    std::fs::write(&path, content).expect("write temp journal");
    path
}

/// Editing-enabled state bound to a fresh temp copy of `content` (returns the
/// state and the temp path so tests can read the file back).
fn state_for(content: &str) -> (AppState, PathBuf) {
    let path = temp_journal(content);
    let state = AppState::from_journal_path(&path).expect("editor opens");
    (state, path)
}

/// Issue one request against a fresh router over `state` (its editor + snapshot
/// are shared across clones, so edits persist between calls). A `Some(body)` is
/// sent as a JSON request body.
async fn request(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, "https://spa.example");
    let request = match body {
        Some(json) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&json).expect("serialize body"),
            ))
            .expect("request builds"),
        None => builder.body(Body::empty()).expect("request builds"),
    };
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// The number of transactions the snapshot currently serves at `GET /transactions`.
async fn transaction_count(state: &AppState) -> usize {
    let (status, body) = request(state, "GET", "/transactions", None).await;
    assert_eq!(status, StatusCode::OK);
    body.as_array().expect("transactions array").len()
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_removes_transaction_from_snapshot_and_file() {
    let (state, path) = state_for(THREE_TXNS);
    assert_eq!(transaction_count(&state).await, 3);

    let (status, body) = request(&state, "DELETE", "/api/transactions/2", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deletedIndex"], 2);
    assert_eq!(body["remaining"], 2);

    // Snapshot reflects the delete...
    assert_eq!(transaction_count(&state).await, 2);
    // ...and so does the file on disk (B is gone, A and C remain).
    let on_disk = std::fs::read_to_string(&path).expect("read journal");
    assert!(
        !on_disk.contains("* B"),
        "B was deleted from disk:\n{on_disk}"
    );
    assert!(on_disk.contains("* A") && on_disk.contains("* C"));
    // Re-parseable: exactly one blank line between the two survivors.
    assert_eq!(
        on_disk,
        "\
2024-01-01 * A
    expenses:a  $1.00
    assets:bank

2024-01-03 * C
    expenses:c  $3.00
    assets:bank
"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn delete_unknown_transaction_is_404() {
    let (state, path) = state_for(THREE_TXNS);
    let (status, _) = request(&state, "DELETE", "/api/transactions/99999", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Nothing changed on disk.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), THREE_TXNS);
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// Add
// ---------------------------------------------------------------------------

#[tokio::test]
async fn add_appends_transaction_to_snapshot_and_file() {
    let (state, path) = state_for(&sample_text());
    let before = transaction_count(&state).await;

    let body = json!({
        "date": "2026-07-20",
        "status": "cleared",
        "description": "Safeway | groceries",
        "postings": [
            { "account": "expenses:food:groceries",
              "amount": { "commodity": "$", "quantity": { "mantissa": "5624", "places": 2 } } },
            { "account": "liabilities:cc:visa" }
        ]
    });
    let (status, response) = request(&state, "POST", "/api/transactions", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "add should be 201: {response}");

    // The response carries the added transaction (native shape) + its index.
    assert!(response["index"].as_u64().is_some());
    assert_eq!(
        response["transaction"]["description"],
        "Safeway | groceries"
    );
    assert_eq!(response["transaction"]["status"], "cleared");
    assert_eq!(
        response["transaction"]["postings"][0]["amounts"][0]["quantity"]["mantissa"],
        "5624"
    );
    // The inferred (elided) leg came back filled in as -$56.24.
    assert_eq!(
        response["transaction"]["postings"][1]["amounts"][0]["quantity"]["mantissa"],
        "-5624"
    );

    // Snapshot grew by one...
    assert_eq!(transaction_count(&state).await, before + 1);
    // ...and the file on disk carries the new transaction with a left, unspaced $.
    let on_disk = std::fs::read_to_string(&path).expect("read journal");
    assert!(
        on_disk.contains("2026-07-20 * Safeway | groceries"),
        "{on_disk}"
    );
    assert!(
        on_disk.contains("expenses:food:groceries  $56.24"),
        "{on_disk}"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn add_infers_comma_decimal_eur_style_from_the_journal() {
    // The sample journal declares `commodity 1.000,00 EUR` (comma decimal, symbol
    // right, spaced). A naive '.'-decimal render of 100.00 would re-parse (under
    // EUR's canonical comma) as 10000 — a 100x corruption the editor's round-trip
    // guard would reject with a 400. A 201 here proves the style was inferred.
    let (state, path) = state_for(&sample_text());

    let body = json!({
        "date": "2026-07-21",
        "status": "cleared",
        "description": "Berlin cafe",
        "postings": [
            { "account": "expenses:food:restaurants",
              "amount": { "commodity": "EUR", "quantity": { "mantissa": "10000", "places": 2 } } },
            { "account": "assets:bank:wise:eur" }
        ]
    });
    let (status, response) = request(&state, "POST", "/api/transactions", Some(body)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "EUR add should be 201: {response}"
    );

    let on_disk = std::fs::read_to_string(&path).expect("read journal");
    // Comma decimal, symbol on the right, space before it — EUR's journal style.
    assert!(
        on_disk.contains("100,00 EUR"),
        "EUR rendered in journal style:\n{on_disk}"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn add_unbalanced_transaction_is_400_and_leaves_file_unchanged() {
    let (state, path) = state_for(THREE_TXNS);
    let before_file = std::fs::read_to_string(&path).unwrap();
    let before_count = transaction_count(&state).await;

    // Two explicit legs that do not sum to zero, no elided leg to absorb it.
    let body = json!({
        "date": "2024-06-01",
        "description": "bad",
        "postings": [
            { "account": "expenses:x",
              "amount": { "commodity": "$", "quantity": { "mantissa": "500", "places": 2 } } },
            { "account": "assets:bank",
              "amount": { "commodity": "$", "quantity": { "mantissa": "-400", "places": 2 } } }
        ]
    });
    let (status, _) = request(&state, "POST", "/api/transactions", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Neither the snapshot nor the file changed.
    assert_eq!(transaction_count(&state).await, before_count);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before_file);
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// External-change (409) + editing-disabled (501)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_change_yields_409_and_resyncs_snapshot() {
    let (state, path) = state_for(THREE_TXNS);
    assert_eq!(transaction_count(&state).await, 3);

    // Simulate a concurrent external edit that replaces the whole file.
    let external = "\
2099-01-01 * external edit
    expenses:x  $1.00
    assets:y
";
    std::fs::write(&path, external).unwrap();

    // A delete now finds Tindex(2) in the STALE in-memory journal, mutates it, then
    // `save` detects the content change and refuses → 409.
    let (status, _) = request(&state, "DELETE", "/api/transactions/2", None).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The editor re-synced to disk, so the snapshot now reflects the external file
    // (1 transaction) — the client should re-fetch and retry.
    assert_eq!(transaction_count(&state).await, 1);
    // The external content was NOT clobbered.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn edit_endpoints_are_501_when_no_editor_is_bound() {
    // State built from an already-parsed journal (no backing file) → editing off.
    let state = AppState::from_journal(&common::fixture_journal());

    let (status, _) = request(&state, "DELETE", "/api/transactions/1", None).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);

    let body = json!({
        "date": "2026-07-20",
        "description": "x",
        "postings": [
            { "account": "expenses:a",
              "amount": { "commodity": "$", "quantity": { "mantissa": "100", "places": 2 } } },
            { "account": "assets:bank" }
        ]
    });
    let (status, _) = request(&state, "POST", "/api/transactions", Some(body)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}

// ---------------------------------------------------------------------------
// PATCH (surgical partial edit) + PUT (full, in-place replace)
// ---------------------------------------------------------------------------

/// A ledger with header + posting comments, so surgical edits can be checked for
/// leaving the surrounding lines/comments byte-identical on disk.
const WITH_COMMENTS: &str = "\
2024-01-01 * A  ; first txn
    expenses:a  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
";

#[tokio::test]
async fn patch_description_changes_only_that_field_on_disk() {
    let (state, path) = state_for(WITH_COMMENTS);

    let body = json!({ "description": "A renamed" });
    let (status, response) = request(&state, "PATCH", "/api/transactions/1", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "patch should be 200: {response}");
    assert_eq!(response["transaction"]["description"], "A renamed");

    // Only the header's description changed: the header comment and BOTH posting
    // lines (accounts, amounts, comments, whitespace) are byte-identical.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        "\
2024-01-01 * A renamed  ; first txn
    expenses:a  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    // GET /transactions reflects the change.
    let (_, txns) = request(&state, "GET", "/transactions", None).await;
    assert!(
        txns.to_string().contains("A renamed"),
        "snapshot reflects the rename: {txns}"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn patch_posting_account_changes_only_the_account_on_disk() {
    let (state, path) = state_for(WITH_COMMENTS);

    let body = json!({ "postings": [ { "index": 0, "account": "expenses:groceries" } ] });
    let (status, response) = request(&state, "PATCH", "/api/transactions/1", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "patch should be 200: {response}");

    // Only "expenses:a" -> "expenses:groceries"; the amount, its gap, and the
    // trailing comment are preserved, and every other line is unchanged.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        "\
2024-01-01 * A  ; first txn
    expenses:groceries  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    let (_, txns) = request(&state, "GET", "/transactions", None).await;
    assert!(
        txns.to_string().contains("expenses:groceries"),
        "snapshot reflects the category change: {txns}"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn put_replaces_transaction_in_place_and_round_trips_comments() {
    let (state, path) = state_for(WITH_COMMENTS);

    let body = json!({
        "date": "2024-01-01",
        "status": "cleared",
        "description": "A replaced",
        "comment": "first txn",
        "postings": [
            { "account": "expenses:a",
              "amount": { "commodity": "$", "quantity": { "mantissa": "150", "places": 2 } },
              "comment": "the expense" },
            { "account": "assets:bank", "comment": "from checking" }
        ]
    });
    let (status, response) = request(&state, "PUT", "/api/transactions/1", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "put should be 200: {response}");
    assert_eq!(response["transaction"]["description"], "A replaced");

    // The whole transaction is rewritten in place (comments round-tripped) and
    // neighbor B is byte-identical.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        "\
2024-01-01 * A replaced  ; first txn
    expenses:a  $1.50  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn patch_invalid_description_is_400_and_file_unchanged() {
    let (state, path) = state_for(WITH_COMMENTS);
    let before = std::fs::read_to_string(&path).unwrap();

    // A ';' would parse as a comment, so the description cannot round-trip.
    let body = json!({ "description": "A ; sneaky" });
    let (status, _) = request(&state, "PATCH", "/api/transactions/1", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn patch_out_of_range_posting_is_404_and_file_unchanged() {
    let (state, path) = state_for(WITH_COMMENTS);
    let before = std::fs::read_to_string(&path).unwrap();

    let body = json!({ "postings": [ { "index": 9, "account": "assets:x" } ] });
    let (status, _) = request(&state, "PATCH", "/api/transactions/1", Some(body)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn put_unbalanced_is_400_and_file_unchanged() {
    let (state, path) = state_for(WITH_COMMENTS);
    let before = std::fs::read_to_string(&path).unwrap();

    let body = json!({
        "date": "2024-01-01",
        "description": "bad",
        "postings": [
            { "account": "expenses:a",
              "amount": { "commodity": "$", "quantity": { "mantissa": "500", "places": 2 } } },
            { "account": "assets:bank",
              "amount": { "commodity": "$", "quantity": { "mantissa": "-400", "places": 2 } } }
        ]
    });
    let (status, _) = request(&state, "PUT", "/api/transactions/1", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn put_and_patch_are_501_when_no_editor_is_bound() {
    let state = AppState::from_journal(&common::fixture_journal());

    let put_body = json!({
        "date": "2026-07-20",
        "description": "x",
        "postings": [
            { "account": "expenses:a",
              "amount": { "commodity": "$", "quantity": { "mantissa": "100", "places": 2 } } },
            { "account": "assets:bank" }
        ]
    });
    let (status, _) = request(&state, "PUT", "/api/transactions/1", Some(put_body)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);

    let patch_body = json!({ "description": "y" });
    let (status, _) = request(&state, "PATCH", "/api/transactions/1", Some(patch_body)).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn put_round_trips_date2_tag_comment_and_pending_posting() {
    let (state, path) = state_for(WITH_COMMENTS);

    // One PUT carrying all the newly-wired fields: a secondary date, a tag-bearing
    // transaction comment, and a per-posting `pending` status.
    let body = json!({
        "date": "2024-01-01",
        "date2": "2024-01-03",
        "status": "cleared",
        "description": "A",
        "comment": "category:food",
        "postings": [
            { "account": "expenses:a",
              "status": "pending",
              "amount": { "commodity": "$", "quantity": { "mantissa": "100", "places": 2 } } },
            { "account": "assets:bank" }
        ]
    });
    let (status, response) = request(&state, "PUT", "/api/transactions/1", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "put should be 200: {response}");
    // The response echoes the secondary date and posting status.
    assert_eq!(response["transaction"]["date2"], "2024-01-03");
    assert_eq!(response["transaction"]["postings"][0]["status"], "pending");

    // On disk: `DATE=DATE2`, the `; …tag…` comment, and the posting `!` marker;
    // neighbor B stays byte-identical.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        "\
2024-01-01=2024-01-03 * A  ; category:food
    ! expenses:a  $1.00
    assets:bank

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );

    // GET /transactions shows the secondary date (`tdate2`) and the parsed tag.
    let (_, txns) = request(&state, "GET", "/transactions", None).await;
    let txn = txns
        .as_array()
        .expect("transactions array")
        .iter()
        .find(|t| t["tdate2"] == "2024-01-03")
        .expect("the edited transaction with a secondary date");
    assert_eq!(txn["ttags"], json!([["category", "food"]]));
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn patch_status_changes_only_the_header_marker_on_disk() {
    let (state, path) = state_for(WITH_COMMENTS);

    // Flip transaction 1 from cleared (`*`) to pending (`!`); everything else —
    // the header comment and both posting lines — must stay byte-identical.
    let body = json!({ "status": "pending" });
    let (status, response) = request(&state, "PATCH", "/api/transactions/1", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "patch should be 200: {response}");
    assert_eq!(response["transaction"]["status"], "pending");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        "\
2024-01-01 ! A  ; first txn
    expenses:a  $1.00  ; the expense
    assets:bank  ; from checking

2024-01-02 * B
    expenses:b  $2.00
    assets:bank
"
    );
    let _ = std::fs::remove_file(&path);
}

/// The sample fixture's text, copied into each temp journal under test.
fn sample_text() -> String {
    std::fs::read_to_string(common::fixture_journal_path()).expect("sample.journal readable")
}
