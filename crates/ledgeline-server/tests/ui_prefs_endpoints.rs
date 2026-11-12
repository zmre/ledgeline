//! `GET`/`PATCH /api/ui-prefs` (plan 38, WP-5): the SPA's view state in
//! `ledgeline.toml` beside the journal, over `ui-prefs.toml` in the config
//! directory, over the built-in defaults.
//!
//! Every state here is given `AppDirs::under` a scratch directory, so no test
//! reads the developer's real config directory. Every journal is a scratch
//! copy, so no test writes beside a committed fixture.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{AppDirs, AppState, router_with_state};
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

    fn open(&self, ledger: &str) {
        self.state
            .rebind_editor(self.path(&format!("{ledger}/main.journal")))
            .expect("the other ledger opens");
    }
}

async fn send(state: &AppState, method: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri("/api/ui-prefs")
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

#[tokio::test]
async fn a_ledger_with_no_file_answers_the_defaults_and_writes_nothing() {
    let tree = Tree::new();
    let (status, body) = tree.get().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["storage"], "ledger");
    assert_eq!(body["ledgerHasUi"], false);
    assert_eq!(body["issues"], json!([]));
    assert_eq!(body["prefs"]["matchWindow"], 10);
    assert_eq!(body["prefs"]["columns"]["date"], true);
    assert_eq!(body["prefs"]["lastProjectionId"], Value::Null);
    assert!(
        tree.read("a/ledgeline.toml").is_none(),
        "a read creates no file"
    );
}

#[tokio::test]
async fn a_patch_writes_only_the_changed_keys_beside_the_journal() {
    let tree = Tree::new();
    let (status, body) = tree
        .patch(json!({"matchWindow": 20, "columns": {"status": false}}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["prefs"]["matchWindow"], 20);
    assert_eq!(body["prefs"]["columns"]["status"], false);
    assert_eq!(body["prefs"]["columns"]["date"], true);
    assert_eq!(body["ledgerHasUi"], true);

    let text = tree
        .read("a/ledgeline.toml")
        .expect("written beside the journal");
    let document: toml_edit::DocumentMut = text.parse().expect("valid TOML");
    let ui = document["ui"].as_table().expect("a [ui] table");
    let keys: Vec<&str> = ui.iter().map(|(key, _)| key).collect();
    assert_eq!(
        keys,
        ["matchWindow", "columns"],
        "only what changed: {text}"
    );
    assert_eq!(ui["columns"].as_table().map(toml_edit::Table::len), Some(1));
    assert!(
        text.starts_with("# Ledgeline's settings for this ledger."),
        "{text}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(tree.path("a/ledgeline.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "created owner-only: {mode:o}");
    }

    // A second save edits the same file.
    let (status, _) = tree.patch(json!({"lastProjectionId": null})).await;
    assert_eq!(status, StatusCode::OK);
    let text = tree.read("a/ledgeline.toml").unwrap();
    assert!(text.contains("lastProjectionId = \"\""), "{text}");
    assert!(text.contains("matchWindow = 20"), "{text}");
}

