//! The `/api/aliases` HTTP surface, and the alias half of
//! `/api/import/capabilities`.
//!
//! Everything here is hermetic. `PUT /api/aliases/{*id}` rewrites a line of a
//! journal and never runs a subprocess, so there is nothing for a real hledger
//! to contribute — the checks that *do* need one (that a forwarded alias
//! actually maps a CSV, and that the dry-run and the commit agree about it) live
//! in `import_endpoints.rs` behind `LEDGELINE_HLEDGER_IMPORT_CHECK`.
//!
//! The properties this file exists to pin, in order of how much a regression
//! would cost:
//!
//! 1. **An isolated edit changes one line and nothing else.** Asserted on the
//!    bytes, over a journal deliberately full of things that must not move:
//!    CRLF-free alignment, a comment, a transaction, a second alias.
//!    A rewrite that reformats a journal is unrecoverable damage to the most
//!    valuable file this application touches.
//! 2. **A stale revision is a 409**, and nothing is written.
//! 3. **An unmodelled alias line is read-only** and a `PUT` naming it is
//!    refused, rather than being silently rewritten into a different mapping.
//! 4. **The token guard covers both routes.**
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

/// A journal whose every byte is a hostage.
///
/// The alignment, the comment, the blank lines, the trailing-comment alias and
/// the transaction are all here so that "only the line I edited changed" is a
/// claim with something to be false about.
const JOURNAL: &str = "; how this file maps bank-speak to accounts\n\
                       alias PW Roth IRA - 3077   = assets:morganstanley:pw-roth-ira\n\
                       alias CHK 8842             = assets:bank:checking\n\
                       alias legacy = old:name ; kept for the 2024 statements\n\
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

    fn aliased() -> Self {
        Self::with(JOURNAL)
    }

    fn router(&self) -> axum::Router {
        router_with_state(self.state.clone())
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(relative)).expect("read back")
    }

    /// Every file in the tree, keyed by its relative path.
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

/// The `main.journal` entry of a `GET /api/aliases` body.
fn main_file(body: &Value) -> &Value {
    body["files"]
        .as_array()
        .expect("files is an array")
        .iter()
        .find(|file| file["journalId"] == json!("main.journal"))
        .expect("main.journal is listed")
}

/// The current revision of `main.journal`.
async fn revision(tree: &Tree) -> String {
    let (status, body) = get(tree, "/api/aliases").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    main_file(&body)["revision"]
        .as_str()
        .expect("a revision")
        .to_string()
}

// ===========================================================================
// Reading
// ===========================================================================

/// Every alias, in file order, with both of its independent verdicts: will an
/// import use it, and will the GUI rewrite it.
#[tokio::test]
async fn the_listing_reports_forwarding_and_editability_separately() {
    let tree = Tree::aliased();
    let (status, body) = get(&tree, "/api/aliases").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["editable"], json!(true));

    let file = main_file(&body);
    assert_eq!(file["label"], json!("main.journal"));
    assert_eq!(file["writable"], json!(true));
    let aliases = file["aliases"].as_array().expect("aliases is an array");
    assert_eq!(aliases.len(), 3);

    assert_eq!(aliases[0]["pattern"], json!("PW Roth IRA - 3077"));
    assert_eq!(
        aliases[0]["replacement"],
        json!("assets:morganstanley:pw-roth-ira")
    );
    assert_eq!(aliases[0]["regex"], json!(false));
    assert_eq!(aliases[0]["index"], json!(0));
    assert_eq!(aliases[0]["line"], json!(2));
    assert_eq!(aliases[0]["forwarded"], json!(true));
    assert_eq!(aliases[0]["editable"], json!(true));

    // The third carries a `;`, which hledger reads as part of the account name.
    // Forwarded — hledger will use it, so hiding it would be a lie — but never
    // rewritten, because rewriting it would cement a reading its author almost
    // certainly did not intend.
    assert_eq!(
        aliases[2]["replacement"],
        json!("old:name ; kept for the 2024 statements")
    );
    assert_eq!(aliases[2]["forwarded"], json!(true));
    assert_eq!(aliases[2]["editable"], json!(false));
    assert_eq!(aliases[2]["lock"], json!("commentLike"));
    assert!(
        aliases[2]["lockMessage"]
            .as_str()
            .is_some_and(|why| why.contains("not as a comment")),
        "the lock must say WHY: {}",
        aliases[2]
    );
}

