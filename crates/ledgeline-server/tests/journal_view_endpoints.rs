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
        "/api/journal/rows?limit=5&today=2026-01-01",
        "/api/journal/summary?accounts=expenses",
        "/api/journal/chart?commodity=%24&mode=line",
        "/api/journal/position?index=3",
        "/api/transactions/3?today=2026-01-01",
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

// ---------------------------------------------------------------------------
// The windowed journal (B5)
// ---------------------------------------------------------------------------

fn sample_state() -> AppState {
    AppState::from_journal(&fixture_journal())
}

fn tindexes(body: &Value) -> Vec<u64> {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["index"].as_u64().expect("index"))
        .collect()
}

/// `rows` windows the filtered journal newest-first, `asc` is its exact
/// reverse, and `total` is the whole filtered count whatever the window.
#[tokio::test]
async fn rows_window_the_filtered_journal_in_display_order() {
    let state = sample_state();
    let all = json(&state, "/api/journal/rows?limit=1000").await;
    assert_eq!(all["total"], 189, "the e2e footer's all-time count");
    assert_eq!(all["offset"], 0);
    assert!(all["generation"].as_str().is_some_and(|g| !g.is_empty()));
    let order = tindexes(&all);
    assert_eq!(order.len(), 189);
    let dates: Vec<&str> = all["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["date"].as_str().expect("date"))
        .collect();
    assert!(
        dates.windows(2).all(|pair| pair[0] >= pair[1]),
        "newest first"
    );

    let window = json(&state, "/api/journal/rows?offset=10&limit=5").await;
    assert_eq!(window["total"], 189);
    assert_eq!(window["offset"], 10);
    assert_eq!(tindexes(&window), order[10..15]);

    let oldest = json(&state, "/api/journal/rows?order=asc&limit=1000").await;
    let reversed: Vec<u64> = order.iter().rev().copied().collect();
    assert_eq!(tindexes(&oldest), reversed);
}

#[tokio::test]
async fn rows_apply_dates_accounts_and_a_case_insensitive_query() {
    let state = sample_state();
    let ranged = json(
        &state,
        "/api/journal/rows?from=2026-07-01&to=2026-07-31&limit=1000",
    )
    .await;
    assert!(ranged["rows"].as_array().expect("rows").iter().all(|row| {
        let date = row["date"].as_str().expect("date");
        ("2026-07-01"..="2026-07-31").contains(&date)
    }));
    let food = json(
        &state,
        "/api/journal/rows?accounts=expenses:food&limit=1000",
    )
    .await;
    assert!(food["total"].as_u64().expect("total") > 0);
    assert!(food["rows"].as_array().expect("rows").iter().all(|row| {
        row["postings"]
            .as_array()
            .expect("postings")
            .iter()
            .any(|p| {
                p["account"]
                    .as_str()
                    .is_some_and(|a| a.starts_with("expenses:food"))
            })
    }));
    let lower = json(&state, "/api/journal/rows?q=safeway&limit=1000").await;
    let upper = json(&state, "/api/journal/rows?q=%20%20SAFEWAY%20&limit=1000").await;
    assert_eq!(tindexes(&lower), tindexes(&upper));
    assert!(lower["total"].as_u64().expect("total") > 0);
}

/// Rows carry the domain shape the SPA renders, string mantissas, and their
/// check findings as `flags`.
#[tokio::test]
async fn rows_embed_their_flags_and_string_mantissas() {
    let state = edge_state();
    let body = json(&state, "/api/journal/rows?limit=1000&today=2025-01-15").await;
    let rows = body["rows"].as_array().expect("rows");
    let pending = rows
        .iter()
        .find(|row| row["index"] == 3)
        .expect("the pending row");
    assert_eq!(pending["status"], "pending");
    assert_eq!(pending["flags"][0]["rule"], "pending");
    assert_eq!(
        pending["postings"][0]["amounts"][0]["qty"],
        serde_json::json!({"m": "123456", "p": 3})
    );
    let clean = rows
        .iter()
        .find(|row| row["index"] == 2)
        .expect("the paycheck");
    assert_eq!(clean["flags"], serde_json::json!([]));
    // `today` moves the future-date flag.
    let future = rows
        .iter()
        .find(|row| row["index"] == 18)
        .expect("the 2030 rent");
    assert_eq!(future["flags"][0]["rule"], "future-date");
    let later = json(&state, "/api/journal/rows?limit=1000&today=2031-01-01").await;
    let future = later["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["index"] == 18)
        .expect("rent");
    assert_eq!(future["flags"], serde_json::json!([]));
}

#[tokio::test]
async fn rows_refuse_a_bad_order_or_date_and_cap_the_limit() {
    let state = sample_state();
    for uri in [
        "/api/journal/rows?order=sideways",
        "/api/journal/rows?from=soon",
    ] {
        let (status, _, _) = get_with(&state, uri, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    }
    let capped = json(&state, "/api/journal/rows?limit=50000").await;
    assert!(capped["rows"].as_array().expect("rows").len() <= 1000);
}

#[tokio::test]
async fn summary_answers_the_footer_and_the_selectors() {
    let state = sample_state();
    let body = json(&state, "/api/journal/summary").await;
    assert_eq!(body["total"], 189);
    assert_eq!(body["primary"], "$");
    assert_eq!(body["commodities"][0], "$");
    assert!(body["maxDepth"].as_u64().expect("depth") >= 2);
    let dollars = &body["perCommodity"][0];
    assert_eq!(dollars["commodity"], "$");
    assert!(
        dollars["categories"]
            .as_array()
            .expect("categories")
            .contains(&serde_json::json!("expense"))
    );
    assert_eq!(body["visibleNet"][0]["commodity"], "$");

    let none = json(&state, "/api/journal/summary?q=zzz-no-such-thing").await;
    assert_eq!(none["total"], 0);
    assert_eq!(none["visibleNet"], serde_json::json!([]));
    assert_eq!(none["primary"], Value::Null);
}

#[tokio::test]
async fn chart_draws_the_requested_mode_only() {
    let state = sample_state();
    let pie = json(&state, "/api/journal/chart?commodity=%24&category=expense").await;
    assert!(pie["line"].is_null());
    let slices = pie["pie"].as_array().expect("pie");
    assert!(!slices.is_empty() && slices.len() <= 6);
    assert_eq!(pie["groups"][0], slices[0]["account"]);

    let line = json(
        &state,
        "/api/journal/chart?commodity=%24&mode=line&interval=weekly&depth=1&max=2",
    )
    .await;
    assert!(line["pie"].is_null());
    let series = line["line"].as_array().expect("line");
    assert!(series.len() <= 2);
    let buckets = series[0]["points"].as_array().expect("points");
    assert!(
        buckets[0]["bucket"]
            .as_str()
            .expect("bucket")
            .contains("-W")
    );

    for uri in [
        "/api/journal/chart?commodity=%24&mode=bar",
        "/api/journal/chart?commodity=%24&mode=line&interval=hourly",
        "/api/journal/chart?commodity=%24&category=widgets",
        "/api/journal/chart",
    ] {
        let (status, _, _) = get_with(&state, uri, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    }
}

/// `position` finds a transaction's row in the filtered view, and says when the
/// filter hides it — with its date, so the client can widen the range.
#[tokio::test]
async fn position_locates_a_transaction_in_the_filtered_view() {
    let state = sample_state();
    let all = json(&state, "/api/journal/rows?limit=1000").await;
    let order = tindexes(&all);
    let target = order[42];
    let found = json(&state, &format!("/api/journal/position?index={target}")).await;
    assert_eq!(found["position"], 42);
    assert_eq!(found["total"], 189);
    let date = found["date"].as_str().expect("a date").to_string();

    let hidden = json(
        &state,
        &format!("/api/journal/position?index={target}&q=zzz-no-such-thing"),
    )
    .await;
    assert_eq!(hidden["position"], Value::Null);
    assert_eq!(hidden["date"], date.as_str());

    let missing = json(&state, "/api/journal/position?index=999999").await;
    assert_eq!(missing["position"], Value::Null);
    assert_eq!(missing["date"], Value::Null);
}

/// One transaction by tindex — and the collection path still answers no GET,
/// because the SPA's "can I edit?" probe depends on its `405`.
#[tokio::test]
async fn a_single_transaction_is_served_by_tindex() {
    let state = edge_state();
    let row = json(&state, "/api/transactions/3?today=2025-01-15").await;
    assert_eq!(row["index"], 3);
    assert_eq!(row["description"], "Grocery Mart | weekly shop");
    assert_eq!(row["flags"][0]["rule"], "pending");

    let (missing, _, _) = get_with(&state, "/api/transactions/9999", &[]).await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
    let (collection, _, _) = get_with(&state, "/api/transactions", &[]).await;
    assert_eq!(collection, StatusCode::METHOD_NOT_ALLOWED);
}

/// The hledger-web `/transactions` body is built on first request now; it must
/// still be exactly the core serializer's bytes, and the same bytes again.
#[tokio::test]
async fn the_lazy_transactions_body_is_byte_identical() {
    let journal = fixture_journal();
    let state = AppState::from_journal(&journal);
    let expected = serde_json::to_vec(&ledgeline_core::wire::journal_to_transactions(&journal))
        .expect("serializes");
    let (status, _, first) = get_with(&state, "/transactions", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first, expected);
    let (_, _, second) = get_with(&state, "/transactions", &[]).await;
    assert_eq!(second, expected);
}
