//! The transfer-matching routes over HTTP (plan 35 Phase 2): proposals, the
//! link / reject / unlink / forget / ignore / link-all writes, the batch's
//! posting-tag ops, and linked pairs collapsing into one journal row (D11).
//!
//! Every test runs over a fresh temp-dir journal written here (never
//! `fixtures/sample.journal`, whose totals e2e asserts). Write tests assert
//! the response, what a later GET serves, and the bytes on disk.

mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

const TODAY: &str = "2026-12-31";
const MAIN: &str = "main.journal";

/// Transactions, in file order:
///
/// 1. checking → transfers:cc (+$500), clearing posting 1
/// 2. transfers:cc (−$500) → amex (elided), clearing posting 0
/// 3. groceries (not a transfer)
/// 4. checking → transfers:bank (+$200), already rejected with 5
/// 5. transfers:bank (−$200) → savings, already rejected with 4
/// 6. transfers:bank (−$200) → savings
/// 7. checking → transfers:bank (+$200)
///
/// Proposed: (1, 2) gap 2 and (6, 7) gap 2. Rejecting (6, 7) frees
/// (4, 6) and (5, 7), each gap 4.
const JOURNAL: &str = "\
account assets:checking   ; type: C
account assets:savings    ; type: C
account liabilities:amex  ; type: L
account expenses:food     ; type: X
account transfers         ; clearing:

2026-12-01 * ACH AMEX EPAYMENT
    assets:checking             $-500.00
    transfers:cc                 $500.00

2026-12-03 * PAYMENT THANK YOU
    transfers:cc                $-500.00
    liabilities:amex

2026-12-05 Groceries
    expenses:food                 $84.12
    assets:checking

2026-12-10 * TRANSFER TO SAVINGS
    assets:checking             $-200.00
    transfers:bank               $200.00  ; nomatch:rej001

2026-12-12 * ONLINE TRANSFER FROM CHK
    assets:savings               $200.00
    transfers:bank              $-200.00  ; nomatch:rej001

2026-12-14 * ANOTHER TRANSFER IN
    assets:savings               $200.00
    transfers:bank              $-200.00

2026-12-16 * TRANSFER TO SAVINGS AGAIN
    assets:checking             $-200.00
    transfers:bank               $200.00
";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh temp dir holding `text` as `main.journal`.
fn journal_dir(text: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join("ledgeline-match-endpoint-tests")
        .join(format!("{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join(MAIN), text).expect("write journal");
    dir
}

fn state_in(dir: &Path) -> AppState {
    AppState::from_journal_path(dir.join(MAIN)).expect("editor opens")
}

fn read(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(MAIN)).expect("read journal")
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
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
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

async fn get(state: &AppState, uri: &str) -> Value {
    let (status, body) = send(state, "GET", uri, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "GET {uri}: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).expect("JSON")
}

/// `POST uri` with `body`: the status, the JSON body (or null) and its text.
async fn post(state: &AppState, uri: &str, body: Value) -> (StatusCode, Value, String) {
    let (status, bytes) = send(state, "POST", uri, Some(body)).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        text,
    )
}

/// `POST uri`, which must answer `200`.
async fn post_ok(state: &AppState, uri: &str, body: Value) -> Value {
    let (status, json, text) = post(state, uri, body).await;
    assert_eq!(status, StatusCode::OK, "POST {uri}: {text}");
    json
}

async fn matches(state: &AppState, extra: &str) -> Value {
    get(
        state,
        &format!("/api/matches?today={TODAY}&lookback=0{extra}"),
    )
    .await
}

/// `(a.index, b.index)` of every proposal.
fn pairs(body: &Value) -> Vec<(u64, u64)> {
    body["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .map(|p| {
            (
                p["a"]["index"].as_u64().unwrap(),
                p["b"]["index"].as_u64().unwrap(),
            )
        })
        .collect()
}

/// The proposal whose halves are `a` and `b`.
fn proposal(body: &Value, a: u64, b: u64) -> Value {
    body["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .find(|p| p["a"]["index"] == a && p["b"]["index"] == b)
        .unwrap_or_else(|| panic!("no proposal ({a}, {b}) in {body}"))
        .clone()
}

/// A half as a write names it: `{index, posting, fingerprint}`.
fn half(value: &Value) -> Value {
    json!({
        "index": value["index"],
        "posting": value["posting"],
        "fingerprint": value["fingerprint"],
    })
}

fn pair_body(proposal: &Value) -> Value {
    json!({ "a": half(&proposal["a"]), "b": half(&proposal["b"]) })
}

/// The id a write reports, checked to be a fresh 6-character base-36 id.
fn id_of(body: &Value) -> String {
    let id = body["id"].as_str().expect("id").to_string();
    assert_eq!(id.len(), 6, "{id}");
    assert!(
        id.bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase()),
        "{id}"
    );
    id
}

fn edited_indices(body: &Value) -> Vec<u64> {
    body["edited"]
        .as_array()
        .expect("edited")
        .iter()
        .map(|txn| txn["index"].as_u64().unwrap())
        .collect()
}

// ---------------------------------------------------------------------------
// GET /api/matches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proposals_carry_both_halves_as_review_rows_ready_to_link() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let body = matches(&state, "").await;
    assert_eq!(body["clearing"], true);
    assert_eq!(pairs(&body), vec![(1, 2), (6, 7)]);
    assert_eq!(body["counts"], json!({ "clear": 2, "check": 0 }));

    let amex = proposal(&body, 1, 2);
    assert_eq!(amex["gapDays"], 2);
    assert_eq!(amex["clear"], true);
    assert_eq!(amex["ambiguous"], false);
    // Written-posting positions: the clearing leg is second in #1, first in #2.
    assert_eq!(amex["a"]["posting"], 1);
    assert_eq!(amex["b"]["posting"], 0);
    assert_eq!(amex["a"]["description"], "ACH AMEX EPAYMENT");
    // The card's file line: the name only, never the directory.
    assert_eq!(amex["a"]["file"], MAIN);
    assert_eq!(amex["b"]["file"], MAIN);
    for side in ["a", "b"] {
        assert!(
            amex[side]["fingerprint"].is_string(),
            "{side} has a fingerprint"
        );
        assert!(amex[side]["unknown"].is_boolean() && amex[side]["postings"].is_array());
    }
    assert_eq!(
        amex["from"],
        json!({ "index": 1, "account": "assets:checking", "label": "checking" })
    );
    assert_eq!(
        amex["to"],
        json!({ "index": 2, "account": "liabilities:amex", "label": "amex" })
    );
    assert!(
        amex["alternatives"]
            .as_array()
            .expect("alternatives")
            .is_empty()
    );
}

