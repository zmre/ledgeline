//! The server over a vault with no local path: `AppState::from_vault` on a
//! `MemVault`, the read and edit routes through it, and `refresh()` — the
//! change check a host with no file watcher calls.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppState, Refreshed, router_with_state};
use ledgeline_core::vault::{Availability, MemVault, Vault, VaultPath};
use serde_json::{Value, json};
use tower::ServiceExt;

const MAIN: &str = "include 2026.journal\n";
const TXNS: &str = "2026-01-02 coffee\n    expenses:food  $3.00\n    assets:cash\n";
const MORE: &str = "\n2026-01-03 lunch\n    expenses:food  $12.00\n    assets:cash\n";

fn vp(path: &str) -> VaultPath {
    VaultPath::new(path).expect("valid vault path")
}

fn books() -> Arc<MemVault> {
    Arc::new(
        MemVault::new()
            .with_file("books/main.journal", MAIN)
            .with_file("books/2026.journal", TXNS),
    )
}

fn state_over(vault: &Arc<MemVault>) -> AppState {
    let shared: Arc<dyn Vault> = Arc::clone(vault) as Arc<dyn Vault>;
    AppState::from_vault(shared, vp("books/main.journal")).expect("opens")
}

async fn request(
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
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn transaction_count(state: &AppState) -> usize {
    let (status, body) = request(state, "GET", "/transactions", None).await;
    assert_eq!(status, StatusCode::OK);
    body.as_array().expect("transactions array").len()
}

#[tokio::test]
async fn a_state_over_a_mem_vault_serves_the_journal_with_no_local_path() {
    let vault = books();
    let state = state_over(&vault);
    assert_eq!(transaction_count(&state).await, 1);
    let location = state.location().expect("bound to storage");
    assert_eq!(location.main(), &vp("books/main.journal"));
    assert_eq!(location.vault().local_path(location.main()), None);
    assert_eq!(
        state.source_files(),
        vec![
            vault.key(&vp("books/main.journal")),
            vault.key(&vp("books/2026.journal"))
        ]
    );
}

#[tokio::test]
async fn an_edit_writes_through_the_vault() {
    let vault = books();
    let state = state_over(&vault);
    let (status, body) = request(&state, "DELETE", "/api/transactions/1", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(transaction_count(&state).await, 0);
    assert!(!vault.text("books/2026.journal").unwrap().contains("coffee"));
    // Our own write is not something to refresh from.
    assert_eq!(state.refresh().expect("refreshes"), Refreshed::Unchanged);
}

#[tokio::test]
async fn refresh_reloads_only_when_a_source_changed() {
    let vault = books();
    let state = state_over(&vault);
    assert_eq!(state.refresh().expect("refreshes"), Refreshed::Unchanged);

    // An identical rewrite is not a change.
    vault.modify_externally("books/2026.journal", TXNS);
    assert_eq!(state.refresh().expect("refreshes"), Refreshed::Unchanged);

    vault.modify_in_place("books/2026.journal", format!("{TXNS}{MORE}"));
    assert_eq!(state.refresh().expect("refreshes"), Refreshed::Reloaded);
    assert_eq!(transaction_count(&state).await, 2);
    assert_eq!(state.refresh().expect("refreshes"), Refreshed::Unchanged);
}

#[tokio::test]
async fn an_external_change_before_an_edit_is_a_conflict_not_a_clobber() {
    let vault = books();
    let state = state_over(&vault);
    let external = format!("{TXNS}{MORE}");
    vault.modify_in_place("books/2026.journal", &external);
    let (status, _) = request(
        &state,
        "PATCH",
        "/api/transactions/1",
        Some(json!({"description": "espresso"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(vault.text("books/2026.journal").unwrap(), external);
}

#[tokio::test]
async fn a_journal_still_in_the_cloud_cannot_be_opened_until_downloaded() {
    let vault = books();
    vault.set_availability("books/2026.journal", Availability::NotDownloaded);
    let shared: Arc<dyn Vault> = Arc::clone(&vault) as Arc<dyn Vault>;
    let error = AppState::from_vault(Arc::clone(&shared), vp("books/main.journal"))
        .err()
        .expect("not downloaded");
    assert!(error.to_string().contains("not downloaded"), "{error}");
    vault.request_download(&[vp("books/2026.journal")]);
    AppState::from_vault(shared, vp("books/main.journal")).expect("opens once local");
}
