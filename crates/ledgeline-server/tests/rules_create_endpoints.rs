//! End-to-end HTTP tests for **creating** a rules file: `POST
//! /api/rules-create`, and the `PUT /api/rules/{*id}` create branch it feeds.
//!
//! Same harness as `rules_endpoints.rs` — the real axum `Router` driven through
//! `tower`'s `oneshot` over an [`AppState`] bound to a temp journal tree, so
//! every test asserts the HTTP status and body *and* what is left on disk.
//!
//! # Why this is a separate file from `rules_endpoints.rs`
//!
//! Because it is a **different write boundary**, and the argument for it is
//! different. Every other rules write is confined by `Discovery::resolve`, which
//! can only return a file a scan already found — the guarantee is that a client
//! string is only ever *compared*, never turned into a path. A create cannot
//! have that: the file is not there yet. So `Discovery::resolve_new` performs
//! the one `root.join(id)` in the codebase, and the tests that hold that join
//! honest belong together and belong labelled.
//!
//! What is pinned here, in order of how much it would cost to get wrong:
//!
//! 1. **A create never overwrites.** Not through a taken name, not through a
//!    symlink, not through a race, and not through an edit-shaped request.
//! 2. **Confinement.** A traversal, an absolute path, a hidden directory and a
//!    `node_modules` never become a path — and the refusals for the two that
//!    could answer a question about the filesystem are indistinguishable.
//! 3. **The round trip.** A draft, saved unchanged, produces a file the ordinary
//!    read route can open — which is the whole feature.

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

/// A journal that parses, so an editor can be bound to it. The tree beside it is
/// what these tests are about.
const JOURNAL: &str = "\
2026-01-03 COFFEE HOUSE
    expenses:food:coffee  $6.45
    assets:bank:checking
";

/// The upload every test drafts from: a plain, well-behaved bank export, so a
/// failure here is about the route rather than about the mapping (the mapping
/// itself is `ledgeline-core`'s `rules_generate.rs`).
const STATEMENT: &str = "\
Posted Date,Description,Amount
01/02/2026,COFFEE ROASTERS,-4.50
01/15/2026,PAYROLL DIRECT DEP,2400.00
";

const FILENAME: &str = "x-ledgeline-filename";

static SEQ: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Tree {
    dir: PathBuf,
    state: AppState,
}

impl Tree {
    fn new(files: &[(&str, &str)]) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "ledgeline-rules-create/{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let journal = dir.join("main.journal");
        std::fs::write(&journal, JOURNAL).expect("write journal");
        for (relative, contents) in files {
            let path = dir.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("temp subdir");
            }
            std::fs::write(&path, contents).expect("write file");
        }
        let state = AppState::from_journal_path(&journal).expect("editor opens");
        Self { dir, state }
    }

    /// A tree with an `import/2026/` directory to write into, and one rules
    /// file already in it so "that name is taken" is reachable.
    fn standard() -> Self {
        Self::new(&[(
            "import/2026/taken.csv.rules",
            "skip 1\nfields date, description, amount\naccount1 assets:bank:checking\n",
        )])
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.join(relative)
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).expect("read file")
    }

    /// Every spelling of this tree's own directory that could leak into a
    /// response, including the canonical form macOS gains a `/private` prefix
    /// in.
    fn secret_paths(&self) -> Vec<String> {
        let mut paths = vec![self.dir.to_string_lossy().into_owned()];
        if let Ok(canonical) = self.dir.canonicalize() {
            paths.push(canonical.to_string_lossy().into_owned());
        }
        paths
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(value) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&value).expect("serialize")))
            .expect("request builds"),
        None => builder.body(Body::empty()).expect("request builds"),
    };
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