#[tokio::test]
async fn without_a_clearing_account_there_is_nothing_to_propose() {
    let dir = journal_dir(&JOURNAL.replace("account transfers         ; clearing:\n", ""));
    let state = state_in(&dir);
    let body = matches(&state, "").await;
    assert_eq!(body["clearing"], false);
    assert_eq!(body["counts"], json!({ "clear": 0, "check": 0 }));
    assert_eq!(pairs(&body), vec![]);
}

#[tokio::test]
async fn since_scopes_to_pairs_involving_an_imports_last_transactions() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    // An import that appended one transaction (#7) to main.journal.
    let body = matches(&state, &format!("&since={MAIN}:1")).await;
    assert_eq!(pairs(&body), vec![(6, 7)]);
    assert_eq!(body["counts"]["clear"], 1);
    let all = matches(&state, &format!("&since={MAIN}:100")).await;
    assert_eq!(pairs(&all), vec![(1, 2), (6, 7)]);

    for bad in ["nocolon", "main.journal:x", "other.journal:1"] {
        let (status, _) = send(
            &state,
            "GET",
            &format!("/api/matches?today={TODAY}&since={bad}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn settings_out_of_range_are_refused() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    for query in ["window=0", "window=61", "lookback=36501"] {
        let (status, _) = send(
            &state,
            "GET",
            &format!("/api/matches?today={TODAY}&{query}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
    }
}

// ---------------------------------------------------------------------------
// Link, unlink
// ---------------------------------------------------------------------------

#[tokio::test]
async fn link_writes_one_tag_per_side_and_unlink_restores_the_bytes() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let amex = proposal(&matches(&state, "").await, 1, 2);

    let linked = post_ok(&state, "/api/matches/link", pair_body(&amex)).await;
    let id = id_of(&linked);
    assert_eq!(edited_indices(&linked), vec![1, 2]);
    let expected = JOURNAL
        .replace(
            "    transfers:cc                 $500.00\n",
            &format!("    transfers:cc                 $500.00  ; match:{id}\n"),
        )
        .replace(
            "    transfers:cc                $-500.00\n",
            &format!("    transfers:cc                $-500.00  ; match:{id}\n"),
        );
    assert_eq!(
        read(&dir),
        expected,
        "only the two tags are written; the elision stays"
    );

    // The pair is no longer proposed.
    assert_eq!(pairs(&matches(&state, "").await), vec![(6, 7)]);

    // Unlink with the fingerprints the link answered with.
    let halves: Vec<Value> = linked["edited"]
        .as_array()
        .unwrap()
        .iter()
        .zip([1, 0])
        .map(|(txn, posting)| json!({ "index": txn["index"], "posting": posting, "fingerprint": txn["fingerprint"] }))
        .collect();
    let unlinked = post_ok(
        &state,
        "/api/matches/unlink",
        json!({ "matchId": id, "halves": halves }),
    )
    .await;
    assert_eq!(unlinked["id"], id.as_str());
    assert_eq!(read(&dir), JOURNAL, "byte for byte as before the link");
    assert_eq!(pairs(&matches(&state, "").await), vec![(1, 2), (6, 7)]);
}

#[tokio::test]
async fn link_with_description_edits_writes_both_in_one_batch() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let amex = proposal(&matches(&state, "").await, 1, 2);
    let mut body = pair_body(&amex);
    body["a"]["description"] = json!("Amex payment");

    // A bad description refuses the whole write: no tag lands either.
    let mut bad = body.clone();
    bad["b"]["description"] = json!("line\nbreak");
    let (status, _, _) = post(&state, "/api/matches/link", bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(read(&dir), JOURNAL);

    let linked = post_ok(&state, "/api/matches/link", body).await;
    let id = id_of(&linked);
    assert_eq!(linked["edited"][0]["description"], "Amex payment");
    let text = read(&dir);
    assert!(text.contains("2026-12-01 * Amex payment\n"), "{text}");
    assert_eq!(text.matches(&format!("match:{id}")).count(), 2, "{text}");
}

#[tokio::test]
async fn a_stale_fingerprint_is_refused_and_nothing_is_written() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let amex = proposal(&matches(&state, "").await, 1, 2);
    // Another tab renames #1 first.
    post_ok(
        &state,
        "/api/transactions/batch",
        json!({ "edits": [{
            "index": 1,
            "fingerprint": amex["a"]["fingerprint"],
            "ops": [{ "op": "setDescription", "description": "Renamed" }],
        }] }),
    )
    .await;
    let before = read(&dir);
    let (status, _, text) = post(&state, "/api/matches/link", pair_body(&amex)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{text}");
    assert!(text.contains("#1"), "{text}");
    assert_eq!(read(&dir), before);
}

#[tokio::test]
async fn a_pair_that_does_not_cancel_is_refused() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let body = matches(&state, "").await;
    let amex = proposal(&body, 1, 2);
    let bank = proposal(&body, 6, 7);
    let (status, _, text) = post(
        &state,
        "/api/matches/link",
        json!({ "a": half(&amex["a"]), "b": half(&bank["a"]) }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert_eq!(read(&dir), JOURNAL);
}

// ---------------------------------------------------------------------------
// Reject, forget, cleanup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reject_is_remembered_and_each_half_pairs_elsewhere() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let bank = proposal(&matches(&state, "").await, 6, 7);
    let rejected = post_ok(&state, "/api/matches/reject", pair_body(&bank)).await;
    let id = id_of(&rejected);
    assert_eq!(edited_indices(&rejected), vec![6, 7]);
    let text = read(&dir);
    assert_eq!(text.matches(&format!("nomatch:{id}")).count(), 2, "{text}");

    let after = matches(&state, "").await;
    assert_eq!(
        pairs(&after),
        vec![(1, 2), (4, 6), (5, 7)],
        "6 and 7 each pair with someone else"
    );

    // Forget the rejection: the bytes come back.
    let halves: Vec<Value> = rejected["edited"]
        .as_array()
        .unwrap()
        .iter()
        .map(
            |txn| json!({ "index": txn["index"], "posting": 1, "fingerprint": txn["fingerprint"] }),
        )
        .collect();
    post_ok(
        &state,
        "/api/matches/forget",
        json!({ "value": id, "halves": halves }),
    )
    .await;
    assert_eq!(read(&dir), JOURNAL);
    assert_eq!(pairs(&matches(&state, "").await), vec![(1, 2), (6, 7)]);
}

#[tokio::test]
async fn linking_cleans_up_every_rejection_on_both_sides() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let bank = proposal(&matches(&state, "").await, 6, 7);
    post_ok(&state, "/api/matches/reject", pair_body(&bank)).await;

    // #4 carries rej001 (with #5); #6 now carries the new rejection (with #7).
    let four_six = proposal(&matches(&state, "").await, 4, 6);
    let linked = post_ok(&state, "/api/matches/link", pair_body(&four_six)).await;
    let id = id_of(&linked);
    assert_eq!(edited_indices(&linked), vec![4, 5, 6, 7]);
    let text = read(&dir);
    assert!(
        !text.contains("nomatch"),
        "every rejection is gone:\n{text}"
    );
    assert_eq!(text.matches(&format!("match:{id}")).count(), 2);
}

#[tokio::test]
async fn unlink_and_forget_refuse_halves_that_do_not_carry_the_tag() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let amex = proposal(&matches(&state, "").await, 1, 2);
    let halves = json!([half(&amex["a"])]);
    let (status, _, _) = post(
        &state,
        "/api/matches/unlink",
        json!({ "matchId": "zzz999", "halves": halves }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = post(
        &state,
        "/api/matches/forget",
        json!({ "value": "rej001", "halves": halves }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = post(
        &state,
        "/api/matches/unlink",
        json!({ "matchId": "x", "halves": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(read(&dir), JOURNAL);
}

#[tokio::test]
async fn ignore_takes_a_posting_out_of_matching() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let amex = proposal(&matches(&state, "").await, 1, 2);
    let ignored = post_ok(
        &state,
        "/api/matches/ignore",
        json!({ "half": half(&amex["a"]) }),
    )
    .await;
    assert_eq!(ignored["id"], "ignore");
    assert!(read(&dir).contains("    transfers:cc                 $500.00  ; match:ignore\n"));
    assert_eq!(pairs(&matches(&state, "").await), vec![(6, 7)]);
}

// ---------------------------------------------------------------------------
// Link all
// ---------------------------------------------------------------------------

#[tokio::test]
async fn link_all_links_every_pair_and_refuses_a_posting_twice() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let body = matches(&state, "").await;
    let all: Vec<Value> = body["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .map(pair_body)
        .collect();

    let twice = json!({ "pairs": [all[0].clone(), all[0].clone()] });
    let (status, _, _) = post(&state, "/api/matches/link-all", twice).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(read(&dir), JOURNAL);

    let linked = post_ok(&state, "/api/matches/link-all", json!({ "pairs": all })).await;
    assert_eq!(linked["linked"], 2);
    let ids: Vec<&str> = linked["matchIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    let text = read(&dir);
    for id in &ids {
        assert_eq!(text.matches(&format!("match:{id}")).count(), 2, "{text}");
    }
    assert_eq!(pairs(&matches(&state, "").await), vec![]);
}

/// 600 pairs touch 1,200 transactions: more than one `MAX_BATCH` (1,000), so
/// the write goes out in two batches — and every pair still lands.
#[tokio::test]
async fn link_all_past_one_batch_writes_every_pair() {
    let mut text = String::from("account transfers  ; clearing:\n\n");
    for k in 0..600_u32 {
        let date = chrono_like(k * 20);
        let _ = write!(
            text,
            "{date} Out {k}\n    assets:checking  $-{k}.50\n    transfers:bank    ${k}.50\n\n\
             {date} In {k}\n    assets:savings    ${k}.50\n    transfers:bank   $-{k}.50\n\n"
        );
    }
    let dir = journal_dir(&text);
    let state = state_in(&dir);
    let body = matches(&state, "").await;
    assert_eq!(body["counts"]["clear"], 600);
    let all: Vec<Value> = body["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .map(pair_body)
        .collect();
    let linked = post_ok(&state, "/api/matches/link-all", json!({ "pairs": all })).await;
    assert_eq!(linked["linked"], 600);
    assert_eq!(read(&dir).matches("; match:").count(), 1200);
    assert_eq!(pairs(&matches(&state, "").await), vec![]);
    assert_eq!(rows(&state, "").await["total"], 600);
}

// ---------------------------------------------------------------------------
// The batch's posting-tag ops
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_batch_sets_adds_and_removes_posting_tags() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let item = get(&state, &format!("/api/review/item/5?today={TODAY}")).await;
    let edited = post_ok(
        &state,
        "/api/transactions/batch",
        json!({ "edits": [{
            "index": 5,
            "fingerprint": item["item"]["fingerprint"],
            "ops": [
                { "op": "addPostingTagValue", "posting": 1, "name": "nomatch", "value": "two002" },
                { "op": "removePostingTagValue", "posting": 1, "name": "nomatch", "value": "rej001" },
                { "op": "setPostingTag", "posting": 0, "name": "note", "value": "hi" },
            ],
        }] }),
    )
    .await;
    let postings = &edited["edited"][0]["postings"];
    assert!(
        postings[1]["tags"]
            .as_array()
            .unwrap()
            .contains(&json!(["nomatch", "two002"]))
    );
    assert!(
        !postings[1]["tags"]
            .as_array()
            .unwrap()
            .contains(&json!(["nomatch", "rej001"]))
    );
    let text = read(&dir);
    assert!(
        text.contains("    assets:savings               $200.00  ; note:hi\n"),
        "{text}"
    );
    assert!(
        text.contains("    transfers:bank              $-200.00  ; nomatch:two002\n"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// Rows (D11)
// ---------------------------------------------------------------------------

async fn rows(state: &AppState, query: &str) -> Value {
    get(state, &format!("/api/journal/rows?today={TODAY}{query}")).await
}

fn row_indices(page: &Value) -> Vec<u64> {
    page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["index"].as_u64().unwrap())
        .collect()
}

#[tokio::test]
async fn a_linked_pair_is_one_row_and_every_count_agrees() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let amex = proposal(&matches(&state, "").await, 1, 2);
    let id = id_of(&post_ok(&state, "/api/matches/link", pair_body(&amex)).await);

    let page = rows(&state, "").await;
    assert_eq!(page["total"], 6);
    assert_eq!(row_indices(&page), vec![7, 6, 5, 4, 3, 1]);
    let transfer_row = &page["rows"][5];
    assert_eq!(transfer_row["linked"], true);
    let transfer = &transfer_row["transfer"];
    assert_eq!(transfer["matchId"], id.as_str());
    assert_eq!(transfer["halves"][0]["index"], 1);
    assert_eq!(transfer["halves"][1]["index"], 2);
    assert_eq!(transfer["halves"][1]["description"], "PAYMENT THANK YOU");
    assert_eq!(transfer["from"]["label"], "checking");
    assert_eq!(transfer["to"]["label"], "amex");
    assert!(page["rows"][0].get("transfer").is_none() && page["rows"][0].get("linked").is_none());

    // Summary and position count rows; the figures still sum both halves.
    let summary = get(&state, &format!("/api/journal/summary?today={TODAY}")).await;
    assert_eq!(summary["total"], 6);
    assert_eq!(summary["transfers"], 1);
    let position = get(&state, "/api/journal/position?index=2").await;
    assert_eq!(
        position["position"], 5,
        "the hidden half jumps to its transfer row"
    );
    assert_eq!(position["total"], 6);

    // Filtered to Amex, the row anchors on the Amex half.
    let amex_rows = rows(&state, "&accounts=liabilities:amex").await;
    assert_eq!(amex_rows["total"], 1);
    assert_eq!(row_indices(&amex_rows), vec![2]);
    assert_eq!(amex_rows["rows"][0]["transfer"]["matchId"], id.as_str());
    let position = get(
        &state,
        "/api/journal/position?index=2&accounts=liabilities:amex",
    )
    .await;
    assert_eq!(position["position"], 0);

    // An export reads transactions, not rows.
    let flat = rows(&state, "&collapse=false&order=asc").await;
    assert_eq!(flat["total"], 7);
    assert_eq!(row_indices(&flat), vec![1, 2, 3, 4, 5, 6, 7]);
    assert!(
        flat["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("transfer").is_none())
    );
    assert_eq!(flat["rows"][1]["linked"], true);

    // Searching the other half's description finds the transfer row.
    let found = rows(&state, "&q=thank%20you").await;
    assert_eq!(row_indices(&found), vec![1]);
    assert_eq!(found["total"], 1);
}

/// Plan 36 meets D11: with an account pinned, a transfer row carries the
/// running cell of the half it anchors on — the half the pin's filter keeps.
#[tokio::test]
async fn a_pinned_transfer_row_carries_its_anchoring_halfs_running_cell() {
    let dir = journal_dir(JOURNAL);
    let state = state_in(&dir);
    let amex = proposal(&matches(&state, "").await, 1, 2);
    post_ok(&state, "/api/matches/link", pair_body(&amex)).await;

    for (account, anchor) in [("liabilities:amex", 2), ("assets:checking", 1)] {
        let page = rows(&state, &format!("&accounts={account}&balance={account}")).await;
        let transfer_row = page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row.get("transfer").is_some())
            .unwrap_or_else(|| panic!("{account}: a transfer row"));
        assert_eq!(transfer_row["index"], anchor, "{account}");
        assert_eq!(transfer_row["running"]["touches"], true, "{account}");
        for half in transfer_row["transfer"]["halves"].as_array().unwrap() {
            assert!(half.get("running").is_none(), "{account}: {half}");
        }
    }

    let unpinned = rows(&state, "").await;
    assert!(
        unpinned["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("running").is_none())
    );
}

#[tokio::test]
async fn a_transaction_in_two_links_stays_ordinary_rows_with_the_link_flag() {
    let text = "\
account transfers  ; clearing:

2026-12-01 Transfer and fee
    assets:checking   $-505
    transfers:cc       $500  ; match:aaa111
    transfers:cc         $5  ; match:bbb222

2026-12-02 Payment
    transfers:cc      $-500  ; match:aaa111
    liabilities:amex

2026-12-03 Fee
    transfers:cc        $-5  ; match:bbb222
    expenses:fees
";
    let dir = journal_dir(text);
    let state = state_in(&dir);
    let page = rows(&state, "").await;
    assert_eq!(page["total"], 3);
    for row in page["rows"].as_array().unwrap() {
        assert_eq!(row["linked"], true, "{row}");
        assert!(row.get("transfer").is_none(), "{row}");
    }
}

/// A linked pair whose halves straddle the client's 200-row block boundary
/// (newest first before collapsing, the later half is row 199 and the
/// earlier one row 202): paging through every block sees the transfer once,
/// in the second block, every other transaction once, and the collapsed
/// total.
#[tokio::test]
async fn a_pair_straddling_a_block_boundary_shows_once() {
    let mut text = String::from("account transfers  ; clearing:\n\n");
    // 400 fillers on odd days from 2020-01-01; the halves sit on even days
    // 398 and 402, so 199 fillers are newer than the later half.
    let day = chrono_like;
    for n in 0..400 {
        if n == 199 {
            let _ = write!(
                text,
                "{} Out\n    assets:checking  $-50\n    transfers:bank    $50  ; match:edge01\n\n",
                day(n * 2)
            );
        }
        if n == 201 {
            let _ = write!(
                text,
                "{} In\n    assets:savings    $50\n    transfers:bank   $-50  ; match:edge01\n\n",
                day(n * 2)
            );
        }
        let _ = write!(
            text,
            "{} Filler {n}\n    expenses:misc  $1\n    assets:checking\n\n",
            day(n * 2 + 1)
        );
    }
    let dir = journal_dir(&text);
    let state = state_in(&dir);
    let blocks = [
        rows(&state, "&offset=0&limit=200").await,
        rows(&state, "&offset=200&limit=200").await,
        rows(&state, "&offset=400&limit=200").await,
    ];
    assert_eq!(
        blocks[0]["total"], 401,
        "402 transactions, one pair collapsed"
    );
    let seen: Vec<u64> = blocks.iter().flat_map(row_indices).collect();
    assert_eq!(seen.len(), 401);
    let transfer_rows: Vec<&Value> = blocks
        .iter()
        .flat_map(|block| block["rows"].as_array().unwrap())
        .filter(|row| row.get("transfer").is_some())
        .collect();
    assert_eq!(transfer_rows.len(), 1);
    assert!(
        blocks[1]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row.get("transfer").is_some()),
        "the transfer row is in the second block"
    );
    assert!(
        blocks[0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["description"] != "In"),
        "the later half is not left behind in the first block"
    );
    assert_eq!(
        transfer_rows[0]["description"], "Out",
        "anchored at the earlier half"
    );
    let out = transfer_rows[0]["index"].clone();
    let into = transfer_rows[0]["transfer"]["halves"][1]["index"].clone();
    assert!(
        !seen.contains(&into.as_u64().unwrap()),
        "the later half is not a row of its own"
    );
    let unique: std::collections::HashSet<u64> = seen.iter().copied().collect();
    assert_eq!(unique.len(), 401);

    // Position agrees with the blocks: the later half jumps to the row the
    // earlier half occupies.
    let at = get(&state, &format!("/api/journal/position?index={into}")).await;
    let row = at["position"].as_u64().unwrap() as usize;
    assert_eq!(blocks[row / 200]["rows"][row % 200]["index"], out);
    assert_eq!(at["total"], 401);
}

/// `YYYY-MM-DD` for day `n` after 2020-01-01 (n < 1400), without a date crate.
fn chrono_like(n: u32) -> String {
    let months = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let (mut year, mut left) = (2020, n);
    loop {
        let len = if year % 4 == 0 { 366 } else { 365 };
        if left < len {
            break;
        }
        left -= len;
        year += 1;
    }
    let mut month = 0;
    loop {
        let len = if month == 1 && year % 4 != 0 {
            28
        } else {
            months[month]
        };
        if left < len {
            break;
        }
        left -= len;
        month += 1;
    }
    format!("{year}-{:02}-{:02}", month + 1, left + 1)
}

#[tokio::test]
async fn proposals_on_a_read_only_fixture_have_no_fingerprints() {
    // `fixtures/sample.journal` declares no clearing account: the route still
    // answers, with nothing to propose.
    let state = AppState::from_journal(&common::fixture_journal());
    let body = get(&state, &format!("/api/matches?today={TODAY}")).await;
    assert_eq!(body["clearing"], false);
    assert_eq!(body["proposals"], json!([]));
}

// ---------------------------------------------------------------------------
// Amendment A: `older` (A1), one transaction's matcher (A2), group links
// (A3) and suggested combinations (A4)
// ---------------------------------------------------------------------------

/// Transactions, in file order:
///
/// 1. checking → transfers:bills (+$2,247.50): a Bill.com debit
/// 2. transfers:bills (−$2,000) → rent, rejected with 4
/// 3. transfers:bills (−$247.50) → hosting
/// 4. savings → transfers:bills (+$2,000), rejected with 2
/// 5. transfers:bills (−$2,247.50), a year and a half earlier
/// 6. checking → transfers:cc (+$10), two years back
/// 7. transfers:cc (−$10), the next day
///
/// With the default 365-day lookback: one combination, 1 ← (2, 3). With no
/// lookback, also the old pair (6, 7): `older` is 1.
const BILLS: &str = "\
account assets:checking   ; type: C
account assets:savings    ; type: C
account expenses:rent     ; type: X
account expenses:hosting  ; type: X
account transfers         ; clearing:

2026-12-01 * BILL.COM DEBIT
    assets:checking            $-2247.50
    transfers:bills             $2247.50

2026-12-01 Rent bill
    transfers:bills            $-2000.00  ; nomatch:rej001
    expenses:rent

2026-12-01 Hosting bill
    transfers:bills             $-247.50
    expenses:hosting

2026-12-03 Savings in
    assets:savings             $-2000.00
    transfers:bills             $2000.00  ; nomatch:rej001

2025-06-01 Old refund
    transfers:bills            $-2247.50
    assets:checking

2024-12-01 Old transfer out
    assets:checking               $-10.00
    transfers:cc                   $10.00

2024-12-02 Old transfer in
    transfers:cc                  $-10.00
    assets:savings
";

fn indices_of(halves: &Value) -> Vec<u64> {
    halves
        .as_array()
        .expect("halves")
        .iter()
        .map(|half| half["index"].as_u64().unwrap())
        .collect()
}

#[tokio::test]
async fn a_unique_combination_is_proposed_with_its_group_and_older_counts_the_rest() {
    let dir = journal_dir(BILLS);
    let state = state_in(&dir);
    let body = get(&state, &format!("/api/matches?today={TODAY}")).await;
    assert_eq!(pairs(&body), vec![(1, 2)]);
    assert_eq!(body["counts"], json!({ "clear": 0, "check": 1 }));
    assert_eq!(body["older"], 1);
    let combination = &body["proposals"][0];
    assert_eq!(indices_of(&combination["group"]), vec![2, 3]);
    assert_eq!(combination["b"]["index"], combination["group"][0]["index"]);
    assert_eq!(combination["clear"], false);
    assert_eq!(combination["gapDays"], 0);
    for member in combination["group"].as_array().unwrap() {
        assert!(member["fingerprint"].is_string() && member["posting"] == 0);
    }
    assert_eq!(combination["from"]["label"], "checking");

    // With no lookback the old pair is proposed too, and nothing is older.
    let all = matches(&state, "").await;
    assert_eq!(pairs(&all), vec![(6, 7), (1, 2)]);
    assert_eq!(all["older"], 0);
    // An ordinary pair has an empty group.
    assert_eq!(proposal(&all, 6, 7)["group"], json!([]));
}

#[tokio::test]
async fn the_matcher_for_one_transaction_lists_exact_combinations_nearby_and_links() {
    let dir = journal_dir(BILLS);
    let state = state_in(&dir);
    let body = get(&state, &format!("/api/matches/for/1?today={TODAY}")).await;
    assert_eq!(body["clearing"], true);
    assert_eq!(body["transaction"]["index"], 1);
    assert!(body["transaction"]["fingerprint"].is_string());
    assert_eq!(body["links"], json!([]));
    let postings = body["postings"].as_array().expect("postings");
    assert_eq!(postings.len(), 1);
    let debit = &postings[0];
    assert_eq!(debit["posting"], 1);
    // The exact partner is 548 days away: past the window AND the default
    // lookback, offered anyway.
    assert_eq!(indices_of(&debit["exact"]), vec![5]);
    assert_eq!(debit["exact"][0]["gapDays"], 548);
    assert_eq!(debit["exact"][0]["posting"], 0);
    assert_eq!(debit["exact"][0]["file"], MAIN);
    assert!(debit["exact"][0]["fingerprint"].is_string());
    let combinations = debit["combinations"].as_array().expect("combinations");
    assert_eq!(combinations.len(), 1);
    assert_eq!(indices_of(&combinations[0]["group"]), vec![2, 3]);
    assert_eq!(indices_of(&debit["nearby"]), vec![2, 3]);
    assert_eq!(debit["nearby"][0]["gapDays"], 0);

    // The rent bill: its only exact partner (#4) is rejected with it; nearby
    // lists it anyway, for picking by hand; the combination includes it.
    let rent = get(&state, &format!("/api/matches/for/2?today={TODAY}")).await;
    let rent = &rent["postings"][0];
    assert_eq!(rent["exact"], json!([]));
    assert_eq!(indices_of(&rent["nearby"]), vec![1, 4]);
    assert_eq!(rent["nearby"][1]["gapDays"], 2);
    assert_eq!(rent["combinations"].as_array().map(Vec::len), Some(1));

    // A narrower window drops #4 from nearby.
    let narrow = get(
        &state,
        &format!("/api/matches/for/2?today={TODAY}&window=1"),
    )
    .await;
    assert_eq!(indices_of(&narrow["postings"][0]["nearby"]), vec![1]);

    let (status, _) = send(
        &state,
        "GET",
        &format!("/api/matches/for/99?today={TODAY}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &state,
        "GET",
        &format!("/api/matches/for/1?today={TODAY}&window=0"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_group_link_writes_one_id_everywhere_cleans_up_and_unlink_restores_the_bytes() {
    let dir = journal_dir(BILLS);
    let state = state_in(&dir);
    let body = get(&state, &format!("/api/matches?today={TODAY}")).await;
    let combination = &body["proposals"][0];
    let members = json!({
        "members": [
            half(&combination["a"]),
            half(&combination["group"][0]),
            half(&combination["group"][1]),
        ]
    });
    let written = post_ok(&state, "/api/matches/link", members).await;
    let id = id_of(&written);
    // #4 is touched only to drop the rejection it shared with #2.
    assert_eq!(edited_indices(&written), vec![1, 2, 3, 4]);
    let cleaned = BILLS
        .replace("$-2000.00  ; nomatch:rej001\n", "$-2000.00\n")
        .replace("$2000.00  ; nomatch:rej001\n", "$2000.00\n");
    let expected = cleaned
        .replace(
            "transfers:bills             $2247.50\n",
            &format!("transfers:bills             $2247.50  ; match:{id}\n"),
        )
        .replace(
            "transfers:bills            $-2000.00\n",
            &format!("transfers:bills            $-2000.00  ; match:{id}\n"),
        )
        .replace(
            "transfers:bills             $-247.50\n",
            &format!("transfers:bills             $-247.50  ; match:{id}\n"),
        );
    assert_eq!(read(&dir), expected);

    // The transaction's matcher now shows the link, and nothing to match.
    let one = get(&state, &format!("/api/matches/for/1?today={TODAY}")).await;
    assert_eq!(one["postings"], json!([]));
    assert_eq!(one["links"][0]["matchId"], id.as_str());
    assert_eq!(one["links"][0]["broken"], false);
    assert_eq!(indices_of(&one["links"][0]["members"]), vec![1, 2, 3]);

    // A group never collapses; its rows are linked.
    let page = rows(&state, "").await;
    assert_eq!(page["total"], 7);
    let linked: Vec<u64> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["linked"] == true)
        .map(|row| row["index"].as_u64().unwrap())
        .collect();
    let mut linked = linked;
    linked.sort_unstable();
    assert_eq!(linked, vec![1, 2, 3]);
    assert!(
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("transfer").is_none())
    );

    let halves: Vec<Value> = one["links"][0]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(half)
        .collect();
    post_ok(
        &state,
        "/api/matches/unlink",
        json!({ "matchId": id, "halves": halves }),
    )
    .await;
    assert_eq!(read(&dir), cleaned);
}

#[tokio::test]
async fn group_links_are_validated_and_link_all_takes_only_pairs() {
    let dir = journal_dir(BILLS);
    let state = state_in(&dir);
    let body = get(&state, &format!("/api/matches?today={TODAY}")).await;
    let combination = &body["proposals"][0];
    let (single, rent, hosting) = (
        half(&combination["a"]),
        half(&combination["group"][0]),
        half(&combination["group"][1]),
    );
    let refused = [
        // Short of zero.
        json!({ "members": [single, rent] }),
        // One member, a member twice, and neither shape.
        json!({ "members": [single] }),
        json!({ "members": [single, rent, rent, hosting] }),
        json!({ "a": single, "members": [rent, hosting] }),
        json!({ "a": single }),
        // A combination as a pair, to link-all: its two do not cancel.
    ];
    for request in refused {
        let (status, _, text) = post(&state, "/api/matches/link", request.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{request}: {text}");
    }
    let (status, _, text) = post(
        &state,
        "/api/matches/link-all",
        json!({ "pairs": [{ "a": single, "b": rent }] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let (status, _, text) = post(
        &state,
        "/api/matches/link-all",
        json!({ "pairs": [{ "members": [single, rent, hosting] }] }),
    )
    .await;
    assert!(status.is_client_error(), "{status}: {text}");
    assert_eq!(read(&dir), BILLS);
}
