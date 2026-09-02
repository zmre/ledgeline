//! The one thing `qb_journal_endpoints.rs` cannot prove hermetically: that the
//! journal `POST /api/import/qb-journal/commit` writes is one the REAL
//! `hledger` binary accepts — `hledger check` reports no error, `hledger
//! print` re-reads it without complaint, and every transaction it just wrote
//! still balances by hledger's own arithmetic, not just this crate's.
//!
//! Gated behind `LEDGELINE_HLEDGER_QBJOURNAL_CHECK=1`, joining the existing
//! `LEDGELINE_HLEDGER_*_CHECK` suites so `cargo test` stays hermetic. Run by
//! `just hledger-checks`.

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use common::fixtures_dir;
use http_body_util::BodyExt;
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use tower::ServiceExt;

const OPT_IN: &str = "LEDGELINE_HLEDGER_QBJOURNAL_CHECK";

macro_rules! require_hledger {
    () => {
        if std::env::var_os(OPT_IN).is_none() {
            eprintln!("skipping: set {OPT_IN}=1 (or run `just hledger-checks`)");
            return;
        }
    };
}

const FILENAME: &str = "x-ledgeline-filename";

const OPENING: &str = "2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

struct Tree {
    dir: TempDir,
    state: AppState,
}

impl Tree {
    fn bare() -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), OPENING).expect("write journal");
        let state = AppState::from_journal_path(dir.path().join("main.journal"))
            .expect("the scratch journal opens");
        Self { dir, state }
    }

    fn router(&self) -> axum::Router {
        router_with_state(self.state.clone())
    }

    fn journal_path(&self) -> PathBuf {
        self.dir.path().join("main.journal")
    }
}

fn qb_fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = fixtures_dir().join("import/qb-journal").join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {name} should read: {error}"))
}

async fn send(router: axum::Router, request: Request<Body>) -> (StatusCode, String) {
    let response = router.oneshot(request).await.expect("router responds");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn json_or_text(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_string()))
}

async fn get(tree: &Tree, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    let (status, body) = send(tree.router(), request).await;
    (status, json_or_text(&body))
}

async fn post(tree: &Tree, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("request builds");
    let (status, text) = send(tree.router(), request).await;
    (status, json_or_text(&text))
}

async fn put(tree: &Tree, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("PUT")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("request builds");
    let (status, text) = send(tree.router(), request).await;
    (status, json_or_text(&text))
}

async fn upload(tree: &Tree, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/api/import/stage")
        .header(HeaderName::from_static(FILENAME), name)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("request builds");
    let (status, text) = send(tree.router(), request).await;
    (status, json_or_text(&text))
}

async fn staged(tree: &Tree, fixture: &str) -> String {
    let (status, body) = upload(tree, fixture, qb_fixture(fixture)).await;
    assert_eq!(status, StatusCode::OK, "{fixture}: {body}");
    body["stageId"]
        .as_str()
        .unwrap_or_else(|| panic!("{fixture}: no stageId in {body}"))
        .to_string()
}

async fn map_every_unmapped_account(tree: &Tree, stage_id: &str) {
    let (status, preview) = get(tree, &format!("/api/import/qb-journal/{stage_id}")).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let unmapped: Vec<String> = preview["unmappedAccounts"]
        .as_array()
        .expect("unmappedAccounts array")
        .iter()
        .map(|value| value.as_str().expect("string").to_string())
        .collect();
    if unmapped.is_empty() {
        return;
    }

    let (status, aliases) = get(tree, "/api/aliases").await;
    assert_eq!(status, StatusCode::OK, "{aliases}");
    let root = &aliases["files"][0];
    let journal_id = root["journalId"].as_str().expect("journalId");
    let revision = root["revision"].as_str().expect("revision");

    let edits: Vec<Value> = unmapped
        .iter()
        .enumerate()
        .map(|(index, account)| {
            json!({
                "kind": "append",
                "pattern": account,
                "replacement": format!("assets:qb:{index}"),
                "regex": false,
            })
        })
        .collect();
    let (status, response) = put(
        tree,
        &format!("/api/aliases/{journal_id}"),
        json!({ "revision": revision, "edits": edits }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
}

/// `hledger --no-conf -f JOURNAL check`, over the file `commit` just wrote.
///
/// `--no-conf` ahead of the subcommand, for the reason `docs/imports.md` §
/// *No hledger we run reads a config file* gives: a config file can replace
/// the command.
fn hledger_check(journal: &PathBuf) -> Result<(), String> {
    let output = Command::new("hledger")
        .arg("--no-conf")
        .arg("-f")
        .arg(journal)
        .arg("check")
        .output()
        .map_err(|e| format!("could not run hledger: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "hledger check exited {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// `hledger --no-conf -f JOURNAL print -O json`.
fn hledger_print_json(journal: &PathBuf) -> Result<Value, String> {
    let output = Command::new("hledger")
        .arg("--no-conf")
        .arg("-f")
        .arg(journal)
        .arg("print")
        .arg("-O")
        .arg("json")
        .output()
        .map_err(|e| format!("could not run hledger: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "hledger print exited {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("hledger JSON: {e}"))
}

#[tokio::test]
async fn hledger_accepts_and_balances_the_journal_qb_journal_commit_writes() {
    require_hledger!();
    let tree = Tree::bare();
    let id = staged(&tree, "simple.xlsx").await;
    map_every_unmapped_account(&tree, &id).await;

    let (status, body) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["imported"], 2);

    hledger_check(&tree.journal_path()).expect("hledger check accepts the written journal");
    let printed =
        hledger_print_json(&tree.journal_path()).expect("hledger print re-reads the journal");
    let transactions = printed.as_array().expect("an array");
    // The opening balance, plus the two QuickBooks transactions.
    assert_eq!(transactions.len(), 3, "{printed}");

    let tagged: Vec<&str> = transactions
        .iter()
        .filter_map(|txn| {
            txn["ttags"].as_array().and_then(|tags| {
                tags.iter().find_map(|pair| {
                    (pair[0] == "id").then(|| pair[1].as_str().expect("tag value"))
                })
            })
        })
        .collect();
    assert_eq!(
        tagged.len(),
        2,
        "both QuickBooks transactions carry hledger's own `id` tag: {printed}"
    );
    assert!(tagged.contains(&"441"));
    assert!(tagged.contains(&"33"));
}

/// The full-size round-trip fixture: 45 groups, 100 posting rows, every
/// transaction type the real export uses.
#[tokio::test]
async fn hledger_accepts_the_full_report_fixture() {
    require_hledger!();
    let tree = Tree::bare();
    let id = staged(&tree, "report.xlsx").await;
    map_every_unmapped_account(&tree, &id).await;

    let (status, body) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["imported"], 45, "{body}");

    hledger_check(&tree.journal_path()).expect("hledger check accepts the written journal");
    let printed =
        hledger_print_json(&tree.journal_path()).expect("hledger print re-reads the journal");
    assert_eq!(
        printed.as_array().expect("an array").len(),
        46,
        "the opening balance plus the report's 45 transactions"
    );
}
