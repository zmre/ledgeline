//! The transaction-review routes over HTTP (plan 32 Phase 2): the queues,
//! similar transactions, the payee and tag vocabularies, the summary's review
//! counts, the rows' review markers, and the batch write.
//!
//! Every test runs over a fresh copy of `fixtures/review/` (the main file and
//! its include) read as of 2026-10-30, the date the fixture's header is
//! written for. The model's arithmetic is pinned in `ledgeline-core`'s
//! `tests/review.rs`; these pin what the HTTP layer adds — parameters, shapes,
//! fingerprints, status codes — and, for the batch, the three things every
//! write test here asserts: the response, the snapshot a later GET serves, and
//! the bytes on disk.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

const TODAY: &str = "2026-10-30";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh copy of `fixtures/review/`, plus an `hledger.conf` saying `conf`
/// when one is given.
fn fixture_copy(conf: Option<&str>) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join("ledgeline-review-endpoint-tests")
        .join(format!("{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let source = common::fixtures_dir().join("review");
    for name in ["review.journal", "review-2025.journal"] {
        std::fs::copy(source.join(name), dir.join(name)).expect("copy fixture");
    }
    if let Some(conf) = conf {
        std::fs::write(dir.join("hledger.conf"), conf).expect("write conf");
    }
    dir
}

fn state_in(dir: &Path) -> AppState {
    AppState::from_journal_path(dir.join("review.journal")).expect("editor opens")
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("read journal")
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, header::HeaderMap, Vec<u8>) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(json) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&json).expect("serialize")))
            .expect("request builds"),
        None => builder.body(Body::empty()).expect("request builds"),
    };
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

/// `GET uri`, which must answer `200` with JSON.
async fn get(state: &AppState, uri: &str) -> Value {
    let (status, _, body) = send(state, "GET", uri, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "GET {uri}: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).unwrap_or_else(|e| panic!("GET {uri} is JSON: {e}"))
}

/// `POST /api/transactions/batch` with `edits`: the status and the body (JSON
/// on success, the plain-text message otherwise).
async fn batch(state: &AppState, edits: Value) -> (StatusCode, Value, String) {
    let (status, _, body) = send(
        state,
        "POST",
        "/api/transactions/batch",
        Some(json!({ "edits": edits })),
    )
    .await;
    let text = String::from_utf8_lossy(&body).to_string();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
        text,
    )
}

async fn queue(state: &AppState, kind: &str, extra: &str) -> Value {
    get(
        state,
        &format!("/api/review/queue?kind={kind}&today={TODAY}{extra}"),
    )
    .await
}

/// The queued item dated `date` whose description contains `needle`.
fn item<'a>(items: &'a Value, date: &str, needle: &str) -> &'a Value {
    items
        .as_array()
        .expect("items")
        .iter()
        .find(|item| {
            item["date"] == date
                && item["description"]
                    .as_str()
                    .is_some_and(|d| d.contains(needle))
        })
        .unwrap_or_else(|| panic!("no {date} {needle} in {items}"))
}

fn dates(items: &Value) -> Vec<String> {
    items
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["date"].as_str().expect("date").to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// GET /api/review/queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknowns_queue_serves_full_rows_with_fingerprints_keys_and_suggestions() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let body = queue(&state, "unknowns", "").await;
    assert_eq!(body["kind"], "unknowns");
    assert_eq!(body["total"], 8);
    assert_eq!(body["offset"], 0);
    assert_eq!(body["notEditable"], 0);
    assert!(body["generation"].is_string());
    assert_eq!(
        dates(&body["items"]),
        [
            "2026-10-20",
            "2026-10-14",
            "2026-10-12",
            "2026-10-05",
            "2026-10-03",
            "2026-09-28",
            "2025-12-02",
            "2025-10-30",
        ]
    );
    let items = &body["items"];
    for entry in items.as_array().expect("items") {
        assert!(
            entry["fingerprint"].as_str().is_some_and(|f| !f.is_empty()),
            "every item can be edited: {entry}"
        );
        assert_eq!(entry["unknown"], true);
        assert!(entry["postings"].is_array() && entry["flags"].is_array());
        assert!(entry["suggestions"].is_array());
    }
    // A hand-reviewed unknown is still queued, and marked reviewed.
    assert_eq!(item(items, "2026-10-20", "Venmo")["reviewed"], true);

    let card = item(items, "2026-10-12", "4412");
    assert_eq!(card["reviewed"], false);
    assert_eq!(card["key"], "sq blue bottle oakland");
    let top = &card["suggestions"][0];
    assert_eq!(top["account"], "expenses:food:coffee");
    assert_eq!(top["source"], "exactKey");
    assert_eq!(top["strong"], true);
    assert_eq!(
        top["description"], "Blue Bottle Coffee",
        "the rename rides along"
    );
    assert!(top["confidence"].as_u64().is_some_and(|c| c >= 5_000));
    // The renamed Netflix charge is matched as a recurring one.
    let nflx = item(items, "2026-10-14", "NFLX");
    assert_eq!(nflx["suggestions"][0]["account"], "expenses:streaming");
    assert_eq!(nflx["suggestions"][0]["source"], "recurring");
}

