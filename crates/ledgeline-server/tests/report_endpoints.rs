//! End-to-end HTTP tests for the Phase-3 native report + budget endpoints.
//!
//! Drives the real axum `Router` through `tower`'s `oneshot` (no sockets), then
//! spot-checks each endpoint's JSON against the committed hledger goldens under
//! `fixtures/golden/` and `fixtures/budget/` — reusing the same
//! sum-lots-per-commodity + canonical `(mantissa, places)` reconciliation as the
//! core golden suites (`reports_golden.rs`, `budget_golden.rs`):
//!   - hledger keeps different-cost-basis lots as separate amounts; we sum per
//!     commodity with exact `Dec` math.
//!   - comparisons are on canonical `(mantissa, places)` (trailing zeros stripped,
//!     zero commodities dropped) — never floats.
//!   - net worth uses `--infer-market-prices`: the endpoint infers prices from
//!     `@`/`@@` costs (incl. the GLD gift's reverse `@ 0.005 GLD`), so every held
//!     commodity is valued and `meta` is absent.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::fixtures_dir;
use http_body_util::BodyExt;
use ledgeline_core::{Dec, Journal, parse_journal};
use ledgeline_server::app;
use serde_json::Value;
use std::collections::BTreeMap;
use tower::ServiceExt;

// ---- fixtures ----

fn sample_journal() -> Journal {
    common::fixture_journal()
}

fn budget_fixture_journal(name: &str) -> Journal {
    let path = fixtures_dir().join("budget").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    parse_journal(&text, &path.to_string_lossy()).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

fn golden(dir: &str, name: &str) -> Value {
    let path = fixtures_dir().join(dir).join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

// ---- HTTP driver ----

/// Issue `GET uri` (with an `Origin` header) against a fresh app over `journal`,
/// returning status, the `access-control-allow-origin` header, and — for a 200 —
/// the parsed JSON body (`Value::Null` otherwise, since errors are plain text).
async fn get_on(journal: &Journal, uri: &str) -> (StatusCode, Option<String>, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::ORIGIN, "https://spa.example")
        .body(Body::empty())
        .expect("request builds");
    let response = app(journal)
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let allow_origin = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    let body = if status == StatusCode::OK {
        serde_json::from_slice(&bytes).expect("body is JSON")
    } else {
        Value::Null
    };
    (status, allow_origin, body)
}

async fn body_ok(journal: &Journal, uri: &str) -> Value {
    let (status, _, body) = get_on(journal, uri).await;
    assert_eq!(status, StatusCode::OK, "GET {uri} should be 200 OK");
    body
}

// ---- canonical exact comparison (identical to the core golden suites) ----

type Canon = BTreeMap<String, (i128, u32)>;

fn canon(mut mantissa: i128, mut places: u32) -> (i128, u32) {
    while places > 0 && mantissa % 10 == 0 {
        mantissa /= 10;
        places -= 1;
    }
    (mantissa, places)
}

/// Our wire `MixedAmount` (`{commodity: {mantissa, places}}`) → canonical map.
fn wire_ma(value: &Value) -> Canon {
    value
        .as_object()
        .expect("mixed amount is an object")
        .iter()
        .map(|(commodity, dec)| {
            let mantissa: i128 = dec["mantissa"]
                .as_str()
                .expect("mantissa string")
                .parse()
                .expect("mantissa");
            let places = u32::try_from(dec["places"].as_u64().expect("places")).unwrap();
            (commodity.clone(), canon(mantissa, places))
        })
        .collect()
}

/// Sum a golden hledger MixedAmount (array of `GAmount`) per commodity with exact
/// `Dec` math, then canonicalize and drop zeros — the golden side of a compare.
fn sum_golden(amounts: &Value) -> Canon {
    let mut merged: BTreeMap<String, Dec> = BTreeMap::new();
    for amount in amounts.as_array().expect("amount array") {
        let commodity = amount["acommodity"]
            .as_str()
            .expect("acommodity")
            .to_string();
        let quantity = &amount["aquantity"];
        let mantissa = i128::from(
            quantity["decimalMantissa"]
                .as_i64()
                .expect("decimalMantissa"),
        );
        let places =
            u32::try_from(quantity["decimalPlaces"].as_u64().expect("decimalPlaces")).unwrap();
        let dec = Dec::new(mantissa, places);
        merged
            .entry(commodity)
            .and_modify(|prev| *prev = prev.add(dec).expect("no overflow"))
            .or_insert(dec);
    }
    merged
        .into_iter()
        .map(|(commodity, dec)| (commodity, canon(dec.mantissa, dec.places)))
        .filter(|(_, (mantissa, _))| *mantissa != 0)
        .collect()
}

/// Find a named section in a `SectionedReport` body.
fn section<'a>(body: &'a Value, title: &str) -> &'a Value {
    body["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["title"] == title)
        .unwrap_or_else(|| panic!("section {title} exists"))
}

