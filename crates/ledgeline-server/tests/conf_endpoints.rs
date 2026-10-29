//! `GET`/`PUT /api/hledger-conf` — Settings › hledger config (plan 30,
//! Decision 10).
//!
//! Hermetic: every config file here lives in a temp dir, and the user-level
//! tiers are pointed at a temp "home" through `AppDirs` — never the real
//! `~/.hledger.conf`. The one check that runs hledger itself (does hledger read
//! back the depth we wrote?) is opt-in behind `LEDGELINE_HLEDGER_CONF_CHECK`,
//! like the other cross-checks in `just hledger-checks`.
//!
//! What this pins:
//!
//! 1. **Create in the ledger root** when no file is in force, with the header.
//! 2. **Update where found**: above the journal's directory, or `~/.hledger.conf`.
//! 3. **409 on a stale revision**, nothing written.
//! 4. **A hijacked file is refused**, with the sentence, nothing written.
//! 5. **A write republishes**: the generation changes. So does an external
//!    change noticed by a read.
//! 6. **Handles are never absolute paths.**
//! 7. **The token guard covers both methods.**

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{
    AccessToken, AppDirs, AppState, Security, router_with_security, router_with_state,
};
use ledgeline_core::hledger_conf::UserConfDirs;
use serde_json::{Value, json};
use std::path::PathBuf;
use tempfile::TempDir;
use tower::ServiceExt;

const JOURNAL: &str = "2026-01-01 opening\n\
                       \x20   assets:bank:checking      $100\n\
                       \x20   assets:inv:broker:fund    $50\n\
                       \x20   equity:opening\n";

/// A scratch layout: `<tmp>/books/main.journal`, a fake home at `<tmp>/home`,
/// and the app's own dirs under `<tmp>/app`.
struct Tree {
    dir: TempDir,
    state: AppState,
}

impl Tree {
    fn new() -> Self {
        Self::with_home(false)
    }

