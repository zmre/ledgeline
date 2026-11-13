//! `GET`/`PATCH /api/ui-prefs` (plan 38, WP-5; save routing reworked
//! 2026-11-12): the SPA's view state in `ui-prefs.toml` in the config
//! directory, overridden key by key by a hand-made `ledgeline.toml` beside the
//! journal, over the built-in defaults. A save goes to `ledgeline.toml` only
//! for a key its `[ui]` table already sets; everything else goes to the
//! global file. The app never creates `ledgeline.toml`.
//!
//! Every state here is given `AppDirs::under` a scratch directory, so no test
//! reads or writes the developer's real config directory. Every journal is a
//! scratch copy, so no test writes beside a committed fixture.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{
    AccessToken, AppDirs, AppState, Security, router_with_security, router_with_state,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tower::ServiceExt;

const JOURNAL: &str = "2026-01-01 opening\n\
                       \x20   assets:bank:checking      $100\n\
                       \x20   equity:opening\n";

/// `<tmp>/a/main.journal` and `<tmp>/b/main.journal`, the app's own dirs
/// under `<tmp>/app`, and a state opened on ledger `a`.
struct Tree {
    dir: TempDir,
    state: AppState,
}

impl Tree {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        for ledger in ["a", "b"] {
            std::fs::create_dir_all(dir.path().join(ledger)).unwrap();
            std::fs::write(dir.path().join(ledger).join("main.journal"), JOURNAL).unwrap();
        }
        let dirs = AppDirs::under(dir.path().join("app"));
        dirs.create_all().expect("app dirs");
        let state = AppState::from_journal_path(dir.path().join("a/main.journal"))
            .expect("the scratch journal opens")
            .with_app_dirs(dirs);
        Self { dir, state }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn read(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.path(relative)).ok()
    }

    fn write(&self, relative: &str, text: &str) {
        std::fs::write(self.path(relative), text).unwrap();
    }

    async fn get(&self) -> (StatusCode, Value) {
        send(&self.state, "GET", None).await
    }

    async fn patch(&self, body: Value) -> (StatusCode, Value) {
        send(&self.state, "PATCH", Some(body)).await
    }

    async fn pin(&self, key: &str, pinned: bool) -> (StatusCode, Value) {
        send_pin(&self.state, json!({"key": key, "pinned": pinned})).await
    }

    fn open(&self, ledger: &str) {
        self.state
            .rebind_editor(self.path(&format!("{ledger}/main.journal")))
            .expect("the other ledger opens");
    }
}

async fn send(state: &AppState, method: &str, body: Option<Value>) -> (StatusCode, Value) {
    send_to(state, method, "/api/ui-prefs", body).await
}

async fn send_pin(state: &AppState, body: Value) -> (StatusCode, Value) {
    send_to(state, "POST", "/api/ui-prefs/pin", Some(body)).await
}

