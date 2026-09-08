//! The `/api/accounts` HTTP surface.
//!
//! Everything here is hermetic — `PUT /api/accounts/{*id}` rewrites a line of
//! a journal and `POST /api/accounts/file` creates one beside it; neither
//! runs a subprocess.
//!
//! The properties this file exists to pin, in order of how much a regression
//! would cost:
//!
//! 1. **An isolated edit changes one line and nothing else.** Asserted on the
//!    bytes, over a journal deliberately full of things that must not move.
//! 2. **A stale revision is a 409**, and nothing is written.
//! 3. **`accounts.journal` is created and included correctly**, and never
//!    twice, and never over an existing file.
//! 4. **The token guard covers all three routes.**
//! 5. **No response body contains an absolute path.**

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{AccessToken, AppState, Security, router_with_security, router_with_state};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// The scratch tree
// ---------------------------------------------------------------------------

/// A journal whose every byte is a hostage: alignment, a comment, a blank
/// line, and a transaction that must not move.
const JOURNAL: &str = "; the chart of accounts\n\
                       account assets:bank:checking  ; type: A\n\
                       account expenses:food          ; type: X, groceries and dining\n\
                       account equity:opening\n\
                       \n\
                       2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

struct Tree {
    dir: TempDir,
    state: AppState,
}

impl Tree {
    fn with(text: &str) -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), text).expect("write journal");
        let state = AppState::from_journal_path(dir.path().join("main.journal"))
            .expect("the scratch journal opens");
        Self { dir, state }
    }

    fn declared() -> Self {
        Self::with(JOURNAL)
    }

    fn router(&self) -> axum::Router {
        router_with_state(self.state.clone())
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).expect("read back")
    }

    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        walk(self.dir.path(), self.dir.path(), &mut files);
        files
    }
}

fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, into);
        } else if let Ok(bytes) = std::fs::read(&path)
            && let Ok(relative) = path.strip_prefix(root)
        {
            into.insert(relative.to_string_lossy().into_owned(), bytes);
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP helpers
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

async fn send_json(tree: &Tree, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("request builds");
    let (status, text) = send(tree.router(), request).await;
    (status, json_or_text(&text))
}

/// The `main.journal` entry of a `GET /api/accounts` body.
fn main_file(body: &Value) -> &Value {
    body["files"]
        .as_array()
        .expect("files is an array")
        .iter()
        .find(|file| file["journalId"] == json!("main.journal"))
        .expect("main.journal is listed")
}

async fn revision(tree: &Tree) -> String {
    let (status, body) = get(tree, "/api/accounts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    main_file(&body)["revision"]
        .as_str()
        .expect("a revision")
        .to_string()
}

// ===========================================================================
// Reading
// ===========================================================================

#[tokio::test]
async fn the_listing_reports_names_tags_and_notes() {
    let tree = Tree::declared();
    let (status, body) = get(&tree, "/api/accounts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["editable"], json!(true));
    assert_eq!(
        body["canCreateFile"],
        json!(false),
        "accounts already exist"
    );

    let file = main_file(&body);
    assert_eq!(file["label"], json!("main.journal"));
    assert_eq!(file["writable"], json!(true));
    let accounts = file["accounts"].as_array().expect("accounts is an array");
    assert_eq!(accounts.len(), 3);

    assert_eq!(accounts[0]["name"], json!("assets:bank:checking"));
    assert_eq!(accounts[0]["tags"], json!([{"name": "type", "value": "A"}]));
    assert_eq!(accounts[0]["note"], json!(""));
    assert_eq!(accounts[0]["index"], json!(0));
    assert_eq!(accounts[0]["line"], json!(2));

    assert_eq!(accounts[1]["name"], json!("expenses:food"));
    assert_eq!(accounts[1]["tags"], json!([{"name": "type", "value": "X"}]));
    assert_eq!(accounts[1]["note"], json!("groceries and dining"));

    assert_eq!(accounts[2]["name"], json!("equity:opening"));
    assert_eq!(accounts[2]["tags"], json!([]));
    assert_eq!(accounts[2]["note"], json!(""));
}

/// The root journal is offered even with no accounts at all, because it is
/// where a first one goes.
#[tokio::test]
async fn a_journal_with_no_accounts_still_offers_its_root() {
    let tree = Tree::with("2026-01-01 x\n    a  $1\n    b\n");
    let (status, body) = get(&tree, "/api/accounts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let file = main_file(&body);
    assert_eq!(file["accounts"], json!([]));
    assert_eq!(file["writable"], json!(true));
    assert_eq!(body["canCreateFile"], json!(true));
    assert_eq!(body["createFileName"], json!("accounts.journal"));
}

// ===========================================================================
// Writing
// ===========================================================================

/// The property the whole write path exists for: one line changes, and every
/// other byte of the journal comes back identical.
#[tokio::test]
async fn an_edit_rewrites_one_line_and_leaves_every_other_byte_alone() {
    let tree = Tree::declared();
    let revision = revision(&tree).await;
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/accounts/main.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "replace", "index": 0,
                       "tags": [{"name": "type", "value": "A"}, {"name": "bsgroup", "value": "Cash"}],
                       "note": ""}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let after = tree.read("main.journal");
    assert_eq!(
        after,
        JOURNAL.replace(
            "account assets:bank:checking  ; type: A\n",
            "account assets:bank:checking  ; type: A, bsgroup: Cash\n"
        ),
        "exactly one line changed"
    );
    let differing: Vec<(&str, &str)> = JOURNAL
        .lines()
        .zip(after.lines())
        .filter(|(before, after)| before != after)
        .collect();
    assert_eq!(differing.len(), 1, "{differing:?}");

    assert_ne!(body["revision"], json!(revision));
    assert_eq!(
        body["accounts"][0]["tags"],
        json!([{"name": "type", "value": "A"}, {"name": "bsgroup", "value": "Cash"}])
    );
}

/// Declaring a previously-undeclared account appends it after the last
/// declaration, and every other byte is untouched.
#[tokio::test]
async fn declaring_an_account_appends_after_the_last_declaration() {
    let tree = Tree::declared();
    let revision = revision(&tree).await;
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/accounts/main.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "declare", "name": "liabilities:credit-card",
                       "tags": [{"name": "type", "value": "L"}], "note": ""}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("main.journal"),
        JOURNAL.replace(
            "account equity:opening\n",
            "account equity:opening\naccount liabilities:credit-card  ; type: L\n",
        ),
        "the new line joins the block; every other byte is untouched"
    );
    assert_eq!(
        body["accounts"][3]["name"],
        json!("liabilities:credit-card")
    );
}

/// Removing a declaration deletes exactly that line and its terminator, and
/// leaves every other byte — including the postings that still name the
/// account — untouched.
#[tokio::test]
async fn deleting_a_declaration_removes_exactly_one_line() {
    let tree = Tree::declared();
    let revision = revision(&tree).await;
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/accounts/main.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "delete", "index": 1}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("main.journal"),
        JOURNAL.replace(
            "account expenses:food          ; type: X, groceries and dining\n",
            ""
        ),
    );
    assert_eq!(body["accounts"].as_array().expect("accounts").len(), 2);
}