// ===========================================================================
// Balance sheet — vs fixtures/golden/bs-d1.json
// ===========================================================================

#[tokio::test]
async fn balancesheet_matches_bs_d1_golden() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/balancesheet?asOf=2026-06-30&depth=1",
    )
    .await;

    assert_eq!(body["asOf"], "2026-06-30");
    assert!(
        body.get("from").is_none(),
        "point-in-time report omits from"
    );
    assert!(body.get("to").is_none(), "point-in-time report omits to");

    let g = golden("golden", "bs-d1.json");
    let g_assets = &g["cbrSubreports"][0];
    let g_liab = &g["cbrSubreports"][1];
    assert_eq!(g_assets[0], "Assets");
    assert_eq!(g_liab[0], "Liabilities");

    assert_eq!(
        wire_ma(&section(&body, "Assets")["total"]),
        sum_golden(&g_assets[1]["prTotals"]["prrAmounts"][0]),
        "assets total"
    );
    assert_eq!(
        wire_ma(&section(&body, "Liabilities")["total"]),
        sum_golden(&g_liab[1]["prTotals"]["prrAmounts"][0]),
        "liabilities total"
    );
    assert_eq!(
        wire_ma(&body["grandTotal"]),
        sum_golden(&g["cbrTotals"]["prrAmounts"][0]),
        "grand total"
    );

    // Depth 1 clamps each section to a single root row whose inclusive equals the
    // section total.
    let assets = section(&body, "Assets");
    assert_eq!(assets["rows"].as_array().unwrap().len(), 1);
    assert_eq!(assets["rows"][0]["account"], "assets");
    assert_eq!(assets["rows"][0]["own"], serde_json::json!({}));
    assert_eq!(
        wire_ma(&assets["rows"][0]["inclusive"]),
        wire_ma(&assets["total"])
    );
}

// ===========================================================================
// Income statement — vs fixtures/golden/is-d2.json
// ===========================================================================

#[tokio::test]
async fn incomestatement_matches_is_d2_golden() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/incomestatement?from=2026-01-01&to=2026-06-30&depth=2",
    )
    .await;

    assert_eq!(body["from"], "2026-01-01");
    assert_eq!(body["to"], "2026-06-30");
    assert!(body.get("asOf").is_none(), "range report omits asOf");

    let g = golden("golden", "is-d2.json");
    let g_rev = &g["cbrSubreports"][0];
    let g_exp = &g["cbrSubreports"][1];
    assert_eq!(g_rev[0], "Revenues");
    assert_eq!(g_exp[0], "Expenses");

    assert_eq!(
        wire_ma(&section(&body, "Revenues")["total"]),
        sum_golden(&g_rev[1]["prTotals"]["prrAmounts"][0]),
        "revenues total"
    );
    assert_eq!(
        wire_ma(&section(&body, "Expenses")["total"]),
        sum_golden(&g_exp[1]["prTotals"]["prrAmounts"][0]),
        "expenses total"
    );
    assert_eq!(
        wire_ma(&body["grandTotal"]),
        sum_golden(&g["cbrTotals"]["prrAmounts"][0]),
        "net income (grand total)"
    );
}

// ===========================================================================
// Cash flow — vs fixtures/golden/cf-monthly.json (per-bucket totals)
// ===========================================================================

