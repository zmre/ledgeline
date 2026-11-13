//! QIF uploads through `POST /api/import/stage` and the whole-book routes
//! `/api/import/qb-journal/*` (plan 39 Phase 2): the same stage, preview,
//! mapping and commit the QuickBooks Journal import uses, with the source
//! reported as QIF.
//!
//! Hermetic: nothing here shells out to hledger.

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use common::fixtures_dir;
use http_body_util::BodyExt;
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

const FILENAME: &str = "x-ledgeline-filename";

const OPENING: &str = "2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

struct Tree {
    dir: TempDir,
    state: AppState,
}

impl Tree {
    fn with_journal(text: &str) -> Self {
        common::use_built_workbook_worker();
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), text).expect("write journal");
        let state = AppState::from_journal_path(dir.path().join("main.journal"))
            .expect("the scratch journal opens");
        Self { dir, state }
    }

    fn bare() -> Self {
        Self::with_journal(OPENING)
    }

    fn router(&self) -> axum::Router {
        router_with_state(self.state.clone())
    }

    fn journal_text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("main.journal")).expect("journal readable")
    }
}

fn qif_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join("import/qif").join(name))
        .unwrap_or_else(|error| panic!("fixture {name} should read: {error}"))
}

async fn send(router: axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.oneshot(request).await.expect("router responds");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let text = String::from_utf8_lossy(&body).into_owned();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

async fn get(tree: &Tree, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    send(tree.router(), request).await
}

async fn with_json(tree: &Tree, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("request builds");
    send(tree.router(), request).await
}

async fn upload(tree: &Tree, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/api/import/stage")
        .header(HeaderName::from_static(FILENAME), name)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("request builds");
    send(tree.router(), request).await
}

async fn staged(tree: &Tree, fixture: &str) -> String {
    let (status, body) = upload(tree, fixture, qif_fixture(fixture)).await;
    assert_eq!(status, StatusCode::OK, "{fixture}: {body}");
    assert_eq!(body["format"], "qif", "{body}");
    body["stageId"].as_str().expect("stageId").to_string()
}

async fn preview(tree: &Tree, stage_id: &str) -> Value {
    let (status, body) = get(tree, &format!("/api/import/qb-journal/{stage_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Map each source name to the journal account given, through the existing
/// alias route.
async fn map(tree: &Tree, mappings: &[(&str, &str)]) {
    let (status, aliases) = get(tree, "/api/aliases").await;
    assert_eq!(status, StatusCode::OK, "{aliases}");
    let root = &aliases["files"][0];
    let edits: Vec<Value> = mappings
        .iter()
        .map(|(pattern, replacement)| {
            json!({"kind": "append", "pattern": pattern, "replacement": replacement, "regex": false})
        })
        .collect();
    let (status, response) = with_json(
        tree,
        "PUT",
        &format!("/api/aliases/{}", root["journalId"].as_str().expect("id")),
        json!({"revision": root["revision"], "edits": edits}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
}

const BANKTIVITY_MAPPING: [(&str, &str); 8] = [
    ("Joint Checking", "assets:bank:checking"),
    ("High Yield Savings", "assets:bank:savings"),
    ("Rewards Card", "liabilities:card"),
    ("Equity:Opening Balances", "equity:opening"),
    ("Paycheck", "revenue:salary"),
    ("Utilities", "expenses:utilities"),
    ("Groceries", "expenses:groceries"),
    ("Interest Income", "revenue:interest"),
];

#[tokio::test]
async fn capabilities_offer_qif_and_qmtf() {
    let tree = Tree::bare();
    let (status, body) = get(&tree, "/api/import/capabilities").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let formats: Vec<&str> = body["formats"]
        .as_array()
        .expect("formats")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(formats.ends_with(&["qif", "qmtf"]), "{formats:?}");
}

#[tokio::test]
async fn a_qif_upload_previews_with_its_source_and_source_names() {
    let tree = Tree::bare();
    let stage_id = staged(&tree, "banktivity.qif").await;
    let body = preview(&tree, &stage_id).await;
    assert_eq!(body["source"], "qif");
    assert_eq!(body["sourceLabel"], "QIF");
    assert_eq!(body["transactionCount"], 10, "mirrored transfers collapse");
    assert_eq!(body["dateFormat"]["ambiguous"], false);
    let unmapped: Vec<&str> = body["unmappedAccounts"]
        .as_array()
        .expect("unmapped")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        unmapped,
        vec![
            "Joint Checking",
            "Equity:Opening Balances",
            "Paycheck",
            "High Yield Savings",
            "Utilities:Electric",
            "Utilities:Internet",
            "Rewards Card",
            "Groceries",
            "Interest Income",
        ]
    );
    assert!(body["idMatches"].is_null());
}

#[tokio::test]
async fn a_qmtf_upload_is_staged_as_qif() {
    let tree = Tree::bare();
    staged(&tree, "quicken-mac.qmtf").await;
}

#[tokio::test]
async fn a_qif_named_file_that_is_not_qif_is_refused_by_name() {
    let tree = Tree::bare();
    let (status, body) = upload(&tree, "export.qif", b"Date,Amount\n1/5/26,1\n".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.as_str()
            .is_some_and(|text| text.contains("no QIF header")),
        "{body}"
    );
}

#[tokio::test]
async fn an_unmapped_commit_names_qif_accounts() {
    let tree = Tree::bare();
    let stage_id = staged(&tree, "banktivity.qif").await;
    let (status, body) = with_json(
        &tree,
        "POST",
        "/api/import/qb-journal/commit",
        json!({"stageId": stage_id}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.as_str()
            .is_some_and(|text| text.contains("these QIF accounts have no matching alias")),
        "{body}"
    );
}

#[tokio::test]
async fn a_mapped_qif_commits_once_and_reimports_as_unchanged() {
    let tree = Tree::bare();
    map(&tree, &BANKTIVITY_MAPPING).await;
    let stage_id = staged(&tree, "banktivity.qif").await;
    let body = preview(&tree, &stage_id).await;
    assert_eq!(body["unmappedAccounts"], json!([]), "{body}");
    assert_eq!(body["idMatches"]["new"], 10);

    let (status, result) = with_json(
        &tree,
        "POST",
        "/api/import/qb-journal/commit",
        json!({"stageId": stage_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["imported"], 10);

    let journal = tree.journal_text();
    assert!(journal.contains("; id: qif-"), "{journal}");
    assert!(
        journal.contains("2026-01-01 * Opening Balance"),
        "cleared status is written: {journal}"
    );
    assert!(
        journal.contains("expenses:utilities:Electric"),
        "a parent mapping covers the sub-category: {journal}"
    );
    assert!(journal.contains("class: Personal"), "{journal}");
    assert_eq!(
        journal.matches("Transfer from checking").count(),
        0,
        "the mirrored copy is not written: {journal}"
    );

    // The same file again: every id is already in the journal.
    let again = staged(&tree, "banktivity.qif").await;
    let body = preview(&tree, &again).await;
    assert_eq!(body["idMatches"]["new"], 0, "{body}");
    assert_eq!(body["idMatches"]["unchanged"], 10, "{body}");
}

#[tokio::test]
async fn a_check_number_becomes_the_transaction_code() {
    let tree = Tree::bare();
    map(
        &tree,
        &[
            ("Checking", "assets:bank:checking"),
            ("Equity:Opening Balances", "equity:opening"),
            ("Food", "expenses:food"),
            ("Household", "expenses:household"),
            ("Salary", "revenue:salary"),
            ("Taxes", "expenses:taxes"),
            ("Dining", "expenses:dining"),
        ],
    )
    .await;
    let stage_id = staged(&tree, "splits.qif").await;
    let (status, result) = with_json(
        &tree,
        "POST",
        "/api/import/qb-journal/commit",
        json!({"stageId": stage_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let journal = tree.journal_text();
    assert!(
        journal.contains("2026-01-05 * (1041) Harbor Market"),
        "{journal}"
    );
    assert!(journal.contains("Produce and pantry"), "{journal}");
}