    /// `home`: give the state user-level tiers pointing at `<tmp>/home`.
    fn with_home(home: bool) -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("books")).unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        std::fs::write(dir.path().join("books/main.journal"), JOURNAL).unwrap();
        let mut state = AppState::from_journal_path(dir.path().join("books/main.journal"))
            .expect("the scratch journal opens");
        if home {
            state = state.with_app_dirs(AppDirs {
                hledger_conf: UserConfDirs {
                    home: Some(dir.path().join("home")),
                    xdg_config_home: None,
                },
                ..AppDirs::under(dir.path().join("app"))
            });
        }
        Self { dir, state }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn read(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.path(relative)).ok()
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    async fn get(&self) -> (StatusCode, Value) {
        send(
            router_with_state(self.state.clone()),
            Request::builder()
                .method("GET")
                .uri("/api/hledger-conf")
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn put(&self, body: Value) -> (StatusCode, Value) {
        send(
            router_with_state(self.state.clone()),
            Request::builder()
                .method("PUT")
                .uri("/api/hledger-conf")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
    }

    async fn generation(&self) -> String {
        let (_, info) = send(
            router_with_state(self.state.clone()),
            Request::builder()
                .method("GET")
                .uri("/api/journal")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        info["generation"]
            .as_str()
            .expect("a generation")
            .to_string()
    }
}

async fn send(router: axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.oneshot(request).await.expect("router responds");
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

#[tokio::test]
async fn with_no_file_anywhere_a_write_creates_one_beside_the_journal() {
    let tree = Tree::with_home(true);
    let (status, body) = tree.get().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["handle"], Value::Null);
    assert_eq!(body["createHandle"], "hledger.conf");
    assert_eq!(body["editable"], true);
    assert_eq!(body["revision"], "");
    assert_eq!(body["auto"], false);
    assert_eq!(
        body["depth"]["general"],
        json!({"all": null, "perRegex": []})
    );

    let before = tree.generation().await;
    let (status, body) = tree
        .put(json!({"revision": "", "auto": true, "depth": {"balancesheet": {"all": 2}}}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["handle"], "hledger.conf");
    assert_eq!(body["origin"], "ledger");
    assert_eq!(body["auto"], true);
    assert_eq!(body["effectiveDepth"]["balancesheet"]["all"], 2);
    assert_ne!(body["revision"], "");
    assert_no_absolute_path(&tree, &body);

    let written = tree
        .read("books/hledger.conf")
        .expect("created beside the journal");
    assert!(written.starts_with("# hledger's config file"), "{written}");
    assert!(written.contains("\n--auto\n"), "{written}");
    assert!(
        written.ends_with("\n[balancesheet]\n--depth 2\n"),
        "{written}"
    );
    assert_eq!(tree.read("home/.hledger.conf"), None, "home untouched");
    assert_ne!(tree.generation().await, before, "the write republished");
}

#[tokio::test]
async fn a_file_above_the_journal_is_updated_in_place() {
    let tree = Tree::new();
    tree.write(
        "hledger.conf",
        "# shared by every set of books\n--forecast\n",
    );
    let (_, body) = tree.get().await;
    assert_eq!(body["handle"], "../hledger.conf");
    assert_eq!(body["createHandle"], Value::Null);

    let (status, body) = tree
        .put(json!({"revision": body["revision"], "auto": true}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("hledger.conf").unwrap(),
        "# shared by every set of books\n--forecast\n--auto\n"
    );
    assert_eq!(
        tree.read("books/hledger.conf"),
        None,
        "nothing created beside it"
    );
}

#[tokio::test]
async fn the_home_file_is_found_and_updated_and_named_by_tilde() {
    let tree = Tree::with_home(true);
    tree.write("home/.hledger.conf", "--depth 3\n");
    let (status, body) = tree.get().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["handle"], "~/.hledger.conf");
    assert_eq!(body["origin"], "home");
    assert_eq!(body["depth"]["general"]["all"], 3);
    assert_eq!(body["effectiveDepth"]["cashflow"]["all"], 3);
    assert_no_absolute_path(&tree, &body);

    let (status, body) = tree
        .put(json!({
            "revision": body["revision"],
            "depth": {"general": {"all": null}, "cashflow": {"all": 1, "perRegex": [{"regex": "assets:inv", "depth": 3}]}}
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("home/.hledger.conf").unwrap(),
        "\n[cashflow]\n--depth 1\n--depth assets:inv=3\n"
    );
    assert_eq!(
        body["effectiveDepth"]["cashflow"],
        json!({"all": 1, "perRegex": [{"regex": "assets:inv", "depth": 3}]})
    );
}

#[tokio::test]
async fn without_user_dirs_the_home_file_is_never_read() {
    // The default state — what every in-process caller gets — has no home.
    let tree = Tree::new();
    tree.write("home/.hledger.conf", "--auto\n");
    let (_, body) = tree.get().await;
    assert_eq!(body["handle"], Value::Null);
    assert_eq!(body["auto"], false);
}

#[tokio::test]
async fn a_stale_revision_is_a_409_and_writes_nothing() {
    let tree = Tree::new();
    tree.write("books/hledger.conf", "--depth 2\n");
    let (status, body) = tree.put(json!({"revision": "stale", "auto": true})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(tree.read("books/hledger.conf").unwrap(), "--depth 2\n");
    // "" claims there is no file; there is one.
    let (status, _) = tree.put(json!({"revision": "", "auto": true})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(tree.read("books/hledger.conf").unwrap(), "--depth 2\n");
}

#[tokio::test]
async fn a_hijacked_file_is_shown_and_never_written() {
    let tree = Tree::new();
    tree.write("books/hledger.conf", "balance\n--auto\n");
    let (_, body) = tree.get().await;
    assert!(
        body["hijacked"].as_str().unwrap().contains("`balance`"),
        "{body}"
    );
    assert_eq!(body["editable"], false);
    assert_eq!(body["auto"], false, "a hijacked file applies nothing");
    let (status, message) = tree
        .put(json!({"revision": body["revision"], "auto": false}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert!(message.as_str().unwrap().contains("`balance`"), "{message}");
    assert_eq!(
        tree.read("books/hledger.conf").unwrap(),
        "balance\n--auto\n"
    );
}

#[tokio::test]
async fn an_explicit_general_section_is_read_and_written_as_hledger_reads_it() {
    // hledger 1.52 reads the last `[general]` as the general section and
    // discards the `--depth 1` above it.
    let tree = Tree::new();
    tree.write("books/hledger.conf", "--depth 1\n[general]\n--auto\n");
    let (_, body) = tree.get().await;
    assert_eq!(body["auto"], true, "{body}");
    assert_eq!(body["depth"]["general"]["all"], Value::Null, "{body}");
    assert_eq!(body["editable"], true, "{body}");

    let (status, body) = tree
        .put(json!({"revision": body["revision"], "auto": false}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["auto"], false);
    assert_eq!(
        tree.read("books/hledger.conf").unwrap(),
        "--depth 1\n[general]\n"
    );
}

#[tokio::test]
async fn a_hijack_in_an_explicit_general_section_is_shown_and_never_written() {
    let tree = Tree::new();
    tree.write("books/hledger.conf", "--auto\n[general]\nprint\n");
    let (_, body) = tree.get().await;
    assert!(
        body["hijacked"].as_str().unwrap().contains("`print`"),
        "{body}"
    );
    assert_eq!(body["editable"], false);
    let (status, _) = tree
        .put(json!({"revision": body["revision"], "auto": true}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        tree.read("books/hledger.conf").unwrap(),
        "--auto\n[general]\nprint\n"
    );
}

#[tokio::test]
async fn a_no_op_write_leaves_the_bytes_and_the_generation_alone() {
    let tree = Tree::new();
    tree.write("books/hledger.conf", "--auto   # rules on\r\n");
    let (_, body) = tree.get().await;
    let before = tree.generation().await;
    let (status, after) = tree
        .put(json!({"revision": body["revision"], "auto": true}))
        .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["revision"], body["revision"]);
    assert_eq!(
        tree.read("books/hledger.conf").unwrap(),
        "--auto   # rules on\r\n"
    );
    assert_eq!(tree.generation().await, before);
}

#[tokio::test]
async fn an_alias_named_section_is_warned_about() {
    let tree = Tree::new();
    tree.write("books/hledger.conf", "[bs]\n--depth 1\n");
    let (_, body) = tree.get().await;
    assert_eq!(body["warnings"][0]["code"], "aliasSection", "{body}");
    assert!(
        body["warnings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("[balancesheet]")
    );
    assert_eq!(body["effectiveDepth"]["balancesheet"]["all"], Value::Null);
}

#[tokio::test]
async fn an_external_change_is_noticed_by_the_next_read_and_republished() {
    let tree = Tree::new();
    tree.write("books/hledger.conf", "--depth 2\n");
    let (_, body) = tree.get().await;
    assert_eq!(body["depth"]["general"]["all"], 2);
    let before = tree.generation().await;
    tree.write("books/hledger.conf", "--depth 4\n");
    let (_, body) = tree.get().await;
    assert_eq!(body["depth"]["general"]["all"], 4);
    assert_ne!(tree.generation().await, before);
    assert_eq!(tree.state.conf_settings().general.all, Some(4));
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_file_is_read_but_not_written_through() {
    let tree = Tree::new();
    tree.write("elsewhere/real.conf", "--depth 2\n");
    std::os::unix::fs::symlink(tree.path("elsewhere/real.conf"), tree.path("hledger.conf"))
        .unwrap();
    let (_, body) = tree.get().await;
    assert_eq!(body["handle"], "../hledger.conf");
    assert_eq!(body["depth"]["general"]["all"], 2);
    assert_eq!(body["editable"], false);
    assert!(body["readOnlyReason"].as_str().unwrap().contains("symlink"));
    let (status, _) = tree
        .put(json!({"revision": body["revision"], "auto": true}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(tree.read("elsewhere/real.conf").unwrap(), "--depth 2\n");
}

#[tokio::test]
async fn a_read_only_server_reports_and_refuses() {
    let journal = ledgeline_core::parse_journal(JOURNAL, "-").expect("parses");
    let state = AppState::from_journal(&journal);
    let (status, body) = send(
        router_with_state(state.clone()),
        Request::builder()
            .uri("/api/hledger-conf")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["editable"], false);
    let (status, _) = send(
        router_with_state(state),
        Request::builder()
            .method("PUT")
            .uri("/api/hledger-conf")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"revision":"","auto":true}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn every_conf_route_requires_the_token() {
    const HOST: &str = "127.0.0.1:5099";
    let tree = Tree::new();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");
    for method in ["GET", "PUT"] {
        for auth in [None, Some("Bearer integration-test-token")] {
            let mut builder = Request::builder()
                .method(method)
                .uri("/api/hledger-conf")
                .header(HeaderName::from_static("host"), HOST)
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(value) = auth {
                builder = builder.header(header::AUTHORIZATION, value);
            }
            let status =
                router_with_security(tree.state.clone(), Security::local(token.clone(), 5099))
                    .oneshot(builder.body(Body::from(r#"{"revision":"x"}"#)).unwrap())
                    .await
                    .unwrap()
                    .status();
            if auth.is_none() {
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} without a token");
            } else {
                assert_ne!(status, StatusCode::UNAUTHORIZED, "{method} with the token");
            }
        }
    }
    assert_eq!(tree.read("books/hledger.conf"), None);
}

/// Opt-in (`LEDGELINE_HLEDGER_CONF_CHECK=1`, run by `just hledger-checks`):
/// write a config through this route, then ask a real `hledger bs` — WITHOUT
/// `--no-conf`, from the journal's directory, with `$HOME` pointed at the
/// scratch home — whether it applies the depth we wrote.
#[tokio::test]
async fn hledger_reads_the_depth_this_route_writes() {
    if std::env::var_os("LEDGELINE_HLEDGER_CONF_CHECK").is_none() {
        return;
    }
    let tree = Tree::new();
    let (status, body) = tree
        .put(json!({
            "revision": "",
            "depth": {"balancesheet": {"all": 1, "perRegex": [{"regex": "inv", "depth": 3}]}}
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let hledger = std::env::var("LEDGELINE_HLEDGER").unwrap_or_else(|_| "hledger".to_string());
    let run = |command: &str| {
        let output = std::process::Command::new(&hledger)
            .args([command, "-f", "main.journal", "--flat"])
            .current_dir(tree.path("books"))
            .env("HOME", tree.path("home"))
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .expect("hledger runs");
        assert!(output.status.success(), "{output:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let bs = run("bs");
    // Depth 1 everywhere except accounts matching `inv`, which go to 3.
    assert!(bs.contains("assets:inv:broker "), "{bs}");
    assert!(!bs.contains("assets:bank"), "{bs}");
    assert!(!bs.contains("assets:inv:broker:fund"), "{bs}");
    // The section is `[balancesheet]` only: `balance` is unclamped.
    assert!(run("balance").contains("assets:bank:checking"));
}
