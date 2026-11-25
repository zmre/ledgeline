//! `GET /api/dashboard` and `POST /api/ledgeline/open` (PLAN 5a), in-process
//! over private fixture copies. The whole answer on the accrual and cash
//! fixtures is a golden in `tests/snapshots/` (as the CLI's are):
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p ledgeline-invoicer dashboard   # re-record
//! ```

use std::path::Path;
use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{Value, json};

use super::native::Native;
use super::native::mock::MockNative;
use super::tests::{app_on, authed, json_of, post_json};

/// The date every golden is computed on.
const TODAY: &str = "2026-11-22";

/// Compare `actual` with `tests/snapshots/<name>`, or write it there when
/// `UPDATE_SNAPSHOTS` is set.
fn snapshot(name: &str, actual: &Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(name);
    let actual = format!(
        "{}\n",
        serde_json::to_string_pretty(actual).expect("serializes")
    );
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &actual).expect("writes the snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no snapshot {}; run with UPDATE_SNAPSHOTS=1 to record it",
            path.display()
        )
    });
    assert!(
        expected == actual,
        "{name} differs from its snapshot (UPDATE_SNAPSHOTS=1 re-records):\n--- expected\n{expected}\n--- actual\n{actual}"
    );
}

async fn dashboard_on(fixture: &str) -> Value {
    let app = app_on(fixture, None);
    let (status, body) = json_of(&app, authed(&format!("/api/dashboard?today={TODAY}"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn the_accrual_dashboard_is_its_golden() {
    let body = dashboard_on("accrual").await;
    snapshot("accrual-api-dashboard.json", &body);
    // The figures the SPA and its e2e lean on, spelled out.
    assert_eq!(body["basis"], "accrual");
    assert_eq!(body["open_ledgeline"], false, "no window here");
    let usd = &body["commodities"][0];
    assert_eq!(usd["commodity"], "$");
    let keys: Vec<&str> = usd["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .filter_map(|b| b["key"].as_str())
        .collect();
    assert_eq!(keys, ["current", "1-30", "31-60", "61-90", "91-"]);
    // Overdue plus not yet due is the total, and the bar's scale is at least $10,000.
    let amount = |v: &Value| {
        v["amount"]
            .as_str()
            .expect("amount")
            .parse::<f64>()
            .expect("number")
    };
    let total = amount(&usd["outstanding"]);
    assert!((amount(&usd["overdue"]) + amount(&usd["not_due"]) - total).abs() < 0.005);
    assert!(amount(&usd["scale"]) >= total.max(10_000.0));
    let buckets: f64 = usd["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .map(amount)
        .sum();
    assert!((buckets - total).abs() < 0.005);
    assert!(
        body["drafts"]["count"].as_u64().is_some_and(|n| n >= 1),
        "108 is a draft"
    );
    assert!(
        body["recognition"].is_object(),
        "the fixture has a recognition file"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cash_dashboard_is_its_golden() {
    let body = dashboard_on("cash").await;
    snapshot("cash-api-dashboard.json", &body);
    assert_eq!(body["basis"], "cash");
    assert_eq!(body["recognition"], Value::Null, "no recognition file");
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_books_have_an_empty_dashboard() {
    let body = dashboard_on("empty").await;
    assert_eq!(body["invoices"], 0);
    assert_eq!(body["commodities"], json!([]));
    assert_eq!(body["drafts"]["count"], 0);
    assert_eq!(
        body["average_days_to_pay"],
        json!({"invoices": 0, "average": null})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_today_is_refused() {
    let app = app_on("accrual", None);
    let (status, _) = json_of(&app, authed("/api/dashboard?today=tomorrow")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_books_open_in_ledgeline_from_the_window_only() {
    let app = app_on("accrual", None);
    let (status, body) = json_of(&app, post_json("/api/ledgeline/open", &json!({}))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");

    let native = Arc::new(MockNative::everything());
    let app = app_on("accrual", Some(Arc::clone(&native) as Arc<dyn Native>));
    let (status, body) = json_of(&app, authed(&format!("/api/dashboard?today={TODAY}"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["open_ledgeline"], true);
    let (status, body) = json_of(&app, post_json("/api/ledgeline/open", &json!({}))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"done": true})));
    let opened = native.ledgeline.lock().expect("lock").clone();
    assert_eq!(opened, [app.books.journal().to_path_buf()]);
}
