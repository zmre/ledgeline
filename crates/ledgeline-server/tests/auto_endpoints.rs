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
use axum::http::{HeaderName, Request, StatusCode, header};
use ledgeline::{AccessToken, AppState, Security, router_with_security, router_with_state};
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

/// `--auto` in `~/.hledger.conf` alone — no config beside the journal — turns
/// the rules on, as it does for hledger, which falls back to the home file
/// (plan 30 Decision 10). The user directories arrive through
/// `with_app_dirs`, as the desktop binary hands them over, AFTER the first
/// snapshot was built: that snapshot must be re-read under them.
#[tokio::test]
async fn a_home_hledger_conf_s_auto_applies_the_rules() {
    let dir = fixture_copy(false);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::write(home.join(".hledger.conf"), "--auto\n").expect("home conf");

    // Without the home directory known, nothing applies.
    let state = state_in(&dir);
    assert_eq!(accounts(&january_sale(&state).await).len(), 2);

    let mut dirs = ledgeline::AppDirs::under(dir.join("app"));
    dirs.hledger_conf.home = Some(home);
    let state = state_in(&dir).with_app_dirs(dirs);
    assert_eq!(accounts(&january_sale(&state).await).len(), 4);
    let listed = get(&state, "/api/auto-rules").await;
    assert_eq!(listed["inForce"], true);
    assert_eq!(listed["conf"], "~/.hledger.conf");
}

// ===========================================================================
// Editing (plan 30 amendment 22): `PUT /api/auto-rules`
// ===========================================================================

async fn put(state: &AppState, body: Value) -> (StatusCode, Value) {
    request(state, "PUT", "/api/auto-rules", Some(body)).await
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("read")
}

/// The first rule of `rules.journal`, as listed.
async fn sales_rule(state: &AppState) -> Value {
    get(state, "/api/auto-rules").await["rules"][0].clone()
}

/// Each rule carries what its editor needs, and a new rule goes to the main
/// file.
#[tokio::test]
async fn the_listing_carries_each_rule_s_file_block_revision_and_form() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let listed = get(&state, "/api/auto-rules").await;
    assert_eq!(listed["editable"], true);
    assert_eq!(listed["newRule"]["journalId"], "main.journal");
    assert!(listed["newRule"]["revision"].is_string());
    let rules = listed["rules"].as_array().expect("rules");
    let blocks: Vec<(Value, Value)> = rules
        .iter()
        .map(|rule| (rule["journalId"].clone(), rule["block"].clone()))
        .collect();
    assert_eq!(
        blocks,
        vec![
            (json!("rules.journal"), json!(0)),
            (json!("rules.journal"), json!(1)),
            (json!("rules.journal"), json!(2)),
        ]
    );
    // The sales-tax rule is a form: 10% of a sale, revenue into a liability,
    // balanced against checking, for every transaction.
    let form = &rules[0]["form"];
    assert_eq!(form["matchAccount"], "revenue:sales");
    assert_eq!(form["postings"][0]["account"], "liabilities:tax:sales");
    assert_eq!(form["postings"][0]["kind"], "percent");
    assert_eq!(form["counter"], "assets:checking");
    assert_eq!(form["forecastOnly"], false);
    // A tag query is not an account a form can match.
    assert_eq!(rules[1]["form"], Value::Null);
}

