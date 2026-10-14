//! "Resolve once, pin" at the HTTP surface: the symlink-swap TOCTOU.
//!
//! A journal reached through a symlink keeps working — it is resolved to its
//! real path when the app learns it, and edited there. What must never happen
//! is a write that follows a link swapped in *after* that: the window used to
//! span whole `hledger` runs, and the write landed on whatever the link named
//! (`~/.zshrc`), with the victim's own mode copied forward to hide it.
//!
//! Every swap below points at a victim with **byte-identical** content to the
//! journal, so the content fingerprint alone cannot tell the difference — only
//! the refusal to follow a link and the pinned `(dev, ino)` can.
//!
//! Hermetic and Unix-only: no hledger, and symlinks are the subject.

#![cfg(unix)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use ledgeline::{AppState, router_with_state};
use serde_json::{Value, json};
use std::path::Path;
use tempfile::TempDir;
use tower::ServiceExt;

const JOURNAL: &str = "alias CHK 8842 = assets:bank:checking\n\
                       \n\
                       2026-01-01 opening balances\n\
                       \x20   assets:bank:checking   $1000.00\n\
                       \x20   equity:opening\n";

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, "https://spa.example");
    let request = match body {
        Some(json) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&json).expect("serialize")))
            .expect("request builds"),
        None => builder.body(Body::empty()).expect("request builds"),
    };
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

fn add_body() -> Value {
    json!({
        "date": "2026-02-01",
        "status": "cleared",
        "description": "Coffee",
        "postings": [
            { "account": "expenses:coffee",
              "amount": { "commodity": "$", "quantity": { "mantissa": "500", "places": 2 } } },
            { "account": "assets:bank:checking" }
        ]
    })
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Move `journal` aside and put a symlink to a byte-identical `victim` in its
/// place. Returns the victim's path.
fn swap_in_a_link(journal: &Path) -> std::path::PathBuf {
    let dir = journal.parent().expect("a parent");
    let victim = dir.join("victim.rc");
    std::fs::write(&victim, std::fs::read(journal).expect("read journal")).expect("victim");
    std::fs::rename(journal, dir.join("moved-aside.journal")).expect("move aside");
    std::os::unix::fs::symlink(&victim, journal).expect("swap in the link");
    victim
}

/// The feature that must keep working: a journal opened THROUGH a symlink is
/// edited at its real path, and the link survives.
#[tokio::test]
async fn a_journal_opened_through_a_symlink_is_edited_at_its_real_path() {
    let dir = TempDir::new().expect("temp dir");
    std::fs::create_dir(dir.path().join("books")).expect("books dir");
    let real = dir.path().join("books/main.journal");
    let link = dir.path().join("link.journal");
    std::fs::write(&real, JOURNAL).expect("write journal");
    std::os::unix::fs::symlink(&real, &link).expect("link");
    let state = AppState::from_journal_path(&link).expect("opens through the link");

    let (status, body) = send(&state, "POST", "/api/transactions", Some(add_body())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    assert!(is_symlink(&link), "the link survives the save");
    let text = std::fs::read_to_string(&real).expect("read real");
    assert!(text.contains("2026-02-01 * Coffee"), "{text}");
    assert!(text.starts_with(JOURNAL), "nothing else moved: {text}");
}

/// The transaction editor pinned the real path's inode at load; a link swapped
/// in there afterwards is refused, and the reload that follows does not
/// re-resolve to the link's target either.
#[tokio::test]
async fn a_transaction_edit_refuses_a_symlink_swapped_in_after_load() {
    let dir = TempDir::new().expect("temp dir");
    let journal = dir.path().join("main.journal");
    std::fs::write(&journal, JOURNAL).expect("write journal");
    let state = AppState::from_journal_path(&journal).expect("opens");

    let victim = swap_in_a_link(&journal);
    let (status, body) = send(&state, "POST", "/api/transactions", Some(add_body())).await;
    assert!(
        !status.is_success(),
        "a write through a swapped-in link must be refused: {status} {body}"
    );
    assert_eq!(
        std::fs::read_to_string(&victim).expect("victim"),
        JOURNAL,
        "the link's target is untouched"
    );
    assert!(is_symlink(&journal), "the link is not replaced either");

    // And a retry does not find an editor re-pinned to the victim.
    let (status, _) = send(&state, "POST", "/api/transactions", Some(add_body())).await;
    assert!(!status.is_success(), "{status}");
    assert_eq!(std::fs::read_to_string(&victim).expect("victim"), JOURNAL);
}

/// A route that reads, plans and writes in one request (aliases; budget,
/// accounts and prices share the shape) — the link is swapped in between the
/// client's read and its save, with identical bytes, so the revision matches.
#[tokio::test]
async fn an_alias_save_refuses_a_symlink_swapped_in_after_the_read() {
    let dir = TempDir::new().expect("temp dir");
    let journal = dir.path().join("main.journal");
    std::fs::write(&journal, JOURNAL).expect("write journal");
    let state = AppState::from_journal_path(&journal).expect("opens");

    let (status, listing) = send(&state, "GET", "/api/aliases", None).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    let revision = listing["files"]
        .as_array()
        .and_then(|files| {
            files
                .iter()
                .find(|f| f["journalId"] == json!("main.journal"))
        })
        .and_then(|file| file["revision"].as_str())
        .expect("a revision")
        .to_string();

    let victim = swap_in_a_link(&journal);
    let (status, body) = send(
        &state,
        "PUT",
        "/api/aliases/main.journal",
        Some(json!({"revision": revision, "edits": [{"kind": "delete", "index": 0}]})),
    )
    .await;
    // Refused by whichever guard sees it first: the route's own `writable`
    // check (400) when the swap precedes the request, as here, or the pinned
    // read/replace (409) when it lands mid-request — a window this test cannot
    // reach from outside, and which `ledgeline_core::pinned`'s unit tests drive
    // directly.
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::CONFLICT,
        "{status} {body}"
    );
    assert_eq!(std::fs::read_to_string(&victim).expect("victim"), JOURNAL);
    assert!(is_symlink(&journal));
}
