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
use super::tests::{app_on, app_on_prepared, authed, json_of, post_json};

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

/// Noon UTC on `date`: the same local day from UTC−12 to UTC+11.
fn noon(date: &str) -> std::time::SystemTime {
    let days = ledgeline_core::reports::periods::days_from_iso(date);
    let seconds = u64::try_from(days * 86_400 + 12 * 3_600).expect("after 1970");
    std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds)
}

/// Every file under `dir` modified at `at`.
fn touch_all(dir: &Path, at: std::time::SystemTime) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            touch_all(&path, at);
        } else {
            let file = std::fs::File::options()
                .write(true)
                .open(&path)
                .expect("opens");
            file.set_modified(at).expect("sets the time");
        }
    }
}

/// The file the golden shows as edited last (R15): saved the day before
/// `TODAY`, every other file long before.
fn edited_last(fixture: &str) -> Option<&'static str> {
    match fixture {
        "accrual" => Some("invoices/2026-08-01 Wingtip Invoice 106.md"),
        "cash" => Some("invoices/2026-10-20 Contoso Invoice 201.md"),
        _ => None,
    }
}

/// The app on a copy of `fixture` with fixed file times, so "recently
/// updated" doesn't depend on when the fixture was checked out.
fn dashboard_app(fixture: &str) -> super::tests::App {
    app_on_prepared(fixture, None, |books| {
        touch_all(books, noon("2026-10-01"));
        if let Some(edited) = edited_last(fixture) {
            let file = std::fs::File::options()
                .write(true)
                .open(books.join(edited))
                .expect("opens");
            file.set_modified(noon("2026-11-21"))
                .expect("sets the time");
        }
    })
}

async fn dashboard_on(fixture: &str) -> Value {
    let app = dashboard_app(fixture);
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
    assert_eq!(keys, ["current", "1-30", "31-60", "61-90", "91+"]);
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
    // Paid in the last 30 days (the setting's default), through today.
    assert_eq!(body["paid_window_days"], 30);
    assert_eq!(
        (
            &body["paid"]["days"],
            &body["paid"]["from"],
            &body["paid"]["to"]
        ),
        (&json!(30), &json!("2026-10-24"), &json!(TODAY))
    );
    // Recently updated: latest first, at most eight, 106 edited yesterday first.
    let recent = body["recent"].as_array().expect("recent");
    assert!(!recent.is_empty() && recent.len() <= 6);
    assert_eq!(
        (
            &recent[0]["number"],
            &recent[0]["event"],
            &recent[0]["updated"]
        ),
        (&json!("106"), &json!("edited"), &json!("2026-11-21"))
    );
    assert_eq!(recent[0]["status"], "overdue");
    let updated: Vec<&str> = recent
        .iter()
        .filter_map(|r| r["updated"].as_str())
        .collect();
    assert!(updated.windows(2).all(|w| w[0] >= w[1]), "{updated:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cash_dashboard_is_its_golden() {
    let body = dashboard_on("cash").await;
    snapshot("cash-api-dashboard.json", &body);
    assert_eq!(body["basis"], "cash");
    assert_eq!(body["recent"][0]["number"], "201");
    assert_eq!(body["recognition"], Value::Null, "no recognition file");
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_books_have_an_empty_dashboard() {
    let body = dashboard_on("empty").await;
    assert_eq!(body["invoices"], 0);
    assert_eq!(body["commodities"], json!([]));
    assert_eq!(body["drafts"]["count"], 0);
    assert_eq!(body["recent"], json!([]));
    assert_eq!(body["paid"]["commodities"], json!([]));
    assert_eq!(
        body["average_days_to_pay"],
        json!({"invoices": 0, "average": null})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_paid_window_is_the_pages_to_pick() {
    let app = dashboard_app("accrual");
    let paid = |body: &Value| body["paid"].clone();
    let (status, year) = json_of(
        &app,
        authed(&format!("/api/dashboard?today={TODAY}&paid_days=365")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{year}");
    assert_eq!(
        (&paid(&year)["days"], &paid(&year)["from"]),
        (&json!(365), &json!("2025-11-23"))
    );
    // The setting stays what the page defaults to.
    assert_eq!(year["paid_window_days"], 30);
    let (_, week) = json_of(
        &app,
        authed(&format!("/api/dashboard?today={TODAY}&paid_days=7")),
    )
    .await;
    assert_eq!(paid(&week)["from"], "2026-11-16");
    // A wider window holds at least as much, per commodity and count.
    let total = |v: &Value| -> (f64, u64) {
        v["commodities"]
            .as_array()
            .expect("commodities")
            .iter()
            .fold((0.0, 0), |(a, n), c| {
                (
                    a + c["amount"]
                        .as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0),
                    n + c["count"].as_u64().unwrap_or(0),
                )
            })
    };
    let (week, year) = (total(&paid(&week)), total(&paid(&year)));
    assert!(week.0 <= year.0 && week.1 <= year.1, "{week:?} {year:?}");
    assert!(year.1 > 0, "the fixture has payments in the year");
    for bad in ["0", "abc", "4000", "-1"] {
        let (status, _) = json_of(
            &app,
            authed(&format!("/api/dashboard?today={TODAY}&paid_days={bad}")),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "paid_days={bad}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_paid_figure_filters_the_list_by_payment_date() {
    let app = dashboard_app("accrual");
    let (_, body) = json_of(
        &app,
        authed(&format!("/api/dashboard?today={TODAY}&paid_days=365")),
    )
    .await;
    let paid = &body["paid"];
    let usd = &paid["commodities"][0];
    let filed = usd["count"].as_u64().expect("count") - usd["unfiled"].as_u64().expect("unfiled");
    let (status, list) = json_of(
        &app,
        authed(&format!(
            "/api/invoices?today={TODAY}&paid_from={}&paid_to={}",
            paid["from"].as_str().expect("from"),
            paid["to"].as_str().expect("to")
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let rows = list.as_array().expect("rows");
    assert_eq!(rows.len() as u64, filed, "{list}");
    let (status, _) = json_of(&app, authed("/api/invoices?paid_from=yesterday")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
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
