//! The `/api/budget/{lines,file,reference}` HTTP surface — the budget editor.
//!
//! Everything here is hermetic: no subprocess, no network, a scratch journal per
//! test.
//!
//! The properties this file exists to pin, in order of how much a regression
//! would cost:
//!
//! 1. **An isolated edit changes one number and nothing else.** Asserted on the
//!    bytes, over a journal deliberately full of things that must not move: a
//!    column-aligned rule, a comment, a second rule, real transactions. A
//!    rewrite that reformats a journal is unrecoverable damage to the most
//!    valuable file this application touches.
//! 2. **A rule stays balanced.** Editing a goal in an explicitly-balanced rule
//!    moves its counter-leg by exactly the delta; an ambiguous one is refused
//!    rather than guessed at.
//! 3. **Income signs round-trip.** A user types `1200` for an income goal, the
//!    journal says `$-1200`, and reading it back offers `1200` again.
//! 4. **A stale revision is a 409**, and nothing is written.
//! 5. **Creating `budget.journal` never overwrites anything**, and writes the
//!    new file before the `include` that names it.
//! 6. **The token guard covers all four routes.**
//! 7. **No response body contains an absolute path.**

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
/// The alignment, the header comment, the second rule and the transactions are
/// all here so that "only the number I edited changed" is a claim with something
/// to be false about.
const JOURNAL: &str = "\
; household books
;
; The budget lives at the top, the ledger below it.

~ monthly  household budget
    (expenses:food)      $400
    (expenses:bus)        $20

~ yearly  annual budget
    (income:interest)  $-1200

2026-01-05 grocery
    expenses:food     $352.10
    assets:checking

2026-01-12 bus pass
    expenses:bus       $23.00
    assets:checking
";

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

    fn budgeted() -> Self {
        Self::with(JOURNAL)
    }

    fn router(&self) -> axum::Router {
        router_with_state(self.state.clone())
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(relative)).expect("read back")
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
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

/// The `main.journal` entry of a `GET /api/budget/lines` body.
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
    let (status, body) = get(tree, "/api/budget/lines").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    main_file(&body)["revision"]
        .as_str()
        .expect("a revision")
        .to_string()
}

/// A `set` change against the current revision.
async fn set(tree: &Tree, index: usize, mantissa: &str, places: u32) -> (StatusCode, Value) {
    let revision = revision(tree).await;
    send_json(
        tree,
        "PUT",
        "/api/budget/lines/main.journal",
        json!({
            "revision": revision,
            "change": {"kind": "set", "index": index,
                       "value": {"mantissa": mantissa, "places": places}}
        }),
    )
    .await
}

// ===========================================================================
// Reading
// ===========================================================================

/// Every goal, grouped by rule, with the amount as the file writes it AND the
/// magnitude the user is meant to type.
#[tokio::test]
async fn the_listing_reports_rules_goals_and_both_signs() {
    let tree = Tree::budgeted();
    let (status, body) = get(&tree, "/api/budget/lines").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["editable"], json!(true));
    assert_eq!(body["defaultTarget"], json!("main.journal"));
    // The journal already has rules, so there is nothing to create.
    assert_eq!(body["canCreateFile"], json!(false));

    let file = main_file(&body);
    assert_eq!(file["writable"], json!(true));
    let rules = file["rules"].as_array().expect("rules is an array");
    assert_eq!(rules.len(), 2);

    assert_eq!(rules[0]["period"], json!("monthly"));
    assert_eq!(rules[0]["description"], json!("household budget"));
    assert_eq!(rules[0]["line"], json!(5));
    let food = &rules[0]["lines"][0];
    assert_eq!(food["account"], json!("expenses:food"));
    assert_eq!(food["index"], json!(0));
    assert_eq!(food["unbalanced"], json!(true));
    assert_eq!(food["inverted"], json!(false));
    assert_eq!(food["entry"]["commodity"], json!("$"));
    assert_eq!(food["entry"]["value"]["mantissa"], json!("400"));
    assert!(food["locked"].is_null(), "an ordinary goal is editable");

    // The income goal is the sign case: `$-1200` in the file, `1200` in the box.
    let interest = &rules[1]["lines"][0];
    assert_eq!(interest["account"], json!("income:interest"));
    assert_eq!(interest["inverted"], json!(true));
    assert_eq!(interest["entry"]["value"]["mantissa"], json!("1200"));
    assert_eq!(interest["amount"]["$"]["mantissa"], json!("-1200"));
}

