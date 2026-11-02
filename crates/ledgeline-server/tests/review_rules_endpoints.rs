//! "Remember as a rule" from the Journal view over HTTP (plan 32 D27):
//! `GET /api/review/rules-for/{index}` and `POST /api/review/rule-preview`.
//!
//! Every test runs over a fresh copy of `fixtures/review-rules/` (markers
//! included). The selection rules themselves are pinned in `ledgeline-core`'s
//! `tests/rules_infer.rs`; these pin the wire, both preview bases, the
//! refusals, and that a preview writes nothing.
//!
//! The CSV-basis preview shells out to a real `hledger` (`print` over the
//! statement), so those tests are gated behind `LEDGELINE_HLEDGER_IMPORT_CHECK=1`
//! like `import_endpoints.rs`, and run by `just hledger-checks`. Everything
//! else here is hermetic.

mod common;

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

/// Opts in to the checks that shell out to a real `hledger`. Set by
/// `just hledger-checks`.
const IMPORT_CHECK: &str = "LEDGELINE_HLEDGER_IMPORT_CHECK";

/// Skip, loudly, unless the opt-in variable is set.
macro_rules! require_hledger {
    () => {
        if std::env::var_os(IMPORT_CHECK).is_none() {
            eprintln!("skipping: set {IMPORT_CHECK}=1 (or run `just hledger-checks`)");
            return;
        }
    };
}

/// A copy of the fixture tree, removed on drop.
struct Tree {
    dir: tempfile::TempDir,
    state: AppState,
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy");
        }
    }
}

fn tree() -> Tree {
    let dir = tempfile::tempdir().expect("temp dir");
    copy_tree(&common::fixtures_dir().join("review-rules"), dir.path());
    let state = AppState::from_journal_path(dir.path().join("main.journal")).expect("opens");
    Tree { dir, state }
}

impl Tree {
    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, header::HeaderMap, Value) {
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
        .expect("router responds");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body")
        .to_bytes();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, headers, value)
}

/// The tindex of the fixture transaction described `description`: its
/// 1-based position in `main.journal`.
fn index_of(description: &str) -> u32 {
    let text = std::fs::read_to_string(common::fixtures_dir().join("review-rules/main.journal"))
        .expect("fixture");
    let headers = text
        .lines()
        .filter(|line| line.starts_with("20"))
        .collect::<Vec<_>>();
    let at = headers
        .iter()
        .position(|line| line.contains(description))
        .unwrap_or_else(|| panic!("no {description} in the fixture"));
    u32::try_from(at + 1).expect("small")
}

