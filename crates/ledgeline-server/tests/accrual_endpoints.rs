//! Plan 45 Phase 2: the accrual routes over a temp copy of
//! `fixtures/accrual` (synthetic; clock 2026-11-17), and the gate off on
//! `sample.journal`. Every write is checked on disk.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppDirs, AppState, AutoRecognized, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

const TODAY: &str = "2026-11-17";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A scratch copy of the fixture: its two journals, the recognition file
/// unless `without_rules`, and `ledgeline.toml` unless `without_books`.
fn fixture_copy(without_rules: bool, without_books: bool) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join("ledgeline-accrual-endpoint-tests")
        .join(format!("{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let fixture = common::fixtures_dir().join("accrual");
    let mut files = vec!["books.journal", "books-2025.journal"];
    if !without_rules {
        files.push("recognition.journal");
    }
    if !without_books {
        files.push("ledgeline.toml");
    }
    for name in files {
        std::fs::copy(fixture.join(name), dir.join(name)).expect("copy");
    }
    dir
}

fn state_in(dir: &Path) -> (AppState, tempfile::TempDir) {
    let scratch = tempfile::TempDir::new().expect("scratch app dirs");
    let state = AppState::from_journal_path(dir.join("books.journal"))
        .expect("editor opens")
        .with_app_dirs(AppDirs::under(scratch.path()));
    (state, scratch)
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
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
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn get(state: &AppState, uri: &str) -> Value {
    let (status, body) = send(state, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "GET {uri}: {body}");
    body
}

#[tokio::test]
async fn the_gate_is_off_on_the_household_sample() {
    let scratch = tempfile::TempDir::new().expect("scratch");
    let state = AppState::from_journal(&common::fixture_journal())
        .with_app_dirs(AppDirs::under(scratch.path()));
    for uri in [
        "/api/accrual",
        "/api/recognition/due",
        "/api/recognition/needs",
        "/api/settlements",
        "/api/recognition/for/1",
        "/api/settlements/for/1",
    ] {
        assert_eq!(get(&state, uri).await, json!({"enabled": false}), "{uri}");
    }
    let (status, _) = send(
        &state,
        "POST",
        "/api/recognition/post",
        Some(json!({"generation": "x"})),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn accrual_answers_the_gate_counts_and_file() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let body = get(&state, &format!("/api/accrual?today={TODAY}")).await;
    assert_eq!(body["enabled"], true);
    assert_eq!(body["closedThrough"], "2026-09-30");
    assert_eq!(
        body["file"],
        json!({"path": "recognition.journal", "exists": true, "included": false})
    );
    assert_eq!(body["counts"]["due"], 10, "{body}");
    assert_eq!(body["counts"]["dueClosed"], 1);
    assert_eq!(body["counts"]["needsSchedule"], 1);
    // Clear only (Wingtip, exact by number); Tailspin's $30 short is to check.
    assert_eq!(body["counts"]["invoices"], 1, "{body}");
    assert_eq!(body["counts"]["invoicesCheck"], 1);
    // Under the user's own settings: no tolerance, no fee proposal.
    let strict = get(
        &state,
        &format!("/api/accrual?today={TODAY}&settlementTolerance=0"),
    )
    .await;
    assert_eq!(strict["counts"]["invoices"], 1);
    assert_eq!(strict["counts"]["invoicesCheck"], 0, "{strict}");
    // A lookback that ends before Wingtip's payment leaves it out.
    let short = get(
        &state,
        &format!("/api/accrual?today={TODAY}&settlementLookback=30"),
    )
    .await;
    assert_eq!(short["counts"]["invoices"], 0, "{short}");
    assert_eq!(
        body["roles"]["assets:prepaid"],
        json!({"role": "prepaid", "declared": true})
    );
    assert_eq!(body["reason"]["kind"], "declared");
}

#[tokio::test]
async fn needs_lists_the_unscheduled_deferrals_inside_the_lookback() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let body = get(&state, &format!("/api/recognition/needs?today={TODAY}")).await;
    assert_eq!(body["enabled"], true);
    assert_eq!(body["lookback"], 90);
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["description"], "Fourth Coffee Invoice 2004");
    assert_eq!(items[0]["invoice"], "2004");
    assert!(
        items[0]["account"]
            .as_str()
            .is_some_and(|a| a.contains("deferred")),
        "{body}"
    );
    // The same count `/api/accrual` serves for the badge.
    let accrual = get(&state, &format!("/api/accrual?today={TODAY}")).await;
    assert_eq!(accrual["counts"]["needsSchedule"], items.len());
    // A lookback that ends before the invoice leaves it out; 0 is everything.
    let narrow = get(
        &state,
        &format!("/api/recognition/needs?today={TODAY}&lookback=7"),
    )
    .await;
    assert_eq!(narrow["items"], json!([]));
    let all = get(
        &state,
        &format!("/api/recognition/needs?today={TODAY}&lookback=0"),
    )
    .await;
    assert!(all["items"].as_array().is_some_and(|a| !a.is_empty()));
}

#[tokio::test]
async fn due_lists_months_groups_ties_and_the_closed_section() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let body = get(&state, &format!("/api/recognition/due?today={TODAY}")).await;
    let months = body["months"].as_array().expect("months");
    assert_eq!(months.len(), 1);
    assert_eq!(months[0]["month"], "2026-11");
    assert_eq!(months[0]["count"], 10);
    assert!(months[0]["ties"].as_array().is_some_and(|t| !t.is_empty()));
    assert_eq!(body["closed"]["count"], 1);
    assert_eq!(body["closed"]["entries"][0]["recognized"], "2026-09-10");
    // A client's later date never raises the server's; an earlier one lowers it.
    let earlier = get(&state, "/api/recognition/due?today=2026-11-05").await;
    assert_eq!(earlier["months"][0]["count"], 6);
}