/// The history strip: subaccount-inclusive, oldest first, with the running
/// period flagged rather than silently shown as a whole one.
#[tokio::test]
async fn the_reference_strip_reports_recent_actuals() {
    let tree = Tree::with(
        "~ monthly  b\n    (expenses:food)  $400\n\n\
         2026-01-05 a\n    expenses:food          $100.00\n    assets:checking\n\n\
         2026-01-20 b\n    expenses:food:dining    $25.00\n    assets:checking\n\n\
         2026-02-04 c\n    expenses:food           $60.00\n    assets:checking\n",
    );
    let (status, body) = get(
        &tree,
        "/api/budget/reference?account=expenses:food&interval=monthly&count=2&asOf=2026-02-15",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["account"], json!("expenses:food"));
    assert_eq!(body["inverted"], json!(false));

    let periods = body["periods"].as_array().expect("periods");
    assert_eq!(periods.len(), 2);
    assert_eq!(periods[0]["key"], json!("2026-01"));
    assert_eq!(periods[0]["complete"], json!(true));
    // $100 + $25 of dining: the subaccount is included, which is what makes the
    // figure comparable to the goal it sits beside.
    assert_eq!(periods[0]["total"]["$"]["mantissa"], json!("12500"));
    assert_eq!(periods[1]["key"], json!("2026-02"));
    assert_eq!(periods[1]["complete"], json!(false));
    assert_eq!(periods[1]["end"], json!("2026-02-15"));

    // The average is the figure a budget is set from, so it covers the COMPLETE
    // periods only — January's $125, not January-and-a-bit-of-February.
    assert_eq!(body["averagedPeriods"], json!(1));
    assert_eq!(body["average"]["$"]["mantissa"], json!("12500"));
}

