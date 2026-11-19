//! Plan 45: how long one **Link** takes, end to end, on the 200k bench corpus —
//! the write, then what the client reads right after it (`/api/journal`, the
//! status for the badge, and the settlements list).
//!
//! Ignored by default: it needs the generated corpus (`LEDGELINE_BENCH_SIZES=
//! 200000 cargo bench -p ledgeline-core --bench accrual` writes
//! `target/perf/synthetic-v1-200000-accrual*.journal`) and a release build:
//!
//! ```sh
//! cargo test --release -p ledgeline --test accrual_link_timing -- --ignored --nocapture
//! ```
//!
//! The corpus has no receivables, so this appends `PAIRS` synthetic invoice ↔
//! payment pairs (exact amounts, matching invoice numbers: all clear).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppDirs, AppState, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

const PAIRS: usize = 60;
const TODAY: &str = "2026-11-17";

fn corpus(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/perf")
        .join(name)
}

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

async fn timed(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (Duration, Value) {
    let started = Instant::now();
    let (status, value) = send(state, method, uri, body).await;
    let took = started.elapsed();
    assert_eq!(status, StatusCode::OK, "{method} {uri}: {value}");
    (took, value)
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the 200k bench corpus and a release build"]
async fn one_link_round_trip_on_the_200k_corpus() {
    let main = corpus("synthetic-v1-200000-accrual.journal");
    let rules = corpus("synthetic-v1-200000-accrual-recognition.journal");
    assert!(
        main.exists(),
        "generate the corpus first: {}",
        main.display()
    );
    let dir = tempfile::TempDir::new().expect("temp");
    let mut text = std::fs::read_to_string(&main).expect("corpus");
    text.push_str("\naccount assets:accounts receivable  ; type: A, kpi: receivable\naccount assets:bank:settle  ; type: C\n");
    for n in 0..PAIRS {
        let invoice = 9000 + n;
        let amount = 100 + n;
        text.push_str(&format!(
            "\n2026-10-{day:02} Synthetic Invoice {invoice}  ; invoice: {invoice}\n    assets:accounts receivable  ${amount}.00  ; customer: Synthetic {n}\n    revenue:subscriptions\n\n2026-11-{day:02} Synthetic Payment {invoice}\n    assets:bank:settle  ${amount}.00\n    assets:accounts receivable  $-{amount}.00  ; customer: Synthetic {n}\n",
            day = 1 + n % 28,
        ));
    }
    std::fs::write(dir.path().join("books.journal"), text).expect("write main");
    std::fs::copy(&rules, dir.path().join("recognition.journal")).expect("copy rules");
    let scratch = tempfile::TempDir::new().expect("app dirs");
    let state = AppState::from_journal_path(dir.path().join("books.journal"))
        .expect("opens")
        .with_app_dirs(AppDirs::under(scratch.path()));

    let settlements_uri = format!("/api/settlements?today={TODAY}&lookback=180&tolerance=300");
    let accrual_uri =
        format!("/api/accrual?today={TODAY}&settlementLookback=180&settlementTolerance=300");
    let (cold, _) = timed(&state, "GET", &accrual_uri, None).await;
    let (first_list, list) = timed(&state, "GET", &settlements_uri, None).await;
    eprintln!(
        "cold /api/accrual {} · first /api/settlements {}",
        ms(cold),
        ms(first_list)
    );
    let clear = list["counts"]["clear"].as_u64().expect("clear");
    assert!(
        clear >= 3,
        "the appended pairs are clear: {}",
        list["counts"]
    );

    for round in 0..3 {
        let (_, list) = timed(&state, "GET", &settlements_uri, None).await;
        let proposal = list["proposals"]
            .as_array()
            .expect("proposals")
            .iter()
            .find(|p| p["clear"] == true)
            .expect("a clear proposal")
            .clone();
        let side = |which: &str| {
            json!({
                "index": proposal[which]["index"],
                "posting": proposal[which]["posting"],
                "fingerprint": proposal[which]["fingerprint"],
            })
        };
        let started = Instant::now();
        let (link, _) = timed(
            &state,
            "POST",
            "/api/settlements/link",
            Some(json!({"invoice": side("invoice"), "payment": side("payment"), "tolerance": 300})),
        )
        .await;
        let (meta, _) = timed(&state, "GET", "/api/journal", None).await;
        // The client reads these two in parallel; timed apart here.
        let (status, _) = timed(&state, "GET", &accrual_uri, None).await;
        let (relist, after) = timed(&state, "GET", &settlements_uri, None).await;
        let total = started.elapsed();
        eprintln!(
            "round {round}: link {} · /api/journal {} · /api/accrual {} · /api/settlements {} · total {}",
            ms(link),
            ms(meta),
            ms(status),
            ms(relist),
            ms(total)
        );
        for p in after["proposals"].as_array().expect("proposals") {
            assert!(
                p["invoice"]["fingerprint"].is_string() && p["payment"]["fingerprint"].is_string(),
                "{p}"
            );
        }
    }
}

/// The 200k corpus plus `pairs` clear invoice ↔ payment pairs, served.
fn corpus_state(pairs: usize) -> (AppState, tempfile::TempDir, tempfile::TempDir) {
    let main = corpus("synthetic-v1-200000-accrual.journal");
    let rules = corpus("synthetic-v1-200000-accrual-recognition.journal");
    assert!(
        main.exists(),
        "generate the corpus first: {}",
        main.display()
    );
    let dir = tempfile::TempDir::new().expect("temp");
    let mut text = std::fs::read_to_string(&main).expect("corpus");
    text.push_str("\naccount assets:accounts receivable  ; type: A, kpi: receivable\naccount assets:bank:settle  ; type: C\n");
    for n in 0..pairs {
        let invoice = 9000 + n;
        let amount = 100 + n;
        text.push_str(&format!(
            "\n2026-10-{day:02} Synthetic Invoice {invoice}  ; invoice: {invoice}\n    assets:accounts receivable  ${amount}.00  ; customer: Synthetic {n}\n    revenue:subscriptions\n\n2026-11-{day:02} Synthetic Payment {invoice}\n    assets:bank:settle  ${amount}.00\n    assets:accounts receivable  $-{amount}.00  ; customer: Synthetic {n}\n",
            day = 1 + n % 28,
        ));
    }
    std::fs::write(dir.path().join("books.journal"), text).expect("write main");
    std::fs::copy(&rules, dir.path().join("recognition.journal")).expect("copy rules");
    let scratch = tempfile::TempDir::new().expect("app dirs");
    let state = AppState::from_journal_path(dir.path().join("books.journal"))
        .expect("opens")
        .with_app_dirs(AppDirs::under(scratch.path()));
    (state, dir, scratch)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the 200k bench corpus and a release build"]
async fn link_all_on_the_200k_corpus() {
    let settlements_uri = format!("/api/settlements?today={TODAY}&lookback=180&tolerance=300");
    let accrual_uri =
        format!("/api/accrual?today={TODAY}&settlementLookback=180&settlementTolerance=300");

    // Before: what the old loop amounted to — one full write per pair.
    {
        let (state, _dir, _scratch) = corpus_state(25);
        let started = Instant::now();
        for _ in 0..25 {
            let (_, list) = timed(&state, "GET", &settlements_uri, None).await;
            let proposal = list["proposals"]
                .as_array()
                .expect("proposals")
                .iter()
                .find(|p| p["clear"] == true)
                .expect("a clear proposal")
                .clone();
            let side = |which: &str| json!({"index": proposal[which]["index"], "posting": proposal[which]["posting"], "fingerprint": proposal[which]["fingerprint"]});
            timed(
                &state,
                "POST",
                "/api/settlements/link",
                Some(json!({"invoice": side("invoice"), "payment": side("payment")})),
            )
            .await;
        }
        eprintln!("before: 25 one-at-a-time links {}", ms(started.elapsed()));
    }

    for pairs in [25, 60] {
        let (state, _dir, _scratch) = corpus_state(pairs);
        let (_, list) = timed(&state, "GET", &settlements_uri, None).await;
        assert_eq!(
            list["counts"]["clear"].as_u64(),
            Some(pairs as u64),
            "{}",
            list["counts"]
        );
        let started = Instant::now();
        let (write, linked) = timed(
            &state,
            "POST",
            "/api/settlements/link-all",
            Some(json!({"generation": list["generation"], "today": TODAY, "lookback": 180, "tolerance": 300})),
        )
        .await;
        assert_eq!(linked["linked"].as_u64(), Some(pairs as u64));
        let (meta, _) = timed(&state, "GET", "/api/journal", None).await;
        let (status, _) = timed(&state, "GET", &accrual_uri, None).await;
        let (relist, _) = timed(&state, "GET", &settlements_uri, None).await;
        eprintln!(
            "after: link-all of {pairs}: write {} · /api/journal {} · /api/accrual {} · /api/settlements {} · total {}",
            ms(write),
            ms(meta),
            ms(status),
            ms(relist),
            ms(started.elapsed())
        );
    }
}