#[tokio::test]
async fn recents_queue_pages_by_limit_and_offset_without_moving_its_total() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let all = queue(&state, "recents", "").await;
    assert_eq!(all["total"], 17);
    assert_eq!(all["items"].as_array().map(Vec::len), Some(17));
    assert!(
        all["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["reviewed"] == false),
        "Recents holds only untagged transactions"
    );
    let page = queue(&state, "recents", "&limit=5&offset=5").await;
    assert_eq!(page["total"], 17);
    assert_eq!(page["offset"], 5);
    assert_eq!(dates(&page["items"]), dates(&all["items"])[5..10]);
    // `maxDays=0` lifts the window: the 2024 and 2025-10-29 unknowns join.
    let unlimited = queue(&state, "unknowns", "&maxDays=0").await;
    assert_eq!(unlimited["total"], 10);
}

#[tokio::test]
async fn the_queue_refuses_an_unknown_kind_and_a_bad_date() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    for uri in [
        "/api/review/queue?today=2026-10-30",
        "/api/review/queue?kind=all&today=2026-10-30",
        "/api/review/queue?kind=recents&today=yesterday",
    ] {
        let (status, _, _) = send(&state, "GET", uri, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "GET {uri}");
    }
}

#[tokio::test]
async fn the_queue_revalidates_against_the_snapshot_and_its_date() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let uri = format!("/api/review/queue?kind=unknowns&today={TODAY}");
    let (status, headers, _) = send(&state, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .to_string();
    assert!(etag.ends_with(&format!("-{TODAY}\"")), "dated: {etag}");
    let request = Request::builder()
        .uri(&uri)
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .expect("request builds");
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn a_read_only_server_serves_the_queue_without_fingerprints() {
    let path = common::fixtures_dir().join("review/review.journal");
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    let journal =
        ledgeline_core::parse_journal(&text, &path.to_string_lossy()).expect("fixture parses");
    let state = AppState::from_journal(&journal);
    let body = queue(&state, "unknowns", "").await;
    assert_eq!(body["total"], 8);
    assert!(
        body["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["fingerprint"].is_null()),
        "no editor, nothing to fingerprint against"
    );
}

// ---------------------------------------------------------------------------
// GET /api/review/similar/{index}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn similar_lists_same_key_candidates_marks_the_queued_ones_and_names_the_precedent() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let unknowns = queue(&state, "unknowns", "").await;
    let card = item(&unknowns["items"], "2026-10-12", "4412");
    let index = card["index"].as_u64().expect("index");
    let queued: Vec<u64> = unknowns["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["index"].as_u64().expect("index"))
        .collect();
    let queue_param = queued
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let body = get(
        &state,
        &format!("/api/review/similar/{index}?today={TODAY}&queue={queue_param}"),
    )
    .await;
    assert_eq!(body["index"], index);
    assert_eq!(body["key"], "sq blue bottle oakland");
    let candidates = body["candidates"].as_array().expect("candidates");
    assert_eq!(body["total"].as_u64(), Some(candidates.len() as u64));
    assert!(
        candidates
            .iter()
            .all(|c| c["index"] != index && c["reviewed"] == false && c["fingerprint"].is_string()),
        "unreviewed others, each editable: {candidates:?}"
    );
    let candidate_dates: Vec<&str> = candidates
        .iter()
        .map(|c| c["date"].as_str().expect("date"))
        .collect();
    for date in ["2026-10-03", "2026-09-28", "2025-12-02", "2024-03-10"] {
        assert!(
            candidate_dates.contains(&date),
            "{date} in {candidate_dates:?}"
        );
    }
    let in_queue: Vec<u64> = body["inQueue"]
        .as_array()
        .expect("inQueue")
        .iter()
        .map(|i| i.as_u64().expect("index"))
        .collect();
    assert!(!in_queue.is_empty());
    assert!(in_queue.iter().all(|i| queued.contains(i)));
    // The 2024 one is outside the window, so not queued, so not marked.
    let old = candidates
        .iter()
        .find(|c| c["date"] == "2024-03-10")
        .expect("the 2024 candidate");
    assert!(!in_queue.contains(&old["index"].as_u64().expect("index")));
    assert_eq!(body["precedent"]["account"], "expenses:food:coffee");
    assert!(body["precedent"]["count"].as_u64().is_some_and(|c| c >= 2));

    // Without `queue=`, nothing is marked.
    let bare = get(
        &state,
        &format!("/api/review/similar/{index}?today={TODAY}"),
    )
    .await;
    assert_eq!(bare["inQueue"], json!([]));
}

#[tokio::test]
async fn similar_answers_404_for_a_missing_transaction_and_400_for_a_bad_queue() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let (status, _, _) = send(&state, "GET", "/api/review/similar/9999", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(&state, "GET", "/api/review/similar/3?queue=1,x", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// GET /api/payees, GET /api/tags
// ---------------------------------------------------------------------------

#[tokio::test]
async fn payees_and_tags_list_the_journals_vocabulary() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let payees = get(&state, "/api/payees").await;
    assert!(payees["generation"].is_string());
    let list = payees["payees"].as_array().expect("payees");
    assert_eq!(list[0]["payee"], "Netflix", "six uses, the most");
    assert_eq!(list[0]["count"], 6);
    assert_eq!(list[0]["last"], "2026-09-14");
    let safeway = list
        .iter()
        .find(|p| p["payee"] == "Safeway")
        .expect("Safeway");
    assert_eq!(safeway["count"], 4);

    let tags = get(&state, "/api/tags").await;
    let reviewed = tags["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .find(|t| t["name"] == "reviewed")
        .expect("the reviewed tag");
    assert_eq!(reviewed["count"], 5);
    assert!(
        reviewed["values"]
            .as_array()
            .expect("values")
            .iter()
            .any(|v| v["value"] == "ignore" && v["count"] == 1)
    );
}

// ---------------------------------------------------------------------------
// The summary's review counts and the rows' markers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_summary_counts_both_queues_whatever_the_filter() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let summary = get(&state, &format!("/api/journal/summary?today={TODAY}")).await;
    assert_eq!(
        summary["review"],
        json!({"unknown": 8, "unreviewedRecent": 17, "notEditable": 0, "notEditableRecent": 0})
    );
    // The filter narrows the summary, never the review counts (D4 Q3).
    let filtered = get(
        &state,
        &format!(
            "/api/journal/summary?today={TODAY}&accounts=assets&reviewLimit=5&reviewMaxDays=0"
        ),
    )
    .await;
    assert_eq!(filtered["review"]["unknown"], 10);
    assert_eq!(
        filtered["review"]["unreviewedRecent"], 5,
        "capped at the limit"
    );
    // The existing fields are all still there.
    for field in [
        "generation",
        "total",
        "visibleNet",
        "commodities",
        "perCommodity",
    ] {
        assert!(!summary[field].is_null(), "{field}");
    }
}

#[tokio::test]
async fn journal_rows_carry_the_review_markers() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let rows = get(
        &state,
        &format!("/api/journal/rows?today={TODAY}&limit=100"),
    )
    .await;
    let rows = &rows["rows"];
    let marker = |date: &str, needle: &str| {
        let row = item(rows, date, needle);
        (row["unknown"].clone(), row["reviewed"].clone())
    };
    assert_eq!(marker("2026-10-12", "4412"), (json!(true), json!(false)));
    // Ignored: reviewed, and no longer "unknown" for the stripe.
    assert_eq!(marker("2026-05-01", "4400"), (json!(false), json!(true)));
    assert_eq!(marker("2026-10-20", "Venmo"), (json!(true), json!(true)));
    assert_eq!(
        marker("2026-10-25", "Safeway"),
        (json!(false), json!(false))
    );
    assert_eq!(marker("2026-10-18", "Safeway"), (json!(false), json!(true)));
    let single = get(&state, "/api/transactions/1").await;
    assert!(single["unknown"].is_boolean() && single["reviewed"].is_boolean());
}

// ---------------------------------------------------------------------------
// --auto: a generated posting to an unknown account counts (amendment A)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn under_auto_a_generated_unknown_posting_queues_the_transaction_and_names_its_rule() {
    let dir = fixture_copy(Some("--auto\n"));
    let state = state_in(&dir);
    let unknowns = queue(&state, "unknowns", "").await;
    assert_eq!(unknowns["total"], 9);
    let rename = item(&unknowns["items"], "2026-09-01", "Blue Bottle Coffee");
    assert_eq!(rename["unknown"], true);
    assert_eq!(rename["reviewed"], true);
    let generated: Vec<&Value> = rename["postings"]
        .as_array()
        .expect("postings")
        .iter()
        .filter(|p| !p["generated"].is_null())
        .collect();
    assert!(!generated.is_empty());
    assert!(
        generated.iter().all(
            |p| p["account"] == "expenses:unknown" && p["generated"]["query"] == "desc:coffee$"
        )
    );
    // The Problems drawer agrees.
    let diagnostics = get(&state, &format!("/api/diagnostics?today={TODAY}")).await;
    let index = rename["index"].clone();
    assert!(
        diagnostics["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|d| d["rule"] == "uncategorized" && d["tindex"] == index),
        "{diagnostics}"
    );
    let summary = get(&state, &format!("/api/journal/summary?today={TODAY}")).await;
    assert_eq!(summary["review"]["unknown"], 9);
}

// ---------------------------------------------------------------------------
// POST /api/transactions/batch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_batch_saves_a_card_and_its_similar_transactions_across_two_files_in_one_edit() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let before = queue(&state, "unknowns", "").await;
    let card = item(&before["items"], "2026-10-12", "4412");
    let sibling = item(&before["items"], "2026-10-03", "0917");
    let included = item(&before["items"], "2025-12-02", "7777");
    let to_coffee =
        json!({"op": "setPostingAccount", "posting": 0, "account": "expenses:food:coffee"});
    let (status, body, text) = batch(
        &state,
        json!([
            {"index": card["index"], "fingerprint": card["fingerprint"], "ops": [
                to_coffee,
                {"op": "setDescription", "description": "Blue Bottle Coffee"},
                {"op": "setTxnTag", "name": "reviewed", "value": TODAY},
                {"op": "setTxnTag", "name": "payee-key", "value": "sq blue bottle oakland"},
            ]},
            {"index": sibling["index"], "fingerprint": sibling["fingerprint"], "ops": [to_coffee]},
            {"index": included["index"], "fingerprint": included["fingerprint"], "ops": [to_coffee]},
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_ne!(body["generation"], before["generation"]);
    let edited = body["edited"].as_array().expect("edited");
    assert_eq!(edited.len(), 3);
    assert_eq!(edited[0]["index"], card["index"], "request order");
    assert_eq!(edited[0]["description"], "Blue Bottle Coffee");
    assert_eq!(edited[0]["reviewed"], true);
    assert!(edited.iter().all(|e| e["unknown"] == false));
    assert!(
        edited
            .iter()
            .zip([card, sibling, included])
            .all(|(after, before)| after["fingerprint"].is_string()
                && after["fingerprint"] != before["fingerprint"]),
        "every edited transaction has a new fingerprint"
    );

    // The snapshot: all three left Unknowns at once.
    let after = queue(&state, "unknowns", "").await;
    assert_eq!(after["total"], 5);
    assert_eq!(after["generation"], body["generation"]);

    // The bytes: surgical, elision intact, the house `"  ; "` for a new comment.
    let main = read(&dir, "review.journal");
    assert!(
        main.contains(
            "2026-10-12 Blue Bottle Coffee  ; reviewed:2026-10-30, payee-key:sq blue bottle oakland\n    ; receipt: 0042\n    expenses:food:coffee  $6.75\n    liabilities:visa\n"
        ),
        "{main}"
    );
    assert!(
        main.contains("2026-10-03 SQ *BLUE BOTTLE 0917 OAKLAND\n    expenses:food:coffee  $5.50\n    liabilities:visa\n"),
        "{main}"
    );
    let include = read(&dir, "review-2025.journal");
    assert!(
        include
            .contains("2025-12-02 SQ *BLUE BOTTLE 7777 OAKLAND\n    expenses:food:coffee  $5.75\n"),
        "{include}"
    );
}

#[tokio::test]
async fn a_stale_fingerprint_is_a_409_naming_the_transaction_and_writes_nothing() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let before = queue(&state, "unknowns", "").await;
    let card = item(&before["items"], "2026-10-12", "4412");
    let sibling = item(&before["items"], "2026-10-03", "0917");
    let main = read(&dir, "review.journal");
    let (status, _, text) = batch(
        &state,
        json!([
            {"index": card["index"], "fingerprint": card["fingerprint"],
             "ops": [{"op": "setTxnTag", "name": "reviewed", "value": TODAY}]},
            {"index": sibling["index"], "fingerprint": "0-0000000000000000",
             "ops": [{"op": "setTxnTag", "name": "reviewed", "value": TODAY}]},
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        text.contains(&format!("#{}", sibling["index"])),
        "names the stale one: {text}"
    );
    assert_eq!(read(&dir, "review.journal"), main, "nothing written");
    assert_eq!(
        queue(&state, "unknowns", "").await["generation"],
        before["generation"]
    );
}

#[tokio::test]
async fn an_edit_the_engine_cannot_take_is_a_400_naming_the_transaction_and_writes_nothing() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let before = queue(&state, "unknowns", "").await;
    let card = item(&before["items"], "2026-10-12", "4412");
    let sibling = item(&before["items"], "2026-10-03", "0917");
    let main = read(&dir, "review.journal");
    let (status, _, text) = batch(
        &state,
        json!([
            {"index": card["index"], "fingerprint": card["fingerprint"],
             "ops": [{"op": "setTxnTag", "name": "reviewed", "value": TODAY}]},
            {"index": sibling["index"], "fingerprint": sibling["fingerprint"],
             "ops": [{"op": "setTxnTag", "name": "trip", "value": "a, b"}]},
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(
        text.starts_with(&format!("transaction #{}:", sibling["index"])),
        "{text}"
    );
    assert_eq!(read(&dir, "review.journal"), main, "nothing written");
}

#[tokio::test]
async fn a_batch_refuses_empty_unknown_and_control_character_edits() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let before = queue(&state, "unknowns", "").await;
    let card = item(&before["items"], "2026-10-12", "4412");
    let (status, _, _) = batch(&state, json!([])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = batch(
        &state,
        json!([{"index": 9999, "fingerprint": "x", "ops": []}]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, text) = batch(
        &state,
        json!([{"index": card["index"], "fingerprint": card["fingerprint"],
                "ops": [{"op": "setDescription", "description": "a\nP 2026-01-01 X $1"}]}]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let (status, _, _) = batch(
        &state,
        json!([{"index": card["index"], "fingerprint": card["fingerprint"],
                "ops": [{"op": "frobnicate"}]}]),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unknown op is a malformed body"
    );
}

#[tokio::test]
async fn a_replace_op_rewrites_the_transaction_and_later_ops_still_apply() {
    let dir = fixture_copy(None);
    let state = state_in(&dir);
    let before = queue(&state, "unknowns", "").await;
    let card = item(&before["items"], "2026-10-03", "0917");
    let dollars =
        |mantissa: &str| json!({"commodity": "$", "quantity": {"mantissa": mantissa, "places": 2}});
    let (status, body, text) = batch(
        &state,
        json!([{"index": card["index"], "fingerprint": card["fingerprint"], "ops": [
            {"op": "replace", "transaction": {
                "date": "2026-10-03",
                "description": "SQ *BLUE BOTTLE 0917 OAKLAND",
                "postings": [
                    {"account": "expenses:food:coffee", "amount": dollars("400")},
                    {"account": "expenses:food:groceries", "amount": dollars("150")},
                    {"account": "liabilities:visa"},
                ],
            }},
            {"op": "setTxnTag", "name": "reviewed", "value": TODAY},
        ]}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        body["edited"][0]["postings"].as_array().map(Vec::len),
        Some(3)
    );
    let main = read(&dir, "review.journal");
    assert!(
        main.contains("2026-10-03 SQ *BLUE BOTTLE 0917 OAKLAND  ; reviewed:2026-10-30\n"),
        "{main}"
    );
    assert!(main.contains("expenses:food:groceries  $1.50"), "{main}");
}

#[tokio::test]
async fn a_batch_needs_an_editor() {
    let path = common::fixtures_dir().join("review/review.journal");
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    let journal =
        ledgeline_core::parse_journal(&text, &path.to_string_lossy()).expect("fixture parses");
    let state = AppState::from_journal(&journal);
    let (status, _, _) = batch(&state, json!([{"index": 1, "fingerprint": "x", "ops": []}])).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}