/// An `end aliases` is honoured: the alias is listed and editable, and is
/// deliberately not handed to an import, because `--alias` is global and the
/// user wrote down where this one stops.
#[tokio::test]
async fn a_scoped_alias_is_listed_but_not_forwarded() {
    let tree = Tree::with("alias closed = a:closed\nend aliases\nalias open = a:open\n");
    let (status, body) = get(&tree, "/api/aliases").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let aliases = main_file(&body)["aliases"]
        .as_array()
        .expect("aliases is an array");

    assert_eq!(aliases[0]["forwarded"], json!(false));
    assert_eq!(aliases[0]["refusal"], json!("scoped"));
    assert_eq!(aliases[0]["editable"], json!(true));
    assert_eq!(aliases[1]["forwarded"], json!(true));
}

/// The root journal is offered even with no aliases at all, because it is where
/// a first one goes and there is otherwise nowhere to add it.
#[tokio::test]
async fn a_journal_with_no_aliases_still_offers_its_root() {
    let tree = Tree::with("2026-01-01 x\n    a  $1\n    b\n");
    let (status, body) = get(&tree, "/api/aliases").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let file = main_file(&body);
    assert_eq!(file["aliases"], json!([]));
    assert_eq!(file["writable"], json!(true));
}

/// `capabilities` and the alias editor are built by the same function, so the
/// New Transactions screen cannot advertise a different set from the one the
/// editor shows — which would make showing it pointless.
#[tokio::test]
async fn capabilities_carries_the_same_aliases_the_editor_lists() {
    let tree = Tree::aliased();
    let (status, capabilities) = get(&tree, "/api/import/capabilities").await;
    assert_eq!(status, StatusCode::OK, "{capabilities}");
    let (_, listing) = get(&tree, "/api/aliases").await;

    assert_eq!(capabilities["aliases"], main_file(&listing)["aliases"]);
    assert_eq!(
        capabilities["aliases"]
            .as_array()
            .expect("aliases is an array")
            .len(),
        3
    );
}

// ===========================================================================
// Writing
// ===========================================================================

/// The property the whole write path exists for: one line changes, and every
/// other byte of the journal comes back identical — the alignment, the comment,
/// the blank line, the transaction, the other aliases.
#[tokio::test]
async fn an_edit_rewrites_one_line_and_leaves_every_other_byte_alone() {
    let tree = Tree::aliased();
    let revision = revision(&tree).await;
    let (status, body) = put(
        &tree,
        "/api/aliases/main.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "replace", "index": 1,
                       "pattern": "CHK 8842", "replacement": "assets:bank:everyday",
                       "regex": false}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let after = tree.read("main.journal");
    assert_eq!(
        after,
        JOURNAL.replace("assets:bank:checking\n", "assets:bank:everyday\n"),
        "exactly one replacement, and the column alignment is untouched"
    );
    // Said the other way round, on the bytes: every line but one is identical.
    let differing: Vec<(&str, &str)> = JOURNAL
        .lines()
        .zip(after.lines())
        .filter(|(before, after)| before != after)
        .collect();
    assert_eq!(differing.len(), 1, "{differing:?}");

    // And the response is the file at its new revision, so the next save is
    // against bytes the client has seen.
    assert_ne!(body["revision"], json!(revision));
    assert_eq!(
        body["aliases"][1]["replacement"],
        json!("assets:bank:everyday")
    );
}