/// A note that would be read back as a tag is refused before a byte is
/// rendered.
#[tokio::test]
async fn a_note_that_would_read_back_as_a_tag_is_refused() {
    let tree = Tree::declared();
    let before = tree.snapshot();
    let revision = revision(&tree).await;
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/accounts/main.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "replace", "index": 2, "tags": [],
                       "note": "ping bob, re: taxes"}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("read back as a tag"),
        "the refusal must say why: {body}"
    );
    assert_eq!(tree.snapshot(), before);
}

/// A save against a superseded revision is a 409, and the file on disk is
/// untouched.
#[tokio::test]
async fn a_stale_revision_is_a_conflict_and_writes_nothing() {
    let tree = Tree::declared();
    let before = tree.snapshot();
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/accounts/main.journal",
        json!({
            "revision": "0-deadbeefdeadbeef",
            "edits": [{"kind": "replace", "index": 0, "tags": [], "note": "x"}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(tree.snapshot(), before, "nothing may be written on a 409");
}

/// A no-op save writes nothing at all.
#[tokio::test]
async fn a_no_op_save_does_not_touch_the_file() {
    let tree = Tree::declared();
    let revision = revision(&tree).await;
    let before = std::fs::metadata(tree.path("main.journal"))
        .and_then(|meta| meta.modified())
        .expect("mtime");
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/accounts/main.journal",
        json!({"revision": revision, "edits": []}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(tree.read("main.journal"), JOURNAL);
    assert_eq!(
        std::fs::metadata(tree.path("main.journal"))
            .and_then(|meta| meta.modified())
            .expect("mtime"),
        before
    );
}

// ===========================================================================
// Creating an accounts file
// ===========================================================================

/// The first-declaration path: create `accounts.journal`, and `include` it at
/// EOF.
#[tokio::test]
async fn creating_an_accounts_file_writes_it_and_includes_it() {
    let tree = Tree::with("2026-01-05 grocery\n    expenses:food   $10.00\n    assets:checking\n");
    let (status, body) = get(&tree, "/api/accounts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["canCreateFile"], json!(true));
    assert_eq!(body["createFileName"], json!("accounts.journal"));

    let (status, body) = send_json(&tree, "POST", "/api/accounts/file", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["journalId"], json!("accounts.journal"));
    assert_eq!(body["includedAs"], json!("include accounts.journal"));
    assert_eq!(body["mainJournalId"], json!("main.journal"));

    assert!(
        tree.read("accounts.journal")
            .starts_with("; Chart of accounts.")
    );
    let main = tree.read("main.journal");
    assert!(main.ends_with("include accounts.journal\n"), "{main}");
    assert!(main.starts_with("2026-01-05 grocery\n"), "{main}");

    // A declaration can now be added to the newly created file.
    let (status, listing) = get(&tree, "/api/accounts").await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert_eq!(listing["canCreateFile"], json!(false));
    let revision = listing["files"]
        .as_array()
        .expect("files is an array")
        .iter()
        .find(|file| file["journalId"] == json!("accounts.journal"))
        .expect("accounts.journal is listed")["revision"]
        .as_str()
        .expect("a revision")
        .to_string();
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/accounts/accounts.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "declare", "name": "assets:checking", "tags": [], "note": ""}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        tree.read("accounts.journal")
            .ends_with("account assets:checking\n"),
        "{}",
        tree.read("accounts.journal")
    );
}

/// An existing `accounts.journal` is never written over.
#[tokio::test]
async fn an_existing_accounts_file_is_never_overwritten() {
    let tree = Tree::with("2026-01-05 grocery\n    expenses:food   $10.00\n    assets:checking\n");
    std::fs::write(tree.path("accounts.journal"), "; someone else's notes\n").expect("write");
    let before = tree.snapshot();

    let (status, body) = get(&tree, "/api/accounts").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["canCreateFile"],
        json!(false),
        "the button must not be offered when it would fail"
    );

    let (status, body) = send_json(&tree, "POST", "/api/accounts/file", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(tree.snapshot(), before, "nothing may be written");
}

/// A journal that already declares accounts is not given a second home for
/// them.
#[tokio::test]
async fn a_journal_with_accounts_is_not_given_another_accounts_file() {
    let tree = Tree::declared();
    let before = tree.snapshot();
    let (status, body) = send_json(&tree, "POST", "/api/accounts/file", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(tree.snapshot(), before);
}

// ===========================================================================
// Handles
// ===========================================================================

#[tokio::test]
async fn a_handle_that_is_not_a_journal_file_never_reaches_the_filesystem() {
    let tree = Tree::declared();
    let body = json!({"revision": "x", "edits": []});
    for (id, expected) in [
        ("../escape.journal", StatusCode::BAD_REQUEST),
        ("/etc/passwd", StatusCode::BAD_REQUEST),
        ("a/../../b.journal", StatusCode::BAD_REQUEST),
        ("nope.journal", StatusCode::NOT_FOUND),
    ] {
        let (status, response) =
            send_json(&tree, "PUT", &format!("/api/accounts/{id}"), body.clone()).await;
        assert_eq!(status, expected, "{id}: {response}");
    }
}

/// SEC-1. All three routes rewrite or create a file beside the user's
/// journal, so they are registered ABOVE the `route_layer` token guard.
#[tokio::test]
async fn every_account_route_requires_the_token() {
    const PORT: u16 = 5098;
    const HOST: &str = "127.0.0.1:5098";
    let tree = Tree::declared();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");

    let probe = |method: &'static str, uri: &'static str, auth: Option<&'static str>| {
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
                .body(Body::from(r#"{"revision":"x","edits":[]}"#))
                .expect("request builds");
            router_with_security(state, security)
                .oneshot(request)
                .await
                .expect("router responds")
                .status()
        }
    };

    for (method, uri) in [
        ("GET", "/api/accounts"),
        ("PUT", "/api/accounts/main.journal"),
        ("POST", "/api/accounts/file"),
    ] {
        assert_eq!(
            probe(method, uri, None).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} without a token must be 401"
        );
        assert_ne!(
            probe(method, uri, Some("Bearer integration-test-token")).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with the token must not be 401"
        );
    }
}

// ===========================================================================
// Layer 5 — no absolute path in any response
// ===========================================================================

#[tokio::test]
async fn no_account_response_body_contains_an_absolute_path() {
    let tree = Tree::declared();
    let revision = revision(&tree).await;
    let mut bodies: Vec<(String, Value)> = Vec::new();

    let (_, body) = get(&tree, "/api/accounts").await;
    bodies.push(("/api/accounts".to_string(), body));

    for (id, request) in [
        (
            "main.journal",
            json!({"revision": revision, "edits": [
                {"kind": "replace", "index": 0, "tags": [], "note": "x"}]}),
        ),
        ("main.journal", json!({"revision": "stale", "edits": []})),
        ("nope.journal", json!({"revision": "x", "edits": []})),
        ("../escape", json!({"revision": "x", "edits": []})),
    ] {
        let (_, body) = send_json(&tree, "PUT", &format!("/api/accounts/{id}"), request).await;
        bodies.push((format!("PUT {id}"), body));
    }

    for (what, body) in bodies {
        assert_no_absolute_path(&tree, &body, &what);
    }
}

fn assert_no_absolute_path(tree: &Tree, body: &Value, what: &str) {
    let text = body.to_string();
    let secrets: Vec<PathBuf> = [tree.dir.path().to_path_buf(), std::env::temp_dir()]
        .into_iter()
        .flat_map(|path| {
            let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            [path, canonical]
        })
        .collect();
    for secret in secrets {
        let raw = secret.to_string_lossy().into_owned();
        for spelling in [raw.clone(), raw.replace('/', "\\/")] {
            assert!(
                !text.contains(&spelling),
                "{what} disclosed {spelling}:\n{text}"
            );
        }
    }
}
