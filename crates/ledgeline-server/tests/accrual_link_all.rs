//! Plan 45: **Link all** links exactly the clear pairs listed at the request's
//! generation, in ONE write; and **Post** writes N entries in one write too.
//!
//! One test in its own binary, on purpose: generations come from a
//! process-wide counter, so counting how many a request published only works
//! when nothing else in the process is publishing.

mod common;

use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppDirs, AppState, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

const TODAY: &str = "2026-11-17";
const PAIRS: usize = 6;

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(json) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&json).expect("serialize")))
            .expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    };
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("responds");
    let status = response.status();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get(state: &AppState, uri: &str) -> Value {
    let (status, body) = send(state, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "GET {uri}: {body}");
    body
}

/// The publish counter in a generation (`PREFIX-COUNTER`, hex).
fn counter(generation: &Value) -> u64 {
    let text = generation.as_str().expect("generation");
    u64::from_str_radix(text.rsplit('-').next().expect("counter"), 16).expect("hex")
}

async fn generation(state: &AppState) -> Value {
    get(state, &format!("/api/accrual?today={TODAY}")).await["generation"].clone()
}

fn state_in(dir: &Path) -> (AppState, tempfile::TempDir) {
    let scratch = tempfile::TempDir::new().expect("app dirs");
    let state = AppState::from_journal_path(dir.join("books.journal"))
        .expect("opens")
        .with_app_dirs(AppDirs::under(scratch.path()));
    (state, scratch)
}

/// `PAIRS` exact pairs, each its own amount; and a rival trio: two $300
/// invoices for one customer, a payment naming 7001 (clear), and a payment
/// naming neither (ambiguous between the two until 7001 is linked).
fn receivables_book() -> String {
    let mut text = String::from(
        "account assets:accounts receivable  ; type: A, kpi: receivable\naccount assets:bank  ; type: C\naccount revenue:services  ; type: R\n",
    );
    for n in 0..PAIRS {
        let (number, amount) = (1001 + n, 100 + n);
        text.push_str(&format!(
            "\n2026-10-01 Invoice {number}  ; invoice: {number}\n    assets:accounts receivable  ${amount}.00  ; customer: Client {n}\n    revenue:services\n\n2026-10-15 Payment {number}\n    assets:bank  ${amount}.00\n    assets:accounts receivable  $-{amount}.00  ; customer: Client {n}\n"
        ));
    }
    text.push_str(
        "\n2026-10-03 Invoice 7001  ; invoice: 7001\n    assets:accounts receivable  $300.00  ; customer: Rival\n    revenue:services\n\
         \n2026-10-03 Invoice 7002  ; invoice: 7002\n    assets:accounts receivable  $300.00  ; customer: Rival\n    revenue:services\n\
         \n2026-10-20 Payment 7001\n    assets:bank  $300.00\n    assets:accounts receivable  $-300.00  ; customer: Rival\n\
         \n2026-10-20 Rival transfer\n    assets:bank  $300.00\n    assets:accounts receivable  $-300.00  ; customer: Rival\n",
    );
    text
}

fn description(side: &Value) -> &str {
    side["description"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn link_all_is_one_write_of_exactly_the_listed_pairs_and_post_is_one_write() {
    // ---- Link all ---------------------------------------------------------
    let dir = tempfile::TempDir::new().expect("dir");
    std::fs::write(dir.path().join("books.journal"), receivables_book()).expect("write");
    let (state, _scratch) = state_in(dir.path());

    let listed = get(&state, &format!("/api/settlements?today={TODAY}")).await;
    let proposals = listed["proposals"].as_array().expect("proposals").clone();
    let clear: Vec<&Value> = proposals.iter().filter(|p| p["clear"] == true).collect();
    assert_eq!(clear.len(), PAIRS + 1, "the exact pairs and 7001: {listed}");
    assert!(
        clear
            .iter()
            .all(|p| description(&p["payment"]) != "Rival transfer"),
        "the unnamed payment is not clear while two invoices fit it: {listed}"
    );

    let before = listed["generation"].clone();
    let (status, linked) = send(
        &state,
        "POST",
        "/api/settlements/link-all",
        Some(json!({"generation": before, "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked["linked"], PAIRS + 1);
    let after = generation(&state).await;
    assert_eq!(
        counter(&after) - counter(&before),
        1,
        "N links publish ONE generation"
    );
    assert_eq!(linked["generation"], after);

    // Exactly the listed pairs: the rival transfer, clear only now that 7001
    // is linked, was never shown as clear, so it is not linked.
    let books = std::fs::read_to_string(dir.path().join("books.journal")).expect("read");
    let transfer = books
        .split("Rival transfer")
        .nth(1)
        .expect("the transfer")
        .split("\n\n")
        .next()
        .unwrap_or_default();
    assert!(
        !transfer.contains("match:"),
        "the rival transfer was linked: {transfer}"
    );
    let now = get(&state, &format!("/api/settlements?today={TODAY}")).await;
    let left = now["proposals"].as_array().expect("proposals");
    assert_eq!(left.len(), 1, "{now}");
    assert_eq!(description(&left[0]["payment"]), "Rival transfer");
    assert_eq!(
        left[0]["clear"], true,
        "it is clear now, for the next Link all"
    );
    // The editor and the snapshot agree: every side still carries its fingerprint.
    for p in left {
        assert!(
            p["invoice"]["fingerprint"].is_string() && p["payment"]["fingerprint"].is_string(),
            "{p}"
        );
    }

    // A stale generation is refused and writes nothing.
    let bytes = std::fs::read(dir.path().join("books.journal")).expect("read");
    let (status, _) = send(
        &state,
        "POST",
        "/api/settlements/link-all",
        Some(json!({"generation": before, "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        std::fs::read(dir.path().join("books.journal")).expect("read"),
        bytes
    );

    // ---- Post: N entries, one write ---------------------------------------
    let fixture = tempfile::TempDir::new().expect("dir");
    for name in [
        "books.journal",
        "books-2025.journal",
        "recognition.journal",
        "ledgeline.toml",
    ] {
        std::fs::copy(
            common::fixtures_dir().join("accrual").join(name),
            fixture.path().join(name),
        )
        .expect("copy");
    }
    let (state, _scratch) = state_in(fixture.path());
    let due = get(&state, &format!("/api/accrual?today={TODAY}")).await;
    let before = due["generation"].clone();
    let count = due["counts"]["due"].as_u64().expect("due");
    assert!(count > 1, "{due}");
    let (status, posted) = send(
        &state,
        "POST",
        "/api/recognition/post",
        Some(json!({"generation": before, "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{posted}");
    assert_eq!(
        posted["posted"].as_array().map(Vec::len),
        Some(count as usize)
    );
    let after = generation(&state).await;
    assert_eq!(
        counter(&after) - counter(&before),
        1,
        "N entries publish ONE generation"
    );
}