/// The average excludes the running period, and says how many it covers so the
/// UI can label it — and can tell "no complete periods" from "an average of nil".
#[tokio::test]
async fn the_reference_average_covers_only_whole_periods() {
    let tree = Tree::with(
        "~ monthly  b\n    (expenses:food)  $400\n\n\
         2026-01-10 a\n    expenses:food   $100.00\n    assets:checking\n\n\
         2026-02-10 b\n    expenses:food   $200.00\n    assets:checking\n\n\
         2026-03-02 c\n    expenses:food   $900.00\n    assets:checking\n",
    );
    let (status, body) = get(
        &tree,
        "/api/budget/reference?account=expenses:food&interval=monthly&count=3&asOf=2026-03-15",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // ($100 + $200) / 2 = $150 — March's $900 is a half-month and is left out,
    // or the mean would swing with the calendar rather than with spending.
    assert_eq!(body["averagedPeriods"], json!(2));
    assert_eq!(body["average"]["$"]["mantissa"], json!("15000"));

    // With nothing complete there is no average at all, reported as a count of
    // zero rather than as a confident $0.00.
    let (status, body) = get(
        &tree,
        "/api/budget/reference?account=expenses:food&interval=yearly&count=1&asOf=2026-03-15",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["averagedPeriods"], json!(0));
    assert_eq!(body["average"], json!({}));
}

/// An income account's average is oriented the same way its periods are, so the
/// strip cannot show a positive month beside a negative mean.
#[tokio::test]
async fn the_reference_average_is_oriented_like_its_periods() {
    let tree = Tree::with(
        "~ yearly  b\n    (income:interest)  $-1200\n\n\
         2026-01-31 d\n    assets:checking   $80.00\n    income:interest\n\n\
         2026-02-28 d\n    assets:checking  $120.00\n    income:interest\n",
    );
    let (status, body) = get(
        &tree,
        // 2026-02-28 IS February's last day, so both buckets are complete and
        // both count toward the mean.
        "/api/budget/reference?account=income:interest&interval=monthly&count=2&asOf=2026-02-28",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["inverted"], json!(true));
    // The journal holds $-80 and $-120; the strip shows 80 and 120, and the mean
    // shows 100 — not -100.
    assert_eq!(body["periods"][0]["total"]["$"]["mantissa"], json!("8000"));
    assert_eq!(body["average"]["$"]["mantissa"], json!("10000"));
    assert_eq!(body["averagedPeriods"], json!(2));
}

/// Income reads back the way it is typed here too, so the strip and the box
/// agree about which way round the numbers go.
#[tokio::test]
async fn the_reference_strip_orients_income_like_the_goal_box() {
    let tree = Tree::with(
        "~ yearly  b\n    (income:interest)  $-1200\n\n\
         2026-03-31 dividend\n    assets:checking   $80.00\n    income:interest\n",
    );
    let (status, body) = get(
        &tree,
        "/api/budget/reference?account=income:interest&interval=yearly&count=1&asOf=2026-12-31",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["inverted"], json!(true));
    // The journal says `$-80`; the strip says `80`, like the box does.
    assert_eq!(body["periods"][0]["total"]["$"]["mantissa"], json!("8000"));
}

// ===========================================================================
// Writing
// ===========================================================================

/// The headline property: one number moves, every other byte is where it was.
#[tokio::test]
async fn setting_a_goal_changes_one_number_and_nothing_else() {
    let tree = Tree::budgeted();
    let (status, body) = set(&tree, 0, "45000", 2).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let after = tree.read("main.journal");
    assert_eq!(after, JOURNAL.replace("$400", "$450.00"));
    // Said plainly as well as by equality: the alignment, the comment block, the
    // second rule and both transactions are untouched.
    assert!(after.contains("    (expenses:bus)        $20\n"));
    assert!(after.contains("; The budget lives at the top, the ledger below it.\n"));
    assert!(after.contains("    expenses:food     $352.10\n"));

    // The response describes the file that now exists, so the next save needs no
    // extra round trip.
    let food = &body["rules"][0]["lines"][0];
    assert_eq!(food["entry"]["value"]["mantissa"], json!("45000"));
    assert_eq!(food["entry"]["value"]["places"], json!(2));
}

/// An income goal typed as a magnitude lands negative, and reads back as the
/// magnitude again — the round trip the whole sign rule exists for.
#[tokio::test]
async fn an_income_goal_round_trips_through_its_magnitude() {
    let tree = Tree::budgeted();
    // Index 2 is `income:interest` — the third goal line in the file.
    let (status, body) = set(&tree, 2, "1500", 0).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        tree.read("main.journal")
            .contains("    (income:interest)  $-1500\n"),
        "{}",
        tree.read("main.journal")
    );

    let (status, body2) = get(&tree, "/api/budget/lines").await;
    assert_eq!(status, StatusCode::OK, "{body2}");
    let interest = &main_file(&body2)["rules"][1]["lines"][0];
    assert_eq!(interest["entry"]["value"]["mantissa"], json!("1500"));
    assert_eq!(interest["amount"]["$"]["mantissa"], json!("-1500"));
    let _ = body;
}

/// A rule whose amounts are all written down has its counter-leg moved by
/// exactly the delta, so the rule still balances — which the parser then
/// enforces on the way back in.
#[tokio::test]
async fn editing_an_explicitly_balanced_rule_moves_its_counter_leg() {
    let tree = Tree::with(
        "~ monthly  budget\n\
         \x20   expenses:food      $400\n\
         \x20   expenses:rent     $1500\n\
         \x20   assets:checking  $-1900\n",
    );
    let (status, body) = set(&tree, 0, "450", 0).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = tree.read("main.journal");
    assert!(after.contains("    expenses:food      $450\n"), "{after}");
    assert!(after.contains("    expenses:rent     $1500\n"), "{after}");
    assert!(after.contains("    assets:checking  $-1950\n"), "{after}");
}