/// A new alias joins the existing block rather than landing at the end of the
/// file: the furthest-forward position that is provably still in force where an
/// import appends AND provably unable to change what anything already in the
/// file means. Everything else is byte-identical.
#[tokio::test]
async fn an_append_joins_the_existing_alias_block() {
    let tree = Tree::aliased();
    let revision = revision(&tree).await;
    let (status, body) = put(
        &tree,
        "/api/aliases/main.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "append", "pattern": "^SAV ([0-9]+)$",
                       "replacement": "assets:bank:savings", "regex": true}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("main.journal"),
        JOURNAL.replace(
            "alias legacy = old:name ; kept for the 2024 statements\n",
            "alias legacy = old:name ; kept for the 2024 statements\n\
             alias /^SAV ([0-9]+)$/ = assets:bank:savings\n",
        ),
        "the new line joins the block; every other byte is untouched"
    );
    assert_eq!(body["aliases"][3]["regex"], json!(true));
    assert_eq!(body["aliases"][3]["pattern"], json!("^SAV ([0-9]+)$"));
    // The response describes the journal as it NOW is, so a freshly added alias
    // reports itself in force rather than telling the user to reload.
    assert_eq!(body["aliases"][3]["forwarded"], json!(true));
}

/// Deleting removes the line and its terminator, and nothing else.
#[tokio::test]
async fn a_delete_removes_exactly_one_line() {
    let tree = Tree::aliased();
    let revision = revision(&tree).await;
    let (status, body) = put(
        &tree,
        "/api/aliases/main.journal",
        json!({"revision": revision, "edits": [{"kind": "delete", "index": 1}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        tree.read("main.journal"),
        JOURNAL.replace("alias CHK 8842             = assets:bank:checking\n", "")
    );
}

/// A save against a revision somebody else has superseded is a 409, and the file
/// on disk is untouched — the whole point of carrying a revision.
#[tokio::test]
async fn a_stale_revision_is_a_conflict_and_writes_nothing() {
    let tree = Tree::aliased();
    let before = tree.snapshot();
    let (status, body) = put(
        &tree,
        "/api/aliases/main.journal",
        json!({
            "revision": "0-deadbeefdeadbeef",
            "edits": [{"kind": "replace", "index": 0, "pattern": "x",
                       "replacement": "y", "regex": false}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(tree.snapshot(), before, "nothing may be written on a 409");
}

/// The same, for a file edited underneath a client that read it a moment ago.
#[tokio::test]
async fn an_external_edit_between_read_and_save_is_a_conflict() {
    let tree = Tree::aliased();
    let revision = revision(&tree).await;
    std::fs::write(
        tree.dir.path().join("main.journal"),
        format!("{JOURNAL}alias somebody = else:entirely\n"),
    )
    .expect("the other writer");

    let (status, body) = put(
        &tree,
        "/api/aliases/main.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "delete", "index": 0}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        tree.read("main.journal")
            .contains("alias somebody = else:entirely"),
        "the other writer's line must survive"
    );
}

/// A line this server presents read-only is refused rather than rewritten into
/// something the author did not write.
#[tokio::test]
async fn rewriting_an_unmodelled_alias_is_refused() {
    let tree = Tree::aliased();
    let before = tree.snapshot();
    let revision = revision(&tree).await;
    let (status, body) = put(
        &tree,
        "/api/aliases/main.journal",
        json!({
            "revision": revision,
            "edits": [{"kind": "replace", "index": 2, "pattern": "legacy",
                       "replacement": "new:name", "regex": false}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("not as a comment"),
        "the refusal must say why: {body}"
    );
    assert_eq!(tree.snapshot(), before);
}

/// Values that would be written and then read back as something else are
/// refused before a byte is rendered.
#[tokio::test]
async fn values_that_would_not_read_back_are_refused() {
    let tree = Tree::aliased();
    let before = tree.snapshot();
    let revision = revision(&tree).await;
    for (pattern, replacement, needle) in [
        // A newline is how one would smuggle a second directive into one line.
        ("CHK 8842", "a:b\nalias sneaky = evil", "control character"),
        ("CHK 8842", "", "may not be empty"),
        // hledger splits at the first `=`, so this would not be the mapping asked for.
        ("CHK=8842", "a:b", "splits the line"),
        ("CHK 8842", "a:b ; note", "`;` or `#`"),
        ("CHK 8842", " a:b", "whitespace"),
    ] {
        let (status, body) = put(
            &tree,
            "/api/aliases/main.journal",
            json!({
                "revision": revision,
                "edits": [{"kind": "replace", "index": 1, "pattern": pattern,
                           "replacement": replacement, "regex": false}],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{pattern:?}: {body}");
        assert!(
            body.to_string().contains(needle),
            "{pattern:?}/{replacement:?}: {body}"
        );
    }
    assert_eq!(tree.snapshot(), before);
}

/// A no-op writes nothing at all. Writing byte-identical content still bumps
/// mtime, and a user's own `entr` or watch loop would see a change that did not
/// happen.
#[tokio::test]
async fn a_no_op_save_does_not_touch_the_file() {
    let tree = Tree::aliased();
    let revision = revision(&tree).await;
    let before = std::fs::metadata(tree.dir.path().join("main.journal"))
        .and_then(|meta| meta.modified())
        .expect("mtime");
    let (status, body) = put(
        &tree,
        "/api/aliases/main.journal",
        json!({"revision": revision, "edits": []}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(tree.read("main.journal"), JOURNAL);
    assert_eq!(
        std::fs::metadata(tree.dir.path().join("main.journal"))
            .and_then(|meta| meta.modified())
            .expect("mtime"),
        before
    );
}

// ===========================================================================
// Handles
// ===========================================================================

/// Layer 1 and layer 2: a handle is refused on its shape before any filesystem
/// call, and one that is merely absent from the set the parse produced is a 404
/// with the same sentence every other resolution failure gets.
#[tokio::test]
async fn a_handle_that_is_not_a_journal_file_never_reaches_the_filesystem() {
    let tree = Tree::aliased();
    let body = json!({"revision": "x", "edits": []});
    for (id, expected) in [
        ("../escape.journal", StatusCode::BAD_REQUEST),
        ("/etc/passwd", StatusCode::BAD_REQUEST),
        ("a/../../b.journal", StatusCode::BAD_REQUEST),
        ("nope.journal", StatusCode::NOT_FOUND),
        ("import/bank.csv.rules", StatusCode::NOT_FOUND),
    ] {
        let (status, response) = put(&tree, &format!("/api/aliases/{id}"), body.clone()).await;
        assert_eq!(status, expected, "{id}: {response}");
    }
}

/// SEC-1. `PUT /api/aliases/{*id}` rewrites a line of the user's journal, so it
/// is registered ABOVE the `route_layer` token guard; below it, it would do that
/// with no credential at all.
#[tokio::test]
async fn every_alias_route_requires_the_token() {
    const PORT: u16 = 5099;
    const HOST: &str = "127.0.0.1:5099";
    let tree = Tree::aliased();
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
        ("GET", "/api/aliases"),
        ("PUT", "/api/aliases/main.journal"),
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

/// Not one response — success or failure — may name the scratch directory or any
/// absolute path. An alias route is the one place a *journal file's* path could
/// leak, which is why the whole-journal parse refusal deliberately carries no
/// diagnostic text.
#[tokio::test]
async fn no_alias_response_body_contains_an_absolute_path() {
    let tree = Tree::aliased();
    let revision = revision(&tree).await;
    let mut bodies: Vec<(String, Value)> = Vec::new();

    for uri in ["/api/aliases", "/api/import/capabilities"] {
        let (_, body) = get(&tree, uri).await;
        bodies.push((uri.to_string(), body));
    }
    for (id, request) in [
        (
            "main.journal",
            json!({"revision": revision, "edits": [
                {"kind": "replace", "index": 2, "pattern": "legacy",
                 "replacement": "x", "regex": false}]}),
        ),
        ("main.journal", json!({"revision": "stale", "edits": []})),
        ("nope.journal", json!({"revision": "x", "edits": []})),
        ("../escape", json!({"revision": "x", "edits": []})),
    ] {
        let (_, body) = put(&tree, &format!("/api/aliases/{id}"), request).await;
        bodies.push((format!("PUT {id}"), body));
    }

    for (what, body) in bodies {
        assert_no_absolute_path(&tree, &body, &what);
    }
}

/// The scratch directory and the temp root, in every spelling they can reach a
/// response in: raw, canonicalized (macOS temp dirs go through `/private/var`),
/// and JSON-escaped.
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
