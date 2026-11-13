//! File → New (plan 39 Phase 6): the `/api/new-ledger*` routes, driven through
//! the real router with a fake native shell standing in for the folder picker.
//!
//! What is pinned here: the folder is chosen by the host and named by a
//! one-time id, never by a client path; a folder holding anything but
//! dotfiles is refused and nothing in it is touched; an existing file is never
//! overwritten; no response carries an absolute path; the new journal is
//! served by the very next request; and without a host (the headless server)
//! nothing can be written at all.

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{
    AccessToken, AppDirs, AppState, DesktopHost, SAMPLE_JOURNAL, Security, router_with_security,
    router_with_state,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tower::ServiceExt;

/// The native shell, faked: picks whatever folder the test set, and records
/// every journal the server says it opened.
#[derive(Default)]
struct FakeHost {
    folder: Mutex<Option<PathBuf>>,
    opened: Mutex<Vec<PathBuf>>,
}

impl FakeHost {
    fn will_pick(&self, folder: Option<&Path>) {
        *self.folder.lock().unwrap() = folder.map(Path::to_path_buf);
    }

    fn opened(&self) -> Vec<PathBuf> {
        self.opened.lock().unwrap().clone()
    }
}

impl DesktopHost for FakeHost {
    fn pick_folder(&self) -> Option<PathBuf> {
        self.folder.lock().unwrap().clone()
    }

    fn journal_opened(&self, main: &Path) {
        self.opened.lock().unwrap().push(main.to_path_buf());
    }
}

/// A desktop session: an open journal, a fake host, and a scratch area that
/// holds the folders a test picks.
struct Desktop {
    _journal_dir: TempDir,
    scratch: TempDir,
    host: Arc<FakeHost>,
    state: AppState,
}

impl Desktop {
    fn new() -> Self {
        let journal_dir = TempDir::new().expect("temp dir");
        let main = journal_dir.path().join("main.journal");
        std::fs::write(&main, "; Old books\n2026-01-01 x\n    a  $1\n    b\n").unwrap();
        let state = AppState::from_journal_path(&main).expect("opens");
        Self::over(state, journal_dir)
    }

    /// A first run: nothing open, the same empty state the GUI starts with.
    fn first_run() -> Self {
        let empty = ledgeline_core::parse_journal("", "").expect("empty parses");
        Self::over(AppState::from_journal(&empty), TempDir::new().unwrap())
    }

    fn over(state: AppState, journal_dir: TempDir) -> Self {
        let scratch = TempDir::new().expect("scratch");
        let host = Arc::new(FakeHost::default());
        let state = state
            .with_app_dirs(AppDirs::under(scratch.path().join("app")))
            .with_desktop(host.clone());
        Self {
            _journal_dir: journal_dir,
            scratch,
            host,
            state,
        }
    }

    /// A new folder under the scratch area, made empty.
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.scratch.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String) {
        call(&self.state, method, uri, body).await
    }

    async fn pick(&self, folder: &Path) -> Value {
        self.host.will_pick(Some(folder));
        let (status, body) = self.call("POST", "/api/new-ledger/folder", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_no_path(&body, self.scratch.path());
        serde_json::from_str(&body).unwrap()
    }
}

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.map_or_else(Body::empty, |body| {
            Body::from(serde_json::to_vec(&body).unwrap())
        }))
        .expect("request builds");
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// No response may carry the folder's path, in either spelling macOS gives a
/// temp dir.
fn assert_no_path(body: &str, dir: &Path) {
    let raw = dir.to_string_lossy();
    let canonical = dir.canonicalize().unwrap();
    let canonical = canonical.to_string_lossy();
    assert!(!body.contains(raw.as_ref()), "path leaked: {body}");
    assert!(!body.contains(canonical.as_ref()), "path leaked: {body}");
}

fn usd() -> Value {
    json!({"commodity": "$", "side": "left", "spaced": false, "decimals": 2, "decimalMark": "period"})
}

async fn template(desktop: &Desktop, kind: &str) -> Value {
    let (status, body) = desktop
        .call(
            "GET",
            &format!("/api/new-ledger/template?kind={kind}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str::<Value>(&body).unwrap()["accounts"].clone()
}

fn create_body(folder_id: &str, title: &str, accounts: Value) -> Value {
    json!({
        "folderId": folder_id,
        "title": title,
        "currency": usd(),
        "accounts": accounts,
    })
}

#[tokio::test]
async fn templates_are_listed_with_their_tags() {
    let desktop = Desktop::new();
    let business = template(&desktop, "business").await;
    let receivable = business
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["name"] == "assets:receivable")
        .expect("the business chart has receivables");
    assert!(
        receivable["tags"]
            .as_array()
            .unwrap()
            .contains(&json!({"name": "kpi", "value": "receivable"}))
    );
    assert!(
        template(&desktop, "individual")
            .await
            .as_array()
            .unwrap()
            .len()
            > 20
    );
    assert_eq!(template(&desktop, "empty").await, json!([]));

    let (status, _) = desktop
        .call("GET", "/api/new-ledger/template?kind=sideways", None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn status_says_whether_a_ledger_can_be_made_and_one_is_open() {
    let desktop = Desktop::new();
    let (_, body) = desktop.call("GET", "/api/new-ledger", None).await;
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"desktop": true, "journalOpen": true})
    );

    let first = Desktop::first_run();
    let (_, body) = first.call("GET", "/api/new-ledger", None).await;
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"desktop": true, "journalOpen": false})
    );
}