/// A form edit rewrites its block alone: every other byte of the file, the
/// comments above the block included, is as it was, and the rule applies at
/// its new rate.
#[tokio::test]
async fn a_form_edit_rewrites_one_block_and_nothing_else() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let before = read(&dir, "rules.journal");
    let rule = sales_rule(&state).await;
    // The listing's form as the editor sends it back: decimals inbound are
    // `{mantissa, places}`, and a percent posting's amount is `null`.
    let listed = &rule["form"];
    let form = json!({
        "matchAccount": listed["matchAccount"],
        "extraQuery": listed["extraQuery"],
        "postings": [{"account": "liabilities:tax:sales", "kind": "percent",
                      "percent": {"mantissa": "8", "places": 0}, "amount": null}],
        "counter": listed["counter"],
        "comment": listed["comment"],
        "forecastOnly": listed["forecastOnly"],
    });
    let (status, body) = put(
        &state,
        json!({
            "journalId": "rules.journal",
            "revision": rule["revision"],
            "change": {"kind": "replace", "block": 0, "was": rule["raw"], "rule": {"form": form}},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = read(&dir, "rules.journal");
    let new_block = "= ^revenue:sales(:|$)\n    liabilities:tax:sales  *0.08\n    assets:checking        *-0.08\n";
    assert_eq!(
        after,
        before.replace(rule["raw"].as_str().expect("raw"), new_block)
    );
    // The answer is the listing as it now is, and the journal served applies
    // the new rate: 8% of the $-333.33 sale.
    assert_eq!(body["rules"][0]["raw"], new_block);
    let sale = january_sale(&state).await;
    assert_eq!(
        sale["postings"][1]["amounts"][0]["qty"],
        json!({"m": "-266664", "p": 4}),
        "{sale}"
    );
}

/// A rule added as text is appended to the main journal after a blank line,
/// and applies.
#[tokio::test]
async fn an_added_rule_is_appended_to_the_main_journal() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let before = read(&dir, "main.journal");
    let listed = get(&state, "/api/auto-rules").await;
    let (status, body) = put(
        &state,
        json!({
            "journalId": "main.journal",
            "revision": listed["newRule"]["revision"],
            "change": {"kind": "add", "rule": {"raw": "= expenses:food:groceries\n    (budget:tax-reserve)  *0.5"}},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The fixture already ends in a blank line, so none is added.
    assert!(before.ends_with("\n\n"));
    assert_eq!(
        read(&dir, "main.journal"),
        format!("{before}= expenses:food:groceries\n    (budget:tax-reserve)  *0.5\n")
    );
    let rules = body["rules"].as_array().expect("rules");
    assert_eq!(rules.len(), 4);
    assert_eq!(rules[3]["journalId"], "main.journal");
    assert_eq!(rules[3]["block"], 0);
    assert_eq!(rules[3]["generated"], 1);
}

#[tokio::test]
async fn a_removed_rule_is_gone_and_stops_applying() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let rule = sales_rule(&state).await;
    let (status, body) = put(
        &state,
        json!({
            "journalId": "rules.journal",
            "revision": rule["revision"],
            "change": {"kind": "remove", "block": 0, "was": rule["raw"]},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!read(&dir, "rules.journal").contains("= revenue:sales"));
    assert_eq!(body["rules"].as_array().expect("rules").len(), 2);
    assert_eq!(accounts(&january_sale(&state).await).len(), 2);
}

/// A file changed since the listing, or a block that is no longer the one
/// the client saw, is a `409`, and nothing is written.
#[tokio::test]
async fn a_stale_revision_or_block_is_a_409_and_writes_nothing() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let rule = sales_rule(&state).await;
    let before = read(&dir, "rules.journal");
    for change in [
        json!({"journalId": "rules.journal", "revision": "stale",
               "change": {"kind": "remove", "block": 0, "was": rule["raw"]}}),
        json!({"journalId": "rules.journal", "revision": rule["revision"],
               "change": {"kind": "remove", "block": 1, "was": rule["raw"]}}),
        json!({"journalId": "rules.journal", "revision": rule["revision"],
               "change": {"kind": "remove", "block": 9, "was": rule["raw"]}}),
    ] {
        let (status, body) = put(&state, change.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{change}: {body}");
    }
    assert_eq!(read(&dir, "rules.journal"), before);
}

/// What a rule body may not be: a directive smuggled in after the block, an
/// unparseable query, both kinds of rule at once, a file the journal does not
/// read.
#[tokio::test]
async fn a_bad_rule_or_file_is_refused_and_writes_nothing() {
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let main = read(&dir, "main.journal");
    let revision = get(&state, "/api/auto-rules").await["newRule"]["revision"].clone();
    for (rule, journal_id, want) in [
        (
            json!({"raw": "= a\n    (b)  *1\ninclude /etc/passwd"}),
            "main.journal",
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"raw": "= a\n    (b)  *1\n\n2026-01-01 x\n    a  1\n    b"}),
            "main.journal",
            StatusCode::BAD_REQUEST,
        ),
        (json!({}), "main.journal", StatusCode::BAD_REQUEST),
        (
            json!({"raw": "= a\n    (b)  *1"}),
            "other.journal",
            StatusCode::NOT_FOUND,
        ),
        (
            json!({"raw": "= a\n    (b)  *1"}),
            "../main.journal",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, body) = put(
            &state,
            json!({"journalId": journal_id, "revision": revision, "change": {"kind": "add", "rule": rule}}),
        )
        .await;
        assert_eq!(status, want, "{rule} → {journal_id}: {body}");
    }
    assert_eq!(read(&dir, "main.journal"), main);
}

/// The write is token-gated with the read.
#[tokio::test]
async fn the_rule_routes_require_the_token() {
    const PORT: u16 = 5097;
    let dir = fixture_copy(true);
    let state = state_in(&dir);
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");
    for (method, auth, unauthorized) in [
        ("GET", None, true),
        ("PUT", None, true),
        ("GET", Some("Bearer integration-test-token"), false),
        ("PUT", Some("Bearer integration-test-token"), false),
    ] {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/auto-rules")
            .header(HeaderName::from_static("host"), format!("127.0.0.1:{PORT}"))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(value) = auth {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        let request = builder
            .body(Body::from(
                r#"{"journalId":"main.journal","revision":"x","change":{"kind":"remove","block":0,"was":""}}"#,
            ))
            .expect("request builds");
        let status = router_with_security(state.clone(), Security::local(token.clone(), PORT))
            .oneshot(request)
            .await
            .expect("router responds")
            .status();
        assert_eq!(
            status == StatusCode::UNAUTHORIZED,
            unauthorized,
            "{method} with {auth:?}: {status}"
        );
    }
}