#[tokio::test]
async fn cashflow_matches_cf_monthly_golden() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/cashflow?end=2026-06-30&interval=monthly&count=6&depth=99",
    )
    .await;

    let buckets: Vec<&str> = body["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap())
        .collect();
    assert_eq!(
        buckets,
        [
            "2026-01", "2026-02", "2026-03", "2026-04", "2026-05", "2026-06"
        ]
    );

    let g = golden("golden", "cf-monthly.json");
    let sub = &g["cbrSubreports"][0][1];
    let totals = body["totals"].as_array().unwrap();
    for (i, bucket) in buckets.iter().enumerate() {
        assert_eq!(
            wire_ma(&totals[i]),
            sum_golden(&sub["prTotals"]["prrAmounts"][i]),
            "cash-flow total bucket {bucket}"
        );
    }
}

// ===========================================================================
// Net worth — vs fixtures/golden/networth-spot.json (--infer-market-prices)
// ===========================================================================

#[tokio::test]
async fn networth_matches_networth_spot_golden() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/networth?end=2026-06-30&interval=monthly&count=1&depth=1",
    )
    .await;

    let buckets: Vec<&str> = body["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap())
        .collect();
    assert_eq!(buckets, ["2026-06"]);

    // Inference values every held commodity, so nothing is left unpriced.
    assert!(body["meta"].is_null(), "meta should be absent");

    let g = golden("golden", "networth-spot.json");

    // Group golden leaf rows by root account.
    let mut by_root: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in g[0].as_array().expect("bal rows") {
        let account = row[0].as_str().expect("row account");
        let root = account.split(':').next().unwrap().to_string();
        for amount in row[3].as_array().expect("row amounts") {
            by_root
                .entry(root.clone())
                .or_default()
                .push(amount.clone());
        }
    }

    let my_row = |root: &str| -> Canon {
        let row = body["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["account"] == root)
            .unwrap_or_else(|| panic!("net-worth row {root} exists"));
        wire_ma(&row["values"][0])
    };

    for (root, amounts) in &by_root {
        assert_eq!(
            my_row(root),
            sum_golden(&Value::Array(amounts.clone())),
            "valued net worth for {root}"
        );
    }

    let golden_total = g[1].as_array().expect("total amounts").clone();
    assert_eq!(
        wire_ma(&body["totals"][0]),
        sum_golden(&Value::Array(golden_total)),
        "net worth total"
    );
}

/// The `depth` query param surfaces valued sub-account rows (e.g. cost-priced
/// `assets:broker:taxable:aapl`).
#[tokio::test]
async fn networth_depth_surfaces_valued_sub_accounts() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/networth?end=2026-06-30&interval=monthly&count=1&depth=5",
    )
    .await;

    let g = golden("golden", "networth-d5.json");
    let aapl = g[0]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row[0] == "assets:broker:taxable:aapl")
        .expect("golden has the aapl leaf");

    let row = body["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["account"] == "assets:broker:taxable:aapl")
        .expect("depth-5 output has the aapl sub-account");
    assert_eq!(wire_ma(&row["values"][0]), sum_golden(&aapl[3]));
}

// ===========================================================================
// Budget — vs fixtures/budget/basic.budget.json (full cell parity)
// ===========================================================================

/// Assert one of our JSON cells against a golden `[actual, goal|null]` pair.
fn assert_budget_cell(label: &str, cell: &Value, golden_actual: &Value, golden_goal: &Value) {
    assert_eq!(
        wire_ma(&cell["actual"]),
        sum_golden(golden_actual),
        "{label} actual"
    );
    match (cell["goal"].is_null(), golden_goal.is_null()) {
        (true, true) => {}
        (false, false) => {
            assert_eq!(
                wire_ma(&cell["goal"]),
                sum_golden(golden_goal),
                "{label} goal"
            );
        }
        (ours_null, golden_null) => panic!(
            "{label} goal presence mismatch: ours_null={ours_null} golden_null={golden_null}"
        ),
    }
}