#[tokio::test]
async fn post_writes_once_refuses_stale_and_future_and_places_by_year() {
    // No `[books]`: the December 2025 entry, removed from the 2025 file, is
    // then open, and lands back in the 2025 file.
    let dir = fixture_copy(false, true);
    let year = read(&dir, "books-2025.journal");
    let entry_at = year
        .find("2025-12-17 Customer X Recognized")
        .expect("entry");
    std::fs::write(dir.join("books-2025.journal"), &year[..entry_at]).expect("write");
    let (state, _scratch) = state_in(&dir);
    let accrual = get(&state, &format!("/api/accrual?today={TODAY}")).await;
    let generation = accrual["generation"]
        .as_str()
        .expect("generation")
        .to_string();

    let (status, body) = send(
        &state,
        "POST",
        "/api/recognition/post",
        Some(json!({"generation": generation, "through": "2026-11-18", "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "future: {body}");

    let (status, posted) = send(
        &state,
        "POST",
        "/api/recognition/post",
        Some(json!({"generation": generation, "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{posted}");
    // Ten for November, September's (no longer closed), and December 2025.
    assert_eq!(
        posted["posted"].as_array().map(Vec::len),
        Some(12),
        "{posted}"
    );
    assert!(read(&dir, "books-2025.journal").contains("recognized: 2025-12-17"));
    assert!(read(&dir, "books.journal").contains("recognized: 2026-11-17"));

    // The old generation is stale; the new one writes nothing more.
    let (status, _) = send(
        &state,
        "POST",
        "/api/recognition/post",
        Some(json!({"generation": generation, "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let before = read(&dir, "books.journal");
    let (status, again) = send(
        &state,
        "POST",
        "/api/recognition/post",
        Some(json!({"generation": posted["generation"], "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["posted"], json!([]));
    assert_eq!(read(&dir, "books.journal"), before);
}

#[tokio::test]
async fn rows_group_expand_and_hide_with_totals_unchanged() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let grouped = get(&state, "/api/journal/rows?limit=500").await;
    let groups: Vec<&Value> = grouped["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter(|row| row.get("recognitionGroup").is_some())
        .collect();
    assert!(!groups.is_empty());
    let october = groups
        .iter()
        .find(|row| row["recognitionGroup"]["month"] == "2026-10")
        .expect("an October group");
    let count = october["recognitionGroup"]["count"]
        .as_u64()
        .expect("count");
    assert!(count >= 2);

    let expanded = get(&state, "/api/journal/rows?limit=500&expand=2026-10").await;
    assert_eq!(
        expanded["total"].as_u64(),
        grouped["total"].as_u64().map(|t| t + count)
    );

    let hidden = get(&state, "/api/journal/rows?limit=500&recognition=hide").await;
    assert!(hidden["recognitionHidden"].as_u64().is_some_and(|n| n > 0));
    let shown = get(&state, "/api/journal/rows?limit=500&recognition=show").await;
    assert!(shown["total"].as_u64() > grouped["total"].as_u64());

    // Totals are the same whichever way the rows are shown (D8, Q11).
    let summary = |mode: &str| format!("/api/journal/summary?recognition={mode}&today={TODAY}");
    let a = get(&state, &summary("hide")).await;
    let b = get(&state, &summary("show")).await;
    assert!(a["recognitionHidden"].as_u64().is_some_and(|n| n > 0));
    assert_eq!(a["commodities"], b["commodities"]);
}

#[tokio::test]
async fn the_file_is_created_explicitly_once() {
    let dir = fixture_copy(true, false);
    let (state, _scratch) = state_in(&dir);
    let accrual = get(&state, &format!("/api/accrual?today={TODAY}")).await;
    assert_eq!(accrual["file"]["exists"], false);
    let (status, body) = send(&state, "POST", "/api/recognition/file", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["path"], "recognition.journal");
    assert!(read(&dir, "recognition.journal").contains("Do not `include` this file"));
    let (status, _) = send(&state, "POST", "/api/recognition/file", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let accrual = get(&state, &format!("/api/accrual?today={TODAY}")).await;
    assert_eq!(accrual["file"]["exists"], true);
}

#[tokio::test]
async fn save_writes_the_source_then_the_rules() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let body = json!({
        "source": {
            "kind": "add",
            "transaction": {
                "date": "2026-11-10",
                "description": "Northwind Traders Payment",
                "postings": [
                    {"account": "assets:bank:checking",
                     "amount": {"commodity": "$", "quantity": {"mantissa": "240000", "places": 2}}},
                    {"account": "liabilities:deferred-revenue",
                     "amount": {"commodity": "$", "quantity": {"mantissa": "-240000", "places": 2}},
                     "comment": "customer: Northwind Traders, recognition: nt1110, term-months: 12"}
                ]
            }
        },
        "schedules": [{
            "id": "nt1110",
            "lines": [{
                "sourceAccount": "liabilities:deferred-revenue",
                "targetAccount": "revenue:subscriptions",
                "commodity": "$",
                "quantity": "-2400.00",
                "party": {"kind": "customer", "name": "Northwind Traders"}
            }],
            "start": "2026-11-10",
            "term": {"months": 12},
            "description": "Northwind Traders Recognized"
        }],
        "today": TODAY
    });
    let (status, preview) = send(
        &state,
        "POST",
        "/api/recognition/preview",
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(
        preview["appended"][0]["periods"].as_array().map(Vec::len),
        Some(12)
    );
    let rules_before = read(&dir, "recognition.journal");
    assert_eq!(
        read(&dir, "recognition.journal"),
        rules_before,
        "a preview writes nothing"
    );

    let mut save = body;
    save["accrualGeneration"] = preview["accrualGeneration"].clone();
    let (status, saved) = send(&state, "POST", "/api/recognition/save", Some(save.clone())).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(read(&dir, "books.journal").contains("Northwind Traders Payment"));
    let rules = read(&dir, "recognition.journal");
    assert!(rules.starts_with(&rules_before));
    assert!(rules.contains("~ monthly from 2026-11-10 to 2027-10-11  Northwind Traders Recognized  ; recognition: nt1110"));
    // The same save again is stale (the books moved).
    let (status, _) = send(&state, "POST", "/api/recognition/save", Some(save)).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_restatement_with_a_closed_period_previews_and_applies_one_adjustment() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    // Customer Y: $10,000 → $9,400 from Feb 3; closed through Sep 30.
    let body = json!({
        "schedules": [{
            "id": "cy0203",
            "lines": [{
                "sourceAccount": "liabilities:deferred-revenue",
                "targetAccount": "revenue:subscriptions",
                "commodity": "$",
                "quantity": "-9400.00",
                "party": {"kind": "customer", "name": "Customer Y"}
            }],
            "start": "2026-02-03",
            "term": {"months": 12},
            "description": "Customer Y Recognized"
        }],
        "today": TODAY
    });
    let (status, preview) = send(
        &state,
        "POST",
        "/api/recognition/preview",
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let restated = &preview["restatements"][0];
    let actions: Vec<&str> = restated["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["action"].as_str().expect("action"))
        .collect();
    assert_eq!(&actions[..8], &["kept"; 8], "{actions:?}");
    assert_eq!(actions[8], "rewrite");
    assert_eq!(restated["adjustment"]["date"], TODAY);

    let (status, saved) = send(&state, "POST", "/api/recognition/save", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let books = read(&dir, "books.journal");
    assert!(
        books.contains("restated-through: 2026-09-30"),
        "the adjustment is written"
    );
    assert!(read(&dir, "recognition.journal").contains("$783.33"));
}

#[tokio::test]
async fn a_short_payment_links_with_a_fee() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let body = get(&state, &format!("/api/settlements?today={TODAY}")).await;
    let proposal = body["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .find(|p| p["fit"]["kind"] == "short")
        .expect("Tailspin is short")
        .clone();
    let side = |which: &str| {
        json!({
            "index": proposal[which]["index"],
            "posting": proposal[which]["posting"],
            "fingerprint": proposal[which]["fingerprint"],
        })
    };
    let (status, refused) = send(
        &state,
        "POST",
        "/api/settlements/link",
        Some(json!({"invoice": side("invoice"), "payment": side("payment")})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a fee account is needed: {refused}"
    );
    let (status, linked) = send(
        &state,
        "POST",
        "/api/settlements/link",
        Some(json!({
            "invoice": side("invoice"),
            "payment": side("payment"),
            "feeAccount": "expenses:payment fees"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    let id = linked["matchId"].as_str().expect("id");
    let books = read(&dir, "books.journal");
    assert_eq!(books.matches(&format!("match:{id}")).count(), 2, "{books}");
    let after = get(&state, &format!("/api/settlements?today={TODAY}")).await;
    assert_eq!(after["proposals"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn closing_the_books_writes_ledgeline_toml() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let (status, body) = send(
        &state,
        "POST",
        "/api/books/closed-through",
        Some(json!({"date": "2026-10-31"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["closedThrough"], "2026-10-31");
    assert!(read(&dir, "ledgeline.toml").contains("closed-through = \"2026-10-31\""));
    let accrual = get(&state, &format!("/api/accrual?today={TODAY}")).await;
    assert_eq!(accrual["closedThrough"], "2026-10-31");
}

#[tokio::test]
async fn problems_carry_the_recognition_findings() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let body = get(&state, &format!("/api/diagnostics?today={TODAY}")).await;
    let rules: Vec<&str> = body["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter_map(|d| d["rule"].as_str())
        .filter(|rule| rule.starts_with("recognition-"))
        .collect();
    assert!(rules.contains(&"recognition-duplicate"), "{rules:?}");
    assert!(rules.contains(&"recognition-closed-due"), "{rules:?}");
}

#[tokio::test]
async fn projections_never_offer_the_recognition_file() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let body = get(&state, "/api/projections").await;
    let ids: Vec<&str> = body["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(|f| f["id"].as_str())
        .collect();
    assert!(ids.contains(&"books-2025.journal"), "{ids:?}");
    assert!(!ids.contains(&"recognition.journal"), "{ids:?}");
}

#[tokio::test]
async fn auto_recognize_posts_when_asked_and_only_once() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    // Off by default: nothing happens.
    assert_eq!(
        state.run_auto_recognize_once(TODAY),
        Ok(AutoRecognized::Off)
    );
    // The ledger turns it on in its own `[ui]` table.
    let toml = read(&dir, "ledgeline.toml");
    std::fs::write(
        dir.join("ledgeline.toml"),
        format!("{toml}\n[ui]\nautoRecognize = true\n"),
    )
    .expect("write");
    assert_eq!(
        state.run_auto_recognize_once(TODAY),
        Ok(AutoRecognized::Posted(10))
    );
    assert_eq!(
        state.run_auto_recognize_once(TODAY),
        Ok(AutoRecognized::Unchanged)
    );
    // Closed periods are never auto-posted.
    let accrual = get(&state, &format!("/api/accrual?today={TODAY}")).await;
    assert_eq!(accrual["counts"]["due"], 0);
    assert_eq!(accrual["counts"]["dueClosed"], 1);
}

#[tokio::test]
async fn for_a_source_transaction_lists_period_status_and_renewal() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let rows = get(
        &state,
        "/api/journal/rows?limit=500&recognition=show&q=Customer%20X%20Payment",
    )
    .await;
    let index = rows["rows"][0]["index"].as_u64().expect("index");
    let body = get(
        &state,
        &format!("/api/recognition/for/{index}?today={TODAY}"),
    )
    .await;
    let periods = body["schedules"][0]["periods"].as_array().expect("periods");
    assert_eq!(periods.len(), 12);
    assert_eq!(periods[0]["status"], "posted");
    assert_eq!(periods[11]["status"], "due");
}

// ---------------------------------------------------------------------------
// The editor and the served snapshot agree after every accrual write
// (Patrick, 2026-11-18: after one Link, every other Link stayed disabled —
// the listed sides came back with no fingerprints).
// ---------------------------------------------------------------------------

/// Every settlement side `/api/settlements` lists carries a fingerprint: the
/// bound editor and the served snapshot are the same journal.
async fn assert_linkable(state: &AppState, after: &str) {
    let body = get(state, &format!("/api/settlements?today={TODAY}")).await;
    let sides = body["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .flat_map(|p| [&p["invoice"], &p["payment"]])
        .chain(body["open"]["invoices"].as_array().expect("open invoices"))
        .chain(body["open"]["payments"].as_array().expect("open payments"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(!sides.is_empty(), "after {after}: nothing listed: {body}");
    for side in &sides {
        assert!(
            side["fingerprint"].as_str().is_some_and(|f| !f.is_empty()),
            "after {after}: a side has no fingerprint: {side}"
        );
    }
}

/// A plain journal edit still lands: the bound editor isn't stale.
async fn assert_editable(state: &AppState, after: &str) {
    let body = get(state, "/api/journal/rows?limit=1").await;
    let index = body["rows"][0]["index"].as_u64().expect("a row");
    let (status, answer) = send(
        state,
        "PATCH",
        &format!("/api/transactions/{index}"),
        Some(json!({"description": format!("Edited after {after}")})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "after {after}: PATCH refused: {answer}"
    );
}

fn side_of(proposal: &Value, which: &str) -> Value {
    json!({
        "index": proposal[which]["index"],
        "posting": proposal[which]["posting"],
        "fingerprint": proposal[which]["fingerprint"],
    })
}

#[tokio::test]
async fn every_link_stays_linkable_after_the_first() {
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    assert_linkable(&state, "load").await;
    let body = get(&state, &format!("/api/settlements?today={TODAY}")).await;
    let clear = body["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .find(|p| p["clear"] == true)
        .expect("Wingtip is clear")
        .clone();
    let (status, linked) = send(
        &state,
        "POST",
        "/api/settlements/link",
        Some(json!({"invoice": side_of(&clear, "invoice"), "payment": side_of(&clear, "payment")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_linkable(&state, "link").await;

    // The second link, with the fingerprints the re-read served.
    let body = get(&state, &format!("/api/settlements?today={TODAY}")).await;
    let short = body["proposals"][0].clone();
    let (status, second) = send(
        &state,
        "POST",
        "/api/settlements/link",
        Some(json!({
            "invoice": side_of(&short, "invoice"),
            "payment": side_of(&short, "payment"),
            "feeAccount": "expenses:payment fees"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the second link: {second}");
    assert_editable(&state, "two links").await;
}

#[tokio::test]
async fn every_accrual_write_leaves_the_editor_and_the_snapshot_in_step() {
    // Post.
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let generation =
        get(&state, &format!("/api/accrual?today={TODAY}")).await["generation"].clone();
    let (status, posted) = send(
        &state,
        "POST",
        "/api/recognition/post",
        Some(json!({"generation": generation, "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{posted}");
    assert_linkable(&state, "post").await;
    assert_editable(&state, "post").await;

    // Closed-through.
    let (status, closed) = send(
        &state,
        "POST",
        "/api/books/closed-through",
        Some(json!({"date": "2026-10-31"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{closed}");
    assert_linkable(&state, "closed-through").await;
    assert_editable(&state, "closed-through").await;

    // Link all.
    let generation =
        get(&state, &format!("/api/accrual?today={TODAY}")).await["generation"].clone();
    let (status, all) = send(
        &state,
        "POST",
        "/api/settlements/link-all",
        Some(json!({"generation": generation, "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{all}");
    assert_linkable(&state, "link-all").await;
    assert_editable(&state, "link-all").await;

    // Save (rules only: a new schedule over the external invoice's deferral).
    let needs = get(&state, &format!("/api/recognition/needs?today={TODAY}")).await;
    let item = needs["items"][0].clone();
    let quantity = item["amount"]["quantity"].clone();
    let text = {
        let m: i128 = quantity["m"].as_str().expect("m").parse().expect("int");
        let p = quantity["p"].as_u64().expect("p") as u32;
        let scale = 10_i128.pow(p);
        format!(
            "{}{}.{:0width$}",
            if m < 0 { "-" } else { "" },
            m.abs() / scale,
            m.abs() % scale,
            width = p as usize
        )
    };
    let (status, saved) = send(
        &state,
        "POST",
        "/api/recognition/save",
        Some(json!({
            "schedules": [{
                "id": "fc1102",
                "lines": [{
                    "sourceAccount": item["account"],
                    "targetAccount": "revenue:subscriptions",
                    "commodity": item["amount"]["commodity"],
                    "quantity": text
                }],
                "start": "2026-11-02",
                "term": {"months": 12},
                "description": "Fourth Coffee Recognized"
            }],
            "today": TODAY
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_linkable(&state, "save").await;
    assert_editable(&state, "save").await;
}

#[tokio::test]
async fn creating_the_recognition_file_leaves_the_editor_in_step() {
    let dir = fixture_copy(true, false);
    let (state, _scratch) = state_in(&dir);
    let (status, created) = send(&state, "POST", "/api/recognition/file", Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_linkable(&state, "file").await;
    assert_editable(&state, "file").await;
}

#[tokio::test]
async fn a_refresh_after_our_own_write_keeps_the_editor_in_step() {
    // The file watcher fires after every write, ours included, and the
    // recognition file and `ledgeline.toml` are watched but are not sources.
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let generation =
        get(&state, &format!("/api/accrual?today={TODAY}")).await["generation"].clone();
    let (status, _) = send(
        &state,
        "POST",
        "/api/recognition/post",
        Some(json!({"generation": generation, "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // What the watcher now calls: our own write is already in the editor, so
    // nothing is reparsed and the generation the client just read stands.
    let before = get(&state, &format!("/api/accrual?today={TODAY}")).await["generation"].clone();
    assert!(matches!(
        state.refresh().expect("refresh"),
        ledgeline::Refreshed::Unchanged
    ));
    let after = get(&state, &format!("/api/accrual?today={TODAY}")).await["generation"].clone();
    assert_eq!(
        before, after,
        "a refresh after our own write must not republish"
    );
    // An outside edit still reloads.
    let books = dir.join("books.journal");
    let mut text = read(&dir, "books.journal");
    text.push_str(
        "\n2026-11-16 Outside edit\n    assets:bank:checking  $1.00\n    equity:opening\n",
    );
    std::fs::write(&books, text).expect("outside edit");
    assert!(matches!(
        state.refresh().expect("refresh"),
        ledgeline::Refreshed::Reloaded
    ));
    state.reload().expect("reload");
    assert_linkable(&state, "post + watcher").await;
    assert_editable(&state, "post + watcher").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reload_racing_a_listing_never_strips_the_fingerprints() {
    // What the desktop app does after every write: the file watcher sees the
    // bytes change and reloads (a fresh editor and snapshot) while the client
    // is already re-reading the list. The listing must read its snapshot and
    // its fingerprints from one journal, whichever one that is.
    let dir = fixture_copy(false, false);
    let (state, _scratch) = state_in(&dir);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reloader = {
        let (state, stop) = (state.clone(), std::sync::Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                state.reload().expect("reload");
                // Let a listing take the lock between reloads, as the
                // watcher's debounce does.
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        })
    };
    for round in 0..12 {
        assert_linkable(&state, &format!("reload race, round {round}")).await;
    }
    stop.store(true, Ordering::Relaxed);
    reloader.join().expect("reloader");
}

#[tokio::test]
async fn a_payment_before_its_bill_pairs_within_the_lead_days() {
    // Payroll: the draw, then its bill three days later (synthetic).
    let dir = tempfile::TempDir::new().expect("dir");
    std::fs::write(
        dir.path().join("books.journal"),
        "account liabilities:accounts payable  ; type: L, kpi: payable\naccount assets:bank  ; type: C\naccount expenses:payroll  ; type: X\n\
         \n2026-10-15 Gusto debit\n    assets:bank  $-5107.91\n    liabilities:accounts payable  $5107.91  ; vendor: Gusto\n\
         \n2026-10-18 Gusto payroll bill\n    expenses:payroll  $5107.91\n    liabilities:accounts payable  $-5107.91  ; vendor: Gusto\n",
    )
    .expect("write");
    let (state, _scratch) = state_in(dir.path());
    let body = get(&state, &format!("/api/settlements?today={TODAY}")).await;
    let proposal = &body["proposals"][0];
    assert_eq!(proposal["early"], true, "{body}");
    assert_eq!(proposal["gapDays"], 3);
    assert_eq!(proposal["clear"], true);
    assert_eq!(proposal["fit"]["kind"], "exact");
    let strict = get(&state, &format!("/api/settlements?today={TODAY}&lead=0")).await;
    assert_eq!(strict["proposals"], json!([]), "{strict}");
    let counts = get(&state, &format!("/api/accrual?today={TODAY}")).await["counts"].clone();
    assert_eq!(counts["invoices"], 1, "{counts}");
    let counts = get(
        &state,
        &format!("/api/accrual?today={TODAY}&settlementLead=2"),
    )
    .await["counts"]
        .clone();
    assert_eq!(counts["invoices"], 0, "{counts}");
    // It links as listed.
    let (status, linked) = send(
        &state,
        "POST",
        "/api/settlements/link-all",
        Some(json!({"generation": body["generation"], "today": TODAY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked["linked"], 1);
}