/// Editing the funding leg of a three-way rule is genuinely ambiguous — nothing
/// in the file says whether food or rent absorbs it — so it is refused with a
/// sentence, and nothing is written.
#[tokio::test]
async fn an_ambiguous_counter_leg_is_refused_and_writes_nothing() {
    let tree = Tree::with(
        "~ monthly  budget\n\
         \x20   expenses:food      $400\n\
         \x20   expenses:rent     $1500\n\
         \x20   assets:checking  $-1900\n",
    );
    let before = tree.snapshot();
    let (status, body) = set(&tree, 2, "-2000", 0).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("unbalanced"),
        "the refusal must say why: {body}"
    );
    assert_eq!(tree.snapshot(), before, "nothing may be written");
}

/// A goal added to an existing rule lands in that rule, right-aligned onto its
/// amount column, and touches nothing else.
#[tokio::test]
async fn adding_a_goal_appends_to_the_named_rule() {
    let tree = Tree::budgeted();
    let revision = revision(&tree).await;
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/budget/lines/main.journal",
        json!({
            "revision": revision,
            "change": {"kind": "add", "block": 0, "account": "expenses:shopping",
                       "value": {"mantissa": "250", "places": 0}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = tree.read("main.journal");
    assert!(
        after.contains("    (expenses:bus)        $20\n    (expenses:shopping)  $250\n"),
        "{after}"
    );
    assert!(after.contains("~ yearly  annual budget\n"), "{after}");
}

/// A new rule is appended at the end of the file, in the `(account)` idiom, and
/// the original text is still a prefix of the result.
#[tokio::test]
async fn adding_a_rule_appends_it_at_the_end() {
    let tree = Tree::budgeted();
    let revision = revision(&tree).await;
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/budget/lines/main.journal",
        json!({
            "revision": revision,
            "change": {"kind": "addRule", "period": "weekly", "description": "weekly spending",
                       "account": "expenses:coffee",
                       "value": {"mantissa": "1500", "places": 2}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = tree.read("main.journal");
    assert!(
        after.starts_with(JOURNAL),
        "an append rewrote earlier bytes"
    );
    assert!(
        after.ends_with("~ weekly  weekly spending\n    (expenses:coffee)  $15.00\n"),
        "{after}"
    );
}

/// Removing a rule's only goal removes the rule; removing one of several removes
/// just its line.
#[tokio::test]
async fn removing_a_goal_removes_it() {
    let tree = Tree::budgeted();
    let revision = revision(&tree).await;
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/budget/lines/main.journal",
        json!({"revision": revision, "change": {"kind": "remove", "index": 1}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = tree.read("main.journal");
    assert!(!after.contains("expenses:bus)"), "{after}");
    assert!(after.contains("    (expenses:food)      $400\n"), "{after}");
    assert!(after.contains("~ yearly  annual budget\n"), "{after}");
}

/// A period Ledgeline does not model is refused before anything is written,
/// rather than producing a rule whose recurrence it cannot state.
#[tokio::test]
async fn an_unmodelled_period_is_refused() {
    let tree = Tree::budgeted();
    let revision = revision(&tree).await;
    let before = tree.snapshot();
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/budget/lines/main.journal",
        json!({
            "revision": revision,
            "change": {"kind": "addRule", "period": "fortnightly", "description": "",
                       "account": "expenses:coffee",
                       "value": {"mantissa": "15", "places": 0}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(tree.snapshot(), before);
}

/// A revision from before somebody else's write is a 409, and nothing is
/// written — the check that keeps two editors from clobbering each other.
#[tokio::test]
async fn a_stale_revision_is_a_conflict() {
    let tree = Tree::budgeted();
    let stale = revision(&tree).await;
    // Somebody else edits the file.
    std::fs::write(tree.path("main.journal"), JOURNAL.replace("$20", "$25")).expect("write");
    let before = tree.snapshot();

    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/budget/lines/main.journal",
        json!({
            "revision": stale,
            "change": {"kind": "set", "index": 0, "value": {"mantissa": "450", "places": 0}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(tree.snapshot(), before, "nothing may be written");
}

/// Setting a goal to the number it already holds writes NOTHING — a
/// byte-identical write still bumps mtime, and a user's own watch loop would see
/// a change that did not happen.
#[tokio::test]
async fn a_no_op_save_does_not_touch_the_file() {
    let tree = Tree::budgeted();
    let before = std::fs::metadata(tree.path("main.journal"))
        .and_then(|meta| meta.modified())
        .expect("mtime");
    let (status, body) = set(&tree, 0, "400", 0).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(tree.read("main.journal"), JOURNAL);
    let after = std::fs::metadata(tree.path("main.journal"))
        .and_then(|meta| meta.modified())
        .expect("mtime");
    assert_eq!(before, after, "a no-op must not rewrite the file");
}

/// A handle naming a file the parse never read is a 404 that quotes only what
/// the caller sent.
#[tokio::test]
async fn an_unknown_journal_is_a_not_found() {
    let tree = Tree::budgeted();
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/budget/lines/nope.journal",
        json!({
            "revision": "x",
            "change": {"kind": "set", "index": 0, "value": {"mantissa": "1", "places": 0}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// A traversal handle is refused on SYNTAX, before any filesystem call, so the
/// route is never an existence oracle.
#[tokio::test]
async fn a_traversal_handle_is_refused_on_shape() {
    let tree = Tree::budgeted();
    for id in ["../outside.journal", "/etc/passwd", "a/../../b.journal"] {
        let (status, body) = send_json(
            &tree,
            "PUT",
            &format!("/api/budget/lines/{id}"),
            json!({
                "revision": "x",
                "change": {"kind": "set", "index": 0, "value": {"mantissa": "1", "places": 0}}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{id}: {body}");
    }
}

// ===========================================================================
// Creating a budget file
// ===========================================================================

/// The first-goal path: create `budget.journal`, and `include` it from the main
/// journal at EOF.
#[tokio::test]
async fn creating_a_budget_file_writes_it_and_includes_it() {
    let tree = Tree::with("2026-01-05 grocery\n    expenses:food   $10.00\n    assets:checking\n");
    let (status, body) = get(&tree, "/api/budget/lines").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["canCreateFile"], json!(true));
    assert_eq!(body["createFileName"], json!("budget.journal"));
    // With no rules and no transaction-free file, the root is the fallback home:
    // a goal has to be able to go somewhere even before the button is pressed.
    assert_eq!(body["defaultTarget"], json!("main.journal"));
    assert!(
        main_file(&body)["rules"]
            .as_array()
            .expect("rules")
            .is_empty()
    );

    let (status, body) = send_json(&tree, "POST", "/api/budget/file", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["journalId"], json!("budget.journal"));
    assert_eq!(body["includedAs"], json!("include budget.journal"));
    assert_eq!(body["mainJournalId"], json!("main.journal"));

    assert!(tree.read("budget.journal").starts_with("; Budget goals."));
    let main = tree.read("main.journal");
    assert!(main.ends_with("include budget.journal\n"), "{main}");
    assert!(main.starts_with("2026-01-05 grocery\n"), "{main}");

    // The new file is now the default home, and the button is gone.
    let (status, body) = get(&tree, "/api/budget/lines").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["canCreateFile"], json!(false));
    assert_eq!(body["defaultTarget"], json!("budget.journal"));
    // main.journal holds transactions, so it is no longer offered as a home.
    let ids: Vec<&str> = body["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(|file| file["journalId"].as_str())
        .collect();
    assert_eq!(ids, ["budget.journal"]);
}

/// A goal can be added to the file that was just created, which is the whole
/// point of creating it.
#[tokio::test]
async fn a_created_budget_file_accepts_the_first_goal() {
    let tree = Tree::with("2026-01-05 grocery\n    expenses:food   $10.00\n    assets:checking\n");
    let (status, body) = send_json(&tree, "POST", "/api/budget/file", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, listing) = get(&tree, "/api/budget/lines").await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    let revision = listing["files"][0]["revision"]
        .as_str()
        .expect("a revision")
        .to_string();
    let (status, body) = send_json(
        &tree,
        "PUT",
        "/api/budget/lines/budget.journal",
        json!({
            "revision": revision,
            "change": {"kind": "addRule", "period": "monthly", "description": "monthly budget",
                       "account": "expenses:food",
                       "value": {"mantissa": "40000", "places": 2}}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        tree.read("budget.journal")
            .ends_with("~ monthly  monthly budget\n    (expenses:food)  $400.00\n"),
        "{}",
        tree.read("budget.journal")
    );
}

/// An existing `budget.journal` is NEVER written over, even though the journal
/// has no rules and the file is not included.
#[tokio::test]
async fn an_existing_budget_file_is_never_overwritten() {
    let tree = Tree::with("2026-01-05 grocery\n    expenses:food   $10.00\n    assets:checking\n");
    std::fs::write(tree.path("budget.journal"), "; someone else's notes\n").expect("write");
    let before = tree.snapshot();

    let (status, body) = get(&tree, "/api/budget/lines").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["canCreateFile"],
        json!(false),
        "the button must not be offered when it would fail"
    );

    let (status, body) = send_json(&tree, "POST", "/api/budget/file", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(tree.snapshot(), before, "nothing may be written");
}

/// A journal that already has rules is not given a second home for them.
#[tokio::test]
async fn a_journal_with_rules_is_not_given_another_budget_file() {
    let tree = Tree::budgeted();
    let before = tree.snapshot();
    let (status, body) = send_json(&tree, "POST", "/api/budget/file", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(tree.snapshot(), before);
}

// ===========================================================================
// Access control and disclosure
// ===========================================================================

/// All four routes sit above the token guard. Two of them write to the user's
/// journal directory, so this is the test that fails rather than shipping if
/// anyone moves them below it.
#[tokio::test]
async fn every_budget_route_requires_the_token() {
    const PORT: u16 = 5098;
    const HOST: &str = "127.0.0.1:5098";
    let tree = Tree::budgeted();
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
                .body(Body::from(
                    r#"{"revision":"x","change":{"kind":"remove","index":0}}"#,
                ))
                .expect("request builds");
            router_with_security(state, security)
                .oneshot(request)
                .await
                .expect("router responds")
                .status()
        }
    };

    for (method, uri) in [
        ("GET", "/api/budget/lines"),
        ("PUT", "/api/budget/lines/main.journal"),
        ("POST", "/api/budget/file"),
        ("GET", "/api/budget/reference?account=expenses:food"),
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

/// No response ever echoes where the journal lives on disk.
#[tokio::test]
async fn no_response_body_contains_an_absolute_path() {
    let tree = Tree::budgeted();
    let root = tree.dir.path().to_string_lossy().into_owned();

    let mut bodies = Vec::new();
    let (_, body) = get(&tree, "/api/budget/lines").await;
    bodies.push(body.to_string());
    let (_, body) = get(&tree, "/api/budget/reference?account=expenses:food").await;
    bodies.push(body.to_string());
    let (_, body) = send_json(&tree, "POST", "/api/budget/file", json!({})).await;
    bodies.push(body.to_string());
    let (_, body) = send_json(
        &tree,
        "PUT",
        "/api/budget/lines/main.journal",
        json!({"revision": "wrong", "change": {"kind": "remove", "index": 0}}),
    )
    .await;
    bodies.push(body.to_string());

    for body in bodies {
        assert!(
            !body.contains(&root),
            "a response leaked the journal's location: {body}"
        );
    }
}
