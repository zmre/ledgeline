//! Plan 29: the projection run's cached real side never outlives its journal.
//!
//! The cache lives in the snapshot, so anything that changes the journal —
//! an edit, an external change picked up by `refresh()`, a `reload()` —
//! publishes a snapshot with an empty cache. Each case below changes the
//! journal WITHOUT moving the cutoff (the key is unchanged), which is exactly
//! the case a cache keyed on the request alone would answer stale.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ledgeline::{AppState, Refreshed, router_with_state};
use ledgeline_core::vault::{MemVault, Vault, VaultPath};
use serde_json::{Value, json};
use tower::ServiceExt;

const MAIN: &str = "\
account assets:checking  ; type: C
account assets:savings   ; type: A
account equity:opening   ; type: E
account expenses:rent    ; type: X

2026-01-01 opening
    assets:checking   $10,000.00
    assets:savings     $5,000.00
    equity:opening

2026-09-30 rent
    expenses:rent      $1,000.00
    assets:checking
";

fn vp(path: &str) -> VaultPath {
    VaultPath::new(path).expect("valid vault path")
}

fn books(text: &str) -> (Arc<MemVault>, AppState) {
    let vault = Arc::new(MemVault::new().with_file("books/main.journal", text));
    let shared: Arc<dyn Vault> = Arc::clone(&vault) as Arc<dyn Vault>;
    let state = AppState::from_vault(shared, vp("books/main.journal")).expect("opens");
    (vault, state)
}

async fn request(state: &AppState, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, "https://spa.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("request builds");
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// One run, today 2026-10-20: the cutoff is 2026-10-01 for every journal
/// below (each ends in September).
async fn run(state: &AppState) -> Value {
    let body = json!({
        "scenario": {
            "name": "cache", "created": null, "updated": null, "events": [],
            "lines": [{
                "id": "rent", "group": "rule:0", "account": "expenses:rent",
                "amount": {"commodity": "$", "quantity": {"mantissa": "100000", "places": 2}, "precision": 2},
                "period": {"raw": "monthly"}, "growth": null, "note": "", "source": "journal",
                "counter": "assets:checking"
            }]
        },
        "asOf": "2026-10-20",
        "interval": "monthly",
        "count": 3,
        "lookback": 2,
        "accounts": {"mode": "balance", "empty": true}
    });
    let (status, response) = request(state, "POST", "/api/projections/run", body).await;
    assert_eq!(status, StatusCode::OK, "run: {response}");
    assert_eq!(response["actualsEnd"], "2026-10-01");
    response
}

/// The whole-dollar `$` figure of a wire amount.
fn dollars(mixed: &Value) -> i128 {
    let Some(dec) = mixed.get("$") else {
        return 0;
    };
    let mantissa: i128 = dec["mantissa"]
        .as_str()
        .expect("mantissa")
        .parse()
        .expect("parses");
    let places = dec["places"].as_u64().expect("places") as u32;
    mantissa / 10_i128.pow(places)
}

fn opening_cash(run: &Value) -> i128 {
    dollars(&run["cash"]["opening"])
}

#[tokio::test]
async fn a_repeated_run_answers_the_same() {
    let (_vault, state) = books(MAIN);
    let first = run(&state).await;
    assert_eq!(opening_cash(&first), 9_000);
    assert_eq!(run(&state).await, first);
}

#[tokio::test]
async fn an_edit_adding_a_real_transaction_is_seen_by_the_next_run() {
    let (_vault, state) = books(MAIN);
    let before = run(&state).await;
    assert_eq!(opening_cash(&before), 9_000);

    let body = json!({
        "date": "2026-09-15",
        "description": "deposit",
        "postings": [
            { "account": "assets:checking",
              "amount": { "commodity": "$", "quantity": { "mantissa": "50000", "places": 2 } } },
            { "account": "equity:opening" }
        ]
    });
    let (status, response) = request(&state, "POST", "/api/transactions", body).await;
    assert_eq!(status, StatusCode::CREATED, "add: {response}");

    let after = run(&state).await;
    assert_eq!(
        opening_cash(&after),
        9_500,
        "the new deposit is in the opening"
    );
    assert_ne!(after["balanceSheet"], before["balanceSheet"]);
    assert_ne!(after["accounts"], before["accounts"]);
}

#[tokio::test]
async fn an_account_declaration_changed_on_disk_is_seen_after_refresh() {
    let (vault, state) = books(MAIN);
    assert_eq!(opening_cash(&run(&state).await), 9_000);

    // Savings becomes a cash account: the runway's cash now includes it.
    vault.modify_in_place(
        "books/main.journal",
        MAIN.replace(
            "account assets:savings   ; type: A",
            "account assets:savings   ; type: C",
        ),
    );
    assert_eq!(state.refresh().expect("refreshes"), Refreshed::Reloaded);
    assert_eq!(opening_cash(&run(&state).await), 14_000);
}

#[tokio::test]
async fn a_reload_is_seen_by_the_next_run() {
    let (vault, state) = books(MAIN);
    assert_eq!(opening_cash(&run(&state).await), 9_000);

    vault.modify_externally(
        "books/main.journal",
        format!("{MAIN}\n2026-09-20 refund\n    assets:checking   $250.00\n    expenses:rent\n"),
    );
    state.reload().expect("reloads");
    assert_eq!(opening_cash(&run(&state).await), 9_250);
}