async fn rules_for(tree: &Tree, description: &str) -> Value {
    let uri = format!("/api/review/rules-for/{}", index_of(description));
    let (status, headers, body) = send(&tree.state, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert!(headers.get(header::ETAG).is_none());
    body
}

fn coffee_rule(pattern: &str) -> Value {
    json!({
        "matchers": [{"field": "description", "pattern": pattern}],
        "assignments": [{"field": "account2", "value": "expenses:food:coffee"}],
    })
}

async fn preview(
    tree: &Tree,
    description: &str,
    rules_id: &str,
    rule: Value,
) -> (StatusCode, Value) {
    let body = json!({"index": index_of(description), "rulesId": rules_id, "rule": rule});
    let (status, headers, body) =
        send(&tree.state, "POST", "/api/review/rule-preview", Some(body)).await;
    if status == StatusCode::OK {
        assert_eq!(
            headers
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
    }
    (status, body)
}

#[tokio::test]
async fn one_file_is_recommended_and_described() {
    let tree = tree();
    let body = rules_for(&tree, "CHASE THING").await;
    assert_eq!(body["index"], index_of("CHASE THING"));
    assert_eq!(body["needsChoice"], false);
    let candidates = body["candidates"].as_array().expect("array");
    assert_eq!(candidates.len(), 1);
    let chase = &candidates[0];
    assert_eq!(chase["rulesId"], "import/chase.csv.rules");
    assert_eq!(chase["label"], "chase");
    assert_eq!(chase["family"], "chase");
    assert_eq!(chase["familySize"], 1);
    assert_eq!(chase["year"], Value::Null);
    assert_eq!(chase["account1"], "assets:bank:chase");
    assert_eq!(chase["recommended"], true);
    assert_eq!(chase["matchField"], "description");
    assert_eq!(
        chase["fields"],
        json!(["date", "description", "amount", "fitid"])
    );
    assert!(chase["reasons"].as_array().is_some_and(|r| !r.is_empty()));
}

#[tokio::test]
async fn a_year_family_recommends_the_transactions_year_and_lists_the_newer_one() {
    let tree = tree();
    let body = rules_for(&tree, "BLUE BOTTLE COFFEE 1001").await;
    assert_eq!(body["needsChoice"], false);
    let ids: Vec<&str> = body["candidates"]
        .as_array()
        .expect("array")
        .iter()
        .map(|c| c["rulesId"].as_str().expect("id"))
        .collect();
    assert_eq!(
        ids,
        ["import/citi-2025.csv.rules", "import/citi-2026.csv.rules"]
    );
    assert_eq!(body["candidates"][0]["recommended"], true);
    assert_eq!(body["candidates"][0]["year"], 2025);
    assert_eq!(body["candidates"][1]["recommended"], false);
}

#[tokio::test]
async fn a_tie_recommends_nothing_and_says_so() {
    let tree = tree();
    let body = rules_for(&tree, "AMEX STORE").await;
    assert_eq!(body["needsChoice"], true);
    let candidates = body["candidates"].as_array().expect("array");
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().all(|c| c["recommended"] == false));
}

#[tokio::test]
async fn no_file_for_the_account_is_an_empty_list_and_a_missing_index_a_404() {
    let tree = tree();
    let body = rules_for(&tree, "CORNER STORE").await;
    assert_eq!(body["candidates"], json!([]));
    assert_eq!(body["needsChoice"], false);
    let (status, _, _) = send(&tree.state, "GET", "/api/review/rules-for/999", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_csv_basis_previews_the_rule_against_the_newest_statement() {
    require_hledger!();
    let tree = tree();
    let before = std::fs::read(tree.path("import/citi-2026.csv.rules")).expect("rules");
    let (status, body) = preview(
        &tree,
        "BLUE BOTTLE COFFEE 4412",
        "import/citi-2026.csv.rules",
        coffee_rule("blue.*bottle"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["basis"], "csv");
    assert_eq!(body["file"], "citi-2026.csv");
    assert_eq!(body["matched"], 2);
    assert_eq!(body["total"], 3);
    assert_eq!(body["alreadyCategorised"], 0);
    assert_eq!(body["changedTotal"], 2);
    let changed = body["changed"].as_array().expect("array");
    assert_eq!(changed.len(), 2);
    assert!(
        changed[0]["after"]
            .as_str()
            .is_some_and(|t| t.contains("expenses:food:coffee"))
    );
    assert!(
        changed[0]["before"]
            .as_str()
            .is_some_and(|t| t.contains("expenses:unknown"))
    );

    // A row another rule categorised counts as already categorised.
    let (_, shell) = preview(
        &tree,
        "BLUE BOTTLE COFFEE 4412",
        "import/citi-2026.csv.rules",
        coffee_rule("shell"),
    )
    .await;
    assert_eq!(
        (
            shell["matched"].clone(),
            shell["alreadyCategorised"].clone()
        ),
        (json!(1), json!(1))
    );

    // A pattern that meets no row (the D27 caveat: a renamed description is
    // not the bank's text) is matched: 0 on the CSV basis.
    let (_, none) = preview(
        &tree,
        "BLUE BOTTLE COFFEE 4412",
        "import/citi-2026.csv.rules",
        coffee_rule("blue bottle coffee co"),
    )
    .await;
    assert_eq!(
        (none["basis"].clone(), none["matched"].clone()),
        (json!("csv"), json!(0))
    );

    // Nothing the user owns was written.
    assert_eq!(
        std::fs::read(tree.path("import/citi-2026.csv.rules")).expect("rules"),
        before
    );
}

#[tokio::test]
async fn hledgers_refusal_is_reported_without_a_temporary_path() {
    require_hledger!();
    let tree = tree();
    let (status, body) = preview(
        &tree,
        "BLUE BOTTLE COFFEE 4412",
        "import/citi-2026.csv.rules",
        coffee_rule("blue[bottle"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], false);
    let stderr = body["stderr"].as_str().expect("stderr");
    assert!(
        !stderr.contains("candidate.rules") && !stderr.contains("/tmp"),
        "{stderr}"
    );
}

#[tokio::test]
async fn with_no_statement_the_journal_basis_counts_similar_transactions() {
    let tree = tree();
    let (status, body) = preview(
        &tree,
        "SCHWAB FEE",
        "bank/2026/schwab.csv.rules",
        json!({
            "matchers": [{"field": "description", "pattern": "schwab.*fee"}],
            "assignments": [{"field": "account2", "value": "expenses:bank:fees"}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"ok": true, "basis": "journal", "matched": 1, "total": 2})
    );
}

#[tokio::test]
async fn a_rules_file_with_an_include_is_not_previewed() {
    let tree = tree();
    let (status, body) = preview(
        &tree,
        "CHASE THING",
        "import/paypal.csv.rules",
        coffee_rule("paypal"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], false);
    assert!(
        body["stderr"]
            .as_str()
            .is_some_and(|s| s.contains("includes another rules file"))
    );
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let tree = tree();
    // A rules id that resolves to nothing.
    let (status, _) = preview(
        &tree,
        "CHASE THING",
        "import/nope.csv.rules",
        coffee_rule("x"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A rule the rules model refuses: an unescaped `(`, and no assignment.
    let (status, _) = preview(
        &tree,
        "CHASE THING",
        "import/chase.csv.rules",
        coffee_rule("a(b"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = preview(
        &tree,
        "CHASE THING",
        "import/chase.csv.rules",
        json!({"matchers": [{"pattern": "x"}], "assignments": []}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // A missing transaction.
    let body = json!({"index": 999, "rulesId": "import/chase.csv.rules", "rule": coffee_rule("x")});
    let (status, _, _) = send(&tree.state, "POST", "/api/review/rule-preview", Some(body)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // An unknown field.
    let body = json!({"index": 1, "rulesId": "import/chase.csv.rules", "rule": coffee_rule("x"), "extra": 1});
    let (status, _, _) = send(&tree.state, "POST", "/api/review/rule-preview", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
