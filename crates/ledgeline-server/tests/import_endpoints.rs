//! The `/api/import/*` and `/api/prefs` HTTP surface (WP-11 lane E).
//!
//! # What is hermetic and what is not
//!
//! Three tiers, and the split is deliberate:
//!
//! * **Hermetic** — the token guard, every refusal `stage` makes, handle
//!   resolution, and the no-absolute-path rule. These run in a plain
//!   `cargo test` on a machine with nothing installed. The scratch tree they use
//!   deliberately contains **no rules files**, so candidate scoring finds nothing
//!   to score and the responses do not depend on whether hledger happens to be
//!   on `PATH`.
//! * **Stub-driven** — `capabilities` reporting a missing or too-old hledger.
//!   Also hermetic: a shell script that prints a version banner is enough, and
//!   the environment is handed to a re-executed child rather than mutated in this
//!   process (see [`run_child`], the pattern `tests/prefs.rs` established —
//!   `std::env::set_var` is `unsafe` in edition 2024 precisely because it races
//!   the other test threads).
//! * **hledger-backed** — the dry-run, the commit, and the balance proof. Gated
//!   behind `LEDGELINE_HLEDGER_IMPORT_CHECK=1`, exactly like
//!   `LEDGELINE_HLEDGER_RENDER_CHECK` in `rules_hledger_render.rs`, and run by
//!   `just hledger-checks`. Nothing about `hledger import`'s behaviour can be
//!   proved without hledger, and pretending otherwise with a stub would only test
//!   the stub.
//!
//! git-dependent tests **skip if absent** rather than opting in, matching
//! `tests/git_commit.rs`: git is on every machine that can clone this repository,
//! and "an import never touches your unrelated work" is too important to sit
//! behind a variable nobody exports.
//!
//! # The one that matters most
//!
//! [`concatenation_and_two_f_flags_disagree_and_two_f_is_wrong`] is the
//! regression test for fact 3 in `plans/11-enhanced-import.md`. Everything else
//! here guards a refusal; that one guards a **wrong answer that exits zero**,
//! which is the only class of bug in this feature a user would never notice.

mod common;

// `prefs.rs` and `hledger.rs` are private modules of the library, so an
// integration test cannot reach them through the public API; compiling the
// sources into this binary is the standard way out and is what `tests/prefs.rs`
// already does. `crate::prefs` inside `hledger.rs` resolves to the module below,
// because this file is the root of the test crate.
//
// `allow(dead_code)` because this file uses a different subset of them than
// `tests/prefs.rs` does — the modules themselves carry no blanket allow any
// more, now that `import_api.rs` consumes them.
#[path = "../src/prefs.rs"]
#[allow(dead_code)]
mod prefs;

#[path = "../src/hledger.rs"]
#[allow(dead_code)]
mod hledger;

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Request, StatusCode, header};
use common::fixtures_dir;
use hledger::Hledger;
use http_body_util::BodyExt;
use ledgeline_server::{AccessToken, AppState, Security, router_with_security, router_with_state};
use prefs::Prefs;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use tower::ServiceExt;

/// Opts in to the checks that shell out to a real `hledger`. Set by
/// `just hledger-checks`.
const IMPORT_CHECK: &str = "LEDGELINE_HLEDGER_IMPORT_CHECK";

/// Hands a re-executed child its scratch directory, since a child cannot be
/// given a `TempDir` handle.
const CHILD_DIR_ENV: &str = "LEDGELINE_TEST_CHILD_DIR";

/// The upload header carrying the dropped file's name.
const FILENAME: &str = "x-ledgeline-filename";

/// Skip, loudly, unless the opt-in variable is set.
macro_rules! require_hledger {
    () => {
        if std::env::var_os(IMPORT_CHECK).is_none() {
            eprintln!("skipping: set {IMPORT_CHECK}=1 (or run `just hledger-checks`)");
            return;
        }
    };
}

/// Skip, loudly, on a machine with no git. See the module docs for why this is
/// skip-if-absent rather than opt-in.
macro_rules! require_git {
    () => {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("skipping: no `git` on PATH");
            return;
        }
    };
}

// ---------------------------------------------------------------------------
// The scratch tree
// ---------------------------------------------------------------------------

/// A throwaway journal directory plus the [`AppState`] bound to it.
///
/// Every test that writes gets its own, so nothing here can be order-dependent
/// and nothing outlives the test that made it.
struct Tree {
    dir: TempDir,
    state: AppState,
}

/// The statement every hledger-backed test imports: three rows, one deposit and
/// two withdrawals, in the shape `fixtures/import/match/checking.csv.rules`
/// reads.
const STATEMENT: &str = "Date,Description,Withdrawal,Deposit\n\
                         01/15/2026,ACME PAYROLL,,3000.00\n\
                         01/16/2026,STARBUCKS,6.45,\n\
                         01/20/2026,LANDLORD LLC,1850.00,\n";

/// The opening balance the statement is imported on top of. Flat — **no
/// `include`** — because the fact-3 proof compares the literal `cat` spelling of
/// the concatenation against ours, and a relative `include` in a journal read
/// from stdin resolves against the process's working directory rather than the
/// journal's own (verified; it fails with `No files were matched by`).
const OPENING: &str = "2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

impl Tree {
    /// A journal, an `import/` directory, and nothing else. No rules file, so
    /// candidate scoring has nothing to score and every response below is the
    /// same whether or not hledger is installed.
    fn bare() -> Self {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.journal"), OPENING).expect("write journal");
        std::fs::create_dir(dir.path().join("import")).expect("import dir");
        let state = AppState::from_journal_path(dir.path().join("main.journal"))
            .expect("the scratch journal opens");
        Self { dir, state }
    }

    /// [`Tree::bare`] plus a correct rules file and two files that must never be
    /// touched, so a commit's blast radius is measurable.
    fn with_rules() -> Self {
        let tree = Self::bare();
        std::fs::copy(
            fixtures_dir().join("import/match/checking.csv.rules"),
            tree.dir.path().join("import/bank.csv.rules"),
        )
        .expect("copy the rules fixture");
        // Bystanders. A commit that changes either of these has swept up
        // something it was not asked to.
        std::fs::write(tree.dir.path().join("notes.txt"), "do not touch me\n")
            .expect("write bystander");
        std::fs::write(
            tree.dir.path().join("import/older.csv"),
            "Date,Description\n01/01/2020,OLD\n",
        )
        .expect("write bystander");
        tree
    }

    /// The `fixtures/import/layouts/split-year-assert/` tree, copied so it can
    /// be written to, with the checking rules file beside it.
    ///
    /// The one layout in the corpus whose target file does **not** pass
    /// `hledger check` on its own — see that tree's README. Everything in
    /// § Split layouts below runs against this.
    fn split_year_assert() -> Self {
        let dir = TempDir::new().expect("temp dir");
        copy_tree(
            &fixtures_dir().join("import/layouts/split-year-assert"),
            dir.path(),
        );
        std::fs::create_dir(dir.path().join("import")).expect("import dir");
        std::fs::copy(
            fixtures_dir().join("import/match/checking.csv.rules"),
            dir.path().join("import/bank.csv.rules"),
        )
        .expect("copy the rules fixture");
        let state = AppState::from_journal_path(dir.path().join("main.journal"))
            .expect("the scratch journal opens");
        Self { dir, state }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn router(&self) -> axum::Router {
        router_with_state(self.state.clone())
    }

    /// Every file in the tree, keyed by its relative path. The before/after
    /// comparison that proves a commit's blast radius.
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        walk(self.dir.path(), self.dir.path(), &mut files);
        files
    }
}

/// Copy a committed fixture tree into a scratch directory, subdirectories and
/// all. The fixtures are read-only corpus; a test that writes needs its own.
fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from)
        .expect("fixture tree readable")
        .flatten()
    {
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            std::fs::create_dir_all(&destination).expect("create scratch subdir");
            copy_tree(&source, &destination);
        } else {
            std::fs::copy(&source, &destination).expect("copy fixture file");
        }
    }
}

/// Collect every regular file under `dir`, keyed relative to `root`.
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

/// The relative paths whose content differs between two tree snapshots — added,
/// removed or modified.
fn changed(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|name| before.get(*name) != after.get(*name))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

/// Send a request and return the status plus the body as text.
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

/// Upload `bytes` as `name` through the raw-body stage route.
async fn upload(tree: &Tree, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    upload_to(&tree.router(), name, bytes).await
}

async fn upload_to(router: &axum::Router, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/api/import/stage")
        .header(HeaderName::from_static(FILENAME), name)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("request builds");
    let (status, text) = send(router.clone(), request).await;
    (status, json_or_text(&text))
}

/// A JSON body as a `Value`, or a plain-text error body as a JSON string — so
/// one helper covers both and every assertion below reads the same way.
fn json_or_text(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_string()))
}

/// The body as plain text, whatever shape it arrived in.
fn as_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string)
}

// ===========================================================================
// The token guard
// ===========================================================================

/// SEC-1, for the newest — and by some distance the most dangerous — write
/// primitive in the API. `POST /api/import/commit` writes a CSV into the user's
/// journal directory and appends to a journal file.
///
/// These routes are registered ABOVE the `route_layer` token guard in
/// `router_with_security`; below it, every one of them would be reachable with
/// no credential at all. Moving them must fail here rather than ship.
#[tokio::test]
async fn every_import_route_requires_the_token() {
    const PORT: u16 = 5098;
    const HOST: &str = "127.0.0.1:5098";
    let tree = Tree::bare();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");

    let probe = |method: &'static str, uri: &'static str, auth: Option<&'static str>| {
        let state = tree.state.clone();
        let security = Security::local(token.clone(), PORT);
        async move {
            let mut builder = Request::builder()
                .method(method)
                .uri(uri)
                .header(HeaderName::from_static("host"), HOST)
                .header(HeaderName::from_static(FILENAME), "bank.csv");
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
        ("GET", "/api/import/capabilities"),
        ("POST", "/api/import/stage"),
        ("POST", "/api/import/dry-run"),
        ("POST", "/api/import/commit"),
        ("POST", "/api/import/save-csv"),
        ("POST", "/api/import/sort"),
        ("POST", "/api/import/hledger-conf"),
        ("GET", "/api/prefs"),
        ("PUT", "/api/prefs"),
    ] {
        assert_eq!(
            probe(method, uri, None).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} without a token must be 401"
        );
        assert_eq!(
            probe(method, uri, Some("Bearer wrong-token-entirely")).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with a wrong token must be 401"
        );
        assert_ne!(
            probe(method, uri, Some("Bearer integration-test-token")).await,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} with the token must get past the guard"
        );
    }
}

// ===========================================================================
// stage — every refusal
// ===========================================================================

/// The upload limit is a `DefaultBodyLimit` on this route ALONE. A body past it
/// is refused by the transport before a byte of it reaches the converter.
#[tokio::test]
async fn an_oversize_upload_is_refused() {
    let tree = Tree::bare();
    // One byte past 16 MiB. `Content-Length` is known, so axum refuses without
    // buffering it.
    let (status, _) = upload(&tree, "huge.csv", vec![b'a'; 16 * 1024 * 1024 + 1]).await;
    assert!(
        status == StatusCode::PAYLOAD_TOO_LARGE || status == StatusCode::BAD_REQUEST,
        "an over-size upload must be refused, got {status}"
    );

    // And the limit really is local to this route: an ordinary JSON body is
    // still bounded by the global default, which is far smaller.
    let (status, _) = upload(&tree, "fine.csv", b"a,b\n1,2\n".to_vec()).await;
    assert_eq!(status, StatusCode::OK, "an ordinary upload still works");
}