async fn json(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, bytes) = send(state, method, uri, body).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn text(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let (status, bytes) = send(state, method, uri, body).await;
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Upload `STATEMENT` and return the stage id the candidate list would hold.
async fn stage(state: &AppState) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/import/stage")
        .header(FILENAME, "bank.csv")
        .header(header::CONTENT_TYPE, "text/csv")
        .body(Body::from(STATEMENT))
        .expect("request builds");
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK, "the upload stages");
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes();
    let value: Value = serde_json::from_slice(&bytes).expect("stage JSON");
    value["stageId"].as_str().expect("a stage id").to_string()
}

/// Draft `id` from a freshly staged upload.
async fn draft(state: &AppState, id: &str, account1: &str) -> (StatusCode, Value) {
    let stage_id = stage(state).await;
    json(
        state,
        "POST",
        "/api/rules-create",
        Some(json!({"stageId": stage_id, "id": id, "account1": account1})),
    )
    .await
}

/// The `PUT` body that saves a drafted document unchanged: every item typed and
/// id-less, against the "there is no file yet" revision.
///
/// This mirrors what the SPA's `createSaveRequest` builds, and it is the shape
/// the create branch requires — an empty document has no items, so there are no
/// bytes for a `keep` to re-emit.
fn create_body(doc: &Value) -> Value {
    let items: Vec<Value> = doc["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            let mut body = item.clone();
            let object = body.as_object_mut().expect("an item object");
            object.remove("id");
            object.remove("line");
            object.remove("lines");
            body
        })
        .collect();
    json!({"revision": "", "items": items, "delete": []})
}

// ---------------------------------------------------------------------------
// The happy path, end to end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_draft_becomes_a_file_the_ordinary_read_route_can_open() {
    // The whole feature in one test: drop a CSV, draft a rules file for it,
    // save the draft unchanged, and open it again through the route that knows
    // nothing about any of this.
    let tree = Tree::standard();
    let (status, body) = draft(
        &tree.state,
        "import/2026/bank.csv.rules",
        "assets:bank:checking",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["doc"]["id"], "import/2026/bank.csv.rules");
    assert_eq!(body["doc"]["label"], "bank");
    assert_eq!(
        body["doc"]["revision"], "",
        "the empty revision IS the create handle"
    );
    assert_eq!(body["doc"]["settings"]["dateFormat"]["value"], "%m/%d/%Y");
    assert_eq!(
        body["doc"]["settings"]["account1"]["value"],
        "assets:bank:checking"
    );
    assert_eq!(
        body["doc"]["settings"]["fields"]["names"],
        json!(["date", "description", "amount"])
    );
    // Nothing is on disk yet. Drafting and writing are separate operations.
    assert!(!tree.path("import/2026/bank.csv.rules").exists());

    let (status, saved) = json(
        &tree.state,
        "PUT",
        "/api/rules/import/2026/bank.csv.rules",
        Some(create_body(&body["doc"])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_ne!(saved["revision"], "", "a saved file has a real revision");

    let written = tree.read("import/2026/bank.csv.rules");
    assert_eq!(
        written,
        "skip 1\ndate-format %m/%d/%Y\nfields date, description, amount\naccount1 assets:bank:checking\naccount2 expenses:unknown\n",
        "the bytes are the renderer's, and there is no comment line in them"
    );

    let (status, reread) = json(
        &tree.state,
        "GET",
        "/api/rules/import/2026/bank.csv.rules",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reread["revision"], saved["revision"]);
    assert_eq!(reread["items"], saved["items"]);

    // And it appears in the listing, which is what the Edit Rules tab reads.
    let (_, index) = json(&tree.state, "GET", "/api/rules", None).await;
    let ids: Vec<&str> = index["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(|file| file["id"].as_str())
        .collect();
    assert!(ids.contains(&"import/2026/bank.csv.rules"), "{ids:?}");
}

#[tokio::test]
async fn the_draft_reports_how_each_column_was_read() {
    let tree = Tree::standard();
    let (status, body) = draft(&tree.state, "bank.csv.rules", "assets:bank:checking").await;
    assert_eq!(status, StatusCode::OK);

    let columns = body["columns"].as_array().expect("columns");
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0]["field"], "date");
    assert_eq!(columns[1]["field"], "description");
    assert_eq!(columns[2]["field"], "amount");
    for column in columns {
        let confidence = column["confidence"].as_f64().expect("a confidence");
        assert!((0.0..=1.0).contains(&confidence), "{confidence}");
    }

    // The preview is `rules-preview`'s own shape, so the SPA decodes it with
    // the decoder it already has.
    assert_eq!(body["preview"]["available"], true);
    assert_eq!(body["preview"]["separator"], ",");
    assert_eq!(body["preview"]["columns"], 3);
    assert_eq!(
        body["preview"]["header"],
        json!(["Posted Date", "Description", "Amount"])
    );
    assert_eq!(body["preview"]["rows"].as_array().expect("rows").len(), 2);
}