#[tokio::test]
async fn budget_matches_basic_golden() {
    let journal = budget_fixture_journal("basic.journal");
    // -b 2026-01-01 -e 2026-03-01 ≙ 2 monthly buckets ending 2026-02-28.
    let body = body_ok(
        &journal,
        "/api/budget?end=2026-02-28&interval=monthly&count=2",
    )
    .await;

    let buckets: Vec<&str> = body["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap())
        .collect();
    assert_eq!(buckets, ["2026-01", "2026-02"]);

    let g = golden("budget", "basic.budget.json");

    // Full row-name set must match (parents, leaves, and <unbudgeted>).
    let mut ours: Vec<String> = body["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["account"].as_str().unwrap().to_string())
        .collect();
    let mut theirs: Vec<String> = g["prRows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["prrName"].as_str().unwrap().to_string())
        .collect();
    ours.sort();
    theirs.sort();
    assert_eq!(ours, theirs, "budget row account set");

    let my_row = |account: &str| -> Value {
        body["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["account"] == account)
            .unwrap_or_else(|| panic!("budget row {account} exists"))
            .clone()
    };

    for grow in g["prRows"].as_array().unwrap() {
        let name = grow["prrName"].as_str().unwrap();
        let row = my_row(name);
        for (i, bucket) in buckets.iter().enumerate() {
            let gcell = &grow["prrAmounts"][i];
            assert_budget_cell(
                &format!("row {name} bucket {bucket}"),
                &row["cells"][i],
                &gcell[0],
                &gcell[1],
            );
        }
    }

    // Totals row.
    for (i, bucket) in buckets.iter().enumerate() {
        let gcell = &g["prTotals"]["prrAmounts"][i];
        assert_budget_cell(
            &format!("totals bucket {bucket}"),
            &body["totals"][i],
            &gcell[0],
            &gcell[1],
        );
    }

    // A concrete tie to the committed golden numbers (see basic.budget.txt):
    // expenses:food actual/goal are $352/$400 (Jan) and $390/$400 (Feb).
    let food = my_row("expenses:food");
    assert_eq!(
        wire_ma(&food["cells"][0]["actual"]),
        Canon::from([("$".into(), (352, 0))])
    );
    assert_eq!(
        wire_ma(&food["cells"][0]["goal"]),
        Canon::from([("$".into(), (400, 0))])
    );
    assert_eq!(
        wire_ma(&food["cells"][1]["actual"]),
        Canon::from([("$".into(), (390, 0))])
    );
    // <unbudgeted> carries the cash legs with a null goal.
    let unbudgeted = my_row("<unbudgeted>");
    assert_eq!(
        wire_ma(&unbudgeted["cells"][0]["actual"]),
        Canon::from([("$".into(), (-375, 0))])
    );
    assert!(unbudgeted["cells"][0]["goal"].is_null());
}

// ===========================================================================
// Cross-cutting: defaults, bad params, CORS
// ===========================================================================

/// Every report endpoint answers a no-query (all-defaults) request with 200.
#[tokio::test]
async fn default_params_return_ok() {
    let journal = sample_journal();
    for uri in [
        "/api/reports/balancesheet",
        "/api/reports/incomestatement",
        "/api/reports/cashflow",
        "/api/reports/networth",
        "/api/budget",
    ] {
        let (status, _, _) = get_on(&journal, uri).await;
        assert_eq!(status, StatusCode::OK, "GET {uri} (no query) should be 200");
    }
}

/// An unrecognized `interval` is a client error (400), not a panic.
#[tokio::test]
async fn bad_interval_is_400() {
    let journal = sample_journal();
    let (status, _, _) = get_on(&journal, "/api/reports/cashflow?interval=fortnightly").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A non-numeric depth is rejected by the query extractor, also 400.
    let (status, _, _) = get_on(&journal, "/api/reports/balancesheet?depth=lots").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The permissive CORS layer covers the native report routes too.
#[tokio::test]
async fn report_get_carries_permissive_cors_header() {
    let journal = sample_journal();
    let (status, allow_origin, _) = get_on(
        &journal,
        "/api/reports/balancesheet?asOf=2026-06-30&depth=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(allow_origin.as_deref(), Some("*"));
}
