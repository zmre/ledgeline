//! The main journal's `=` rules over HTTP (plan 30 Phase 5): applied when
//! the `hledger.conf` in force says `--auto`, marked on the rows, listed at
//! `GET /api/auto-rules`, and never written back through an edit.
//!
//! Each test runs over a fresh copy of `fixtures/auto-postings/` (main file,
//! included rules file, `hledger.conf`), so edits and config changes never
//! touch the committed fixture.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh copy of the fixture directory; `with_conf` keeps its `hledger.conf`.
fn fixture_copy(with_conf: bool) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join("ledgeline-auto-endpoint-tests")
        .join(format!("{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let source = common::fixtures_dir().join("auto-postings");
    for name in ["main.journal", "rules.journal", "hledger.conf"] {
        if name == "hledger.conf" && !with_conf {
            continue;
        }
        std::fs::copy(source.join(name), dir.join(name)).expect("copy fixture");
    }
    dir
}

fn state_in(dir: &Path) -> AppState {
    AppState::from_journal_path(dir.join("main.journal")).expect("editor opens")
}

async fn request(
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
    // An error is `text/plain`: keep its sentence as a JSON string.
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, value)
}

async fn get(state: &AppState, uri: &str) -> Value {
    let (status, body) = request(state, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

/// Every row, oldest first.
async fn rows(state: &AppState) -> Vec<Value> {
    get(state, "/api/journal/rows?order=asc&limit=1000").await["rows"]
        .as_array()
        .expect("rows")
        .clone()
}

/// The row of the January sale (2026-01-10).
async fn january_sale(state: &AppState) -> Value {
    rows(state)
        .await
        .into_iter()
        .find(|row| row["date"] == "2026-01-10")
        .expect("the January sale")
}

fn accounts(row: &Value) -> Vec<(String, bool)> {
    row["postings"]
        .as_array()
        .expect("postings")
        .iter()
        .map(|posting| {
            (
                posting["account"].as_str().expect("account").to_string(),
                posting.get("generated").is_some(),
            )
        })
        .collect()
}

fn owned(rows: &[(&str, bool)]) -> Vec<(String, bool)> {
    rows.iter().map(|(a, g)| ((*a).to_string(), *g)).collect()
}

#[tokio::test]
async fn rows_mark_generated_postings_when_the_conf_says_auto() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let sale = january_sale(&state).await;
    assert_eq!(
        accounts(&sale),
        owned(&[
            ("revenue:sales", false),
            ("liabilities:tax:sales", true),
            ("assets:checking", true),
            ("assets:checking", false),
        ])
    );
    let generated = &sale["postings"][1]["generated"];
    assert_eq!(generated, &json!({"rule": 0, "query": "revenue:sales"}));
    // hledger's own tag rides along, as `print --auto` shows it.
    assert!(
        sale["postings"][1]["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .any(|tag| tag[0] == "generated-posting" && tag[1] == "= revenue:sales")
    );
}

#[tokio::test]
async fn without_auto_the_rules_are_listed_but_never_applied() {
    let dir = fixture_copy(false);
    let state = state_in(&dir);
    let sale = january_sale(&state).await;
    assert_eq!(
        accounts(&sale),
        owned(&[("revenue:sales", false), ("assets:checking", false)])
    );
    let listed = get(&state, "/api/auto-rules").await;
    assert_eq!(listed["inForce"], false);
    assert_eq!(listed["conf"], Value::Null);
    let generated: Vec<&Value> = listed["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .map(|rule| &rule["generated"])
        .collect();
    assert_eq!(generated, [&json!(0), &json!(0), &json!(0)]);
}

#[tokio::test]
async fn the_rule_list_names_files_relatively_and_counts_what_each_rule_did() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let listed = get(&state, "/api/auto-rules").await;
    assert_eq!(listed["inForce"], true);
    assert_eq!(listed["conf"], "hledger.conf");
    assert_eq!(listed["confHijacked"], false);
    let rules = listed["rules"].as_array().expect("rules");
    let summary: Vec<(String, String, Value, Value)> = rules
        .iter()
        .map(|rule| {
            (
                rule["file"].as_str().expect("file").to_string(),
                rule["query"].as_str().expect("query").to_string(),
                rule["generated"].clone(),
                rule["unsupported"].clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "rules.journal".into(),
                "revenue:sales".into(),
                json!(4),
                Value::Null
            ),
            (
                "rules.journal".into(),
                "tag:taxable".into(),
                json!(2),
                Value::Null
            ),
            (
                "rules.journal".into(),
                "expenses:subscriptions".into(),
                json!(6),
                Value::Null
            ),
        ]
    );
    assert!(
        rules[0]["raw"]
            .as_str()
            .expect("raw")
            .starts_with("= revenue:sales\n")
    );
    assert!(rules[0]["line"].as_u64().expect("line") > 0);
    // No absolute path anywhere in the body (security layer 5).
    let text = listed.to_string();
    assert!(!text.contains(&*dir.to_string_lossy()), "{text}");
}

/// Turning `--auto` off in the config file and republishing (what Settings ›
/// hledger config's write does) drops the generated postings and changes the
/// generation, so a polling client refreshes.
#[tokio::test]
async fn republishing_after_a_conf_change_toggles_the_rules() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let before = get(&state, "/api/journal").await["generation"].clone();
    assert_eq!(accounts(&january_sale(&state).await).len(), 4);

    std::fs::write(dir.join("hledger.conf"), "# no --auto here\n").expect("write conf");
    state.republish();
    let after = get(&state, "/api/journal").await["generation"].clone();
    assert_ne!(before, after);
    assert_eq!(accounts(&january_sale(&state).await).len(), 2);
    assert_eq!(get(&state, "/api/auto-rules").await["inForce"], false);

    std::fs::write(dir.join("hledger.conf"), "--auto\n").expect("write conf");
    state.republish();
    assert_eq!(accounts(&january_sale(&state).await).len(), 4);
}

/// A PATCH addresses postings as SERVED. The sale's written `assets:checking`
/// is served fourth, below the rule's two postings: patching position 3 must
/// rewrite that line, and only that line.
#[tokio::test]
async fn a_patch_below_generated_postings_rewrites_the_written_one() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let sale = january_sale(&state).await;
    let index = sale["index"].as_u64().expect("index");
    let (status, body) = request(
        &state,
        "PATCH",
        &format!("/api/transactions/{index}"),
        Some(json!({"postings": [{"index": 3, "account": "assets:savings"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = std::fs::read_to_string(dir.join("main.journal")).expect("read");
    assert!(
        text.contains("    revenue:sales           $-333.33\n    assets:savings\n"),
        "{text}"
    );
    assert!(
        !text.contains("\n    liabilities:tax:sales"),
        "a generated posting was written: {text}"
    );
    assert_eq!(
        accounts(&january_sale(&state).await),
        owned(&[
            ("revenue:sales", false),
            ("liabilities:tax:sales", true),
            ("assets:checking", true),
            ("assets:savings", false),
        ])
    );
}

#[tokio::test]
async fn a_patch_on_a_generated_posting_is_a_409_naming_the_rule() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let before = std::fs::read_to_string(dir.join("main.journal")).expect("read");
    let index = january_sale(&state).await["index"].as_u64().expect("index");
    let (status, body) = request(
        &state,
        "PATCH",
        &format!("/api/transactions/{index}"),
        Some(json!({"postings": [{"index": 1, "account": "liabilities:tax:other"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{status} {body}");
    let message = body.as_str().expect("a sentence");
    assert!(message.contains("`= revenue:sales`"), "{message}");
    assert!(
        message.contains("Settings › Automatic postings"),
        "{message}"
    );
    let after = std::fs::read_to_string(dir.join("main.journal")).expect("read");
    assert_eq!(before, after);
}

#[tokio::test]
async fn a_put_carrying_a_generated_posting_is_a_409_and_writes_nothing() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let before = std::fs::read_to_string(dir.join("main.journal")).expect("read");
    let index = january_sale(&state).await["index"].as_u64().expect("index");
    let money = |m: &str| json!({"commodity": "$", "quantity": {"mantissa": m, "places": 3}});
    let (status, body) = request(
        &state,
        "PUT",
        &format!("/api/transactions/{index}"),
        Some(json!({
            "date": "2026-01-10",
            "description": "Acme | sale",
            "postings": [
                {"account": "revenue:sales", "amount": money("-333330")},
                {"account": "liabilities:tax:sales", "amount": money("-33333"),
                 "generated": {"rule": 0, "query": "revenue:sales"}},
                {"account": "assets:checking"}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{status} {body}");
    assert!(body.to_string().contains("= revenue:sales"), "{body}");
    assert_eq!(
        before,
        std::fs::read_to_string(dir.join("main.journal")).expect("read")
    );
}

/// The modal's save: the written postings only. The file gains no generated
/// posting, and the rule still fires on what was saved.
#[tokio::test]
async fn a_put_of_the_written_postings_saves_and_the_rule_still_applies() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let index = january_sale(&state).await["index"].as_u64().expect("index");
    let (status, body) = request(
        &state,
        "PUT",
        &format!("/api/transactions/{index}"),
        Some(json!({
            "date": "2026-01-10",
            "description": "Acme | big sale",
            "postings": [
                {"account": "revenue:sales",
                 "amount": {"commodity": "$", "quantity": {"mantissa": "-50000", "places": 2}}},
                {"account": "assets:checking"}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = std::fs::read_to_string(dir.join("main.journal")).expect("read");
    assert!(text.contains("Acme | big sale"), "{text}");
    assert!(!text.contains("\n    liabilities:tax:sales"), "{text}");
    let sale = january_sale(&state).await;
    assert_eq!(sale["postings"][1]["account"], "liabilities:tax:sales");
    assert_eq!(
        sale["postings"][1]["amounts"][0]["qty"],
        json!({"m": "-50", "p": 0})
    );
}

/// Decision 9: a rule hledger `--auto` would refuse the journal over is a
/// Problems-drawer finding, never a refused journal.
#[tokio::test]
async fn a_rule_hledger_refuses_is_a_diagnostic_not_a_refused_journal() {
    let dir = fixture_copy(true);
    std::fs::write(
        dir.join("rules.journal"),
        "= revenue:sales\n    liabilities:tax:sales  $10\n",
    )
    .expect("write rules");
    let state = state_in(&dir);
    assert_eq!(rows(&state).await.len(), 9);
    let diagnostics = get(&state, "/api/diagnostics?today=2026-10-28")
        .await
        .to_string();
    assert!(diagnostics.contains("auto-rule"), "{diagnostics}");
    assert!(diagnostics.contains("= revenue:sales"), "{diagnostics}");
    let listed = get(&state, "/api/auto-rules").await;
    let kinds: Vec<&str> = listed["rules"][0]["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .map(|problem| problem["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds, ["unbalanced", "unbalanced"]);
}