#[tokio::test]
async fn an_ambiguous_date_column_is_flagged_rather_than_guessed_silently() {
    // Every component <= 12, so nothing in the data can tell month-first from
    // day-first. The draft still picks one -- it has to write something -- and
    // says so, which is the difference between a guess and a claim.
    let tree = Tree::standard();
    let request = Request::builder()
        .method("POST")
        .uri("/api/import/stage")
        .header(FILENAME, "ambiguous.csv")
        .header(header::CONTENT_TYPE, "text/csv")
        .body(Body::from(
            "Date,Description,Amount\n01/02/2026,COFFEE,-4.50\n03/04/2026,BOOKS,-12.00\n",
        ))
        .expect("request builds");
    let response = router_with_state(tree.state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes();
    let stage_id: Value = serde_json::from_slice(&bytes).expect("stage JSON");

    let (status, body) = json(
        &tree.state,
        "POST",
        "/api/rules-create",
        Some(json!({
            "stageId": stage_id["stageId"],
            "id": "ambiguous.csv.rules",
            "account1": "assets:bank:checking"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let warnings = body["warnings"].as_array().expect("warnings");
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .is_some_and(|w| w.contains("more than one way"))),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn account1_is_optional_so_the_form_has_something_to_show() {
    let tree = Tree::standard();
    let (status, body) = draft(&tree.state, "bank.csv.rules", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["doc"]["settings"]["account1"]["value"], "");
}

// ---------------------------------------------------------------------------
// Creating never overwrites
// ---------------------------------------------------------------------------

#[tokio::test]
async fn drafting_over_an_existing_name_is_a_409_before_anything_is_read() {
    let tree = Tree::standard();
    let before = tree.read("import/2026/taken.csv.rules");
    let (status, body) = text(
        &tree.state,
        "POST",
        "/api/rules-create",
        Some(json!({
            "stageId": stage(&tree.state).await,
            "id": "import/2026/taken.csv.rules",
            "account1": "assets:bank:checking"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("already exists"), "{body}");
    assert_eq!(tree.read("import/2026/taken.csv.rules"), before);
}

#[tokio::test]
async fn saving_a_create_over_an_existing_name_is_a_409_and_writes_nothing() {
    // The authoritative check, and the one that matters: the draft route's 409
    // is a courtesy that expires the moment it returns.
    let tree = Tree::standard();
    let (_, body) = draft(
        &tree.state,
        "import/2026/free.csv.rules",
        "assets:bank:checking",
    )
    .await;
    let before = tree.read("import/2026/taken.csv.rules");

    let (status, message) = text(
        &tree.state,
        "PUT",
        "/api/rules/import/2026/taken.csv.rules",
        Some(create_body(&body["doc"])),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{message}");
    assert_eq!(
        tree.read("import/2026/taken.csv.rules"),
        before,
        "an existing file must be untouched"
    );
}

#[tokio::test]
async fn a_create_whose_name_is_taken_between_draft_and_save_still_refuses() {
    // The race the exclusive open closes. `resolve_new` said the name was free;
    // by the time the write happens it is not. Simulated by creating the file
    // in between -- which is exactly what a second tab, or vim, would do.
    let tree = Tree::standard();
    let (_, body) = draft(
        &tree.state,
        "import/2026/racy.csv.rules",
        "assets:bank:checking",
    )
    .await;
    std::fs::write(
        tree.path("import/2026/racy.csv.rules"),
        "someone else's file\n",
    )
    .expect("write");

    let (status, message) = text(
        &tree.state,
        "PUT",
        "/api/rules/import/2026/racy.csv.rules",
        Some(create_body(&body["doc"])),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{message}");
    assert_eq!(
        tree.read("import/2026/racy.csv.rules"),
        "someone else's file\n"
    );
}

#[tokio::test]
async fn a_create_never_follows_a_symlink() {
    // A symlink named `*.rules` pointing at the journal itself. Following it
    // would let a create truncate the user's books -- which is why the file
    // type is asked about with `symlink_metadata` and the answer to "something
    // is there" is yes for a link.
    let tree = Tree::standard();
    let link = tree.path("import/2026/link.csv.rules");
    #[cfg(unix)]
    std::os::unix::fs::symlink(tree.path("main.journal"), &link).expect("symlink");
    #[cfg(not(unix))]
    return;

    let journal_before = tree.read("main.journal");
    let (_, body) = draft(
        &tree.state,
        "import/2026/free.csv.rules",
        "assets:bank:checking",
    )
    .await;
    let (status, message) = text(
        &tree.state,
        "PUT",
        "/api/rules/import/2026/link.csv.rules",
        Some(create_body(&body["doc"])),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{message}");
    assert_eq!(
        tree.read("main.journal"),
        journal_before,
        "the journal a symlink pointed at must be untouched"
    );
}

#[tokio::test]
async fn an_edit_shaped_request_cannot_create_a_file() {
    // `keep` names an item's existing bytes. A file that does not exist has
    // none, so a create carrying one is a request that cannot mean anything --
    // and the message has to say that rather than "unknown item 0".
    let tree = Tree::standard();
    for items in [
        json!([{"kind": "keep", "id": 0}]),
        json!([{"kind": "assignment", "id": 0, "field": "account1", "value": "assets:bank:checking"}]),
    ] {
        let (status, message) = text(
            &tree.state,
            "PUT",
            "/api/rules/import/2026/new.csv.rules",
            Some(json!({"revision": "", "items": items, "delete": []})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
        assert!(!tree.path("import/2026/new.csv.rules").exists());
    }

    // And a delete list, which names items that do not exist either.
    let (status, message) = text(
        &tree.state,
        "PUT",
        "/api/rules/import/2026/new.csv.rules",
        Some(json!({
            "revision": "",
            "items": [{"kind": "assignment", "field": "account1", "value": "assets:bank:checking"}],
            "delete": [0]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert!(!tree.path("import/2026/new.csv.rules").exists());
}

#[tokio::test]
async fn an_empty_create_is_refused_rather_than_writing_an_empty_file() {
    let tree = Tree::standard();
    let (status, message) = text(
        &tree.state,
        "PUT",
        "/api/rules/import/2026/new.csv.rules",
        Some(json!({"revision": "", "items": [], "delete": []})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert!(!tree.path("import/2026/new.csv.rules").exists());
}

// ---------------------------------------------------------------------------
// Confinement: the one `root.join(id)` in the codebase
// ---------------------------------------------------------------------------

/// Ids that must never become a path, whichever route is asked.
///
/// Every one of these is refused on SHAPE or on CONFINEMENT, before any write.
/// The list deliberately repeats `rules_endpoints.rs`'s own traversal cases:
/// the create path has its own resolver, so it needs its own proof rather than
/// inheriting one.
const HOSTILE_IDS: &[&str] = &[
    "../escape.rules",
    "a/../../escape.rules",
    "./a.rules",
    "/etc/evil.rules",
    "a\\b.rules",
    "C:/x.rules",
    "a//b.rules",
    "x.txt",
    ".rules",
    "a\u{0}.rules",
    "a\n.rules",
    // Hidden and skipped directories: the scan would never list a file there,
    // so creating one would write something the user could not then open.
    ".hidden/x.rules",
    "node_modules/x.rules",
    // Ten components, one past the scan's own reach.
    "a/b/c/d/e/f/g/h/i/j.rules",
];

#[tokio::test]
async fn no_hostile_id_ever_becomes_a_path() {
    let tree = Tree::standard();
    let outside = tree.dir.parent().expect("a parent").join("escape.rules");
    let _ = std::fs::remove_file(&outside);
    let stage_id = stage(&tree.state).await;

    for id in HOSTILE_IDS {
        let (status, message) = text(
            &tree.state,
            "POST",
            "/api/rules-create",
            Some(json!({"stageId": stage_id, "id": id, "account1": "assets:bank:checking"})),
        )
        .await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
            "{id:?} answered {status}: {message}"
        );
        // The response quotes the caller's own id and nothing else.
        for secret in tree.secret_paths() {
            assert!(
                !message.contains(&secret),
                "{id:?} leaked a path: {message}"
            );
        }
    }
    assert!(
        !outside.exists(),
        "nothing may be written outside the journal's own directory"
    );
}

#[tokio::test]
async fn a_hostile_id_cannot_be_written_through_the_create_put_either() {
    // The draft route is not the write, so refusing there proves nothing about
    // the write. Same list, through the route that actually creates files.
    let tree = Tree::standard();
    let (_, body) = draft(&tree.state, "free.csv.rules", "assets:bank:checking").await;
    let payload = create_body(&body["doc"]);
    let outside = tree.dir.parent().expect("a parent").join("escape.rules");
    let _ = std::fs::remove_file(&outside);

    for id in HOSTILE_IDS {
        // A control character cannot be put in a request TARGET at all — `http`
        // refuses to build the URI, and so would any real client. Those two are
        // covered above, where the id is a JSON string and this ceiling does
        // not apply.
        if id.chars().any(|c| c.is_ascii_control()) {
            continue;
        }
        let (status, message) = text(
            &tree.state,
            "PUT",
            &format!("/api/rules/{id}"),
            Some(payload.clone()),
        )
        .await;
        assert_ne!(status, StatusCode::OK, "{id:?} was accepted: {message}");
        for secret in tree.secret_paths() {
            assert!(
                !message.contains(&secret),
                "{id:?} leaked a path: {message}"
            );
        }
    }
    assert!(!outside.exists(), "nothing escaped the journal's directory");
}

#[tokio::test]
async fn a_missing_directory_and_an_escaping_id_answer_alike() {
    // The two refusals that could answer a question about the filesystem, and
    // therefore the two that MUST be indistinguishable: a route that told them
    // apart would report whether a directory outside the journal exists.
    //
    // Both ids below are syntactically perfect — `validate_id` passes them —
    // so both actually reach `resolve_new`, which is the layer under test. A
    // `../` id would never get that far (it is a `400` decided on shape, by
    // design), so using one here would prove nothing about this pair.
    let tree = Tree::standard();
    #[cfg(not(unix))]
    return;

    // An escape that survives syntax: a directory symlink pointing out of the
    // tree. `confine` canonicalizes, so this resolves outside the root even
    // though every component of the id is a plain name.
    let elsewhere = tree
        .dir
        .parent()
        .expect("a parent")
        .join(format!("ledgeline-outside-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&elsewhere);
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&elsewhere, tree.path("import/2026/out"));

    let stage_id = stage(&tree.state).await;
    let mut answers = Vec::new();
    for id in ["not/a/directory/here.rules", "import/2026/out/evil.rules"] {
        answers.push(
            text(
                &tree.state,
                "POST",
                "/api/rules-create",
                Some(json!({"stageId": stage_id, "id": id, "account1": "a:b"})),
            )
            .await,
        );
    }
    let _ = std::fs::remove_dir_all(&elsewhere);

    // Not the same *string* — each names the caller's own id, which is only
    // ever what it already sent — but the same status and the same sentence.
    assert_eq!(answers[0].0, StatusCode::NOT_FOUND, "{:?}", answers[0]);
    assert_eq!(answers[1].0, answers[0].0, "{:?}", answers[1]);
    for (_, message) in &answers {
        assert!(
            message.contains("is available beside this journal"),
            "{message}"
        );
    }
    assert!(
        !elsewhere.join("evil.rules").exists(),
        "nothing was written through the symlink"
    );
}

#[tokio::test]
async fn a_create_never_makes_a_directory() {
    // A rules file goes beside a journal that already exists. Creating the
    // directories on the way would let one request lay down an arbitrary tree
    // inside the user's journal folder.
    let tree = Tree::standard();
    let (_, body) = draft(&tree.state, "free.csv.rules", "assets:bank:checking").await;
    let (status, message) = text(
        &tree.state,
        "PUT",
        "/api/rules/brand/new/tree/bank.csv.rules",
        Some(create_body(&body["doc"])),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{message}");
    assert!(!tree.path("brand").exists(), "no directory was created");
}

// ---------------------------------------------------------------------------
// The other inputs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unknown_stage_is_a_404_that_tells_a_caller_only_what_it_sent() {
    let tree = Tree::standard();
    for stage_id in [
        "not-a-stage-id",
        // Well-formed but never minted: 32 hex characters.
        "0123456789abcdef0123456789abcdef",
        "",
    ] {
        let (status, message) = text(
            &tree.state,
            "POST",
            "/api/rules-create",
            Some(json!({"stageId": stage_id, "id": "bank.csv.rules", "account1": "a:b"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{stage_id:?}: {message}");
        assert!(message.contains("no longer staged"), "{message}");
    }
}

#[tokio::test]
async fn a_read_only_server_refuses_to_draft_at_all() {
    // A draft whose Save button cannot work is a form that dead-ends. The
    // editor's own 501 is the honest answer, and it is the same one the save
    // route gives.
    let tree = Tree::standard();
    let text_journal = tree.read("main.journal");
    let journal = ledgeline_core::parse_journal(&text_journal, "main.journal").expect("parses");
    let read_only = AppState::from_journal(&journal);
    let (status, message) = text(
        &read_only,
        "POST",
        "/api/rules-create",
        Some(json!({"stageId": "0123456789abcdef0123456789abcdef", "id": "a.csv.rules", "account1": "a:b"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{message}");
}

#[tokio::test]
async fn an_unknown_key_in_the_create_body_is_refused() {
    // `deny_unknown_fields`, like every other write body here: a typo'd key
    // must not silently mean "use the default".
    let tree = Tree::standard();
    let (status, message) = text(
        &tree.state,
        "POST",
        "/api/rules-create",
        Some(json!({
            "stageId": stage(&tree.state).await,
            "id": "bank.csv.rules",
            "acount1": "assets:bank:checking"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
}

#[tokio::test]
async fn a_control_character_in_account1_is_refused_by_the_renderer() {
    // The one caller-supplied string that reaches the renderer. A newline is
    // how a second line would be smuggled into a one-line item, so the engine's
    // own value validation is what answers -- not a second check here.
    let tree = Tree::standard();
    let (status, message) = text(
        &tree.state,
        "POST",
        "/api/rules-create",
        Some(json!({
            "stageId": stage(&tree.state).await,
            "id": "bank.csv.rules",
            "account1": "assets:bank\naccount2 evil:account"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
}

#[tokio::test]
async fn no_response_from_the_create_route_ever_contains_a_path() {
    let tree = Tree::standard();
    let (status, body) = draft(
        &tree.state,
        "import/2026/bank.csv.rules",
        "assets:bank:checking",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rendered = body.to_string();
    for secret in tree.secret_paths() {
        assert!(!rendered.contains(&secret), "leaked {secret}");
    }
    // `dataLabel` is deliberately absent rather than the staged file's name:
    // the draft describes the CONVERTED CSV, whose name is the user's to choose
    // in the destination field, not something the stage knows.
    assert!(body["preview"]["dataLabel"].is_null());
}