#[tokio::test]
async fn the_headless_server_can_list_templates_but_never_write() {
    let dir = TempDir::new().unwrap();
    let main = dir.path().join("main.journal");
    std::fs::write(&main, "").unwrap();
    let state = AppState::from_journal_path(&main).unwrap();

    let (_, body) = call(&state, "GET", "/api/new-ledger", None).await;
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"desktop": false, "journalOpen": true})
    );
    let (status, _) = call(&state, "GET", "/api/new-ledger/template?kind=empty", None).await;
    assert_eq!(status, StatusCode::OK);
    for (uri, body) in [
        ("/api/new-ledger/folder", None),
        ("/api/new-ledger/sample", None),
        (
            "/api/new-ledger",
            Some(create_body("anything", "Books", json!([]))),
        ),
    ] {
        let (status, text) = call(&state, "POST", uri, body).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{uri}: {text}");
        assert!(text.contains("desktop app"), "{text}");
    }
}

#[tokio::test]
async fn create_writes_the_ledger_opens_it_and_names_no_path() {
    let desktop = Desktop::new();
    let folder = desktop.folder("Acme Books");
    std::fs::write(folder.join(".DS_Store"), "finder").unwrap();
    let picked = desktop.pick(&folder).await;
    assert_eq!(picked["folder"]["name"], "Acme Books");
    assert_eq!(picked["folder"]["empty"], true);
    let id = picked["folder"]["id"].as_str().unwrap().to_string();

    let accounts = template(&desktop, "business").await;
    let (status, body) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(&id, "Acme Books", accounts.clone())),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_no_path(&body, desktop.scratch.path());
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"folder": "Acme Books", "files": ["main.journal", "accounts.journal"]})
    );

    let main = std::fs::read_to_string(folder.join("main.journal")).unwrap();
    assert!(main.starts_with("; Acme Books\n"), "{main}");
    assert!(!main.to_lowercase().contains("fiscal"), "{main}");
    assert!(main.contains("D $1,000.00"));
    let declared = std::fs::read_to_string(folder.join("accounts.journal")).unwrap();
    assert!(
        declared.contains(
            "account assets:receivable  ; bsgroup: Accounts receivable, kpi: receivable\n"
        )
    );
    assert!(
        !folder.join("ledgeline.toml").exists(),
        "never auto-created"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join(".DS_Store")).unwrap(),
        "finder"
    );

    // The host heard about it, and the server already serves the new journal.
    assert_eq!(
        desktop.host.opened(),
        vec![folder.join("main.journal").canonicalize().unwrap()]
    );
    let (_, info) = desktop.call("GET", "/api/journal", None).await;
    let info: Value = serde_json::from_str(&info).unwrap();
    assert_eq!(info["title"], "Acme Books", "{info}");
    let (_, listed) = desktop.call("GET", "/api/accounts", None).await;
    assert!(listed.contains("assets:receivable"), "{listed}");
    assert_eq!(
        serde_json::from_str::<Value>(&listed).unwrap()["editable"],
        true
    );

    // The id was spent.
    let (status, _) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(&id, "Again", json!([]))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_first_run_can_create_its_first_ledger() {
    let desktop = Desktop::first_run();
    let folder = desktop.folder("books");
    let id = desktop.pick(&folder).await["folder"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(
                &id,
                "Mine",
                template(&desktop, "individual").await,
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, info) = desktop.call("GET", "/api/new-ledger", None).await;
    assert_eq!(
        serde_json::from_str::<Value>(&info).unwrap()["journalOpen"],
        true
    );
    // Edits now go to the new journal.
    let (status, body) = desktop
        .call(
            "POST",
            "/api/transactions",
            Some(json!({
                "date": "2026-11-12",
                "description": "first",
                "postings": [
                    {"account": "expenses:food:groceries", "amount": {"commodity": "$", "quantity": {"mantissa": "1250", "places": 2}}},
                    {"account": "assets:bank:checking"}
                ]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let main = std::fs::read_to_string(folder.join("main.journal")).unwrap();
    assert!(main.contains("first"), "{main}");
}

#[tokio::test]
async fn a_folder_with_files_in_it_is_refused_and_left_alone() {
    let desktop = Desktop::new();
    let folder = desktop.folder("busy");
    std::fs::write(folder.join("notes.txt"), "mine").unwrap();
    let picked = desktop.pick(&folder).await;
    assert_eq!(picked["folder"]["empty"], false);
    let id = picked["folder"]["id"].as_str().unwrap();

    let (status, body) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(id, "Busy", json!([]))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("isn't empty"), "{body}");
    assert_no_path(&body, desktop.scratch.path());
    let names: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("notes.txt")]);
    assert!(desktop.host.opened().is_empty());
}

#[tokio::test]
async fn an_existing_journal_is_never_overwritten() {
    let desktop = Desktop::new();
    let folder = desktop.folder("taken");
    let id = desktop.pick(&folder).await["folder"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Something lands in the folder between the pick and the create.
    std::fs::write(folder.join("main.journal"), "; precious\n").unwrap();
    let (status, _) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(&id, "New", json!([]))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        std::fs::read_to_string(folder.join("main.journal")).unwrap(),
        "; precious\n"
    );
    assert!(!folder.join("accounts.journal").exists());
}

#[tokio::test]
async fn a_bad_answer_is_refused_and_keeps_the_folder_choice() {
    let desktop = Desktop::new();
    let folder = desktop.folder("retry");
    let id = desktop.pick(&folder).await["folder"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, body) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(&id, "  ", json!([]))),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(
                &id,
                "Fine",
                json!([{"name": "assets;bad", "tags": []}]),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let mut wrong_currency = create_body(&id, "Fine", json!([]));
    wrong_currency["currency"]["commodity"] = json!("1$");
    let (status, _) = desktop
        .call("POST", "/api/new-ledger", Some(wrong_currency))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);

    let (status, body) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(&id, "Fine", json!([]))),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn only_the_picked_folder_can_be_named() {
    let desktop = Desktop::new();
    let (status, _) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body("never-picked", "X", json!([]))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let first = desktop.folder("first");
    let old = desktop.pick(&first).await["folder"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let second = desktop.folder("second");
    desktop.pick(&second).await;
    // Picking again replaces the choice: the first id is gone.
    let (status, _) = desktop
        .call(
            "POST",
            "/api/new-ledger",
            Some(create_body(&old, "X", json!([]))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(std::fs::read_dir(&first).unwrap().count(), 0);

    // A cancelled picker answers null and forgets the previous choice.
    desktop.host.will_pick(None);
    let (status, body) = desktop.call("POST", "/api/new-ledger/folder", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"folder": null})
    );
}

#[tokio::test]
async fn the_sample_opens_from_the_app_cache() {
    let desktop = Desktop::new();
    let (status, body) = desktop.call("POST", "/api/new-ledger/sample", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_no_path(&body, desktop.scratch.path());
    let copy = desktop
        .scratch
        .path()
        .join("app/cache/sample/sample.journal");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), SAMPLE_JOURNAL);
    assert_eq!(desktop.host.opened(), vec![copy.canonicalize().unwrap()]);
    let (_, rows) = desktop.call("GET", "/api/journal/summary", None).await;
    assert!(!rows.contains("\"count\":0"), "{rows}");
}

#[test]
fn the_embedded_sample_is_the_fixture() {
    let fixture = std::fs::read_to_string(common::fixtures_dir().join("sample.journal")).unwrap();
    assert_eq!(
        SAMPLE_JOURNAL, fixture,
        "copy fixtures/sample.journal to crates/ledgeline-server/src/sample.journal"
    );
}

#[tokio::test]
async fn every_new_ledger_route_requires_the_token() {
    const PORT: u16 = 5098;
    const HOST: &str = "127.0.0.1:5098";
    let desktop = Desktop::new();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");
    for (method, uri) in [
        ("GET", "/api/new-ledger"),
        ("POST", "/api/new-ledger"),
        ("GET", "/api/new-ledger/template?kind=empty"),
        ("POST", "/api/new-ledger/folder"),
        ("POST", "/api/new-ledger/sample"),
    ] {
        for (auth, refused) in [(None, true), (Some("Bearer integration-test-token"), false)] {
            let mut builder = Request::builder()
                .method(method)
                .uri(uri)
                .header(HeaderName::from_static("host"), HOST)
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(value) = auth {
                builder = builder.header(header::AUTHORIZATION, value);
            }
            let status =
                router_with_security(desktop.state.clone(), Security::local(token.clone(), PORT))
                    .oneshot(builder.body(Body::from("{}")).unwrap())
                    .await
                    .unwrap()
                    .status();
            assert_eq!(
                status == StatusCode::UNAUTHORIZED,
                refused,
                "{method} {uri} with {auth:?}: {status}"
            );
        }
    }
}