async fn send_to(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    let request = match body {
        Some(body) => request.body(Body::from(serde_json::to_vec(&body).unwrap())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

fn assert_no_absolute_path(tree: &Tree, body: &Value) {
    let text = body.to_string();
    let root = tree.dir.path().to_string_lossy().into_owned();
    assert!(!text.contains(&root), "an absolute path leaked: {text}");
    if let Ok(canonical) = tree.dir.path().canonicalize() {
        assert!(!text.contains(&*canonical.to_string_lossy()), "{text}");
    }
}

fn issue_keys(body: &Value) -> Vec<String> {
    body["issues"]
        .as_array()
        .expect("issues is an array")
        .iter()
        .map(|issue| {
            format!(
                "{}:{}",
                issue["file"].as_str().unwrap_or_default(),
                issue["key"].as_str().unwrap_or("*")
            )
        })
        .collect()
}

const GLOBAL: &str = "app/config/ui-prefs.toml";

/// Every file under the scratch tree, minus the two journals.
fn settings_files(tree: &Tree) -> Vec<String> {
    files_under(tree.dir.path())
        .into_iter()
        .filter(|file| !file.ends_with("main.journal"))
        .collect()
}

#[tokio::test]
async fn opening_with_no_settings_files_answers_the_defaults_and_creates_no_file() {
    let tree = Tree::new();
    let (status, body) = tree.get().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["storage"], "files");
    assert_eq!(body["localKeys"], json!([]));
    assert_eq!(body["issues"], json!([]));
    assert_eq!(body["prefs"]["matchWindow"], 10);
    assert_eq!(body["prefs"]["columns"]["date"], true);
    assert_eq!(body["prefs"]["lastProjectionId"], Value::Null);
    tree.open("b");
    assert_eq!(tree.get().await.0, StatusCode::OK);
    assert_eq!(
        settings_files(&tree),
        Vec::<String>::new(),
        "a read writes nothing"
    );
}

#[tokio::test]
async fn reading_existing_settings_files_leaves_them_byte_for_byte() {
    let tree = Tree::new();
    let ledger = "# hand-made\n[ui]\nmatchWindow = 14 # card\n";
    let global = "# mine\nmatchLookback = 90\n";
    tree.write("a/ledgeline.toml", ledger);
    tree.write(GLOBAL, global);
    let (_, body) = tree.get().await;
    assert_eq!(body["localKeys"], json!(["matchWindow"]));
    assert_eq!(tree.read("a/ledgeline.toml").as_deref(), Some(ledger));
    assert_eq!(tree.read(GLOBAL).as_deref(), Some(global));
    assert_eq!(settings_files(&tree), ["a/ledgeline.toml", GLOBAL]);
}

#[tokio::test]
async fn a_save_goes_to_the_global_file_and_never_creates_ledgeline_toml() {
    let tree = Tree::new();
    let (status, body) = tree
        .patch(json!({"matchWindow": 20, "columns": {"status": false}}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["matchWindow"], 20);
    assert_eq!(body["prefs"]["columns"]["status"], false);
    assert_eq!(body["prefs"]["columns"]["date"], true);
    assert_eq!(body["localKeys"], json!([]));
    assert_eq!(
        settings_files(&tree),
        [GLOBAL],
        "no ledgeline.toml anywhere"
    );

    let text = tree.read(GLOBAL).expect("created in the config dir");
    assert!(
        text.starts_with("# Ledgeline's view settings for every ledger."),
        "{text}"
    );
    let document: toml_edit::DocumentMut = text.parse().expect("valid TOML");
    let keys: Vec<&str> = document.iter().map(|(key, _)| key).collect();
    assert_eq!(
        keys,
        ["matchWindow", "columns"],
        "only what changed: {text}"
    );
    assert_eq!(
        document["columns"].as_table().map(toml_edit::Table::len),
        Some(1)
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(tree.path(GLOBAL))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "created owner-only: {mode:o}");
    }

    // A second save edits the same file, from either ledger.
    tree.open("b");
    let (status, _) = tree.patch(json!({"lastProjectionId": null})).await;
    assert_eq!(status, StatusCode::OK);
    let text = tree.read(GLOBAL).unwrap();
    assert!(text.contains("lastProjectionId = \"\""), "{text}");
    assert!(text.contains("matchWindow = 20"), "{text}");
    assert_eq!(settings_files(&tree), [GLOBAL]);
}

#[tokio::test]
async fn the_global_file_is_created_with_its_config_dir() {
    let tree = Tree::new();
    std::fs::remove_dir_all(tree.path("app/config")).unwrap();
    let (status, body) = tree.patch(json!({"reviewMarkers": false})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(tree.read(GLOBAL).unwrap().contains("reviewMarkers = false"));
}

#[tokio::test]
async fn a_hand_written_comment_in_the_global_file_survives_a_save() {
    let tree = Tree::new();
    tree.write(
        GLOBAL,
        "# My defaults.\nmatchWindow = 14 # card settles slowly\naKeyFromTheFuture = \"kept\"\n\n[columns]\n# nobody uses ! and *\nstatus = false\n",
    );
    let (status, body) = tree
        .patch(json!({"matchWindow": 21, "insightsOpen": false, "columns": {"amount": false}}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read(GLOBAL).unwrap(),
        "# My defaults.\nmatchWindow = 21 # card settles slowly\naKeyFromTheFuture = \"kept\"\ninsightsOpen = false\n\n[columns]\n# nobody uses ! and *\nstatus = false\namount = false\n"
    );
}

#[tokio::test]
async fn a_key_the_ledger_file_sets_is_saved_there_and_everything_else_goes_global() {
    let tree = Tree::new();
    let original = "\
# Business books: keep the journal narrow.
[ui]
# Find Matches looks two weeks apart here.
matchWindow = 14 # card settles slowly
aKeyFromTheFuture = \"kept\"

[theme]
accent = \"teal\"
";
    tree.write("a/ledgeline.toml", original);
    let (_, body) = tree.get().await;
    assert_eq!(body["localKeys"], json!(["matchWindow"]));

    let (status, body) = tree
        .patch(json!({"matchWindow": 21, "insightsOpen": false}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["matchWindow"], 21);
    assert_eq!(body["prefs"]["insightsOpen"], false);
    assert_eq!(
        tree.read("a/ledgeline.toml").unwrap(),
        original.replace("matchWindow = 14", "matchWindow = 21"),
        "only the key it already set; comments and unknown keys kept"
    );
    let global = tree.read(GLOBAL).expect("the rest went global");
    assert!(global.contains("insightsOpen = false"), "{global}");
    assert!(!global.contains("matchWindow"), "{global}");
}

#[tokio::test]
async fn a_ledger_file_without_the_key_leaves_it_to_the_global_file() {
    // Patrick's example: a ledgeline.toml exists, but a key it does not set
    // (his was a future `theme`) is saved globally until someone adds it here.
    let tree = Tree::new();
    let ledger = "[ui]\nmatchWindow = 14\n";
    tree.write("a/ledgeline.toml", ledger);
    let (status, body) = tree.patch(json!({"insightsTab": "balances"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(tree.read("a/ledgeline.toml").as_deref(), Some(ledger));
    assert!(
        tree.read(GLOBAL)
            .unwrap()
            .contains("insightsTab = \"balances\"")
    );

    // Once the key is put in the ledger file by hand, its saves land there.
    tree.write(
        "a/ledgeline.toml",
        "[ui]\nmatchWindow = 14\ninsightsTab = \"activity\"\n",
    );
    let (status, body) = tree.patch(json!({"insightsTab": "balances"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("a/ledgeline.toml").as_deref(),
        Some("[ui]\nmatchWindow = 14\ninsightsTab = \"balances\"\n")
    );
}

#[tokio::test]
async fn a_column_routes_per_leaf() {
    let tree = Tree::new();
    tree.write("a/ledgeline.toml", "[ui.columns]\nstatus = false\n");
    let (_, body) = tree.get().await;
    assert_eq!(body["localKeys"], json!(["columns.status"]));
    let (status, body) = tree
        .patch(json!({"columns": {"status": true, "date": false}}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("a/ledgeline.toml").as_deref(),
        Some("[ui.columns]\nstatus = true\n")
    );
    let global = tree.read(GLOBAL).unwrap();
    assert!(global.contains("[columns]\ndate = false\n"), "{global}");
    assert!(!global.contains("status"), "{global}");
    assert_eq!(body["prefs"]["columns"]["status"], true);
    assert_eq!(body["prefs"]["columns"]["date"], false);
}

#[tokio::test]
async fn the_ledger_beats_the_global_file_which_beats_the_defaults() {
    let tree = Tree::new();
    tree.write(
        GLOBAL,
        "# my defaults\nmatchWindow = 30\nmatchLookback = 90\n[columns]\namount = false\n",
    );
    tree.write("a/ledgeline.toml", "[ui]\nmatchWindow = 20\n");

    let (_, body) = tree.get().await;
    assert_eq!(body["prefs"]["matchWindow"], 20, "ledger");
    assert_eq!(body["prefs"]["matchLookback"], 90, "global");
    assert_eq!(body["prefs"]["matchTransitDays"], 10, "default");
    assert_eq!(
        body["prefs"]["columns"]["amount"], false,
        "global, per member"
    );
}

#[tokio::test]
async fn a_ledgers_own_value_survives_a_global_save_made_in_another_ledger() {
    let tree = Tree::new();
    tree.write("a/ledgeline.toml", "[ui]\nmatchWindow = 20\n");
    tree.open("b");
    let (_, body) = tree.get().await;
    assert_eq!(body["prefs"]["matchWindow"], 10, "b has no override");
    assert_eq!(
        tree.patch(json!({"matchWindow": 5})).await.0,
        StatusCode::OK
    );
    assert_eq!(tree.get().await.1["prefs"]["matchWindow"], 5);

    tree.open("a");
    assert_eq!(tree.get().await.1["prefs"]["matchWindow"], 20);
    assert_eq!(
        tree.read("a/ledgeline.toml").as_deref(),
        Some("[ui]\nmatchWindow = 20\n")
    );
    assert!(tree.read("b/ledgeline.toml").is_none());
}

#[tokio::test]
async fn a_hostile_ledger_file_is_validated_and_never_sets_the_hledger_path() {
    let tree = Tree::new();
    tree.write(
        "a/ledgeline.toml",
        "hledgerPath = \"/bin/sh\"\n[ui]\nhledgerPath = \"/bin/sh\"\nserverToken = \"t\"\n\
         reviewRecentLimit = 99999999\nmatchWindow = \"x\"\nbenchmarks = [\"SPY\", \"$(rm)\"]\n",
    );
    tree.write(GLOBAL, "matchWindow = 33\n");
    let (status, body) = tree.get().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["reviewRecentLimit"], 1000);
    assert_eq!(
        body["prefs"]["matchWindow"], 33,
        "falls through to the global file"
    );
    assert_eq!(body["prefs"]["benchmarks"], json!(["SPY"]));
    assert!(body["prefs"].get("hledgerPath").is_none());
    let keys = issue_keys(&body);
    for expected in [
        "ledgeline.toml:hledgerPath",
        "ledgeline.toml:ui.hledgerPath",
        "ledgeline.toml:ui.serverToken",
        "ledgeline.toml:ui.reviewRecentLimit",
        "ledgeline.toml:ui.matchWindow",
        "ledgeline.toml:ui.benchmarks",
    ] {
        assert!(
            keys.contains(&expected.to_string()),
            "{expected} not in {keys:?}"
        );
    }

    // The settings form's store is untouched by any of it.
    let request = Request::builder()
        .uri("/api/prefs")
        .body(Body::empty())
        .unwrap();
    let response = router_with_state(tree.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"hledgerPath": null})
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_ledger_file_is_never_read_or_written_through() {
    let tree = Tree::new();
    let target = tree.path("elsewhere.toml");
    let planted = "[ui]\nmatchWindow = 55\n";
    std::fs::write(&target, planted).unwrap();
    std::os::unix::fs::symlink(&target, tree.path("a/ledgeline.toml")).unwrap();

    let (status, body) = tree.get().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["matchWindow"], 10, "the link is not followed");
    assert_eq!(body["localKeys"], json!([]));
    assert_eq!(issue_keys(&body), ["ledgeline.toml:*"]);
    assert!(
        body["issues"][0]["message"]
            .as_str()
            .is_some_and(|m| m.contains("symbolic link")),
        "{body}"
    );

    let (status, body) = tree.patch(json!({"matchWindow": 20})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_no_absolute_path(&tree, &body);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), planted);
    assert!(
        tree.path("a/ledgeline.toml")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link is left as it was"
    );
    assert!(tree.read(GLOBAL).unwrap().contains("matchWindow = 20"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_dangling_ledger_symlink_is_not_created_through() {
    let tree = Tree::new();
    let target = tree.path("would-be-created.toml");
    std::os::unix::fs::symlink(&target, tree.path("a/ledgeline.toml")).unwrap();
    let (status, body) = tree.patch(json!({"matchWindow": 20})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!target.exists(), "nothing was created at the link's target");
}

#[tokio::test]
async fn an_oversized_or_unparseable_ledger_file_is_ignored_and_never_overwritten() {
    let big = format!("[ui]\nmatchWindow = 20\n{}", "# padding\n".repeat(8000));
    for (text, phrase) in [
        (big.as_str(), "larger than"),
        ("[ui\nmatchWindow = ", "not valid TOML"),
        ("ui = 5\n", ""),
    ] {
        let tree = Tree::new();
        tree.write("a/ledgeline.toml", text);
        let (status, body) = tree.get().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["prefs"]["matchWindow"], 10);
        assert_eq!(body["localKeys"], json!([]));
        assert!(
            body["issues"][0]["message"]
                .as_str()
                .is_some_and(|m| m.contains(phrase)),
            "{body}"
        );
        let (status, body) = tree.patch(json!({"matchWindow": 20})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(tree.read("a/ledgeline.toml").as_deref(), Some(text));
        assert!(tree.read(GLOBAL).unwrap().contains("matchWindow = 20"));
    }
}

#[tokio::test]
async fn an_unparseable_global_file_is_never_overwritten() {
    for text in ["matchWindow = \n", "columns = 3\n"] {
        let tree = Tree::new();
        tree.write(GLOBAL, text);
        let (status, body) = tree
            .patch(json!({"matchWindow": 20, "columns": {"date": false}}))
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{text} → {body}");
        assert_no_absolute_path(&tree, &body);
        assert_eq!(tree.read(GLOBAL).as_deref(), Some(text));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_global_file_is_read_but_never_written_and_saves_stay_in_memory() {
    // home-manager links ui-prefs.toml into the read-only Nix store.
    let tree = Tree::new();
    let target = tree.path("managed/ui-prefs.toml");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let managed = "matchLookback = 90\n";
    std::fs::write(&target, managed).unwrap();
    std::os::unix::fs::symlink(&target, tree.path(GLOBAL)).unwrap();

    let (_, body) = tree.get().await;
    assert_eq!(body["prefs"]["matchLookback"], 90, "read through the link");
    assert_eq!(body["issues"], json!([]), "no notice until a save is kept");

    let (status, body) = tree.patch(json!({"matchWindow": 7})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["matchWindow"], 7, "kept for the session");
    assert_eq!(issue_keys(&body), ["ui-prefs.toml:*"]);
    assert!(
        body["issues"][0]["message"]
            .as_str()
            .is_some_and(|m| m.contains("managed outside Ledgeline")),
        "{body}"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), managed);
    assert!(
        tree.path(GLOBAL)
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link is left as it was"
    );

    // The session copy sits over the managed file, across ledgers.
    tree.open("b");
    let (_, body) = tree.get().await;
    assert_eq!(body["prefs"]["matchWindow"], 7);
    assert_eq!(body["prefs"]["matchLookback"], 90);
    assert_eq!(settings_files(&tree), [GLOBAL, "managed/ui-prefs.toml"]);
}

#[cfg(unix)]
#[tokio::test]
async fn a_dangling_global_symlink_is_not_created_through() {
    let tree = Tree::new();
    let target = tree.path("nix-store-gone/ui-prefs.toml");
    std::os::unix::fs::symlink(&target, tree.path(GLOBAL)).unwrap();
    let (status, body) = tree.patch(json!({"matchWindow": 7})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["matchWindow"], 7);
    assert!(!target.exists(), "nothing was created at the link's target");
}

#[tokio::test]
async fn a_bad_patch_is_a_400_and_writes_nothing() {
    let tree = Tree::new();
    for body in [
        json!({"matchWindow": 0}),
        json!({"hledgerPath": "/bin/sh"}),
        json!({"columns": {"date": "yes"}}),
        json!("not an object"),
    ] {
        let (status, answer) = tree.patch(body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} → {answer}");
    }
    assert_eq!(settings_files(&tree), Vec::<String>::new());
}

#[tokio::test]
async fn an_empty_patch_creates_no_file() {
    let tree = Tree::new();
    let (status, body) = tree.patch(json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(settings_files(&tree), Vec::<String>::new());
}

#[tokio::test]
async fn with_no_journal_file_saves_go_to_the_global_file() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("app/config")).unwrap();
    std::fs::write(
        dir.path().join("app/config/ui-prefs.toml"),
        "matchLookback = 90\n",
    )
    .unwrap();
    let journal = ledgeline_core::parse::parse_journal(JOURNAL, "in-memory").unwrap();
    let state =
        AppState::from_journal(&journal).with_app_dirs(AppDirs::under(dir.path().join("app")));

    let (_, body) = send(&state, "GET", None).await;
    assert_eq!(body["storage"], "files");
    assert_eq!(body["localKeys"], json!([]));
    assert_eq!(body["prefs"]["matchLookback"], 90, "global still applies");

    let (status, body) = send(&state, "PATCH", Some(json!({"matchWindow": 7}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(send(&state, "GET", None).await.1["prefs"]["matchWindow"], 7);
    assert_eq!(files_under(dir.path()), ["app/config/ui-prefs.toml"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app/config/ui-prefs.toml")).unwrap(),
        "matchLookback = 90\nmatchWindow = 7\n"
    );
}

#[tokio::test]
async fn browser_mode_never_creates_or_modifies_either_settings_file() {
    // The e2e and live-contract harnesses ($LEDGELINE_UI_PREFS_IN_BROWSER).
    let tree = Tree::new();
    let state = tree.state.clone().with_ui_prefs_in_browser();

    // Neither file exists: GET and PATCH create none.
    let (_, body) = send(&state, "GET", None).await;
    assert_eq!(body["storage"], "browser");
    let (status, body) = send(&state, "PATCH", Some(json!({"matchWindow": 7}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["storage"], "browser");
    assert_eq!(settings_files(&tree), Vec::<String>::new());

    // Both exist: neither is read, and neither changes.
    tree.write("a/ledgeline.toml", "[ui]\nmatchWindow = 20\n");
    tree.write(GLOBAL, "matchLookback = 90\n");
    let (_, body) = send(&state, "GET", None).await;
    assert_eq!(body["storage"], "browser");
    assert_eq!(body["localKeys"], json!([]));
    assert_eq!(body["prefs"]["matchWindow"], 10, "no ledger file");
    assert_eq!(body["prefs"]["matchLookback"], 365, "no global file");

    let (status, body) = send(
        &state,
        "PATCH",
        Some(json!({"matchWindow": 7, "matchLookback": 30})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["storage"], "browser");
    assert_eq!(
        body["prefs"]["matchWindow"], 7,
        "the patch is checked and echoed"
    );
    assert_eq!(
        send(&state, "GET", None).await.1["prefs"]["matchWindow"],
        10,
        "and not kept"
    );
    assert_eq!(
        tree.read("a/ledgeline.toml").as_deref(),
        Some("[ui]\nmatchWindow = 20\n")
    );
    assert_eq!(tree.read(GLOBAL).as_deref(), Some("matchLookback = 90\n"));
    assert_eq!(settings_files(&tree), ["a/ledgeline.toml", GLOBAL]);
}

// ---------------------------------------------------------------------------
// `POST /api/ui-prefs/pin` (plan 39): keep `theme`/`uiScale` for one ledger.
// ---------------------------------------------------------------------------

const LEDGER: &str = "a/ledgeline.toml";

#[tokio::test]
async fn pinning_creates_ledgeline_toml_with_the_current_value() {
    let tree = Tree::new();
    tree.write(GLOBAL, "theme = \"nord\"\n");
    let (status, body) = tree.pin("theme", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["storage"], "files");
    assert_eq!(body["localKeys"], json!(["theme"]));
    assert_eq!(body["prefs"]["theme"], "nord");
    assert_no_absolute_path(&tree, &body);

    let text = tree.read(LEDGER).expect("created beside the journal");
    assert!(
        text.starts_with("# Ledgeline view settings for this ledger only."),
        "{text}"
    );
    assert!(text.ends_with("[ui]\ntheme = \"nord\"\n"), "{text}");
    assert_eq!(tree.read(GLOBAL).as_deref(), Some("theme = \"nord\"\n"));
    assert_eq!(settings_files(&tree), [LEDGER, GLOBAL]);

    // The default value pins too, beside the first key.
    let (status, body) = tree.pin("uiScale", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["localKeys"], json!(["theme", "uiScale"]));
    assert!(
        tree.read(LEDGER)
            .unwrap()
            .ends_with("[ui]\ntheme = \"nord\"\nuiScale = 100\n")
    );
}

#[tokio::test]
async fn after_a_pin_saves_of_that_key_go_to_the_ledger_and_other_ledgers_keep_the_global_value() {
    let tree = Tree::new();
    tree.write(GLOBAL, "theme = \"nord\"\n");
    assert_eq!(tree.pin("theme", true).await.0, StatusCode::OK);

    let (status, body) = tree
        .patch(json!({"theme": "dracula", "uiScale": 125}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["theme"], "dracula");
    assert!(
        tree.read(LEDGER)
            .unwrap()
            .ends_with("[ui]\ntheme = \"dracula\"\n")
    );
    assert_eq!(
        tree.read(GLOBAL).as_deref(),
        Some("theme = \"nord\"\nuiScale = 125\n"),
        "the unpinned key still goes global"
    );

    tree.open("b");
    let (_, body) = tree.get().await;
    assert_eq!(body["prefs"]["theme"], "nord");
    assert_eq!(body["prefs"]["uiScale"], 125);
    assert_eq!(body["localKeys"], json!([]));
    assert!(tree.read("b/ledgeline.toml").is_none());
}

#[tokio::test]
async fn pinning_into_an_existing_file_keeps_its_comments_and_keys() {
    let tree = Tree::new();
    let original = "\
# Business books.
title = \"kept\"

[ui]
# two weeks apart here
matchWindow = 14 # card settles slowly

[other]
x = 1
";
    tree.write(LEDGER, original);
    tree.write(GLOBAL, "uiScale = 150\n");
    let (status, body) = tree.pin("uiScale", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["localKeys"], json!(["matchWindow", "uiScale"]));
    assert_eq!(
        tree.read(LEDGER).unwrap(),
        original.replace(
            "# card settles slowly\n",
            "# card settles slowly\nuiScale = 150\n"
        )
    );

    // A file with no [ui] table gets one; one with only [ui.columns] gets
    // its implicit [ui] made explicit.
    for (text, written, local) in [
        (
            "# just a note\n",
            "# just a note\n[ui]\ntheme = \"default\"\n",
            json!(["theme"]),
        ),
        (
            "title = \"x\"\n\n[other]\ny = 1\n# the end\n",
            "title = \"x\"\n\n[other]\ny = 1\n# the end\n[ui]\ntheme = \"default\"\n",
            json!(["theme"]),
        ),
        (
            "[ui.columns]\nstatus = false\n",
            "[ui]\ntheme = \"default\"\n[ui.columns]\nstatus = false\n",
            json!(["columns.status", "theme"]),
        ),
        (
            "ui = { matchWindow = 5 }\n",
            "ui = { matchWindow = 5 , theme = \"default\" }\n",
            json!(["matchWindow", "theme"]),
        ),
    ] {
        let tree = Tree::new();
        tree.write(LEDGER, text);
        let (status, body) = tree.pin("theme", true).await;
        assert_eq!(status, StatusCode::OK, "{text} → {body}");
        assert_eq!(body["localKeys"], local, "{text}");
        assert_eq!(body["issues"], json!([]), "{text}");
        assert_eq!(tree.read(LEDGER).as_deref(), Some(written), "{text}");
    }
}

#[tokio::test]
async fn pinning_a_key_already_set_writes_nothing_but_a_bad_value_is_replaced() {
    let tree = Tree::new();
    let pinned = "[ui]\ntheme = \"gruvbox\" # mine\n";
    tree.write(LEDGER, pinned);
    tree.write(GLOBAL, "theme = \"nord\"\n");
    let (status, body) = tree.pin("theme", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["theme"], "gruvbox");
    assert_eq!(tree.read(LEDGER).as_deref(), Some(pinned));

    tree.write(LEDGER, "[ui]\ntheme = \"no-such-theme\" # mine\n");
    let (status, body) = tree.pin("theme", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["issues"], json!([]));
    assert_eq!(
        tree.read(LEDGER).as_deref(),
        Some("[ui]\ntheme = \"nord\" # mine\n"),
        "the effective value, in place of the bad one"
    );
}

#[tokio::test]
async fn unpinning_removes_the_key_and_keeps_the_file() {
    let tree = Tree::new();
    tree.write(GLOBAL, "theme = \"nord\"\n");
    tree.write(
        LEDGER,
        "# mine\n[ui]\nmatchWindow = 14\n# blue for business\ntheme = \"github\"\n",
    );
    let (status, body) = tree.pin("theme", false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["localKeys"], json!(["matchWindow"]));
    assert_eq!(body["prefs"]["theme"], "nord", "back to the global value");
    assert_eq!(
        tree.read(LEDGER).as_deref(),
        Some("# mine\n[ui]\nmatchWindow = 14\n")
    );

    // Even when that leaves nothing in it.
    tree.write(LEDGER, "[ui]\nuiScale = 90\n");
    let (status, body) = tree.pin("uiScale", false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["localKeys"], json!([]));
    assert_eq!(tree.read(LEDGER).as_deref(), Some("[ui]\n"));
}

#[tokio::test]
async fn unpinning_a_key_that_is_not_pinned_writes_nothing() {
    let tree = Tree::new();
    let (status, body) = tree.pin("theme", false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        settings_files(&tree),
        Vec::<String>::new(),
        "no file is created"
    );

    for text in [
        "[ui]\nmatchWindow = 14\n",
        "theme = \"nord\"\n",
        "[ui.theme]\nx = 1\n",
    ] {
        tree.write(LEDGER, text);
        let (status, body) = tree.pin("theme", false).await;
        assert_eq!(status, StatusCode::OK, "{text} → {body}");
        assert_eq!(tree.read(LEDGER).as_deref(), Some(text));
    }
}

#[tokio::test]
async fn only_theme_and_scale_can_be_pinned_and_a_bad_pin_writes_nothing() {
    let tree = Tree::new();
    for body in [
        json!({"key": "matchWindow", "pinned": true}),
        json!({"key": "hledgerPath", "pinned": true}),
        json!({"key": "columns", "pinned": true}),
        json!({"key": "theme"}),
        json!({"key": "theme", "pinned": true, "value": "nord"}),
        json!("theme"),
    ] {
        let (status, answer) = send_pin(&tree.state, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} → {answer}");
    }
    assert_eq!(settings_files(&tree), Vec::<String>::new());
}

#[tokio::test]
async fn a_ledger_file_that_cannot_be_safely_written_refuses_a_pin() {
    let big = format!("[ui]\nmatchWindow = 20\n{}", "# padding\n".repeat(8000));
    for text in [
        big.as_str(),
        "[ui\nmatchWindow = ",
        "ui = 5\n",
        "[ui.theme]\nx = 1\n",
    ] {
        let tree = Tree::new();
        tree.write(LEDGER, text);
        let (status, body) = tree.pin("theme", true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_no_absolute_path(&tree, &body);
        assert_eq!(tree.read(LEDGER).as_deref(), Some(text));
        assert!(tree.read(GLOBAL).is_none());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_ledger_file_refuses_a_pin_and_is_never_written_through() {
    let tree = Tree::new();
    let target = tree.path("elsewhere.toml");
    let planted = "[ui]\nmatchWindow = 55\n";
    std::fs::write(&target, planted).unwrap();
    std::os::unix::fs::symlink(&target, tree.path(LEDGER)).unwrap();
    let (status, body) = tree.pin("theme", true).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_no_absolute_path(&tree, &body);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), planted);
    assert!(
        tree.path(LEDGER)
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );

    // A dangling one is not created through either.
    let tree = Tree::new();
    let target = tree.path("would-be-created.toml");
    std::os::unix::fs::symlink(&target, tree.path(LEDGER)).unwrap();
    let (status, body) = tree.pin("theme", true).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(!target.exists(), "nothing was created at the link's target");
}

#[tokio::test]
async fn pinning_needs_an_open_ledger_file_and_files_storage() {
    let dir = TempDir::new().unwrap();
    let journal = ledgeline_core::parse::parse_journal(JOURNAL, "in-memory").unwrap();
    let state =
        AppState::from_journal(&journal).with_app_dirs(AppDirs::under(dir.path().join("app")));
    let (status, body) = send_pin(&state, json!({"key": "theme", "pinned": true})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(files_under(dir.path()), Vec::<String>::new());

    let tree = Tree::new();
    let state = tree.state.clone().with_ui_prefs_in_browser();
    for pinned in [true, false] {
        let (status, body) = send_pin(&state, json!({"key": "theme", "pinned": pinned})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(
            body.as_str().is_some_and(|m| m.contains("browser")),
            "{body}"
        );
    }
    assert_eq!(settings_files(&tree), Vec::<String>::new());
}

#[tokio::test]
async fn a_hostile_theme_or_scale_in_ledgeline_toml_falls_back_with_an_issue() {
    let tree = Tree::new();
    tree.write(LEDGER, "[ui]\ntheme = \"$(rm -rf)\"\nuiScale = 5000\n");
    tree.write(GLOBAL, "theme = \"dracula\"\n");
    let (status, body) = tree.get().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["theme"], "dracula");
    assert_eq!(body["prefs"]["uiScale"], 300, "clamped");
    assert_eq!(
        issue_keys(&body),
        ["ledgeline.toml:ui.theme", "ledgeline.toml:ui.uiScale"]
    );
    assert!(!body.to_string().contains("rm -rf"), "{body}");
}

/// Both UI-preference routes write files, so both sit behind the token guard;
/// the pin route is the one that may create `ledgeline.toml`.
#[tokio::test]
async fn every_ui_prefs_route_requires_the_token() {
    const PORT: u16 = 5097;
    let tree = Tree::new();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");
    for (method, uri, body) in [
        ("GET", "/api/ui-prefs", ""),
        ("PATCH", "/api/ui-prefs", "{}"),
        (
            "POST",
            "/api/ui-prefs/pin",
            r#"{"key":"theme","pinned":true}"#,
        ),
    ] {
        for (auth, expected) in [
            (None, Some(StatusCode::UNAUTHORIZED)),
            (Some("Bearer wrong"), Some(StatusCode::UNAUTHORIZED)),
            (Some("Bearer integration-test-token"), None),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header(header::HOST, format!("127.0.0.1:{PORT}"))
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(auth) = auth {
                request = request.header(header::AUTHORIZATION, auth);
            }
            let status =
                router_with_security(tree.state.clone(), Security::local(token.clone(), PORT))
                    .oneshot(request.body(Body::from(body)).unwrap())
                    .await
                    .expect("router responds")
                    .status();
            match expected {
                Some(expected) => assert_eq!(status, expected, "{method} {uri} {auth:?}"),
                None => assert_eq!(status, StatusCode::OK, "{method} {uri} with the token"),
            }
        }
    }
    assert!(tree.read(LEDGER).is_some(), "the authorised pin wrote");
}

/// Every regular file under `root`, relative, sorted.
fn files_under(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}
