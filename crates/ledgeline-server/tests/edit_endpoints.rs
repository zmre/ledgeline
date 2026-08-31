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
use ledgeline::{AppState, router_with_state};
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

// ---------------------------------------------------------------------------
// SEC-2 / SEC-5 — amount validation on the edit wire
// ---------------------------------------------------------------------------

/// An add whose first posting carries `mantissa`/`places` verbatim.
fn amount_body(date: &str, mantissa: &str, places: u64) -> Value {
    json!({
        "date": date,
        "description": "places probe",
        "postings": [
            { "account": "expenses:food:groceries",
              "amount": { "commodity": "$",
                          "quantity": { "mantissa": mantissa, "places": places } } },
            { "account": "liabilities:cc:visa" }
        ]
    })
}

/// SEC-5: `places` above what the PARSER stores is rejected at the wire with a
/// clear message, and — the part the round-trip guard never caught — nothing is
/// written to the user's journal.
///
/// `{"mantissa":"0","places":65534}` used to return `201 Created` and commit a
/// multi-hundred-byte all-zeros amount line to the file. The value genuinely
/// round-trips, so the reparse and round-trip guards both passed: they check
/// semantics, not sanity.
#[tokio::test]
async fn oversized_places_is_rejected_and_writes_nothing() {
    let (state, path) = state_for(&sample_text());
    let before_bytes = std::fs::metadata(&path).expect("stat").len();
    let before_count = transaction_count(&state).await;

    // A zero mantissa is the case that used to slip through every guard.
    for places in [11, 255, 256, 65534, 65535, 4_294_967_295] {
        let (status, body) = request(
            &state,
            "POST",
            "/api/transactions",
            Some(amount_body("2026-07-25", "0", places)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "places={places} must be a 400: {body}"
        );
        assert_eq!(
            std::fs::metadata(&path).expect("stat").len(),
            before_bytes,
            "places={places} must not have written to the journal"
        );
    }
    assert_eq!(transaction_count(&state).await, before_count);
    let _ = std::fs::remove_file(&path);
}

/// The documented maximum (`MAX_PARSE_PLACES` = 10) is still ACCEPTED — the
/// validation rejects only what the parser could never have produced.
#[tokio::test]
async fn places_at_the_documented_max_is_accepted() {
    let (state, path) = state_for(&sample_text());

    let (status, body) = request(
        &state,
        "POST",
        "/api/transactions",
        Some(amount_body("2026-07-25", "1", 10)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "places=10 must be a 201: {body}"
    );
    assert_eq!(
        body["transaction"]["postings"][0]["amounts"][0]["quantity"]["places"],
        10
    );

    // …and one more place is the boundary that fails.
    let (status, _) = request(
        &state,
        "POST",
        "/api/transactions",
        Some(amount_body("2026-07-26", "1", 11)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "places=11 must be a 400");
    let _ = std::fs::remove_file(&path);
}

/// SEC-5's second bound: an absurd mantissa is refused too, and an ordinary one
/// is not. `10^30` is the documented limit.
#[tokio::test]
async fn oversized_mantissa_is_rejected() {
    let (state, path) = state_for(&sample_text());

    // 10^30 + 1, and the largest i128 — both beyond the accepted magnitude.
    for mantissa in [
        "1000000000000000000000000000001",
        "170141183460469231731687303715884105727",
    ] {
        let (status, body) = request(
            &state,
            "POST",
            "/api/transactions",
            Some(amount_body("2026-07-25", mantissa, 2)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "mantissa={mantissa} must be a 400: {body}"
        );
    }

    // A mantissa past i128 entirely is still the pre-existing 400.
    let (status, _) = request(
        &state,
        "POST",
        "/api/transactions",
        Some(amount_body("2026-07-25", &"9".repeat(60), 2)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // An ordinary amount is unaffected.
    let (status, body) = request(
        &state,
        "POST",
        "/api/transactions",
        Some(amount_body("2026-07-25", "5624", 2)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "an ordinary add still works: {body}"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// DL-2 — balance assertions and posting types survive a full replace
// ---------------------------------------------------------------------------

/// CLEANUP's DL-2 example, with an opening balance so the `= $99.00` anchor and
/// the `== $500.00` total anchor genuinely hold under hledger.
const ASSERTIONS_AND_VIRTUALS: &str = "\
2025-12-31 Opening balances
    assets:cash  $100.00
    assets:checking  $502.00
    equity:opening  $-602.00

2026-01-01 A
    expenses:a  $1.00
    assets:cash  $-1.00 = $99.00
    [budget:env]  $1.00
    [budget:avail]  $-1.00

2026-01-02 B
    expenses:b  $2.00
    assets:checking  $-2.00 == $500.00
    (tracking:note)  $7.00
";

/// DL-2, the whole finding in one test: a `PUT` that echoes back what the API
/// served must not destroy a balance assertion or silently promote a virtual
/// posting to a real one.
///
/// Before the fix this returned `200` and wrote four PLAIN postings: the
/// `= $99.00` reconciliation anchor was gone from the file forever, and the two
/// `[budget:…]` balanced-virtual envelope legs had become REAL postings —
/// moving $1.00 onto the balance sheet that was never there, in every report.
#[tokio::test]
async fn put_round_trips_balance_assertions_and_posting_types() {
    let (state, path) = state_for(ASSERTIONS_AND_VIRTUALS);

    // Transaction A: a plain regular posting, a regular posting carrying `=`,
    // and two balanced-virtual `[...]` legs.
    let body = json!({
        "date": "2026-01-01",
        "description": "A",
        "postings": [
            { "account": "expenses:a", "type": "regular",
              "amount": { "commodity": "$", "quantity": { "mantissa": "100", "places": 2 } } },
            { "account": "assets:cash", "type": "regular",
              "amount": { "commodity": "$", "quantity": { "mantissa": "-100", "places": 2 } },
              "balanceAssertion": {
                  "amount": { "commodity": "$", "quantity": { "mantissa": "9900", "places": 2 } },
                  "total": false, "inclusive": false } },
            { "account": "budget:env", "type": "balancedVirtual",
              "amount": { "commodity": "$", "quantity": { "mantissa": "100", "places": 2 } } },
            { "account": "budget:avail", "type": "balancedVirtual",
              "amount": { "commodity": "$", "quantity": { "mantissa": "-100", "places": 2 } } }
        ]
    });
    let (status, response) = request(&state, "PUT", "/api/transactions/2", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "put should be 200: {response}");

    // The response reports what actually landed, so a client can echo it again.
    let postings = &response["transaction"]["postings"];
    assert_eq!(postings[0]["type"], "regular");
    assert_eq!(postings[1]["type"], "regular");
    assert_eq!(postings[1]["balanceAssertion"]["amount"]["commodity"], "$");
    assert_eq!(
        postings[1]["balanceAssertion"]["amount"]["quantity"]["mantissa"],
        "9900"
    );
    assert_eq!(postings[1]["balanceAssertion"]["total"], false);
    assert_eq!(postings[2]["type"], "balancedVirtual");
    assert_eq!(postings[3]["type"], "balancedVirtual");

    // Transaction B: a `==` TOTAL assertion and an unbalanced `(virtual)` leg.
    let body = json!({
        "date": "2026-01-02",
        "description": "B",
        "postings": [
            { "account": "expenses:b", "type": "regular",
              "amount": { "commodity": "$", "quantity": { "mantissa": "200", "places": 2 } } },
            { "account": "assets:checking", "type": "regular",
              "amount": { "commodity": "$", "quantity": { "mantissa": "-200", "places": 2 } },
              "balanceAssertion": {
                  "amount": { "commodity": "$", "quantity": { "mantissa": "50000", "places": 2 } },
                  "total": true, "inclusive": false } },
            { "account": "tracking:note", "type": "virtual",
              "amount": { "commodity": "$", "quantity": { "mantissa": "700", "places": 2 } } }
        ]
    });
    let (status, response) = request(&state, "PUT", "/api/transactions/3", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "put should be 200: {response}");
    assert_eq!(response["transaction"]["postings"][2]["type"], "virtual");
    assert_eq!(
        response["transaction"]["postings"][1]["balanceAssertion"]["total"],
        true
    );

    // Both anchors and both bracket forms are still in the file.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        "\
2025-12-31 Opening balances
    assets:cash  $100.00
    assets:checking  $502.00
    equity:opening  $-602.00

2026-01-01 A
    expenses:a      $1.00
    assets:cash     $-1.00  = $99.00
    [budget:env]    $1.00
    [budget:avail]  $-1.00

2026-01-02 B
    expenses:b       $2.00
    assets:checking  $-2.00  == $500.00
    (tracking:note)  $7.00
"
    );
    let _ = std::fs::remove_file(&path);
}

/// The subaccount-inclusive operators (`=*` and `==*`) round-trip too — they are
/// the `inclusive` flag, orthogonal to `total`.
#[tokio::test]
async fn add_round_trips_subaccount_inclusive_assertions() {
    let (state, path) = state_for(THREE_TXNS);

    let body = json!({
        "date": "2024-01-04",
        "description": "D",
        "position": "append",
        "postings": [
            { "account": "expenses:d",
              "amount": { "commodity": "$", "quantity": { "mantissa": "400", "places": 2 } } },
            { "account": "assets:bank",
              "amount": { "commodity": "$", "quantity": { "mantissa": "-400", "places": 2 } },
              "balanceAssertion": {
                  "amount": { "commodity": "$", "quantity": { "mantissa": "1000", "places": 2 } },
                  "total": true, "inclusive": true } }
        ]
    });
    let (status, response) = request(&state, "POST", "/api/transactions", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "add should be 201: {response}");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        on_disk.contains("==* $10.00"),
        "the `==*` operator must survive; file was:\n{on_disk}"
    );
    let assertion = &response["transaction"]["postings"][1]["balanceAssertion"];
    assert_eq!(assertion["inclusive"], true);
    assert_eq!(assertion["total"], true);
    let _ = std::fs::remove_file(&path);
}

/// An unrecognized posting `type` is a `400` — never a silent fallback to
/// `regular`, which is precisely how DL-2 destroyed envelope postings. Matches
/// `reports_api::parse_interval`'s handling of an unknown enum value.
#[tokio::test]
async fn unknown_posting_type_is_400_and_file_unchanged() {
    let (state, path) = state_for(ASSERTIONS_AND_VIRTUALS);
    let before = std::fs::read_to_string(&path).unwrap();

    for bad in ["sneaky", "Regular", "RegularPosting", "balancedvirtual", ""] {
        let body = json!({
            "date": "2026-01-01",
            "description": "A",
            "postings": [
                { "account": "expenses:a", "type": bad,
                  "amount": { "commodity": "$", "quantity": { "mantissa": "100", "places": 2 } } },
                { "account": "assets:cash" }
            ]
        });
        let (status, _) = request(&state, "PUT", "/api/transactions/2", Some(body)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "type={bad:?} must be a 400, not a silent `regular`"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "type={bad:?} must not have written to the journal"
        );
    }
    let _ = std::fs::remove_file(&path);
}

/// A malformed balance assertion is a `400` before it can reach the core: a
/// blank commodity (which would render as a bare number and re-read as whatever
/// the journal defaults to), an assertion on the elided leg (which the writer
/// would drop on the floor), and the SEC-5 amount bounds applied to the asserted
/// amount as well as the posting amount.
#[tokio::test]
async fn malformed_balance_assertion_is_400_and_file_unchanged() {
    let (state, path) = state_for(ASSERTIONS_AND_VIRTUALS);
    let before = std::fs::read_to_string(&path).unwrap();

    let cash_with_assertion = |assertion: Value, with_amount: bool| {
        let mut posting = json!({ "account": "assets:cash", "balanceAssertion": assertion });
        if with_amount {
            posting["amount"] =
                json!({ "commodity": "$", "quantity": { "mantissa": "-100", "places": 2 } });
        }
        json!({
            "date": "2026-01-01",
            "description": "A",
            "postings": [
                { "account": "expenses:a",
                  "amount": { "commodity": "$", "quantity": { "mantissa": "100", "places": 2 } } },
                posting
            ]
        })
    };
    let dollars = json!({ "commodity": "$", "quantity": { "mantissa": "9900", "places": 2 } });

    let cases: Vec<(&str, Value)> = vec![
        (
            "blank commodity",
            cash_with_assertion(
                json!({ "amount": { "commodity": "  ", "quantity": { "mantissa": "9900", "places": 2 } } }),
                true,
            ),
        ),
        (
            "assertion on the elided leg",
            cash_with_assertion(json!({ "amount": dollars }), false),
        ),
        (
            "oversized asserted mantissa",
            cash_with_assertion(
                json!({ "amount": { "commodity": "$",
                                    "quantity": { "mantissa": "1000000000000000000000000000001", "places": 2 } } }),
                true,
            ),
        ),
        (
            "oversized asserted places",
            cash_with_assertion(
                json!({ "amount": { "commodity": "$", "quantity": { "mantissa": "9900", "places": 65534 } } }),
                true,
            ),
        ),
        (
            "non-numeric asserted mantissa",
            cash_with_assertion(
                json!({ "amount": { "commodity": "$", "quantity": { "mantissa": "9,900", "places": 2 } } }),
                true,
            ),
        ),
    ];

    for (name, body) in cases {
        let (status, message) = request(&state, "PUT", "/api/transactions/2", Some(body)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{name} must be a 400: {message}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "{name} must not have written to the journal"
        );
    }
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// DL-5 — a failed save must never publish the edit that failed
// ---------------------------------------------------------------------------

/// Something that parses as a journal at open time but not afterwards.
const UNPARSEABLE: &str = "this is not a journal ((((\n";

/// DL-5: when a save fails AND the journal cannot be re-read to re-sync, the
/// server must publish NOTHING and say so.
///
/// `resync_from_disk` used to swallow the re-open error and publish
/// `editor.journal()` regardless — the in-memory journal *still carrying the
/// edit that had just failed to write*. The user got a `409`, re-fetched, and
/// was served their change back as though it had been saved, while the file on
/// disk had never contained it and never would.
#[tokio::test]
async fn failed_save_with_unreadable_journal_publishes_nothing() {
    let (state, path) = state_for(THREE_TXNS);
    assert_eq!(transaction_count(&state).await, 3);

    // The file changes under us AND becomes unparseable: `save` refuses (the
    // content no longer matches what was loaded) and the re-open then fails too.
    std::fs::write(&path, UNPARSEABLE).unwrap();

    let body = json!({
        "date": "2024-01-02",
        "description": "PHANTOM EDIT NEVER SAVED",
        "postings": [
            { "account": "expenses:b",
              "amount": { "commodity": "$", "quantity": { "mantissa": "200", "places": 2 } } },
            { "account": "assets:bank" }
        ]
    });
    let (put_status, message) = request(&state, "PUT", "/api/transactions/2", Some(body)).await;

    // THE finding: the served snapshot must still be the last state that was
    // successfully READ. Asserted before the status code because publishing an
    // edit that reached no file is the damage; the status is how we report it.
    let (status, served) = request(&state, "GET", "/transactions", None).await;
    assert_eq!(status, StatusCode::OK);
    let descriptions: Vec<&str> = served
        .as_array()
        .expect("transactions array")
        .iter()
        .map(|txn| txn["tdescription"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        descriptions,
        vec!["A", "B", "C"],
        "an edit that never reached the file must never be published"
    );
    assert_eq!(
        put_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a failed save whose re-sync ALSO fails must not answer as a plain conflict: {message}"
    );

    // The editor was unbound rather than left holding an unpersisted edit, so a
    // later save can never flush that phantom to disk.
    let (status, _) = request(&state, "DELETE", "/api/transactions/1", None).await;
    assert_eq!(
        status,
        StatusCode::NOT_IMPLEMENTED,
        "the editor must be unbound, not left carrying the failed edit"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), UNPARSEABLE);
    let _ = std::fs::remove_file(&path);
}

/// DL-5's other entry point: the mid-patch failure path. `apply_patch` commits
/// its ops to the in-memory rope ONE AT A TIME, so a later one failing leaves an
/// applied-but-unsaved description change; if the re-sync that discards it
/// cannot re-read the file, that change must not be published either.
#[tokio::test]
async fn failed_patch_with_unreadable_journal_publishes_nothing() {
    let (state, path) = state_for(THREE_TXNS);

    std::fs::write(&path, UNPARSEABLE).unwrap();

    // The description op succeeds in memory; the out-of-range posting then fails.
    let body = json!({
        "description": "PHANTOM PATCH NEVER SAVED",
        "postings": [ { "index": 99, "account": "expenses:nope" } ]
    });
    let (status, message) = request(&state, "PATCH", "/api/transactions/2", Some(body)).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the failed re-sync must win over the patch's own 404: {message}"
    );

    let (_, served) = request(&state, "GET", "/transactions", None).await;
    let published = serde_json::to_string(&served).unwrap();
    assert!(
        !published.contains("PHANTOM PATCH NEVER SAVED"),
        "the half-applied patch must never be published: {published}"
    );
    let _ = std::fs::remove_file(&path);
}

/// The Ok branch of the same code path must be untouched: when the file IS still
/// readable, a failed save still re-syncs to disk and publishes THAT — the
/// behavior `external_change_yields_409_and_resyncs_snapshot` pins for DELETE,
/// asserted here for the PUT path and its 409.
#[tokio::test]
async fn failed_save_with_readable_journal_still_resyncs_and_409s() {
    let (state, path) = state_for(THREE_TXNS);

    let external = "\
2099-01-01 * external edit
    expenses:x  $1.00
    assets:y
";
    std::fs::write(&path, external).unwrap();

    let body = json!({
        "date": "2024-01-02",
        "description": "never lands",
        "postings": [
            { "account": "expenses:b",
              "amount": { "commodity": "$", "quantity": { "mantissa": "200", "places": 2 } } },
            { "account": "assets:bank" }
        ]
    });
    let (status, _) = request(&state, "PUT", "/api/transactions/2", Some(body)).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Re-synced to the external file, and the external content is intact.
    assert_eq!(transaction_count(&state).await, 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
    let _ = std::fs::remove_file(&path);
}

/// The sample fixture's text, copied into each temp journal under test.
fn sample_text() -> String {
    std::fs::read_to_string(common::fixture_journal_path()).expect("sample.journal readable")
}

// ---------------------------------------------------------------------------
// Error surface: no absolute path in any write-path body (SEC-15)
// ---------------------------------------------------------------------------

/// Like [`request`], but returns the body as TEXT.
///
/// The error bodies are `text/plain` by contract (see `error_surface.rs`), so
/// `request`'s `serde_json` parse turns every one of them into `Value::Null` —
/// which is exactly how a path could sit in a `500` for as long as it did
/// without a test noticing.
async fn request_text(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let builder = Request::builder().method(method).uri(uri);
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
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A journal line that must never come back to us inside a file path.
const SECRET_DIR: &str = "ledgeline-edit-endpoint-secrets";

/// Editing-enabled state in a directory whose name is easy to spot in a body.
fn state_in_named_dir(content: &str) -> (AppState, PathBuf) {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join(SECRET_DIR)
        .join(format!("{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("main.journal");
    std::fs::write(&path, content).expect("write temp journal");
    let state = AppState::from_journal_path(&path).expect("editor opens");
    (state, path)
}

/// Assert `body` names neither the journal's directory nor its absolute path,
/// in either spelling (`/tmp` canonicalizes to `/private/tmp` on macOS).
fn assert_no_absolute_path(path: &std::path::Path, body: &str, what: &str) {
    let dir = path.parent().expect("the journal has a parent directory");
    let mut secrets = vec![path.to_path_buf(), dir.to_path_buf(), std::env::temp_dir()];
    let canonical: Vec<PathBuf> = secrets
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    secrets.extend(canonical);
    for secret in secrets {
        let rendered = secret.to_string_lossy().into_owned();
        assert!(
            !body.contains(&rendered),
            "{what}: the response body discloses {rendered}\n---\n{body}\n---"
        );
    }
}

/// SEC-15. The `/api/import/*` surface has pinned "no response body contains an
/// absolute path" since WP-11; the WRITE path never had the equivalent, and it
/// leaked one from two places at once. `EditError` renders a parse failure as
/// `{source_name}:{line}: {message}`, and `source_name` is the absolute path the
/// editor was opened with.
///
/// Every failure mode an ordinary edit can reach, swept in one test.
#[tokio::test]
async fn no_edit_response_body_contains_an_absolute_path() {
    let (state, path) = state_in_named_dir(THREE_TXNS);
    let dir = path.parent().expect("parent").to_path_buf();

    let good = json!({
        "date": "2024-02-01",
        "description": "ok",
        "postings": [
            {"account": "expenses:a",
             "amount": {"commodity": "$", "quantity": {"mantissa": "100", "places": 2}}},
            {"account": "assets:bank"}
        ]
    });

    let mut bodies: Vec<(String, String)> = Vec::new();

    // 404: an index nobody has.
    bodies.push((
        "delete/unknown".to_string(),
        request_text(&state, "DELETE", "/api/transactions/99999", None)
            .await
            .1,
    ));

    // 400: a transaction that does not balance.
    let unbalanced = json!({
        "date": "2024-02-01",
        "description": "no",
        "postings": [
            {"account": "expenses:a",
             "amount": {"commodity": "$", "quantity": {"mantissa": "500", "places": 2}}},
            {"account": "assets:bank",
             "amount": {"commodity": "$", "quantity": {"mantissa": "-400", "places": 2}}}
        ]
    });
    bodies.push((
        "add/unbalanced".to_string(),
        request_text(&state, "POST", "/api/transactions", Some(unbalanced))
            .await
            .1,
    ));

    // 409 / 500: the file was changed under us AND is now unparseable, so the
    // save is refused and the re-sync that follows it cannot re-read the file.
    // This is the DL-5 branch, and it is where the absolute path came out.
    std::fs::write(
        &path,
        "2024-01-01 * A\n    expenses:a  $1.00\n    assets:bank\n\n\
         2024-99-99 * SECRET Landlord rent\n    expenses:rent  $1234.56\n    assets:bank\n",
    )
    .expect("corrupt the journal externally");
    let (status, body) = request_text(&state, "POST", "/api/transactions", Some(good)).await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "an edit over an unparseable file must fail: {status} {body}"
    );
    bodies.push(("add/after-external-corruption".to_string(), body));

    for (what, body) in &bodies {
        assert!(!body.is_empty(), "{what}: expected a message");
        assert_no_absolute_path(&path, body, what);
    }

    let _ = std::fs::remove_dir_all(&dir);
}