#[tokio::test]
async fn a_hand_written_comment_survives_an_automatic_save() {
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
    let (status, body) = tree
        .patch(json!({"matchWindow": 21, "insightsOpen": false}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("a/ledgeline.toml").unwrap(),
        "\
# Business books: keep the journal narrow.
[ui]
# Find Matches looks two weeks apart here.
matchWindow = 21 # card settles slowly
aKeyFromTheFuture = \"kept\"
insightsOpen = false

[theme]
accent = \"teal\"
"
    );
}

#[tokio::test]
async fn the_ledger_beats_the_global_file_which_beats_the_defaults_and_global_is_never_written() {
    let tree = Tree::new();
    let global = "# my defaults\nmatchWindow = 30\nmatchLookback = 90\n[columns]\namount = false\n";
    tree.write("app/config/ui-prefs.toml", global);
    tree.write("a/ledgeline.toml", "[ui]\nmatchWindow = 20\n");

    let (_, body) = tree.get().await;
    assert_eq!(body["prefs"]["matchWindow"], 20, "ledger");
    assert_eq!(body["prefs"]["matchLookback"], 90, "global");
    assert_eq!(body["prefs"]["matchTransitDays"], 10, "default");
    assert_eq!(
        body["prefs"]["columns"]["amount"], false,
        "global, per member"
    );

    let (status, _) = tree.patch(json!({"matchLookback": 30})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        tree.read("app/config/ui-prefs.toml").as_deref(),
        Some(global),
        "the app never writes ui-prefs.toml"
    );
    assert!(
        tree.read("a/ledgeline.toml")
            .unwrap()
            .contains("matchLookback = 30")
    );
}

#[tokio::test]
async fn two_ledgers_keep_different_settings_and_opening_one_reloads_them() {
    let tree = Tree::new();
    assert_eq!(
        tree.patch(json!({"matchWindow": 20})).await.0,
        StatusCode::OK
    );

    tree.open("b");
    let (_, body) = tree.get().await;
    assert_eq!(
        body["prefs"]["matchWindow"], 10,
        "b starts from the defaults"
    );
    assert_eq!(body["ledgerHasUi"], false);
    assert_eq!(
        tree.patch(json!({"matchWindow": 5})).await.0,
        StatusCode::OK
    );

    tree.open("a");
    assert_eq!(tree.get().await.1["prefs"]["matchWindow"], 20);
    tree.open("b");
    assert_eq!(tree.get().await.1["prefs"]["matchWindow"], 5);
}

#[tokio::test]
async fn a_hostile_ledger_file_is_validated_and_never_sets_the_hledger_path() {
    let tree = Tree::new();
    tree.write(
        "a/ledgeline.toml",
        "hledgerPath = \"/bin/sh\"\n[ui]\nhledgerPath = \"/bin/sh\"\nserverToken = \"t\"\n\
         reviewRecentLimit = 99999999\nmatchWindow = \"x\"\nbenchmarks = [\"SPY\", \"$(rm)\"]\n",
    );
    tree.write("app/config/ui-prefs.toml", "matchWindow = 33\n");
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
    assert_eq!(issue_keys(&body), ["ledgeline.toml:*"]);
    assert!(
        body["issues"][0]["message"]
            .as_str()
            .is_some_and(|m| m.contains("symbolic link")),
        "{body}"
    );

    let (status, body) = tree.patch(json!({"matchWindow": 20})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
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
}

#[cfg(unix)]
#[tokio::test]
async fn a_dangling_symlink_is_not_created_through() {
    let tree = Tree::new();
    let target = tree.path("would-be-created.toml");
    std::os::unix::fs::symlink(&target, tree.path("a/ledgeline.toml")).unwrap();
    let (status, body) = tree.patch(json!({"matchWindow": 20})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
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
        assert_eq!(
            body["ledgerHasUi"], false,
            "so the SPA's migration is attempted"
        );
        assert!(
            body["issues"][0]["message"]
                .as_str()
                .is_some_and(|m| m.contains(phrase)),
            "{body}"
        );
        let (status, body) = tree.patch(json!({"matchWindow": 20})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_no_absolute_path(&tree, &body);
        assert_eq!(tree.read("a/ledgeline.toml").as_deref(), Some(text));
    }
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
    assert!(tree.read("a/ledgeline.toml").is_none());
}

#[tokio::test]
async fn an_empty_patch_creates_no_file() {
    let tree = Tree::new();
    let (status, body) = tree.patch(json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(tree.read("a/ledgeline.toml").is_none());
}

#[tokio::test]
async fn with_no_journal_file_saves_stay_in_memory() {
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
    assert_eq!(body["storage"], "memory");
    assert_eq!(body["ledgerHasUi"], false);
    assert_eq!(body["prefs"]["matchLookback"], 90, "global still applies");

    let (status, body) = send(&state, "PATCH", Some(json!({"matchWindow": 7}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, body) = send(&state, "GET", None).await;
    assert_eq!(body["prefs"]["matchWindow"], 7);
    assert_eq!(body["ledgerHasUi"], true);
    assert_eq!(files_under(dir.path()), ["app/config/ui-prefs.toml"]);
}

#[tokio::test]
async fn browser_mode_reads_no_file_and_writes_none() {
    let tree = Tree::new();
    let state = tree.state.clone().with_ui_prefs_in_browser();
    tree.write("a/ledgeline.toml", "[ui]\nmatchWindow = 20\n");
    tree.write("app/config/ui-prefs.toml", "matchLookback = 90\n");

    let (_, body) = send(&state, "GET", None).await;
    assert_eq!(body["storage"], "browser");
    assert_eq!(body["ledgerHasUi"], false);
    assert_eq!(body["prefs"]["matchWindow"], 10, "no ledger file");
    assert_eq!(body["prefs"]["matchLookback"], 365, "no global file");

    let (status, body) = send(&state, "PATCH", Some(json!({"matchWindow": 7}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
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