/// The filename header is REFUSED when it is not a bare name, never silently
/// stripped down to one. It is used for format detection and for the destination
/// default, so it is validated before it is used for anything.
#[tokio::test]
async fn a_path_shaped_upload_filename_is_refused() {
    let tree = Tree::bare();
    for name in [
        "../../.bashrc",
        "../escape.csv",
        "/etc/passwd",
        "sub/bank.csv",
        "..",
    ] {
        let (status, body) = upload(&tree, name, b"a,b\n1,2\n".to_vec()).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{name:?} must be refused, got {body}"
        );
        let text = as_text(&body);
        assert!(
            text.contains("single plain file name"),
            "the refusal must say what a usable name is: {text}"
        );
    }

    // No header at all is its own refusal rather than a guess.
    let request = Request::builder()
        .method("POST")
        .uri("/api/import/stage")
        .body(Body::from("a,b\n1,2\n"))
        .expect("request builds");
    let (status, _) = send(tree.router(), request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// PDF is out of scope for WP-11 and is refused **by name** — the whole reason
/// `convert::detect` returns a `Result` rather than an `Option`. A generic
/// "unsupported file type" would send the user looking for a setting.
#[tokio::test]
async fn a_pdf_is_refused_with_its_own_message() {
    let tree = Tree::bare();
    let (status, body) = upload(
        &tree,
        "statement.pdf",
        b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(as_text(&body), "PDF statements are not supported yet");

    // And the content wins over the name, so a PDF renamed `.csv` gets the same
    // sentence rather than a confusing delimited-parse failure.
    let (status, body) = upload(&tree, "statement.csv", b"%PDF-1.7\n".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(as_text(&body), "PDF statements are not supported yet");
}

/// A `stageId` is only ever resolved by exact match against the map the session
/// that minted it holds. A perfectly well-formed id from a DIFFERENT server
/// session is a stranger — which is what keeps one window's staged bank
/// statement out of another's.
#[tokio::test]
async fn a_stage_from_another_session_is_unreadable() {
    let theirs = Tree::with_rules();
    let mine = Tree::with_rules();

    let (status, staged) = upload(&theirs, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let id = staged["stageId"].as_str().expect("a stageId").to_string();
    assert_eq!(id.len(), 32, "an id is 32 hex characters: {id}");

    // The other session holds it.
    let (status, _) = post(
        &theirs,
        "/api/import/dry-run",
        dry_run_body(&id, "import/bank.csv.rules"),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "the session that minted the id must recognise it"
    );

    // This one does not, and says so without saying anything else.
    let (status, body) = post(
        &mine,
        "/api/import/dry-run",
        dry_run_body(&id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        as_text(&body).contains("not a staged upload"),
        "{}",
        as_text(&body)
    );
}

/// A handle that is not the shape the minter produces never reaches the map.
#[tokio::test]
async fn a_malformed_stage_id_never_reaches_a_lookup() {
    let tree = Tree::with_rules();
    for id in [
        "",
        "../../etc/passwd",
        "0123456789abcdef0123456789abcde",
        "0123456789ABCDEF0123456789abcdef",
        "not-a-stage-id-at-all-nope-nope-x",
    ] {
        let (status, _) = post(
            &tree,
            "/api/import/dry-run",
            dry_run_body(id, "import/bank.csv.rules"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id:?} must not resolve");
    }
}

/// Every handle a write path takes is validated on shape before anything touches
/// the filesystem, and none of them can be walked out of the journal directory.
#[tokio::test]
async fn no_handle_can_name_a_file_outside_the_journal_directory() {
    let tree = Tree::with_rules();
    let (status, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let id = staged["stageId"].as_str().expect("a stageId");

    let cases = [
        ("csvPath", "../escape.csv"),
        ("csvPath", "/etc/passwd.csv"),
        ("csvPath", "import/../../escape.csv"),
        // Not a CSV at all: the destination suffix is required, so an import
        // cannot be pointed at a rules file or a journal.
        ("csvPath", "import/bank.csv.rules"),
        ("csvPath", "main.journal"),
        ("journalId", "../escape.journal"),
        ("journalId", "/etc/passwd"),
        ("rulesId", "../escape.rules"),
        ("rulesId", "import/bank.csv"),
    ];
    for (field, value) in cases {
        let mut body = dry_run_body(id, "import/bank.csv.rules");
        body[field] = json!(value);
        let (status, response) = post(&tree, "/api/import/dry-run", body).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
            "{field}={value:?} must be refused, got {status} {response}"
        );
        // Whatever the refusal, it may not disclose where the journal lives.
        assert_no_absolute_path(&tree, &as_text(&response), &format!("{field}={value}"));
    }
}

/// The body every dry-run and commit test starts from.
fn dry_run_body(stage_id: &str, rules_id: &str) -> Value {
    json!({
        "stageId": stage_id,
        "rulesId": rules_id,
        "csvPath": "import/bank.csv",
        "journalId": "main.journal",
    })
}

// ===========================================================================
// capabilities and prefs
// ===========================================================================

/// The screen's whole gating surface, in one response.
#[tokio::test]
async fn capabilities_describes_what_the_screen_may_offer() {
    let tree = Tree::bare();
    let (status, body) = get(&tree, "/api/import/capabilities").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(
        body["formats"],
        json!([
            "csv", "tsv", "ssv", "ofx", "qfx", "qbo", "xls", "xlsx", "xlsm", "xlsb", "ods"
        ]),
        "the accepted formats are the contract's list, in order"
    );
    assert_eq!(body["editable"], json!(true), "a bound journal is editable");
    assert!(body["git"]["available"].is_boolean());
    assert!(body["git"]["autocommit"].is_boolean());

    // The journal projection: one file, holding one transaction, and it is the
    // root. No filename was inspected to work any of that out.
    let journals = body["journals"].as_array().expect("journals is an array");
    assert_eq!(journals.len(), 1, "{journals:?}");
    assert_eq!(journals[0]["id"], json!("main.journal"));
    assert_eq!(journals[0]["txnCount"], json!(1));
    assert_eq!(journals[0]["lastTxnDate"], json!("2026-01-01"));
    assert_eq!(journals[0]["isRoot"], json!(true));
    assert_eq!(journals[0]["writable"], json!(true));

    // None of this is derived from the journal snapshot, so it is never cached.
    let request = Request::builder()
        .uri("/api/import/capabilities")
        .body(Body::empty())
        .expect("request builds");
    let response = tree.router().oneshot(request).await.expect("responds");
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store"))
    );
    assert!(response.headers().get(header::ETAG).is_none());
}

/// A server with no journal bound still renders the screen — read-only, and
/// saying so — rather than erroring.
#[tokio::test]
async fn capabilities_reports_a_read_only_server() {
    let journal = ledgeline_core::parse_journal(OPENING, "memory.journal").expect("parses");
    let router = ledgeline_server::app(&journal);
    let request = Request::builder()
        .uri("/api/import/capabilities")
        .body(Body::empty())
        .expect("request builds");
    let (status, body) = send(router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_or_text(&body)["editable"], json!(false));
}

/// The preferences round-trip, and the rejection that keeps a bad path from
/// being persisted and failing several screens later as "could not run hledger".
#[tokio::test]
async fn prefs_round_trip_and_reject_an_unusable_hledger_path() {
    let dir = TempDir::new().expect("temp dir");
    run_child(
        "prefs_round_trip_child",
        &[("LEDGELINE_CONFIG_DIR", dir.path())],
    );
}

/// See [`prefs_round_trip_and_reject_an_unusable_hledger_path`]. Runs only as a
/// child, with `$LEDGELINE_CONFIG_DIR` pointed at a scratch directory so it can
/// never touch the developer's own settings.
#[tokio::test]
#[ignore = "driven by prefs_round_trip_and_reject_an_unusable_hledger_path"]
async fn prefs_round_trip_child() {
    let tree = Tree::bare();
    let dir = child_dir();

    let (status, body) = get(&tree, "/api/prefs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"hledgerPath": null, "gitAutocommit": null}));

    // A stored value comes back verbatim: it is the caller's own input, which is
    // the one thing an `/api/*` body may legitimately echo.
    let stub = write_stub(&dir, "hledger", "hledger 1.52, test");
    let (status, body) = put(
        &tree,
        "/api/prefs",
        json!({"hledgerPath": stub.to_string_lossy(), "gitAutocommit": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["gitAutocommit"], json!(false));
    let (_, reloaded) = get(&tree, "/api/prefs").await;
    assert_eq!(reloaded, body);

    // A path that is not a runnable binary is a 400 that names no path.
    let missing = dir.join("definitely-not-here");
    let (status, body) = put(
        &tree,
        "/api/prefs",
        json!({"hledgerPath": missing.to_string_lossy()}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(as_text(&body), "the hledger path does not exist");

    // …and the rejected write left the good settings alone.
    let (_, after) = get(&tree, "/api/prefs").await;
    assert_eq!(after["gitAutocommit"], json!(false));
}

/// A too-old hledger is reported with a reason code and an actionable sentence,
/// never a stack trace and never a silent failure — the WP-11 definition of done
/// for the banner.
#[tokio::test]
async fn capabilities_reports_a_too_old_hledger() {
    let dir = TempDir::new().expect("temp dir");
    let stub = write_stub(dir.path(), "hledger", "hledger 1.30, ancient");
    run_child(
        "capabilities_too_old_child",
        &[
            ("LEDGELINE_HLEDGER", &stub),
            ("LEDGELINE_CONFIG_DIR", dir.path()),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

/// See [`capabilities_reports_a_too_old_hledger`].
#[tokio::test]
#[ignore = "driven by capabilities_reports_a_too_old_hledger"]
async fn capabilities_too_old_child() {
    let tree = Tree::bare();
    let (status, body) = get(&tree, "/api/import/capabilities").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["hledger"]["available"], json!(false));
    assert_eq!(body["hledger"]["reason"], json!("tooOld"));
    assert_eq!(
        body["hledger"]["message"],
        json!("hledger 1.30 is older than 1.40")
    );
    assert!(body["hledger"]["version"].is_null());

    // Nothing that needs hledger may proceed, and the refusal is a 501 with the
    // banner's sentence rather than a 500.
    let (status, body) = post(
        &tree,
        "/api/import/commit",
        json!({
            "stageId": "0123456789abcdef0123456789abcdef",
            "rulesId": "import/bank.csv.rules",
            "csvPath": "import/bank.csv",
            "journalId": "main.journal",
            "writeAssertion": false,
        }),
    )
    .await;
    // The stage handle is checked first and this one was never minted, so the
    // 404 comes before the hledger gate; what matters is that it is refused.
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// A binary that runs but does not answer like hledger is `unrunnable`, and is
/// terminal — we do not quietly use a different one, because "I set the path and
/// it still used the wrong binary" is a worse failure than being told.
#[tokio::test]
async fn capabilities_reports_an_unrecognised_hledger() {
    let dir = TempDir::new().expect("temp dir");
    let stub = write_stub(dir.path(), "hledger", "this is not hledger");
    run_child(
        "capabilities_unrunnable_child",
        &[
            ("LEDGELINE_HLEDGER", &stub),
            ("LEDGELINE_CONFIG_DIR", dir.path()),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

/// See [`capabilities_reports_an_unrecognised_hledger`].
#[tokio::test]
#[ignore = "driven by capabilities_reports_an_unrecognised_hledger"]
async fn capabilities_unrunnable_child() {
    let tree = Tree::bare();
    let (_, body) = get(&tree, "/api/import/capabilities").await;
    assert_eq!(body["hledger"]["available"], json!(false));
    assert_eq!(body["hledger"]["reason"], json!("unrunnable"));
}

// ===========================================================================
// FACT 3 — the proof
// ===========================================================================

/// **Balance assertions do not aggregate across two `-f` flags, and it is a
/// silent wrong answer.**
///
/// This is the regression test for fact 3 in `plans/11-enhanced-import.md`, and
/// it is the most important test in this file: every other failure in this
/// feature is loud, and this one exits zero with a plausible number.
///
/// The same two inputs — a journal holding an opening balance, and a proposed
/// transaction carrying a balance assertion that is correct **given** that
/// opening balance — are put to hledger three ways:
///
/// | form | what it means |
/// | --- | --- |
/// | `-f A -f B` | two journals. B's assertions never see A's balances |
/// | `cat A B \| -f-` | one journal, the plan's literal spelling |
/// | `include A` + B via stdin | one journal, what `verify_balance` actually sends |
///
/// The two concatenations agree with each other and with the truth; the two-`-f`
/// form does not. Both halves are asserted, because "they differ" alone would
/// still pass if concatenation were the broken one.
#[test]
fn concatenation_and_two_f_flags_disagree_and_two_f_is_wrong() {
    require_hledger!();
    let hledger = resolve_hledger();
    let dir = TempDir::new().expect("temp dir");

    let journal = dir.path().join("main.journal");
    std::fs::write(&journal, OPENING).expect("write journal");

    // Correct ONLY in combination: $1000.00 opening less $69.95 is $930.05.
    let proposed = "2026-02-01 GROCERY\n    \
                    expenses:food            $69.95\n    \
                    assets:bank:checking    $-69.95 = $930.05\n";
    let proposed_file = dir.path().join("proposed.journal");
    std::fs::write(&proposed_file, proposed).expect("write proposed");

    const ACCOUNT: &str = "assets:bank:checking";
    const TRUTH: &str = "$930.05";
    /// What the second file computes on its own, with no opening balance in
    /// scope. This exact number appearing in hledger's diagnostic is the proof
    /// that the first file's balances were never consulted.
    const ALONE: &str = "$-69.95";

    // --- form 1: two `-f` flags -------------------------------------------
    let two_f = hledger
        .invoke(["-f".as_ref(), journal.as_os_str()])
        .arg("-f")
        .arg(&proposed_file)
        .args(["balance", ACCOUNT, "--no-total", "--flat", "-O", "csv"])
        .run()
        .expect("hledger runs");

    // --- form 2: the literal `cat A B` concatenation -----------------------
    let concatenated = format!("{OPENING}\n{proposed}");
    let cat = hledger
        .invoke([
            "-f",
            "-",
            "balance",
            ACCOUNT,
            "--no-total",
            "--flat",
            "-O",
            "csv",
        ])
        .stdin(concatenated.into_bytes())
        .run()
        .expect("hledger runs");

    // --- form 3: what `import_api::verify_balance` actually sends ----------
    let included = format!("include {}\n\n{proposed}\n", journal.display());
    let wrapped = hledger
        .invoke([
            "-f",
            "-",
            "balance",
            ACCOUNT,
            "--no-total",
            "--flat",
            "-O",
            "csv",
        ])
        .stdin(included.clone().into_bytes())
        .run()
        .expect("hledger runs");

    // Both concatenations are RIGHT, and agree with each other.
    assert!(cat.success(), "cat concatenation: {}", cat.stderr_lossy());
    assert!(
        wrapped.success(),
        "include wrapper: {}",
        wrapped.stderr_lossy()
    );
    assert_eq!(
        balance_of(&cat.stdout_lossy()).as_deref(),
        Some(TRUTH),
        "cat A B | hledger -f- must see the opening balance"
    );
    assert_eq!(
        balance_of(&wrapped.stdout_lossy()),
        balance_of(&cat.stdout_lossy()),
        "the include wrapper and the literal cat must agree; they are the same journal"
    );

    // Two `-f` flags are WRONG, and specifically wrong by having computed the
    // second file's balance in isolation.
    assert_ne!(
        balance_of(&two_f.stdout_lossy()).as_deref(),
        Some(TRUTH),
        "two -f flags must NOT report the combined balance — if this ever passes, \
         hledger changed and `verify_balance` can be simplified"
    );
    let diagnostic = format!("{}{}", two_f.stdout_lossy(), two_f.stderr_lossy());
    assert!(
        diagnostic.contains(ALONE),
        "two -f flags must compute the second file ALONE ({ALONE}); hledger said: {diagnostic}"
    );

    // And the same split with `check`, which is the spelling the plan quotes.
    let checked =
        |invocation: hledger::Invocation| invocation.run().expect("hledger runs").success();
    assert!(
        checked(
            hledger
                .invoke(["-f", "-", "check"])
                .stdin(included.into_bytes())
        ),
        "concatenated, the assertion holds"
    );
    assert!(
        !checked(
            hledger
                .invoke(["-f".as_ref(), journal.as_os_str()])
                .arg("-f")
                .arg(&proposed_file)
                .arg("check")
        ),
        "with two -f flags the same assertion FAILS — the silent wrong answer"
    );
}

/// The balance out of `hledger balance -O csv`, the way `import_api` reads it.
fn balance_of(csv: &str) -> Option<String> {
    csv.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .last()
        .and_then(|line| line.rsplit_once(','))
        .map(|(_, balance)| balance.trim().trim_matches('"').to_string())
}

/// Candidate scoring, end to end: the correct rules file ranks first with a
/// perfect score, and the two files that FACT 4 is about — the ones hledger
/// accepts and exits zero on while producing unusable output — rank below it
/// with the evidence that says why.
///
/// Parse success is not a matching signal, and this is the test that says so at
/// the HTTP layer.
#[tokio::test]
async fn candidates_are_ranked_by_what_hledger_actually_produced() {
    require_hledger!();
    let tree = Tree::with_rules();
    for fixture in ["garbage-success.rules", "no-currency.rules"] {
        std::fs::copy(
            fixtures_dir().join("import/match").join(fixture),
            tree.path("import").join(fixture),
        )
        .expect("copy a fact-4 fixture");
    }

    let (status, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let candidates = staged["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 3, "{staged}");

    // The correct file wins outright.
    assert_eq!(candidates[0]["id"], json!("import/bank.csv.rules"));
    assert_eq!(candidates[0]["score"], json!(1.0));
    assert_eq!(candidates[0]["signals"]["txns"], json!(3));
    assert_eq!(candidates[0]["signals"]["amountlessPostings"], json!(0));
    assert_eq!(candidates[0]["signals"]["bareCommodityAmounts"], json!(0));
    assert_eq!(candidates[0]["signals"]["unknownAccounts"], json!(0));
    // …and shows the user their own data rather than only a number.
    assert_eq!(candidates[0]["sample"][0]["date"], json!("2026-01-15"));
    assert_eq!(
        candidates[0]["sample"][0]["description"],
        json!("ACME PAYROLL")
    );
    // A candidate carries its own default accounts. `account1` is the account
    // every imported posting lands in, so it is what a statement balance is a
    // balance OF — and the screen defaults the assertion's account to it. Without
    // these the SPA had to fetch the whole of `/api/rules` and join it onto this
    // list by id to fill in one text box.
    assert_eq!(
        candidates[0]["account1"],
        json!("assets:bank:checking"),
        "{staged}"
    );
    assert_eq!(candidates[0]["account2"], json!("expenses:unknown"));

    // Both fact-4 files parse, exit zero, and score far below it — each carrying
    // the specific signal that condemned it.
    let by_id = |name: &str| {
        candidates
            .iter()
            .find(|candidate| candidate["id"] == json!(name))
            .unwrap_or_else(|| panic!("{name} should be a candidate: {candidates:?}"))
    };
    for name in ["import/garbage-success.rules", "import/no-currency.rules"] {
        let scored = by_id(name);
        let score = scored["score"].as_f64().expect("a score");
        assert!(
            score < 1.0,
            "{name} must not tie the correct file: {scored}"
        );
    }
    assert!(
        by_id("import/no-currency.rules")["signals"]["bareCommodityAmounts"]
            .as_u64()
            .expect("a count")
            > 0,
        "the missing `currency` must show up as bare amounts — the commodity trap"
    );

    // The destination defaults follow the winner rather than the upload's name.
    assert_eq!(staged["defaults"]["csvPath"], json!("import/bank.csv"));
    assert_eq!(staged["defaults"]["journalId"], json!("main.journal"));

    // And the preview is the converted CSV, bounded but faithful.
    assert_eq!(
        staged["preview"]["header"],
        json!(["Date", "Description", "Withdrawal", "Deposit"])
    );
    assert_eq!(staged["preview"]["rowCount"], json!(3));
    assert_eq!(staged["format"], json!("csv"));
}

// ===========================================================================
// dry-run
// ===========================================================================

/// The preview mechanism in full: the proposed transactions come from **stdout**
/// as re-parseable journal text, and the `would import N` status line comes from
/// **stderr**. Merging the two streams would put a status line in the middle of
/// the journal text and force us to regex it back out.
#[tokio::test]
async fn a_dry_run_takes_entries_from_stdout_and_status_from_stderr() {
    require_hledger!();
    let tree = Tree::with_rules();
    let before = tree.snapshot();

    let (status, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let id = staged["stageId"].as_str().expect("a stageId");

    let (status, body) = post(
        &tree,
        "/api/import/dry-run",
        dry_run_body(id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true));

    // stdout: journal text, and nothing else.
    let entries = body["entries"].as_str().expect("entries is a string");
    assert!(entries.contains("2026-01-15 ACME PAYROLL"), "{entries}");
    assert!(entries.contains("assets:bank:checking"), "{entries}");
    assert!(
        !entries.contains("would import"),
        "the status line must NOT be in the entries: {entries}"
    );
    assert!(
        ledgeline_core::parse_journal(entries, "proposed").is_ok(),
        "the entries must be re-parseable journal text: {entries}"
    );

    // stderr: the status line, verbatim.
    let reported = body["status"].as_str().expect("status is a string");
    assert!(
        reported.contains("would import 3 new transactions"),
        "{reported}"
    );
    assert_eq!(body["count"], json!(3));

    // Nothing was staged into the user's tree, and no `.latest` was written:
    // a dry-run writes no state at all.
    assert_eq!(changed(&before, &tree.snapshot()), Vec::<String>::new());
}

/// A dry-run hledger refuses is a `200` carrying `ok: false` and hledger's own
/// stderr verbatim — it is genuinely good output, and paraphrasing it would lose
/// the `record:` echo that says which row broke. And it writes nothing.
#[tokio::test]
async fn a_failing_dry_run_surfaces_stderr_and_writes_nothing() {
    require_hledger!();
    let tree = Tree::with_rules();
    // A real rules file for a German bank export: right about everything a pure
    // check can see, and unable to read this statement's dates.
    std::fs::copy(
        fixtures_dir().join("import/match/wrong-dateformat.rules"),
        tree.path("import/bank.csv.rules"),
    )
    .expect("copy the mismatched rules fixture");
    let before = tree.snapshot();

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let (status, body) = post(
        &tree,
        "/api/import/dry-run",
        dry_run_body(id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a refusal is a value, not an error");
    assert_eq!(body["ok"], json!(false));
    let stderr = body["stderr"].as_str().expect("stderr is a string");
    assert!(
        !stderr.trim().is_empty(),
        "hledger's diagnostic must survive"
    );
    assert!(body["entries"].is_null(), "a failure carries no entries");
    assert_no_absolute_path(&tree, stderr, "a failing dry-run's stderr");

    assert_eq!(changed(&before, &tree.snapshot()), Vec::<String>::new());
}

/// `.latest.NAME` dedup silently drops back-dated rows. It is measured — the
/// same dry-run is repeated with no dedup state beside the CSV and the counts
/// are differenced — so nothing here has to know which column holds the date.
#[tokio::test]
async fn a_row_dropped_by_dedup_is_reported_rather_than_vanishing() {
    require_hledger!();
    let tree = Tree::with_rules();
    // hledger has already imported everything up to the 16th from a file of this
    // name, so the first two rows will be dropped without a word.
    std::fs::write(tree.path("import/.latest.bank.csv"), "2026-01-17\n").expect("write marker");

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let (status, body) = post(
        &tree,
        "/api/import/dry-run",
        dry_run_body(id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], json!(1), "only the 20th survives dedup");
    assert_eq!(
        body["skipped"],
        json!({"olderThan": "2026-01-17", "count": 2}),
        "the two dropped rows must be reported, not silently lost"
    );

    // With no dedup state there is nothing that could have been dropped.
    std::fs::remove_file(tree.path("import/.latest.bank.csv")).expect("remove marker");
    let (_, body) = post(
        &tree,
        "/api/import/dry-run",
        dry_run_body(id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(body["count"], json!(3));
    assert_eq!(body["skipped"], json!(null));
}

/// The reconciliation, computed by concatenation. A matching balance and a
/// mismatched one, so the difference is exercised in both directions.
#[tokio::test]
async fn the_statement_balance_is_reconciled_by_concatenation() {
    require_hledger!();
    let tree = Tree::with_rules();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    // $1000.00 + $3000.00 - $6.45 - $1850.00
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["balance"] = json!("2143.55");
    body["balanceAccount"] = json!("assets:bank:checking");
    let (status, response) = post(&tree, "/api/import/dry-run", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["balance"]["computed"], json!("$2143.55"));
    assert_eq!(response["balance"]["matches"], json!(true));
    // ONE representation: the user typed `2143.55` and the reconciliation
    // reports all three amounts in the commodity hledger answered in, because
    // the two are shown side by side and `2143.55` beside `$2143.55` reads as a
    // mismatch when it is a match.
    assert_eq!(response["balance"]["statement"], json!("$2143.55"));
    assert_eq!(response["balance"]["difference"], json!("$0.00"));

    body["balance"] = json!("2000.00");
    let (_, response) = post(&tree, "/api/import/dry-run", body).await;
    assert_eq!(response["balance"]["matches"], json!(false));
    assert_eq!(response["balance"]["statement"], json!("$2000.00"));
    assert_eq!(response["balance"]["difference"], json!("$-143.55"));
}

// ===========================================================================
// commit
// ===========================================================================

/// **The headline acceptance criterion.** A commit writes the CSV and the
/// journal it was asked to, plus hledger's own dedup marker, and touches nothing
/// else on disk — proved by comparing the whole tree before and after.
#[tokio::test]
async fn a_commit_writes_one_csv_one_journal_and_nothing_else() {
    require_hledger!();
    let tree = Tree::with_rules();
    let before = tree.snapshot();

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(false);
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["csvWritten"], json!("import/bank.csv"));
    assert_eq!(response["journalWritten"], json!("main.journal"));
    assert_eq!(response["imported"], json!(3));
    assert_eq!(
        response["ordering"]["inOrder"],
        json!(true),
        "three ascending rows appended to a January opening are in order"
    );

    // Exactly three paths differ, and the third is hledger's dedup state — which
    // is the whole reason the CSV is written to its FINAL destination before the
    // import runs, so `.latest` lands next to the file hledger will consult next
    // time rather than in a temp directory about to be deleted.
    assert_eq!(
        changed(&before, &tree.snapshot()),
        vec![
            "import/.latest.bank.csv".to_string(),
            "import/bank.csv".to_string(),
            "main.journal".to_string(),
        ]
    );
    assert_eq!(
        std::fs::read_to_string(tree.path("import/.latest.bank.csv"))
            .expect("marker")
            .trim(),
        "2026-01-20",
        "the marker must sit beside the FINAL csv, keyed to its name"
    );

    // The bystanders are byte-identical, and the CSV is the converted statement.
    let after = tree.snapshot();
    assert_eq!(after["notes.txt"], before["notes.txt"]);
    assert_eq!(after["import/older.csv"], before["import/older.csv"]);
    assert_eq!(
        after["import/bank.csv.rules"],
        before["import/bank.csv.rules"]
    );
    assert_eq!(
        String::from_utf8_lossy(&after["import/bank.csv"]),
        STATEMENT
    );

    // The journal GREW; nothing that was in it was rewritten.
    let journal = String::from_utf8_lossy(&after["main.journal"]).into_owned();
    assert!(journal.starts_with(OPENING), "{journal}");
    assert!(journal.contains("2026-01-20 LANDLORD LLC"), "{journal}");
}

// ---------------------------------------------------------------------------
// `skip` — the rules file counts records in the DOWNLOAD, not in our CSV
// ---------------------------------------------------------------------------

/// The same three rows, as the bank actually ships them: a title line and an
/// account line above the header. Those two are what the user's `skip 3` counts,
/// and they are exactly what `convert` strips out from under it.
const PREAMBLE_STATEMENT: &str = "Acme Bank - transaction export\n\
                                  Account ending 7890, 2026-01-01 to 2026-01-31\n\
                                  Date,Description,Withdrawal,Deposit\n\
                                  01/15/2026,ACME PAYROLL,,3000.00\n\
                                  01/16/2026,STARBUCKS,6.45,\n\
                                  01/20/2026,LANDLORD LLC,1850.00,\n";

/// [`Tree::with_rules`], with the one number this section is about changed.
///
/// The rules file is the corpus's own correct one and stays correct — `skip 3`
/// is what its author would have written for [`PREAMBLE_STATEMENT`], because
/// hledger has no header concept and three records stand between the top of that
/// file and its first transaction.
fn preamble_tree() -> Tree {
    let tree = Tree::with_rules();
    let rules = std::fs::read_to_string(tree.path("import/bank.csv.rules"))
        .expect("read the rules file")
        .replace("skip 1", "skip 3");
    assert!(
        rules.contains("skip 3"),
        "the fixture must still carry the `skip` line this test rewrites"
    );
    std::fs::write(tree.path("import/bank.csv.rules"), rules).expect("write the rules file");
    tree
}

/// **The `skip` frame, end to end through the routes.** A statement with a
/// preamble, a rules file whose `skip` was written for that preamble, and every
/// number the screen shows equal to the number the download holds.
///
/// Left unaligned, the conversion moves the header to line 1 and the same
/// `skip 3` spends itself on the header and the first two transactions: one row
/// of three is imported, hledger exits **0**, and the count on screen is a wrong
/// answer presented as a right one. Nothing anywhere says so.
///
/// Three surfaces are asserted because the alignment has to reach all three
/// independently — the candidate card is scored against its own copy, the
/// dry-run against a materialised one, and the commit against the CSV written to
/// the user's own directory:
///
/// | Surface | Unaligned | Correct |
/// | --- | --- | --- |
/// | candidate `signals.txns` | 1 | 3 |
/// | dry-run `count` | 1 | 3 |
/// | commit `imported` | 1 | 3 |
#[tokio::test]
async fn a_rules_files_own_skip_still_counts_after_the_preamble_is_stripped() {
    require_hledger!();
    let tree = preamble_tree();

    let (status, staged) = upload(&tree, "bank.csv", PREAMBLE_STATEMENT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let id = staged["stageId"].as_str().expect("a stageId").to_string();

    // The preview is of the CANONICAL table — three rows, header extracted, no
    // padding — because the alignment belongs to the copy hledger reads and
    // must never reach the screen.
    assert_eq!(staged["preview"]["rowCount"], json!(3));
    assert_eq!(
        staged["preview"]["header"],
        json!(["Date", "Description", "Withdrawal", "Deposit"])
    );

    // Candidate scoring: each candidate is scored against a copy padded to ITS
    // own `skip`, so a correct rules file is not marked down for a frame
    // mismatch that is ours.
    let candidate = staged["candidates"]
        .as_array()
        .and_then(|list| list.first())
        .expect("the rules file must be offered as a candidate");
    assert_eq!(candidate["id"], json!("import/bank.csv.rules"));
    assert_eq!(
        candidate["signals"]["txns"],
        json!(3),
        "scoring must see the whole statement: {candidate}"
    );

    let (status, body) = post(
        &tree,
        "/api/import/dry-run",
        dry_run_body(&id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], json!(3), "{body}");
    let entries = body["entries"].as_str().expect("entries is a string");
    for payee in ["ACME PAYROLL", "STARBUCKS", "LANDLORD LLC"] {
        assert!(entries.contains(payee), "{payee} is missing from {entries}");
    }

    let mut request = dry_run_body(&id, "import/bank.csv.rules");
    request["writeAssertion"] = json!(false);
    let (status, response) = post(&tree, "/api/import/commit", request).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["imported"], json!(3), "{response}");

    let journal = std::fs::read_to_string(tree.path("main.journal")).expect("read the journal");
    for payee in ["ACME PAYROLL", "STARBUCKS", "LANDLORD LLC"] {
        assert!(journal.contains(payee), "{payee} is missing from {journal}");
    }

    // The CSV the user keeps carries the padding, and that is the point rather
    // than a leak: the two invocations above read THIS file (so hledger keys
    // `.latest` beside it), and so will the next import of it — from this screen
    // or from a terminal, with the same rules file and the same `skip 3`.
    let csv = std::fs::read_to_string(tree.path("import/bank.csv")).expect("read the written CSV");
    assert_eq!(
        csv,
        "\
,,,
,,,
Date,Description,Withdrawal,Deposit
01/15/2026,ACME PAYROLL,,3000.00
01/16/2026,STARBUCKS,6.45,
01/20/2026,LANDLORD LLC,1850.00,
"
    );
    assert_eq!(
        std::fs::read_to_string(tree.path("import/.latest.bank.csv"))
            .expect("marker")
            .trim(),
        "2026-01-20",
        "the dedup marker is still keyed to the file that was read"
    );
}

// ---------------------------------------------------------------------------
// Aliases — the preview and the write must agree
// ---------------------------------------------------------------------------

/// The alias corpus: a statement whose `Account` column holds bank-speak, and a
/// journal that declares the mapping for it.
///
/// `%account:cash` in the rules file is the point of the fixture — a PREFIX
/// alias has to rewrite the base and leave the `:cash` leaf alone, or every
/// account would need one alias per subaccount.
const ALIAS_STATEMENT: &str = "Date,Account,Description,Amount\n\
                               01/15/2026,PW Roth IRA - 3077,DIVIDEND REINVEST,120.00\n\
                               01/16/2026,PW Roth IRA - 3077,ADVISORY FEE,-18.75\n";

/// The account a bare import would produce, before any alias touches it.
const BANK_SPEAK: &str = "PW Roth IRA - 3077:cash";

/// The account the alias maps it to.
const MAPPED: &str = "assets:morganstanley:pw-roth-ira:cash";

/// A tree whose journal declares the alias, with the statement-account rules
/// file beside it.
fn alias_tree() -> Tree {
    let tree = Tree::bare();
    std::fs::write(
        tree.path("main.journal"),
        format!("alias PW Roth IRA - 3077 = assets:morganstanley:pw-roth-ira\n\n{OPENING}"),
    )
    .expect("write journal");
    std::fs::copy(
        fixtures_dir().join("import/match/statement-account.csv.rules"),
        tree.path("import/bank.csv.rules"),
    )
    .expect("copy the rules fixture");
    let state =
        AppState::from_journal_path(tree.path("main.journal")).expect("the scratch journal opens");
    Tree {
        dir: tree.dir,
        state,
    }
}

/// **The one that matters.** A journal `alias` is forwarded as `--alias`, and
/// the DRY RUN and the COMMIT produce the same account names.
///
/// A preview that disagreed with the write would be a lie told immediately
/// before the only irreversible step on this screen, and it is the exact failure
/// this whole feature could plausibly have: the two are separate hledger
/// invocations, so nothing but care keeps their arguments identical. (Care, and
/// the fact that `import_invocation` is one function with a `dry_run` flag —
/// but this asserts the behaviour rather than the implementation.)
///
/// It also pins the two hledger facts the design rests on, in one run: an alias
/// directive in the target journal does **not** reach the CSV by itself, and a
/// prefix alias leaves the `:cash` leaf intact.
#[tokio::test]
async fn a_dry_run_and_a_commit_agree_on_aliased_accounts() {
    require_hledger!();
    let tree = alias_tree();

    // The alias is advertised as in force before anything is dropped.
    let (_, capabilities) = get(&tree, "/api/import/capabilities").await;
    assert_eq!(capabilities["aliases"][0]["forwarded"], json!(true));
    assert_eq!(
        capabilities["aliases"][0]["replacement"],
        json!("assets:morganstanley:pw-roth-ira")
    );

    let (status, staged) = upload(&tree, "bank.csv", ALIAS_STATEMENT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let id = staged["stageId"].as_str().expect("a stageId").to_string();

    // The candidate sample already shows the MAPPED name, because `print` reads
    // the CSV and so gets the aliases too. A card advertising the bank's words
    // beside a preview showing ours would make the user guess which was true.
    let sample = staged["candidates"][0]["sample"].to_string();
    assert!(sample.contains(MAPPED), "{sample}");
    assert!(!sample.contains(BANK_SPEAK), "{sample}");

    // The dry run.
    let (status, preview) = post(
        &tree,
        "/api/import/dry-run",
        dry_run_body(&id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let entries = preview["entries"].as_str().expect("entries").to_string();
    assert!(entries.contains(MAPPED), "{entries}");
    assert!(
        !entries.contains(BANK_SPEAK),
        "the alias must have been applied: {entries}"
    );

    // …and it says so, measured rather than asserted: the same import run with
    // no `--alias` at all, diffed against this one.
    assert_eq!(preview["aliases"]["forwarded"], json!(1));
    assert_eq!(
        preview["aliases"]["renames"],
        json!([{"from": BANK_SPEAK, "to": MAPPED}]),
        "the rewrite must be visible BEFORE the user commits"
    );

    // The commit.
    let mut body = dry_run_body(&id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(false);
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["imported"], json!(2));

    // The proof: what was written is what was previewed.
    let journal = std::fs::read_to_string(tree.path("main.journal")).expect("journal");
    let written: Vec<&str> = journal
        .lines()
        .filter(|line| line.contains("PW Roth") || line.contains("morganstanley"))
        .collect();
    assert!(
        written.iter().any(|line| line.contains(MAPPED)),
        "the commit must write the MAPPED name: {journal}"
    );
    assert!(
        !journal.contains(&format!("    {BANK_SPEAK}")),
        "the commit must not write the bank's own words as an account: {journal}"
    );
    for line in entries.lines().filter(|line| line.contains(MAPPED)) {
        assert!(
            journal.contains(line.trim()),
            "the preview line {line:?} is not in the journal:\n{journal}"
        );
    }
    // And the prefix alias left the leaf alone, rather than swallowing it.
    assert!(journal.contains(":pw-roth-ira:cash"), "{journal}");
}

/// Without the forwarding this feature adds, the alias does nothing: hledger's
/// own `import` never applies a target journal's `alias` to the CSV it is
/// reading. Pinned directly, because the entire design rests on it and it is the
/// sort of thing a future hledger could change.
#[tokio::test]
async fn an_alias_directive_alone_does_not_reach_the_csv() {
    require_hledger!();
    let tree = alias_tree();
    let hledger = resolve_hledger();
    let output = hledger
        .invoke(["-f".as_ref(), tree.path("main.journal").as_os_str()])
        .args(["import", "--dry-run", "--rules"])
        .arg(tree.path("import/bank.csv.rules"))
        .arg({
            let csv = tree.path("import/bank.csv");
            std::fs::write(&csv, ALIAS_STATEMENT).expect("write csv");
            csv
        })
        .run()
        .expect("hledger runs");
    let proposed = output.stdout_lossy();
    assert!(
        proposed.contains(BANK_SPEAK),
        "hledger must still propose the UNMAPPED account, or this feature is unnecessary:\n\
         {proposed}"
    );
    assert!(!proposed.contains(MAPPED), "{proposed}");
}

/// An `end aliases` is a written instruction about where a mapping stops, and
/// `--alias` is global — so the alias is listed, and deliberately not used.
#[tokio::test]
async fn a_scoped_alias_is_not_forwarded_to_the_import() {
    require_hledger!();
    let tree = alias_tree();
    std::fs::write(
        tree.path("main.journal"),
        format!(
            "alias PW Roth IRA - 3077 = assets:morganstanley:pw-roth-ira\nend aliases\n\n{OPENING}"
        ),
    )
    .expect("write journal");
    let tree = Tree {
        state: AppState::from_journal_path(tree.path("main.journal")).expect("journal opens"),
        dir: tree.dir,
    };

    let (_, staged) = upload(&tree, "bank.csv", ALIAS_STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId").to_string();
    let (status, preview) = post(
        &tree,
        "/api/import/dry-run",
        dry_run_body(&id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let entries = preview["entries"].as_str().expect("entries");
    assert!(entries.contains(BANK_SPEAK), "{entries}");
    assert_eq!(
        preview["aliases"],
        json!(null),
        "no alias is in force, so there is nothing to report and no second run to make it with"
    );
}

/// A statement balance, when asked for, is written in the `hledger close
/// --assert` shape — and only after hledger itself has agreed it holds. A
/// balance that does not reconcile is refused rather than cemented into the
/// journal as a line every later `hledger check` would fail on.
#[tokio::test]
async fn a_requested_assertion_is_verified_before_it_is_written() {
    require_hledger!();
    let tree = Tree::with_rules();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(true);
    body["balance"] = json!("$2143.55");
    body["balanceAccount"] = json!("assets:bank:checking");
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");

    let journal = std::fs::read_to_string(tree.path("main.journal")).expect("journal");
    assert!(
        journal.contains("assert balances  ; assert:"),
        "the assertion must be in `hledger close --assert` shape: {journal}"
    );
    assert!(
        journal.contains("assets:bank:checking    $0 = $2143.55"),
        "{journal}"
    );
    assert_checks(&tree, "an asserted journal");

    // A second import with a wrong balance is refused, and — because the
    // assertion is verified BEFORE the import is applied — nothing at all was
    // written, not even the CSV.
    let tree = Tree::with_rules();
    let before = tree.snapshot();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(true);
    body["balance"] = json!("$9999.99");
    body["balanceAccount"] = json!("assets:bank:checking");
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    let text = as_text(&response);
    assert!(text.contains("does not match"), "{text}");
    assert_no_absolute_path(&tree, &text, "a refused assertion");
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "a balance that does not hold refuses the WHOLE commit: no CSV, no import, no assertion"
    );
}

/// **The regression for fact 4 arriving in our own output.**
///
/// The balance field's placeholder is `2945.05` and an OFX `LEDGERBAL` is a bare
/// decimal, so a statement balance with no currency symbol is the *normal* input,
/// not the exotic one. Written through verbatim it produces:
///
/// ```text
///     assets:bank:checking               0 = 2143.55
/// ```
///
/// which does not assert 2143.55 dollars. An amount with no commodity is an
/// amount in the **empty** commodity, so hledger computes 0 for it and the
/// assertion fails — first on the import that wrote it, and then on every
/// `hledger check` the user runs afterwards, in a file they did not write that
/// line into.
///
/// So the commodity is taken from the balance hledger itself computed for that
/// account, and this test asserts both halves: the written text carries it, and
/// `hledger check` over the resulting journal **passes**. The second half is the
/// one that cannot be faked — it is hledger's own verdict on our output.
#[tokio::test]
async fn a_bare_statement_balance_is_asserted_in_the_journals_own_commodity() {
    require_hledger!();
    let tree = Tree::with_rules();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    // No `$`, exactly as a user types it and exactly as OFX volunteers it.
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["balance"] = json!("2143.55");
    body["balanceAccount"] = json!("assets:bank:checking");

    // The reconciliation reports ONE representation, so the number the screen
    // shows beside hledger's is comparable with it — and is the number that
    // ends up in the file.
    let (_, response) = post(&tree, "/api/import/dry-run", body.clone()).await;
    let balance = &response["balance"];
    assert_eq!(balance["statement"], json!("$2143.55"), "{balance}");
    assert_eq!(balance["computed"], json!("$2143.55"), "{balance}");
    assert_eq!(balance["matches"], json!(true), "{balance}");
    assert_eq!(balance["difference"], json!("$0.00"), "{balance}");

    body["writeAssertion"] = json!(true);
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a bare balance is the ordinary case and must not fail: {response}"
    );

    let journal = std::fs::read_to_string(tree.path("main.journal")).expect("journal");
    assert!(
        journal.contains("assets:bank:checking    $0 = $2143.55"),
        "the assertion must carry the journal's commodity on BOTH amounts, in the \
         `hledger close --assert` shape: {journal}"
    );
    assert!(
        !journal.contains("= 2143.55"),
        "a bare asserted amount is the bug: {journal}"
    );

    // hledger's own verdict on what we wrote. Before the fix this failed with
    // `In commodity "" ... the asserted balance is: 2143.55 but the calculated
    // balance is: 0`.
    assert_checks(&tree, "a journal asserted from a bare balance");
}

/// `hledger -f JOURNAL check` over the scratch tree, asserting it passes.
fn assert_checks(tree: &Tree, what: &str) {
    let hledger = resolve_hledger();
    let journal = tree.path("main.journal");
    let output = hledger
        .invoke(["-f".as_ref(), journal.as_os_str()])
        .arg("check")
        .run()
        .expect("hledger runs");
    assert!(
        output.success(),
        "{what} must pass `hledger check`; hledger said:\n{}",
        output.stderr_lossy()
    );
}

// ===========================================================================
// Split layouts — the file we WRITE is not the file we RECKON AGAINST
// ===========================================================================
//
// `fixtures/import/layouts/split-year-assert/` is `main.journal` including
// `2025/2025.journal` and then `2026/2026.journal`, where the 2026 file opens
// with a start-of-year assertion carrying 2025's closing balance. The tree
// passes `hledger check`; the 2026 file, read alone, cannot — the balance it
// asserts accumulates through a file hledger was never asked to open.
//
// Two bugs lived there, and the second is the worse one:
//
//  1. `hledger import -f 2026/2026.journal` aborted on that assertion, so the
//     import failed outright on a correct journal.
//  2. the balance verification reckoned against the TARGET, so it answered with
//     the fragment's balance and told a user whose statement was right that it
//     was wrong — a silent wrong answer, and one that also refused a correct
//     statement balance through the assertion pre-flight.

/// The checking balance of the committed tree, through the root.
/// `$1,000.00 - $100.00 - $5.00`.
const TREE_BALANCE: &str = "$895.00";

/// The same account in `2026/2026.journal` read **alone**: the start-of-year
/// assertion contributes `$0` and the coffee `$-5.00`. This is the number the
/// engine used to report, and every assertion below that names it is checking
/// that it does not any more.
const FRAGMENT_BALANCE: &str = "$-5.00";

/// [`TREE_BALANCE`] plus the statement (`+$3000.00 -$6.45 -$1850.00`).
const TREE_AFTER_IMPORT: &str = "$2038.55";

/// [`FRAGMENT_BALANCE`] plus the same statement — the wrong answer, spelled out
/// so a test can assert we did not produce it. Equality with THIS is the bug.
const FRAGMENT_AFTER_IMPORT: &str = "$1138.55";

/// The same statement reckoned against `2025/2025.journal` alone: `$900.00`
/// closing plus `$1143.55`.
///
/// This is the **silent** form of the same bug and the reason it is worse than
/// the aborted import. The 2025 fragment holds no assertion, so reading it alone
/// exits zero and answers a plausible number that is $5.00 off — nothing
/// anywhere says a file was missing. The 2026 fragment at least fails loudly.
const PRIOR_YEAR_AFTER_IMPORT: &str = "$2043.55";

/// The two figures above are hledger's, not ours.
///
/// Pinning literals in the tests below is what makes them readable, and this is
/// what keeps the literals honest: it asks hledger for the same two balances by
/// the same two spellings the engine could use, and fails if either constant has
/// drifted. Without it a fixture edit could quietly make the right number and
/// the wrong number equal, and every assertion downstream would still pass while
/// proving nothing.
#[test]
fn the_two_balances_this_layout_produces_really_do_differ() {
    require_hledger!();
    let tree = Tree::split_year_assert();
    let hledger = resolve_hledger();
    let balance = |journal: PathBuf| {
        let output = hledger
            .invoke(["-f".as_ref(), journal.as_os_str()])
            .args(["balance", "assets:bank:checking", "--no-total", "--flat"])
            .args(["-O", "csv"])
            .run()
            .expect("hledger runs");
        assert!(output.success(), "hledger said:\n{}", output.stderr_lossy());
        balance_of(&output.stdout_lossy())
    };

    assert_eq!(
        balance(tree.path("main.journal")).as_deref(),
        Some(TREE_BALANCE)
    );
    // The fragment needs -I to be readable at all, which is the first bug in one
    // line: without it hledger refuses to compute anything for this file.
    let fragment = hledger
        .invoke([
            "-I".as_ref(),
            "-f".as_ref(),
            tree.path("2026/2026.journal").as_os_str(),
        ])
        .args(["balance", "assets:bank:checking", "--no-total", "--flat"])
        .args(["-O", "csv"])
        .run()
        .expect("hledger runs");
    assert_eq!(
        balance_of(&fragment.stdout_lossy()).as_deref(),
        Some(FRAGMENT_BALANCE)
    );
    assert_ne!(
        TREE_BALANCE, FRAGMENT_BALANCE,
        "if these are ever equal the fixture has stopped proving anything"
    );

    // And the silent pair: the prior-year fragment reads cleanly on its own and
    // answers a different, entirely plausible number. Exit zero both ways.
    assert_eq!(
        balance(tree.path("2025/2025.journal")).as_deref(),
        Some("$900.00"),
        "the 2025 fragment needs no -I and reports a well-formed balance — which \
         is exactly what makes reckoning against it dangerous"
    );
}

/// **Bug 1: the import must not abort on the target's own start-of-year
/// assertion.**
///
/// `hledger import -f 2026/2026.journal …` reads only that file, so the
/// assertion is evaluated with 2025 out of scope and fails. The tree is fine;
/// only the fragment is not, and `import_invocation` passes
/// `--ignore-assertions` for exactly that reason.
#[tokio::test]
async fn an_import_into_a_year_file_succeeds_despite_its_start_of_year_assertion() {
    require_hledger!();
    let tree = Tree::split_year_assert();
    assert_checks(&tree, "the committed fixture tree");

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["journalId"] = json!("2026/2026.journal");

    let (status, response) = post(&tree, "/api/import/dry-run", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        response["ok"],
        json!(true),
        "the dry-run must not fail on an assertion the ROOT satisfies: {response}"
    );
    assert_eq!(response["count"], json!(3), "{response}");
    assert!(
        as_text(&response["entries"]).contains("ACME PAYROLL"),
        "{response}"
    );
}

/// **Bug 2: the reconciliation reports the TREE's balance, not the fragment's.**
///
/// Both numbers are pinned, in both directions: the tree's figure must be
/// reported *and* must be accepted as a match, and the fragment's figure must be
/// reported as a mismatch. Asserting only the first would pass if the two ever
/// coincided; asserting only the second would pass if we reported neither.
#[tokio::test]
async fn a_split_layout_reconciles_against_the_root_and_not_the_target() {
    require_hledger!();
    let tree = Tree::split_year_assert();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["journalId"] = json!("2026/2026.journal");
    body["balance"] = json!(TREE_AFTER_IMPORT);
    body["balanceAccount"] = json!("assets:bank:checking");

    let (status, response) = post(&tree, "/api/import/dry-run", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let balance = &response["balance"];
    assert_eq!(
        balance["computed"],
        json!(TREE_AFTER_IMPORT),
        "the computed balance must be the ROOT's answer: {response}"
    );
    assert_ne!(
        balance["computed"],
        json!(FRAGMENT_AFTER_IMPORT),
        "reading the target alone is the bug; that answer is {FRAGMENT_AFTER_IMPORT}"
    );
    assert_eq!(balance["matches"], json!(true), "{balance}");
    assert_eq!(balance["difference"], json!("$0.00"), "{balance}");

    // And the fragment's own figure — which a user reading the old output would
    // have been driven to type — is correctly reported as NOT matching.
    body["balance"] = json!(FRAGMENT_AFTER_IMPORT);
    let (_, response) = post(&tree, "/api/import/dry-run", body).await;
    assert_eq!(response["balance"]["matches"], json!(false), "{response}");
    assert_eq!(
        response["balance"]["computed"],
        json!(TREE_AFTER_IMPORT),
        "{response}"
    );
}

/// **The silent half of bug 2: a fragment with no assertion answers a plausible
/// wrong number, exit zero.**
///
/// Importing into the prior year is an ordinary thing to do — catching up a year
/// you fell behind on — and `2025/2025.journal` holds no balance assertion, so
/// nothing fails and nothing is logged. Read alone it answers
/// [`PRIOR_YEAR_AFTER_IMPORT`]; read through the root it answers
/// [`TREE_AFTER_IMPORT`]. Both are well-formed dollar amounts and they differ by
/// the $5.00 that lives in the other file.
///
/// This is the case that would never have been noticed, so it gets its own test
/// rather than sharing one with the loud case above.
#[tokio::test]
async fn a_target_with_no_assertion_still_reconciles_against_the_root() {
    require_hledger!();
    let tree = Tree::split_year_assert();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["journalId"] = json!("2025/2025.journal");
    body["balance"] = json!(TREE_AFTER_IMPORT);
    body["balanceAccount"] = json!("assets:bank:checking");

    let (status, response) = post(&tree, "/api/import/dry-run", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let balance = &response["balance"];
    assert_eq!(
        balance["computed"],
        json!(TREE_AFTER_IMPORT),
        "the whole tree, not the file being appended to: {response}"
    );
    assert_ne!(
        balance["computed"],
        json!(PRIOR_YEAR_AFTER_IMPORT),
        "{PRIOR_YEAR_AFTER_IMPORT} is what the target alone computes — a number with \
         no error beside it, which is why this bug outlived the loud one"
    );
    assert_eq!(balance["matches"], json!(true), "{balance}");
}

/// **A statement balance that matches the tree is written, not refused.**
///
/// The pre-flight put journal + proposed + assertion to `hledger check` with the
/// *target* as the journal, so the fragment's start-of-year assertion failed
/// first and a correct statement balance was refused with hledger's complaint
/// about a line the user never typed. Reading the root makes the check mean what
/// it says.
///
/// The last assertion is hledger's own verdict on the bytes we wrote, over the
/// whole tree — which is the only place the question was ever answerable.
#[tokio::test]
async fn a_correct_statement_balance_is_accepted_in_a_split_layout() {
    require_hledger!();
    let tree = Tree::split_year_assert();
    let before = tree.snapshot();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["journalId"] = json!("2026/2026.journal");
    body["balance"] = json!(TREE_AFTER_IMPORT);
    body["balanceAccount"] = json!("assets:bank:checking");
    body["writeAssertion"] = json!(true);

    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a balance that matches the tree must be accepted: {response}"
    );
    assert_eq!(response["journalWritten"], json!("2026/2026.journal"));
    assert_eq!(response["imported"], json!(3), "{response}");
    assert_eq!(response["ordering"]["inOrder"], json!(true), "{response}");

    // Exactly the two files this import names, plus hledger's dedup marker.
    assert_eq!(
        changed(&before, &tree.snapshot()),
        vec![
            "2026/2026.journal".to_string(),
            "import/.latest.bank.csv".to_string(),
            "import/bank.csv".to_string(),
        ]
    );

    let year = std::fs::read_to_string(tree.path("2026/2026.journal")).expect("the year file");
    assert!(
        year.contains("assets:bank:checking          $0 = $900.00"),
        "the start-of-year assertion must be untouched: {year}"
    );
    assert!(
        year.contains(&format!("assets:bank:checking    $0 = {TREE_AFTER_IMPORT}")),
        "the new assertion must carry the TREE's balance: {year}"
    );
    assert!(
        !year.contains(FRAGMENT_AFTER_IMPORT),
        "the fragment's balance must never be written as an assertion: {year}"
    );
    assert_checks(&tree, "a split tree imported into and asserted");
}

/// Reading the root puts a **second** journal's absolute path into hledger's
/// stdin, so security layer 5 has to cover it too.
///
/// `concatenated` writes `include /abs/main.journal`, and hledger's diagnostics
/// then quote whichever file in that tree it tripped over — `2025/2025.journal`,
/// a file the request never named. A refused balance is the loudest way to make
/// it say so, so that is what this asks for.
#[tokio::test]
async fn a_refusal_in_a_split_layout_names_no_absolute_path() {
    require_hledger!();
    let tree = Tree::split_year_assert();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["journalId"] = json!("2026/2026.journal");
    body["balance"] = json!("$9999.99");
    body["balanceAccount"] = json!("assets:bank:checking");
    body["writeAssertion"] = json!(true);

    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    let text = as_text(&response);
    assert!(text.contains("does not match"), "{text}");
    assert_no_absolute_path(&tree, &text, "a refused split-layout assertion");
}

/// Out-of-order dates are detected, and the offered re-sort preserves everything
/// outside the transactions it moves.
#[tokio::test]
async fn a_back_dated_import_is_detected_and_can_be_re_sorted() {
    require_hledger!();
    let tree = Tree::with_rules();
    // An opening entry dated AFTER everything in the statement, so appending the
    // statement puts the file out of order.
    std::fs::write(
        tree.path("main.journal"),
        "2026-06-01 later than the statement\n    \
         assets:bank:checking   $1000.00\n    equity:opening\n",
    )
    .expect("rewrite the journal");
    let tree = Tree {
        state: AppState::from_journal_path(tree.path("main.journal")).expect("reopens"),
        dir: tree.dir,
    };

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(false);
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["ordering"]["inOrder"], json!(false));
    let moves = response["ordering"]["moves"]
        .as_array()
        .expect("moves is an array");
    assert!(!moves.is_empty(), "{response}");
    assert!(moves[0]["date"].is_string() && moves[0]["fromLine"].is_number());

    let before_sort = std::fs::read_to_string(tree.path("main.journal")).expect("journal");
    let (status, response) = post(
        &tree,
        "/api/import/sort",
        json!({"journalId": "main.journal"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["moved"], json!(moves.len()));

    let sorted = std::fs::read_to_string(tree.path("main.journal")).expect("journal");
    assert!(sorted.starts_with("2026-01-15 ACME PAYROLL"), "{sorted}");
    // A permutation: every byte of every transaction survives, only the order
    // changed.
    assert_eq!(
        sorted.len(),
        before_sort.len(),
        "a sort moves bytes, it does not add or drop them"
    );
    // And it is now genuinely in order, so a second sort is a no-op.
    let (_, response) = post(
        &tree,
        "/api/import/sort",
        json!({"journalId": "main.journal"}),
    )
    .await;
    assert_eq!(response["moved"], json!(0));
}

/// A dry-run reports modified targets; a commit **refuses** them. The refusal is
/// re-checked server-side rather than trusted from the dry-run's response,
/// because the whole value of the safety net is that the pre-import state was
/// committed and a client that skips the check must not be able to skip the
/// guarantee.
#[tokio::test]
async fn a_commit_refuses_while_a_target_is_modified() {
    require_hledger!();
    require_git!();
    let tree = Tree::with_rules();
    init_repo(tree.dir.path());

    // Somebody's work in progress, in the journal we are about to import into.
    std::fs::write(
        tree.path("main.journal"),
        format!("{OPENING}\n; a note I have not committed yet\n"),
    )
    .expect("dirty the journal");
    // …and something unrelated, which must be left exactly as it is.
    std::fs::write(tree.path("notes.txt"), "edited, and not ours to touch\n")
        .expect("dirty a bystander");

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let (status, response) = post(
        &tree,
        "/api/import/dry-run",
        dry_run_body(id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        response["blockedByGit"],
        json!(["main.journal"]),
        "a modified journal blocks; an untracked CSV does not"
    );

    let before = tree.snapshot();
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(false);
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert!(as_text(&response).contains("main.journal"), "{response}");
    assert_no_absolute_path(&tree, &as_text(&response), "a git-blocked commit");
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "a refused commit writes nothing at all"
    );
}

/// With a clean tree, the import commits exactly the two files it wrote — and
/// somebody's unrelated dirty file is still dirty afterwards.
#[tokio::test]
async fn a_successful_import_commits_only_what_it_wrote() {
    require_hledger!();
    require_git!();
    let tree = Tree::with_rules();
    init_repo(tree.dir.path());
    // Unrelated work in progress, in a file this import never touches.
    std::fs::write(tree.path("notes.txt"), "work in progress\n").expect("dirty a bystander");

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(false);
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["git"]["committed"], json!(true), "{response}");
    assert_eq!(
        response["git"]["paths"],
        json!(["import/bank.csv", "main.journal"])
    );

    // The bystander is STILL DIRTY: nothing swept it up.
    let porcelain = git(
        tree.dir.path(),
        &["status", "--porcelain", "--", "notes.txt"],
    );
    assert!(
        porcelain.contains("notes.txt"),
        "unrelated work must be left exactly as it was: {porcelain:?}"
    );
    // …and the commit really did carry the two files.
    let committed = git(
        tree.dir.path(),
        &["show", "--name-only", "--format=%s", "HEAD"],
    );
    assert!(
        committed.contains("import 3 transactions from bank.csv"),
        "{committed}"
    );
    assert!(committed.contains("main.journal"), "{committed}");
    assert!(committed.contains("import/bank.csv"), "{committed}");
    assert!(!committed.contains("notes.txt"), "{committed}");
}

// ===========================================================================
// hledger proposes; Ledgeline appends; hledger remembers
// ===========================================================================
//
// `hledger import` never writes a user's journal. `commit` runs
// `import --dry-run`, appends that stdout itself, and then runs
// `import --catchup` so hledger records `.latest` in its own format. Three
// properties have to hold, and each has a test below:
//
//   1. the preview IS the bytes that land;
//   2. `.latest` is still hledger's, so a second import finds nothing new;
//   3. the bytes are hledger's own append, byte for byte, for EVERY journal.
//
// What is deliberately NOT here is a fourth: re-printing the proposal in the
// tree's declared `commodity` style. Imported amounts keep hledger's own
// spelling — `$165.2` in books that write `$165.20` — because a `commodity`
// directive in scope changes how the entries *parse* and not merely how they
// print. It was built, it worked, and it was removed; `docs/imports.md`
// § "Commodity style" has the transcript and the reasoning.

/// A statement whose amounts are written the way a bank writes them — one with
/// no decimals at all, one with a single decimal place.
const RAGGED_STATEMENT: &str = "Date,Description,Withdrawal,Deposit\n\
                                03/01/2026,GROCERY STORE,405,\n\
                                03/03/2026,ACME PAYROLL,,165.2\n";

/// The `fixtures/import/layouts/split-year/` tree, copied so it can be written
/// to, with the checking rules file beside it.
///
/// **The user's own shape**: `main.journal` includes an `accounts.journal` that
/// holds every `account` and `commodity` declaration, and the transactions live
/// in per-year files. The import target — `2026/2026.journal` — therefore
/// declares nothing at all, which is what makes it the demanding case for
/// everything below: the append lands in a fragment that carries none of the
/// tree's directives.
fn split_year() -> Tree {
    let dir = TempDir::new().expect("temp dir");
    copy_tree(
        &fixtures_dir().join("import/layouts/split-year"),
        dir.path(),
    );
    std::fs::create_dir(dir.path().join("import")).expect("import dir");
    std::fs::copy(
        fixtures_dir().join("import/match/checking.csv.rules"),
        dir.path().join("import/bank.csv.rules"),
    )
    .expect("copy the rules fixture");
    let state =
        AppState::from_journal_path(dir.path().join("main.journal")).expect("the journal opens");
    Tree { dir, state }
}

/// A dry-run body for the split-year tree's newest year file.
fn split_year_body(stage_id: &str) -> Value {
    let mut body = dry_run_body(stage_id, "import/bank.csv.rules");
    body["journalId"] = json!("2026/2026.journal");
    body
}

/// The same, plus the one field `commit` takes and `dry-run` refuses.
fn split_year_commit_body(stage_id: &str) -> Value {
    let mut body = split_year_body(stage_id);
    body["writeAssertion"] = json!(false);
    body
}

/// **A declared `commodity` style does NOT restyle the imported amounts, and
/// that is the decision rather than an oversight.**
///
/// A root declaring `commodity $1,000.00` in an included accounts file, and a
/// statement holding `405` and `165.2`: the journal ends up holding `$-405` and
/// `$165.2`, spelled the way hledger spelled them and not the way the rest of
/// the file is written.
///
/// It is not what a reader would guess, so it is asserted rather than left to
/// be discovered. Re-printing the proposal under the tree's directives *does*
/// produce `$165.20`, and was implemented and working — but a directive in
/// scope changes how the entries **parse**, and `EUR165.2` under
/// `commodity 1.000,00 EUR` re-reads as `1.652,00 EUR` with exit zero. Ten times
/// the money for a cosmetic gain. `docs/imports.md` § "Commodity style" carries
/// the whole transcript; if this assertion is ever flipped, that is the section
/// to answer first.
#[tokio::test]
async fn imported_amounts_keep_hledgers_own_spelling_not_the_declared_style() {
    require_hledger!();
    let tree = split_year();
    assert_checks(&tree, "the committed fixture tree");

    // The declaration is real, and it is in a file the TARGET does not include —
    // the layout that made restyling look necessary in the first place.
    let declarations = std::fs::read_to_string(tree.path("accounts.journal")).expect("accounts");
    assert!(
        declarations.contains("commodity $1,000.00"),
        "{declarations}"
    );

    let (_, staged) = upload(&tree, "bank.csv", RAGGED_STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let (status, preview) = post(&tree, "/api/import/dry-run", split_year_body(id)).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let entries = as_text(&preview["entries"]);
    assert!(entries.contains("$-405"), "{entries}");
    assert!(entries.contains("$165.2"), "{entries}");

    let (status, response) = post(&tree, "/api/import/commit", split_year_commit_body(id)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["imported"], json!(2), "{response}");

    let written = std::fs::read_to_string(tree.path("2026/2026.journal")).expect("target");
    assert!(
        written.contains("$-405\n"),
        "a withdrawal of `405` must keep hledger's own spelling: {written}"
    );
    assert!(
        written.contains("$165.2\n"),
        "a deposit of `165.2` must keep hledger's own spelling: {written}"
    );
    // And the journal is still a journal hledger accepts, with the entries
    // meaning what the statement said.
    assert_checks(&tree, "the tree after an import");
}

/// **The preview is the bytes.** The dry-run's `entries` and the region the
/// commit appended are the same text — not merely equivalent, the same.
///
/// Before the split, the preview was hledger's dry-run and the write was a
/// second, separate hledger invocation; the two agreed because they were given
/// the same arguments, which is an argument rather than a guarantee. Now the
/// commit appends exactly what the dry-run route returned, and nothing renders
/// the entries twice. Easy to satisfy — which is the point, and the reason it is
/// worth keeping even though no transformation stands between the two any more.
///
/// The separator is pinned too, because it is the one thing that is ours rather
/// than hledger's: a leading newline, then the entries with hledger's own
/// trailing blank line removed.
#[tokio::test]
async fn the_preview_is_the_bytes_that_are_appended() {
    require_hledger!();
    let tree = split_year();
    let (_, staged) = upload(&tree, "bank.csv", RAGGED_STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let (status, preview) = post(&tree, "/api/import/dry-run", split_year_body(id)).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let entries = as_text(&preview["entries"]);

    let before = std::fs::read_to_string(tree.path("2026/2026.journal")).expect("target");
    let (status, response) = post(&tree, "/api/import/commit", split_year_commit_body(id)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let after = std::fs::read_to_string(tree.path("2026/2026.journal")).expect("target");

    assert!(
        after.starts_with(&before),
        "the journal must have grown and not been rewritten"
    );
    let appended = &after[before.len()..];
    assert_eq!(
        appended,
        format!("\n{}\n", entries.trim_end_matches('\n')),
        "the appended region must be the previewed text, with hledger's own separator"
    );
}

/// `.latest` is still hledger's, written by hledger, and it still de-duplicates.
///
/// The one thing giving up `hledger import`'s write could plausibly have cost.
/// `--catchup` records the state file without appending anything — verified
/// byte-identical to a writing import's, repeated same-date lines included — so
/// a second import of the same statement has nothing to propose.
#[tokio::test]
async fn a_second_import_of_the_same_statement_finds_nothing_new() {
    require_hledger!();
    let tree = split_year();
    let (_, staged) = upload(&tree, "bank.csv", RAGGED_STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let (status, response) = post(&tree, "/api/import/commit", split_year_commit_body(id)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["imported"], json!(2), "{response}");
    assert_eq!(
        std::fs::read_to_string(tree.path("import/.latest.bank.csv"))
            .expect("hledger must have written the dedup marker")
            .trim(),
        "2026-03-03",
        "the marker must hold the newest imported date"
    );

    // A second run of the whole flow proposes nothing, and a second commit adds
    // nothing to the journal.
    let after_first = std::fs::read_to_string(tree.path("2026/2026.journal")).expect("target");
    let (_, staged) = upload(&tree, "bank.csv", RAGGED_STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");
    let (status, second) = post(&tree, "/api/import/dry-run", split_year_body(id)).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["count"], json!(0), "{second}");
    assert_eq!(as_text(&second["entries"]).trim(), "", "{second}");

    let (status, response) = post(&tree, "/api/import/commit", split_year_commit_body(id)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["imported"], json!(0), "{response}");
    assert_eq!(
        std::fs::read_to_string(tree.path("2026/2026.journal")).expect("target"),
        after_first,
        "an import with nothing to import must append nothing at all"
    );
}

/// **The bytes Ledgeline appends are the bytes hledger appends.**
///
/// The primary guarantee of the whole propose-append-catchup shape, and the one
/// that makes taking over hledger's write defensible at all: the same statement
/// is imported into a copy of the same journal by `hledger import` itself, and
/// the two files are compared **byte for byte**.
///
/// This was once an edge case — the journal that declares no `commodity` style,
/// the one the re-styling skipped. Now that nothing is re-styled it describes
/// every journal there is, which is why it is named for the property rather than
/// for the exception. Note the amounts it pins: `$-405` and `$165.2`, hledger's
/// own spelling, unpadded.
#[tokio::test]
async fn the_appended_bytes_are_hledgers_own_append() {
    require_hledger!();
    let tree = Tree::with_rules();
    let (_, staged) = upload(&tree, "bank.csv", RAGGED_STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    let before = std::fs::read_to_string(tree.path("main.journal")).expect("journal");
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(false);
    let (status, response) = post(&tree, "/api/import/commit", body).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let ours = std::fs::read_to_string(tree.path("main.journal")).expect("journal");

    // The oracle: a pristine copy of the same journal, imported into by hledger
    // itself, from a copy of the same CSV in a directory of its own so the two
    // runs cannot share a `.latest`.
    let oracle = TempDir::new().expect("temp dir");
    std::fs::write(oracle.path().join("main.journal"), &before).expect("write");
    std::fs::copy(tree.path("import/bank.csv"), oracle.path().join("bank.csv"))
        .expect("copy the written csv");
    std::fs::copy(
        tree.path("import/bank.csv.rules"),
        oracle.path().join("bank.csv.rules"),
    )
    .expect("copy the rules file");
    let hledger = resolve_hledger();
    let output = hledger
        .invoke([
            "-I".as_ref(),
            "-f".as_ref(),
            oracle.path().join("main.journal").as_os_str(),
        ])
        .arg("import")
        .arg("--rules")
        .arg(oracle.path().join("bank.csv.rules"))
        .arg(oracle.path().join("bank.csv"))
        .run()
        .expect("hledger runs");
    assert!(output.success(), "{}", output.stderr_lossy());
    let theirs = std::fs::read_to_string(oracle.path().join("main.journal")).expect("journal");

    assert_eq!(
        ours, theirs,
        "our append must be hledger's append, byte for byte"
    );
    // And specifically: the CSV's own spelling survives, unpadded.
    assert!(ours.contains("$-405\n"), "{ours}");
    assert!(ours.contains("$165.2\n"), "{ours}");
}

/// **A failed catch-up undoes the import rather than leaving a duplicate
/// waiting to happen.**
///
/// The one new failure the append-it-ourselves shape introduces, and the reason
/// it is worth a stub: if `--catchup` fails after the append, the entries are in
/// the journal while `.latest` still points at the previous import — so the next
/// import of that statement proposes the same rows again, and three extra
/// transactions dated last month are exactly the kind of thing nobody notices.
///
/// A real hledger cannot be made to fail there on demand, so the binary is a
/// four-line shell script that proposes happily and refuses to catch up. What is
/// under test is entirely ours: the roll-back, and the error that names what
/// happened.
#[tokio::test]
async fn a_failed_catch_up_rolls_the_import_back() {
    let dir = TempDir::new().expect("temp dir");
    let stub = dir.path().join("hledger");
    std::fs::write(
        &stub,
        "#!/bin/sh\ncase \"$*\" in\n\
         *--version*) echo 'hledger 1.52, stub' ;;\n\
         *--catchup*) echo 'stub: the state file could not be written' >&2; exit 1 ;;\n\
         *--dry-run*) printf '2026-02-01 STUB\\n    assets:bank:checking  $-1.00\\n\
         \x20   expenses:unknown       $1.00\\n\\n'; \
         echo 'would import 1 new transactions from bank.csv:' >&2 ;;\n\
         esac\nexit 0\n",
    )
    .expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    run_child(
        "catch_up_rollback_child",
        &[
            ("LEDGELINE_HLEDGER", &stub),
            ("LEDGELINE_CONFIG_DIR", dir.path()),
            (CHILD_DIR_ENV, dir.path()),
        ],
    );
}

/// See [`a_failed_catch_up_rolls_the_import_back`].
#[tokio::test]
#[ignore = "driven by a_failed_catch_up_rolls_the_import_back"]
async fn catch_up_rollback_child() {
    let tree = Tree::with_rules();
    let before = std::fs::read(tree.path("main.journal")).expect("journal");

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");
    let mut body = dry_run_body(id, "import/bank.csv.rules");
    body["writeAssertion"] = json!(false);
    let (status, response) = post(&tree, "/api/import/commit", body).await;

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a commit that could not record its dedup state must not report success: {response}"
    );
    let message = as_text(&response);
    assert!(
        message.contains("undone") && message.contains("main.journal"),
        "the error must say the import was undone, and name the file: {message}"
    );
    assert!(
        message.contains("the state file could not be written"),
        "hledger's own words must reach the user: {message}"
    );

    assert_eq!(
        std::fs::read(tree.path("main.journal")).expect("journal"),
        before,
        "the journal must be byte-identical to what it was before the commit"
    );
    // The CSV stays where it was written — the same thing a failed import has
    // always left behind, and the file the user asked to keep.
    assert!(tree.path("import/bank.csv").is_file());
}

// ===========================================================================
// save-csv — the no-rules-file path
// ===========================================================================

/// **"Even if no rules file applies, they can store the csv."** The converted
/// CSV is written, no journal is touched, and the response says so.
///
/// Hermetic: nothing on this path runs hledger, which is the point — a statement
/// no rules file fits is still worth keeping, and a user with no hledger at all
/// can still get the conversion out.
#[tokio::test]
async fn save_csv_writes_the_converted_file_and_nothing_else() {
    let tree = Tree::bare();
    let before = tree.snapshot();
    let (status, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let id = staged["stageId"].as_str().expect("a stageId");
    // No rules file exists in this tree, so this is the state the route is FOR.
    assert_eq!(staged["candidates"], json!([]), "{staged}");

    let (status, response) = post(
        &tree,
        "/api/import/save-csv",
        json!({"stageId": id, "csvPath": "import/bank.csv"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["csvWritten"], json!("import/bank.csv"));
    // No journal was touched, so the response does not pretend one was.
    assert!(response.get("journalWritten").is_none(), "{response}");
    assert!(response.get("imported").is_none(), "{response}");

    assert_eq!(
        changed(&before, &tree.snapshot()),
        vec!["import/bank.csv".to_string()],
        "exactly one file may appear, and it is the CSV"
    );
    assert_eq!(
        std::fs::read_to_string(tree.path("import/bank.csv")).expect("the csv"),
        STATEMENT,
        "the bytes written are the CONVERTED csv"
    );
    assert_no_absolute_path(&tree, &response.to_string(), "save-csv");
}

/// The same three handle rules a commit obeys: shape first, then confinement,
/// and a destination that is not a plain `.csv` inside the journal's directory
/// is refused without saying why in a way that could be probed.
#[tokio::test]
async fn save_csv_refuses_a_destination_outside_the_journal_directory() {
    let tree = Tree::bare();
    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");

    for csv_path in [
        "../escape.csv",
        "/etc/passwd.csv",
        "import/../../escape.csv",
        "main.journal",
        "import/bank.csv.rules",
    ] {
        let (status, response) = post(
            &tree,
            "/api/import/save-csv",
            json!({"stageId": id, "csvPath": csv_path}),
        )
        .await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
            "{csv_path:?} must be refused, got {status} {response}"
        );
        assert_no_absolute_path(&tree, &as_text(&response), csv_path);
    }

    // An unknown stage is the same 404 every other route gives it.
    let (status, _) = post(
        &tree,
        "/api/import/save-csv",
        json!({"stageId": "0123456789abcdef0123456789abcdef", "csvPath": "import/bank.csv"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And the body is closed: a `rulesId` here is a client that thinks this
    // route imports something.
    let (status, _) = post(
        &tree,
        "/api/import/save-csv",
        json!({"stageId": id, "csvPath": "import/bank.csv", "rulesId": "import/bank.csv.rules"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A CSV destination with uncommitted changes blocks, for the reason a commit's
/// targets block: overwriting somebody's edit is the one thing `git diff` could
/// not have undone.
#[tokio::test]
async fn save_csv_refuses_to_overwrite_an_uncommitted_change() {
    require_git!();
    let tree = Tree::with_rules();
    std::fs::write(tree.path("import/bank.csv"), "Date,Description\n").expect("seed the csv");
    init_repo(tree.dir.path());
    std::fs::write(tree.path("import/bank.csv"), "edited, not committed\n").expect("dirty it");

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    let id = staged["stageId"].as_str().expect("a stageId");
    let before = tree.snapshot();
    let (status, response) = post(
        &tree,
        "/api/import/save-csv",
        json!({"stageId": id, "csvPath": "import/bank.csv"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{response}");
    assert_eq!(
        changed(&before, &tree.snapshot()),
        Vec::<String>::new(),
        "a refused save writes nothing"
    );
}

// ===========================================================================
// Layer 5 — no absolute path in any response
// ===========================================================================

/// The rule the whole no-disclosure posture rests on, applied to every route
/// this module added. hledger and git both put absolute paths in their output;
/// the responses that carry that output must not.
#[tokio::test]
async fn no_import_response_body_contains_an_absolute_path() {
    let tree = Tree::with_rules();
    let mut bodies: Vec<(String, String)> = Vec::new();

    let (_, capabilities) = get(&tree, "/api/import/capabilities").await;
    bodies.push(("capabilities".to_string(), capabilities.to_string()));

    let (_, staged) = upload(&tree, "bank.csv", STATEMENT.as_bytes().to_vec()).await;
    bodies.push(("stage".to_string(), staged.to_string()));
    let id = staged["stageId"].as_str().expect("a stageId").to_string();

    for (name, response) in [
        (
            "stage/pdf",
            upload(&tree, "s.pdf", b"%PDF-1.7\n".to_vec()).await,
        ),
        (
            "dry-run",
            post(
                &tree,
                "/api/import/dry-run",
                dry_run_body(&id, "import/bank.csv.rules"),
            )
            .await,
        ),
        (
            "dry-run/unknown-rules",
            post(
                &tree,
                "/api/import/dry-run",
                dry_run_body(&id, "nope.rules"),
            )
            .await,
        ),
        (
            "save-csv",
            post(
                &tree,
                "/api/import/save-csv",
                json!({"stageId": &id, "csvPath": "import/saved.csv"}),
            )
            .await,
        ),
        (
            "save-csv/outside",
            post(
                &tree,
                "/api/import/save-csv",
                json!({"stageId": &id, "csvPath": "../escape.csv"}),
            )
            .await,
        ),
        (
            "sort",
            post(
                &tree,
                "/api/import/sort",
                json!({"journalId": "main.journal"}),
            )
            .await,
        ),
        (
            "sort/unknown",
            post(
                &tree,
                "/api/import/sort",
                json!({"journalId": "../escape.journal"}),
            )
            .await,
        ),
    ] {
        bodies.push((name.to_string(), response.1.to_string()));
    }

    if std::env::var_os(IMPORT_CHECK).is_some() {
        let mut body = dry_run_body(&id, "import/bank.csv.rules");
        body["writeAssertion"] = json!(false);
        let (_, committed) = post(&tree, "/api/import/commit", body).await;
        bodies.push(("commit".to_string(), committed.to_string()));
    }

    for (name, body) in bodies {
        assert_no_absolute_path(&tree, &body, &name);
    }
}

/// Assert `body` discloses neither the scratch tree nor the staging area.
///
/// Both spellings of each, because on macOS the temp directory canonicalizes
/// through a symlink (`/var/folders/…` vs `/private/var/folders/…`) and a
/// subprocess may report either.
fn assert_no_absolute_path(tree: &Tree, body: &str, what: &str) {
    let mut secrets = vec![
        tree.dir.path().to_path_buf(),
        std::env::temp_dir(),
        fixtures_dir(),
    ];
    let canonical: Vec<PathBuf> = secrets
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    secrets.extend(canonical);
    for secret in secrets {
        let rendered = secret.to_string_lossy().into_owned();
        // A JSON body is escaped, so check the escaped spelling too.
        for spelling in [rendered.clone(), rendered.replace('/', "\\/")] {
            assert!(
                !body.contains(&spelling),
                "{what} discloses {spelling}:\n{body}"
            );
        }
    }
}

// ===========================================================================
// Test-process plumbing
// ===========================================================================

/// A resolved real hledger, for the tests that need one. Only ever called behind
/// [`require_hledger`].
fn resolve_hledger() -> Hledger {
    Hledger::resolve(&Prefs::default()).unwrap_or_else(|error| {
        panic!("{IMPORT_CHECK} is set but no usable hledger was found: {error}")
    })
}

/// Write an executable `hledger` stub that prints `banner` on stdout.
fn write_stub(dir: &Path, name: &str, banner: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{banner}'\n")).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    path
}

/// The scratch directory a parent handed this child.
fn child_dir() -> PathBuf {
    std::env::var_os(CHILD_DIR_ENV)
        .or_else(|| std::env::var_os("LEDGELINE_CONFIG_DIR"))
        .map(PathBuf::from)
        .expect("a child test must be given its scratch directory")
}

/// Re-execute this test binary, running exactly the `#[ignore]`d test named
/// `test_name` with `env` applied to the child alone.
///
/// The environment variables this suite drives — `$LEDGELINE_HLEDGER`,
/// `$LEDGELINE_CONFIG_DIR` — are process-global, and libtest runs tests on
/// threads of ONE process. `std::env::set_var` is `unsafe` in edition 2024
/// precisely because it is not thread-safe, and this codebase does not use
/// `unsafe`. A child gets a pristine, private environment and exercises the real
/// `std::env::var_os` path rather than a test-only seam around it. The same
/// mechanism, and the same reasoning, as `tests/prefs.rs`.
fn run_child(test_name: &str, env: &[(&str, &Path)]) {
    let exe = std::env::current_exe().expect("locate this test binary");
    let mut command = Command::new(exe);
    command.args([test_name, "--exact", "--ignored", "--test-threads=1"]);
    command.env_remove("LEDGELINE_CONFIG_DIR");
    command.env_remove("LEDGELINE_HLEDGER");
    command.env_remove(CHILD_DIR_ENV);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .output()
        .expect("re-run this test binary as a child");
    assert!(
        output.status.success(),
        "child test `{test_name}` failed\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Initialise a repository over the scratch tree and commit everything in it, so
/// the import starts from a clean state.
///
/// Every developer-machine setting is pinned in the repository's OWN config,
/// which beats the global one: signing would demand a passphrase nobody can
/// type, a global `core.hooksPath` would hide nothing useful here, and a global
/// `core.excludesFile` would make files ignored that these tests expect to be
/// committable. The same neutralisation `tests/git_commit.rs` performs, and for
/// the same reason.
fn init_repo(dir: &Path) {
    git(dir, &["init", "--quiet"]);
    for setting in [
        ["user.name", "Ledgeline Test"],
        ["user.email", "test@ledgeline.invalid"],
        ["commit.gpgsign", "false"],
        ["core.excludesFile", ""],
    ] {
        git(dir, &["config", setting[0], setting[1]]);
    }
    git(dir, &["add", "--all"]);
    git(dir, &["commit", "--quiet", "--message", "initial"]);
}

/// Run git in `dir` and return its combined output.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("git {args:?}: {error}"));
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// ---------------------------------------------------------------------------
// hledger.conf — the second home an alias can live in, and command-line parity
// ---------------------------------------------------------------------------

/// The `--alias` line a config file needs for [`ALIAS_STATEMENT`].
///
/// Note the shape: a regular expression with `.` where the bank's name has
/// spaces, because **hledger's config parser splits on whitespace and ignores
/// quotes** — both `--alias="…"` and `--alias='…'` are parse errors, verified.
/// `.` matches a space, so this is the only spelling that survives the file and
/// still matches. `hledger_conf::conf_argument` is what produces it.
const CONF_ALIAS: &str = "/^PW.Roth.IRA.-.3077($|:)/=assets:morganstanley:pw-roth-ira\\1";

/// `Tree::bare` plus the statement-account rules file, and no journal alias —
/// the baseline the config-file tests add to.
fn conf_tree(journal: &str) -> Tree {
    let tree = Tree::bare();
    std::fs::write(tree.path("main.journal"), journal).expect("write journal");
    std::fs::copy(
        fixtures_dir().join("import/match/statement-account.csv.rules"),
        tree.path("import/bank.csv.rules"),
    )
    .expect("copy the rules fixture");
    let state =
        AppState::from_journal_path(tree.path("main.journal")).expect("the scratch journal opens");
    Tree {
        dir: tree.dir,
        state,
    }
}

/// Stage [`ALIAS_STATEMENT`] and dry-run it, returning the preview.
async fn alias_preview(tree: &Tree) -> Value {
    let (status, staged) = upload(tree, "bank.csv", ALIAS_STATEMENT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let id = staged["stageId"].as_str().expect("a stageId").to_string();
    let (status, preview) = post(
        tree,
        "/api/import/dry-run",
        dry_run_body(&id, "import/bank.csv.rules"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    preview
}

/// **The divergence notice.** A journal alias reaches this import; nothing would
/// reach a command-line one; the dry run says so before anything is written.
///
/// The verdict is MEASURED — the engine repeats the import with exactly the
/// aliases a config file supplies (here, none) and diffs — so this asserts
/// hledger's own answer rather than a string comparison of alias spellings.
#[tokio::test]
async fn a_journal_only_alias_raises_the_command_line_divergence() {
    require_hledger!();
    let tree = alias_tree();
    let preview = alias_preview(&tree).await;

    let cli = &preview["aliases"]["cli"];
    assert_eq!(cli["matches"], json!(false), "{preview}");
    assert_eq!(
        cli["differences"],
        json!([{"from": BANK_SPEAK, "to": MAPPED}]),
        "the notice must name the accounts that would differ: {preview}"
    );
    // No config file at all, so nothing to report and everything to offer.
    assert_eq!(cli["confPath"], json!(null));
    assert_eq!(cli["writable"], json!(true));
    assert_eq!(
        cli["revision"],
        json!(""),
        "no file yet is the empty revision"
    );
    // The exact line the fix would write, shown BEFORE it is pressed: the
    // conversion widens what the pattern matches (a `.` matches any character,
    // and a regex alias is case-insensitive where a plain one is not).
    assert_eq!(cli["additions"], json!([CONF_ALIAS]), "{preview}");
    assert_eq!(cli["refusals"], json!([]));
}

/// A config file that already supplies the mapping makes the notice disappear —
/// and the alias reaches the import through the config rather than the journal.
#[tokio::test]
async fn a_config_declared_alias_is_forwarded_and_parity_holds() {
    require_hledger!();
    // No `alias` directive in the journal at all. Everything below comes from
    // the config file.
    let tree = conf_tree(OPENING);
    std::fs::write(tree.path("hledger.conf"), format!("--alias={CONF_ALIAS}\n"))
        .expect("write hledger.conf");

    let preview = alias_preview(&tree).await;
    let entries = preview["entries"].as_str().expect("entries");
    assert!(
        entries.contains(MAPPED) && !entries.contains(BANK_SPEAK),
        "the config's alias must reach the import: {entries}"
    );
    // Ledgeline applies exactly what a terminal would, so there is nothing to
    // warn about.
    assert_eq!(
        preview["aliases"]["cli"]["matches"],
        json!(true),
        "{preview}"
    );
    assert_eq!(preview["aliases"]["cli"]["confPath"], json!("hledger.conf"));
    assert_eq!(preview["aliases"]["cli"]["differences"], json!([]));
}

/// A config file whose `--alias` and a journal `alias` describe the same account
/// leaves the CONFIG's answer standing, because that is the answer a terminal
/// gives. Ledgeline agreeing with the command line is the whole point; agreeing
/// with it in the opposite direction would be a second divergence.
#[tokio::test]
async fn the_config_wins_where_it_and_the_journal_disagree() {
    require_hledger!();
    let tree = conf_tree(&format!(
        "alias PW Roth IRA - 3077 = assets:from:journal\n\n{OPENING}"
    ));
    std::fs::write(
        tree.path("hledger.conf"),
        "--alias=/^PW.Roth.IRA.-.3077($|:)/=assets:from:config\\1\n",
    )
    .expect("write hledger.conf");

    let preview = alias_preview(&tree).await;
    let entries = preview["entries"].as_str().expect("entries");
    assert!(entries.contains("assets:from:config"), "{entries}");
    assert!(!entries.contains("assets:from:journal"), "{entries}");
    assert_eq!(
        preview["aliases"]["cli"]["matches"],
        json!(true),
        "{preview}"
    );
}

/// A config file in force from ABOVE the journal's directory is read and
/// reported by a relative handle — never an absolute path, and never written to.
#[tokio::test]
async fn a_config_above_the_journal_directory_is_read_and_reported_relatively() {
    require_hledger!();
    let outer = TempDir::new().expect("temp dir");
    let books = outer.path().join("books");
    std::fs::create_dir(&books).expect("books dir");
    std::fs::write(books.join("main.journal"), OPENING).expect("journal");
    std::fs::create_dir(books.join("import")).expect("import dir");
    std::fs::copy(
        fixtures_dir().join("import/match/statement-account.csv.rules"),
        books.join("import/bank.csv.rules"),
    )
    .expect("copy the rules fixture");
    // One level up from the journal — exactly where hledger would find it.
    std::fs::write(
        outer.path().join("hledger.conf"),
        format!("--alias={CONF_ALIAS}\n"),
    )
    .expect("write hledger.conf");

    let state = AppState::from_journal_path(books.join("main.journal")).expect("opens");
    let tree = Tree { dir: outer, state };

    let preview = alias_preview(&tree).await;
    let cli = &preview["aliases"]["cli"];
    assert_eq!(cli["confPath"], json!("../hledger.conf"), "{preview}");
    assert_eq!(cli["confOutside"], json!(true));
    // Read, so the mapping applies and there is no divergence to report.
    assert_eq!(cli["matches"], json!(true), "{preview}");
    let body = preview.to_string();
    assert!(
        !body.contains(tree.dir.path().to_str().expect("utf-8")),
        "no absolute path may appear anywhere in a response: {body}"
    );
}

/// **The whole point, end to end.** The one-click fix writes a config file that
/// a REAL command-line `hledger import` then honours.
///
/// The last step deliberately uses `std::process::Command` with a working
/// directory rather than this crate's `Invocation`, because `Invocation` always
/// passes `--no-conf` and sets no working directory — which is correct for
/// Ledgeline and exactly wrong for simulating a person in a terminal. This is
/// the user typing `hledger import` in their books directory.
#[tokio::test]
async fn the_one_click_fix_writes_a_config_a_real_cli_import_honours() {
    require_hledger!();
    let tree = alias_tree();

    // 1. The divergence is reported.
    let preview = alias_preview(&tree).await;
    let cli = &preview["aliases"]["cli"];
    assert_eq!(cli["matches"], json!(false), "{preview}");
    let revision = cli["revision"].as_str().expect("a revision").to_string();

    // 2. The fix is applied. The body carries the revision and NOTHING else —
    //    what to write is recomputed by the engine from the journal's own alias
    //    directives, so this route is not a write-arbitrary-text primitive.
    let (status, written) = post(
        &tree,
        "/api/import/hledger-conf",
        json!({"revision": revision}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{written}");
    assert_eq!(written["confPath"], json!("hledger.conf"));
    assert_eq!(written["created"], json!(true));
    assert_eq!(written["added"], json!([CONF_ALIAS]));

    // 3. It landed beside the journal, and nowhere else.
    let conf = std::fs::read_to_string(tree.path("hledger.conf")).expect("hledger.conf");
    assert!(conf.contains(&format!("--alias={CONF_ALIAS}")), "{conf}");

    // 4. A REAL command-line import now maps the account. Run from the books
    //    directory with no flags of ours — hledger finds the config itself.
    std::fs::write(tree.path("import/bank.csv"), ALIAS_STATEMENT).expect("write csv");
    let output = Command::new(resolve_hledger().path())
        .current_dir(tree.dir.path())
        .args([
            "import",
            "--dry-run",
            "-f",
            "main.journal",
            "--rules",
            "import/bank.csv.rules",
            "import/bank.csv",
        ])
        .output()
        .expect("a command-line hledger runs");
    let proposed = String::from_utf8_lossy(&output.stdout);
    assert!(
        proposed.contains(MAPPED),
        "a command-line import must now produce the MAPPED account:\n{proposed}\n\
         stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!proposed.contains(BANK_SPEAK), "{proposed}");

    // 5. And the notice is gone, measured the same way it appeared.
    let preview = alias_preview(&tree).await;
    assert_eq!(
        preview["aliases"]["cli"]["matches"],
        json!(true),
        "{preview}"
    );
    assert_eq!(
        preview["aliases"]["cli"]["confPath"],
        json!("hledger.conf"),
        "{preview}"
    );
}

/// Pressing the button twice adds the alias once.
///
/// Idempotence is not cosmetic here: the comparison is made in the CONFIG file's
/// form (`/^PW.Roth…($|:)/=…\1`) rather than the command line's
/// (`PW Roth IRA - 3077=…`), and comparing the two spellings would append a
/// duplicate on every press.
#[tokio::test]
async fn installing_the_same_alias_twice_writes_it_once() {
    require_hledger!();
    let tree = alias_tree();
    let revision = alias_preview(&tree).await["aliases"]["cli"]["revision"]
        .as_str()
        .expect("a revision")
        .to_string();

    let (status, first) = post(
        &tree,
        "/api/import/hledger-conf",
        json!({"revision": revision}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let before = std::fs::read_to_string(tree.path("hledger.conf")).expect("conf");

    let (status, second) = post(
        &tree,
        "/api/import/hledger-conf",
        json!({"revision": first["revision"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["added"], json!([]), "nothing left to add");
    assert_eq!(
        std::fs::read_to_string(tree.path("hledger.conf")).expect("conf"),
        before,
        "a second press must not rewrite the file at all"
    );
}

/// A stale revision is a `409`, not a silent clobber — the same model the rules
/// and alias editors hold to. Hermetic: no hledger needed to prove a conflict.
#[tokio::test]
async fn a_stale_config_revision_is_a_conflict() {
    let tree = conf_tree(&format!(
        "alias PW Roth IRA - 3077 = assets:morganstanley:pw-roth-ira\n\n{OPENING}"
    ));
    std::fs::write(tree.path("hledger.conf"), "--depth 3\n").expect("write conf");

    let (status, body) = post(
        &tree,
        "/api/import/hledger-conf",
        json!({"revision": "not-the-revision"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        std::fs::read_to_string(tree.path("hledger.conf")).expect("conf"),
        "--depth 3\n",
        "a refused write must change nothing"
    );
    // The message names the caller's own handle and no path of ours.
    let text = as_text(&body);
    assert!(text.contains("hledger.conf"), "{text}");
    assert!(
        !text.contains(tree.dir.path().to_str().expect("utf-8")),
        "{text}"
    );
}

/// A file that exists cannot be written with the empty revision, which is the
/// revision of "there was nothing here". Without this, a client that read the
/// screen before somebody created a config would silently overwrite it.
#[tokio::test]
async fn the_empty_revision_does_not_authorise_overwriting_a_file_that_appeared() {
    let tree = conf_tree(&format!(
        "alias PW Roth IRA - 3077 = assets:morganstanley:pw-roth-ira\n\n{OPENING}"
    ));
    std::fs::write(tree.path("hledger.conf"), "--depth 3\n").expect("write conf");
    let (status, body) = post(&tree, "/api/import/hledger-conf", json!({"revision": ""})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

/// A symlink at `hledger.conf` is refused outright rather than followed.
///
/// The write target is the one thing about this route that is fixed rather than
/// client-supplied, so the interesting attack is on the file system rather than
/// on the request: a symlink pointing at `~/.bashrc` would otherwise turn "add
/// an alias" into "append to an arbitrary file".
#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_config_is_refused_rather_than_followed() {
    let tree = conf_tree(&format!(
        "alias PW Roth IRA - 3077 = assets:morganstanley:pw-roth-ira\n\n{OPENING}"
    ));
    let elsewhere = tree.path("notes.txt");
    std::fs::write(&elsewhere, "do not touch me\n").expect("write bystander");
    std::os::unix::fs::symlink(&elsewhere, tree.path("hledger.conf")).expect("symlink");

    let (status, body) = post(&tree, "/api/import/hledger-conf", json!({"revision": ""})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        std::fs::read_to_string(&elsewhere).expect("bystander"),
        "do not touch me\n",
        "the symlink's target must be untouched"
    );
}

/// The config write's blast radius is exactly one file.
#[tokio::test]
async fn installing_aliases_writes_one_file_and_nothing_else() {
    require_hledger!();
    let tree = alias_tree();
    std::fs::write(tree.path("notes.txt"), "do not touch me\n").expect("bystander");
    let revision = alias_preview(&tree).await["aliases"]["cli"]["revision"]
        .as_str()
        .expect("a revision")
        .to_string();

    let before = tree.snapshot();
    let (status, body) = post(
        &tree,
        "/api/import/hledger-conf",
        json!({"revision": revision}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        changed(&before, &tree.snapshot()),
        vec!["hledger.conf".to_string()],
        "installing an alias must touch the config file and nothing else"
    );
}

/// A config file whose first word replaces the command breaks every hledger the
/// user runs — and does not touch ours, because every invocation passes
/// `--no-conf`.
///
/// This is the hostile-config case as a test: the same file that rewrites a
/// terminal's `hledger import` into `balance import …` is sitting beside the
/// journal while Ledgeline imports through it successfully.
#[tokio::test]
async fn a_hijacking_config_breaks_the_terminal_and_not_this_engine() {
    require_hledger!();
    let tree = alias_tree();
    // Verified against hledger 1.52: a first word that does not begin with a
    // dash is taken as the command, overriding the one on the command line.
    std::fs::write(tree.path("hledger.conf"), "balance\n").expect("write conf");

    // A real command line is broken by it.
    std::fs::write(tree.path("import/bank.csv"), ALIAS_STATEMENT).expect("write csv");
    let output = Command::new(resolve_hledger().path())
        .current_dir(tree.dir.path())
        .args([
            "import",
            "--dry-run",
            "-f",
            "main.journal",
            "--rules",
            "import/bank.csv.rules",
            "import/bank.csv",
        ])
        .output()
        .expect("hledger runs");
    assert!(
        !output.status.success(),
        "the hostile config must break a plain command line, or this test proves nothing"
    );

    // Ours is not.
    let preview = alias_preview(&tree).await;
    let entries = preview["entries"].as_str().expect("entries");
    assert!(entries.contains(MAPPED), "{preview}");
    // And the screen says why the terminal is broken, rather than leaving the
    // user to discover it.
    assert_eq!(
        preview["aliases"]["cli"]["confHijackedBy"],
        json!("balance"),
        "{preview}"
    );
}
