//! The `/api/import/qb-journal/*` HTTP surface (WP-17 Phase B), plus the
//! QuickBooks Journal branch of `POST /api/import/stage`.
//!
//! Hermetic throughout: nothing here shells out to hledger (this pipeline
//! never does — see `qb_journal_api`'s module docs), so every test in this
//! file runs in a plain `cargo test`. The `LEDGELINE_HLEDGER_QBJOURNAL_CHECK`
//! opt-in suite that proves the *written* journal is one `hledger check`/
//! `hledger print` accept lives in `qb_journal_hledger_check.rs`.

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use common::fixtures_dir;
use http_body_util::BodyExt;
use ledgeline::{AccessToken, AppState, Security, router_with_security, router_with_state};
use serde_json::{Value, json};
use std::path::PathBuf;
use tempfile::TempDir;
use tower::ServiceExt;

/// The upload header carrying the dropped file's name.
const FILENAME: &str = "x-ledgeline-filename";

/// A throwaway journal directory plus the [`AppState`] bound to it.
struct Tree {
    dir: TempDir,
    state: AppState,
}

/// The opening balance every scratch journal starts from, so it is a valid,
/// balanced journal before any import touches it.
const OPENING: &str = "2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

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

    fn journal_text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("main.journal")).expect("journal readable")
    }
}

fn qb_fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = fixtures_dir().join("import/qb-journal").join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {name} should read: {error}"))
}

// ---------------------------------------------------------------------------
// HTTP helpers (mirrors import_endpoints.rs's own, which cannot be shared
// across an integration test binary boundary)
// ---------------------------------------------------------------------------

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

/// Stage `fixture` and return its `stageId`.
async fn staged(tree: &Tree, fixture: &str) -> String {
    let (status, body) = upload(tree, fixture, qb_fixture(fixture)).await;
    assert_eq!(status, StatusCode::OK, "{fixture}: {body}");
    body["stageId"]
        .as_str()
        .unwrap_or_else(|| panic!("{fixture}: no stageId in {body}"))
        .to_string()
}

/// Add a plain alias mapping every account `preview` reports as unmapped to a
/// synthetic `assets:qb:N` account, through the *existing* alias-editing
/// route (`PUT /api/aliases/{*journalId}`) — the plan's "narrow alias
/// exception" reuses this path rather than growing a second one.
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

// ===========================================================================
// The token guard
// ===========================================================================

#[tokio::test]
async fn every_qb_journal_route_requires_the_token() {
    const PORT: u16 = 5099;
    const HOST: &str = "127.0.0.1:5099";
    let tree = Tree::bare();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");

    let probe = |method: &'static str, uri: String, auth: Option<&'static str>| {
        let state = tree.state.clone();
        let security = Security::local(token.clone(), PORT);
        async move {
            let mut builder = Request::builder()
                .method(method)
                .uri(uri)
                .header(HeaderName::from_static("host"), HOST);
            if let Some(value) = auth {
                builder = builder.header(header::AUTHORIZATION, value);
            }
            let request = builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .expect("request builds");
            router_with_security(state, security)
                .oneshot(request)
                .await
                .expect("router responds")
                .status()
        }
    };

    for (method, uri) in [
        (
            "GET",
            "/api/import/qb-journal/0123456789abcdef0123456789abcdef".to_string(),
        ),
        ("POST", "/api/import/qb-journal/commit".to_string()),
    ] {
        assert_eq!(
            probe(method, uri.clone(), None).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} without a token must be 401"
        );
        assert_eq!(
            probe(method, uri.clone(), Some("Bearer wrong-token-entirely")).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with a wrong token must be 401"
        );
        assert_ne!(
            probe(method, uri.clone(), Some("Bearer integration-test-token")).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with the token must get past the guard"
        );
    }
}

// ===========================================================================
// stage — detection and refusal
// ===========================================================================

#[tokio::test]
async fn a_quickbooks_journal_upload_is_detected_and_diverted() {
    let tree = Tree::bare();
    let (status, body) = upload(&tree, "simple.xlsx", qb_fixture("simple.xlsx")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["format"], "quickbooks-journal");
    assert!(body["stageId"].as_str().is_some_and(|id| !id.is_empty()));
    // The ordinary CSV/candidate machinery has nothing to say about this
    // upload: it never ran.
    assert_eq!(body["candidates"], json!([]));
}

#[tokio::test]
async fn an_ordinary_csv_still_takes_the_csv_path() {
    let tree = Tree::bare();
    let csv = b"Date,Description,Amount\n2026-01-05,GROCERY STORE,-54.20\n".to_vec();
    let (status, body) = upload(&tree, "bank.csv", csv).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["format"], "csv");
}

#[tokio::test]
async fn a_damaged_export_is_refused_by_name_rather_than_staged() {
    let tree = Tree::bare();
    let (status, body) = upload(
        &tree,
        "mismatched-total.xlsx",
        qb_fixture("mismatched-total.xlsx"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body.as_str().unwrap_or_default();
    assert!(
        message.contains("sums to") && message.contains("total row says"),
        "{message}"
    );
}

// ===========================================================================
// preview
// ===========================================================================

#[tokio::test]
async fn preview_of_an_unknown_stage_is_a_404() {
    let tree = Tree::bare();
    let (status, body) = get(
        &tree,
        "/api/import/qb-journal/0123456789abcdef0123456789abcdef",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn preview_reports_every_account_unmapped_with_no_aliases_declared() {
    let tree = Tree::bare();
    let id = staged(&tree, "simple.xlsx").await;
    let (status, preview) = get(&tree, &format!("/api/import/qb-journal/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["transactionCount"], 2);
    assert_eq!(preview["postingCount"], 4);
    assert_eq!(preview["idMatches"], Value::Null);
    let unmapped: Vec<&str> = preview["unmappedAccounts"]
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("string"))
        .collect();
    assert!(unmapped.contains(&"Riverbank BUSINESS CHECKING (0002)"));
    assert!(unmapped.contains(&"3000 Member Equity"));
    assert!(unmapped.contains(&"2005 Northbank Credit Card"));
    assert!(
        unmapped.contains(&"6000 Sales and Marketing:6001 Sales & Marketing Tools"),
        "{unmapped:?}"
    );
}

#[tokio::test]
async fn preview_reports_id_matches_once_every_account_is_mapped() {
    let tree = Tree::bare();
    let id = staged(&tree, "simple.xlsx").await;
    map_every_unmapped_account(&tree, &id).await;

    let (status, preview) = get(&tree, &format!("/api/import/qb-journal/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["unmappedAccounts"], json!([]));
    assert_eq!(preview["idMatches"]["new"], 2);
    assert_eq!(preview["idMatches"]["unchanged"], 0);
    assert_eq!(preview["idMatches"]["conflictingTotal"], 0);
}

// ===========================================================================
// commit — refusal
// ===========================================================================

#[tokio::test]
async fn commit_refuses_and_names_every_unmapped_account() {
    let tree = Tree::bare();
    let id = staged(&tree, "simple.xlsx").await;
    let before = tree.journal_text();

    let (status, body) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body.as_str().unwrap_or_default();
    assert!(
        message.contains("Riverbank BUSINESS CHECKING (0002)"),
        "{message}"
    );
    assert!(message.contains("3000 Member Equity"), "{message}");
    assert_eq!(tree.journal_text(), before, "nothing was written");
}

#[tokio::test]
async fn commit_of_an_unknown_stage_is_a_404() {
    let tree = Tree::bare();
    let (status, body) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": "0123456789abcdef0123456789abcdef" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

// ===========================================================================
// commit — write, dedup, conflict
// ===========================================================================

#[tokio::test]
async fn commit_writes_every_transaction_tagged_with_its_quickbooks_id() {
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
    assert_eq!(body["idMatches"]["new"], 2);

    let text = tree.journal_text();
    assert!(text.contains("id: 441"), "{text}");
    assert!(text.contains("id: 33"), "{text}");
    // The deposit's own sign check, carried all the way to the written file.
    assert!(text.contains("74999.71"), "{text}");
    // The scratch journal writes `$1000.00` in its opening balance but never
    // declares a `D` directive, which is the ordinary shape of a real
    // journal. An import that only consults `default_commodity` would write
    // these amounts with NO commodity symbol at all.
    assert!(
        text.contains("$74999.71") || text.contains("74999.71 $"),
        "an amount imported into a $-denominated journal must keep the $ sign: {text}"
    );
}

/// The "re-downloading is safe" property `plans/17-quickbooks-journal-import.md`
/// documents: a WIDER export that re-contains an already-imported transaction
/// alongside a new one commits only the new one, with no duplication.
///
/// `overlap.xlsx` carries group `441` byte-for-byte identical to the one
/// `simple.xlsx` also carries under that id, and group `6`, an id neither
/// `simple.xlsx` nor `default-columns.xlsx` ever uses.
#[tokio::test]
async fn a_wider_re_download_imports_only_the_new_group_and_leaves_the_overlap_alone() {
    let tree = Tree::bare();
    let id = staged(&tree, "simple.xlsx").await;
    map_every_unmapped_account(&tree, &id).await;
    let (status, first) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["imported"], 2);

    let wider = staged(&tree, "overlap.xlsx").await;
    map_every_unmapped_account(&tree, &wider).await;
    let (status, second) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": wider }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["imported"], 1, "only group 6 is new: {second}");
    assert_eq!(second["idMatches"]["new"], 1);
    assert_eq!(second["idMatches"]["unchanged"], 1);
    assert_eq!(second["idMatches"]["conflictingTotal"], 0);

    let text = tree.journal_text();
    assert!(text.contains("id: 6"), "{text}");
    assert_eq!(
        text.matches("id: 441").count(),
        1,
        "the overlapping group must appear exactly once, not duplicated: {text}"
    );
}

#[tokio::test]
async fn a_second_commit_of_the_same_export_imports_nothing_new() {
    let tree = Tree::bare();
    let id = staged(&tree, "simple.xlsx").await;
    map_every_unmapped_account(&tree, &id).await;

    let (status, first) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let after_first = tree.journal_text();

    // The same stage, committed again — the "re-downloading is safe" property
    // the plan documents (`plans/17-quickbooks-journal-import.md`).
    let (status, second) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["imported"], 0);
    assert_eq!(second["idMatches"]["new"], 0);
    assert_eq!(second["idMatches"]["unchanged"], 2);
    assert_eq!(
        tree.journal_text(),
        after_first,
        "a re-commit of an unchanged export must not touch the file"
    );
}

#[tokio::test]
async fn a_hand_edited_transaction_is_reported_conflicting_and_never_overwritten() {
    let tree = Tree::bare();
    let id = staged(&tree, "simple.xlsx").await;
    map_every_unmapped_account(&tree, &id).await;

    let (status, first) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");

    // Hand-edit the deposit's amount directly in the file, simulating a user
    // correction made after the import landed.
    let edited = tree.journal_text().replace("74999.71", "70000.00");
    assert_ne!(edited, tree.journal_text(), "the replacement must have hit");
    std::fs::write(tree.dir.path().join("main.journal"), &edited).expect("hand-edit");
    // The editor's own external-change guard would otherwise refuse the next
    // write as stale; re-bind it to the file exactly as the desktop's
    // File→Open action (and the live-reload watcher) would.
    tree.state
        .rebind_editor(tree.dir.path().join("main.journal"))
        .expect("rebind after the external edit");

    let (status, second) = post(
        &tree,
        "/api/import/qb-journal/commit",
        json!({ "stageId": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["imported"], 0, "a conflict must never be written");
    assert_eq!(second["idMatches"]["unchanged"], 1);
    assert_eq!(second["idMatches"]["conflictingTotal"], 1);
    assert_eq!(second["idMatches"]["conflicting"][0]["id"], "441");

    assert!(
        tree.journal_text().contains("70000.00"),
        "the hand-edit must survive the commit"
    );
    assert!(
        !tree.journal_text().contains("74999.71"),
        "the conflicting row must not have been silently restored either"
    );
}

// ===========================================================================
// ordering
// ===========================================================================

#[tokio::test]
async fn commit_reports_ordering_for_the_file_it_wrote_into() {
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
    assert_eq!(body["ordering"]["inOrder"], true);
    let files = body["ordering"]["files"].as_array().expect("files array");
    assert_eq!(files.len(), 1, "only main.journal was touched");
    assert_eq!(files[0]["journalId"], "main.journal");
}
