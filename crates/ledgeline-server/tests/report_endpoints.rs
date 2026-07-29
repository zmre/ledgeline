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

/// Issue `GET uri` and return the status plus the PLAIN-TEXT body — for the
/// rejection tests, which assert on the message a caller actually sees.
async fn get_error(journal: &Journal, uri: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    let response = app(journal)
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
    (status, String::from_utf8_lossy(&bytes).into_owned())
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
// Insights dashboard — reconciled against the income-statement + net-worth
// endpoints for the same split sub-ranges (no new goldens needed).
// ===========================================================================

#[tokio::test]
async fn insights_reconciles_with_income_statement_and_networth() {
    let journal = sample_journal();
    // 24-month, month-aligned span → a clean 12/12 calendar split at 2025-06-30.
    let ins = body_ok(&journal, "/api/insights?start=2024-07-01&end=2026-06-30").await;

    assert_eq!(ins["period"]["start"], "2024-07-01");
    assert_eq!(ins["period"]["mid"], "2025-06-30");
    assert_eq!(ins["period"]["currStart"], "2025-07-01");
    assert_eq!(ins["period"]["end"], "2026-06-30");
    assert_eq!(ins["base"], "$");

    // Revenue / expenses for each period must equal the income statement over the
    // corresponding half (section totals are depth-independent).
    let is_curr = body_ok(
        &journal,
        "/api/reports/incomestatement?from=2025-07-01&to=2026-06-30&depth=1",
    )
    .await;
    assert_eq!(
        wire_ma(&ins["revenue"]["current"]),
        wire_ma(&section(&is_curr, "Revenues")["total"]),
        "revenue current == income-statement Revenues (current half)"
    );
    assert_eq!(
        wire_ma(&ins["expenses"]["current"]),
        wire_ma(&section(&is_curr, "Expenses")["total"]),
        "expenses current == income-statement Expenses (current half)"
    );

    let is_prev = body_ok(
        &journal,
        "/api/reports/incomestatement?from=2024-07-01&to=2025-06-30&depth=1",
    )
    .await;
    assert_eq!(
        wire_ma(&ins["revenue"]["previous"]),
        wire_ma(&section(&is_prev, "Revenues")["total"]),
        "revenue previous == income-statement Revenues (previous half)"
    );
    assert_eq!(
        wire_ma(&ins["expenses"]["previous"]),
        wire_ma(&section(&is_prev, "Expenses")["total"]),
        "expenses previous == income-statement Expenses (previous half)"
    );

    // Net worth at the current period end must equal the net-worth report's
    // single-bucket total as of the same date.
    let nw = body_ok(
        &journal,
        "/api/reports/networth?end=2026-06-30&interval=monthly&count=1&depth=1",
    )
    .await;
    assert_eq!(
        wire_ma(&ins["netWorth"]["current"]),
        wire_ma(&nw["totals"][0]),
        "net worth current == net-worth report total at end"
    );

    // Structural checks on the remaining boxes.
    assert_eq!(ins["costOfLiving"]["monthsCurrent"], 12);
    assert_eq!(ins["costOfLiving"]["monthsPrevious"], 12);
    assert!(
        ins["investment"]["current"].is_object(),
        "investment box present"
    );
    assert!(
        ins["cashBalance"]["current"].is_object(),
        "cash box present"
    );

    // List boxes (7–10) are present; the 24-month sample has expense changes and
    // large transactions to surface.
    assert!(
        ins["topTxns"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty()),
        "top transactions present"
    );
    assert!(ins["expenseChanges"].is_array(), "expense changes present");
    assert!(ins["revenueChanges"].is_array(), "revenue changes present");
    assert!(ins["movers"].is_array(), "movers present");
}

/// The cost-of-living exclusion list is honoured: excluding every expense root
/// zeroes the cost-of-living totals while leaving the raw expenses box intact.
#[tokio::test]
async fn insights_cost_exclude_param_drops_expenses() {
    let journal = sample_journal();
    let ins = body_ok(
        &journal,
        "/api/insights?start=2024-07-01&end=2026-06-30&exclude=expenses",
    )
    .await;
    assert_eq!(
        ins["costOfLiving"]["currentTotal"],
        serde_json::json!({}),
        "excluding the whole `expenses` root zeroes cost of living"
    );
    assert!(
        ins["expenses"]["current"]
            .as_object()
            .is_some_and(|m| !m.is_empty()),
        "the Expenses box still reflects real spending"
    );
}

// ===========================================================================
// Subscriptions — the detector's own fixture, over HTTP
// ===========================================================================

#[tokio::test]
async fn subscriptions_endpoint_reports_monthly_and_annual_charges() {
    let path = fixtures_dir().join("subscriptions").join("basic.journal");
    let text = std::fs::read_to_string(&path).expect("basic.journal readable");
    let journal = parse_journal(&text, &path.to_string_lossy()).expect("basic.journal parses");

    let body = body_ok(&journal, "/api/subscriptions?asOf=2026-06-30").await;
    assert_eq!(body["asOf"], "2026-06-30");
    assert_eq!(body["lookbackStart"], "2024-06-30");

    let names = |key: &str| -> Vec<String> {
        body[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} is an array"))
            .iter()
            .map(|row| row["payee"].as_str().expect("payee").to_string())
            .collect()
    };
    assert_eq!(
        names("monthly"),
        ["Twilio", "Netflix", "Spotify", "Apple", "Backblaze"]
    );
    assert_eq!(names("annual"), ["State Farm", "Hover"]);
    // Cancelled charges are retired: Hulu stopped billing a card that stayed
    // current, so its silence is real rather than a missing import.
    assert!(!names("monthly").contains(&"Hulu".to_string()));
    // Hand-tagged entries are flagged so the UI can tell them apart, and
    // `subscription:false` keeps Dropbox off despite being detectable.
    let twilio = &body["monthly"][0];
    assert_eq!(twilio["payee"], "Twilio");
    assert_eq!(twilio["manual"], true);
    assert_eq!(body["monthly"][1]["manual"], false);
    assert!(!names("monthly").contains(&"Dropbox".to_string()));

    // Netflix: $15.99/mo → $191.88/yr, with its next charge projected forward.
    let netflix = body["monthly"]
        .as_array()
        .expect("monthly array")
        .iter()
        .find(|row| row["payee"] == "Netflix")
        .expect("Netflix present");
    assert_eq!(netflix["cadence"], "monthly");
    assert_eq!(netflix["typicalAmount"]["mantissa"], "1599");
    assert_eq!(netflix["annualizedCost"]["mantissa"], "19188");
    assert_eq!(netflix["occurrences"], 18);
    assert_eq!(netflix["nextExpected"], "2026-07-15");
}

/// The endpoint applies the default description exclusions (mortgage), and the
/// list is overridable per request.
#[tokio::test]
async fn subscriptions_exclude_mortgage_by_default() {
    let path = fixtures_dir().join("subscriptions").join("basic.journal");
    let text = std::fs::read_to_string(&path).expect("basic.journal readable");
    let journal = parse_journal(&text, &path.to_string_lossy()).expect("basic.journal parses");

    let has_mortgage = |body: &Value| {
        body["monthly"]
            .as_array()
            .expect("monthly array")
            .iter()
            .any(|row| row["payee"] == "Wells Fargo")
    };

    let default = body_ok(&journal, "/api/subscriptions?asOf=2026-06-30").await;
    assert!(!has_mortgage(&default), "mortgage excluded by default");

    // An explicitly empty list excludes nothing, so it comes back.
    let unfiltered = body_ok(&journal, "/api/subscriptions?asOf=2026-06-30&excludeDesc=").await;
    assert!(has_mortgage(&unfiltered));

    // A caller-supplied list replaces the default, so mortgage returns while the
    // named payee drops out instead.
    let custom = body_ok(
        &journal,
        "/api/subscriptions?asOf=2026-06-30&excludeDesc=netflix",
    )
    .await;
    assert!(has_mortgage(&custom));
    assert!(
        !custom["monthly"]
            .as_array()
            .expect("monthly array")
            .iter()
            .any(|row| row["payee"] == "Netflix"),
    );
}

/// The detection thresholds are query-tunable, not baked in.
#[tokio::test]
async fn subscriptions_thresholds_are_tunable() {
    let path = fixtures_dir().join("subscriptions").join("basic.journal");
    let text = std::fs::read_to_string(&path).expect("basic.journal readable");
    let journal = parse_journal(&text, &path.to_string_lossy()).expect("basic.journal parses");

    // The gym has only 4 charges — invisible by default, surfaced at minMonthly=4.
    let strict = body_ok(&journal, "/api/subscriptions?asOf=2026-06-30").await;
    let relaxed = body_ok(&journal, "/api/subscriptions?asOf=2026-06-30&minMonthly=4").await;
    let has_gym = |body: &Value| {
        body["monthly"]
            .as_array()
            .expect("monthly array")
            .iter()
            .any(|row| row["payee"] == "Gold's Gym")
    };
    assert!(!has_gym(&strict));
    assert!(has_gym(&relaxed));
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
        "/api/insights",
        "/api/subscriptions",
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

/// Every endpoint that takes `count`. All four reached `Vec::with_capacity(n)`
/// with an unclamped user `usize`; `/api/budget` additionally indexed
/// `buckets[0]`.
const COUNT_ENDPOINTS: [&str; 4] = [
    "/api/budget",
    "/api/reports/cashflow",
    "/api/reports/networth",
    "/api/holdings/series",
];

/// SEC-2: an out-of-range `count` is a client error (400) with a clear message —
/// NOT a panic, and not a silently clamped report that would render a
/// plausible-looking chart nobody asked for.
///
/// `count=0` used to panic `index out of bounds` in `budget_report`;
/// `count=18446744073709551615` used to panic `capacity overflow` in
/// `last_n_buckets` on all four. Both returned a 500 once `CatchPanicLayer` was
/// installed; neither should reach a panic at all.
#[tokio::test]
async fn out_of_range_count_is_400_on_every_endpoint() {
    let journal = sample_journal();
    for endpoint in COUNT_ENDPOINTS {
        for count in ["0", "1201", "18446744073709551615"] {
            let uri = format!("{endpoint}?count={count}");
            let (status, _, _) = get_on(&journal, &uri).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "GET {uri} must be a 400, not a panic or a clamped report"
            );
        }
        // Negative and non-numeric counts are rejected by the query extractor.
        for count in ["-1", "lots"] {
            let uri = format!("{endpoint}?count={count}");
            let (status, _, _) = get_on(&journal, &uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "GET {uri} must be a 400");
        }
    }
}

/// The boundaries of the accepted range still serve a real report, so the
/// validation rejects only what is genuinely out of range.
#[tokio::test]
async fn in_range_count_is_accepted_on_every_endpoint() {
    let journal = sample_journal();
    for endpoint in COUNT_ENDPOINTS {
        for count in ["1", "12", "1200"] {
            let uri = format!("{endpoint}?count={count}");
            let (status, _, _) = get_on(&journal, &uri).await;
            assert_eq!(status, StatusCode::OK, "GET {uri} must still be a 200");
        }
    }
}

/// `count` is optional: omitting it keeps the documented default rather than
/// tripping the new range check.
#[tokio::test]
async fn absent_count_still_uses_the_default() {
    let journal = sample_journal();
    for endpoint in COUNT_ENDPOINTS {
        let (status, _, _) = get_on(&journal, endpoint).await;
        assert_eq!(status, StatusCode::OK, "GET {endpoint} must be a 200");
    }
    // The default is 12 buckets, so an explicit 12 matches an omitted count.
    assert_eq!(
        body_ok(&journal, "/api/budget").await,
        body_ok(&journal, "/api/budget?count=12").await
    );
}

// ---- RPT-4: query-param date validation ----

/// EVERY (endpoint, date parameter) pair in the native report API.
///
/// RPT-4's point is how BROAD the silent failure was: each of these fed an
/// unvalidated caller string straight into the engine's lexical `&str` date
/// comparisons, where a malformed value cannot fail — it just sorts somewhere
/// wrong and the report comes back plausible-looking with a `200`.
const DATE_PARAMS: [(&str, &str); 12] = [
    ("/api/reports/balancesheet", "asOf"),
    ("/api/reports/incomestatement", "from"),
    ("/api/reports/incomestatement", "to"),
    ("/api/reports/cashflow", "end"),
    ("/api/reports/networth", "end"),
    ("/api/budget", "end"),
    ("/api/insights", "start"),
    ("/api/insights", "end"),
    ("/api/subscriptions", "asOf"),
    ("/api/holdings", "asOf"),
    ("/api/holdings", "gainSince"),
    ("/api/holdings/series", "asOf"),
];

/// RPT-4: a malformed date is a client error on every endpoint and every date
/// param, with a message that names the param and echoes the value.
///
/// The listed values are the realistic ones: a typo (`2026-7-1x`), a
/// paste-gone-wrong (`garbage`), a date that is shaped right but does not exist
/// (`2026-02-30`, `2026-13-01`), a two-digit year, and a truncated date. hledger
/// rejects all six.
#[tokio::test]
async fn malformed_date_is_400_on_every_endpoint_and_param() {
    let journal = sample_journal();
    for (endpoint, param) in DATE_PARAMS {
        for value in [
            "2026-7-1x",
            "garbage",
            "2026-02-30",
            "2026-13-01",
            "26-07-01",
            "2026-07",
        ] {
            let uri = format!("{endpoint}?{param}={value}");
            let (status, message) = get_error(&journal, &uri).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "GET {uri} must be a 400, not a plausible-looking report"
            );
            assert!(
                message.contains(param) && message.contains(value),
                "GET {uri} should say which param and value it rejected, got {message:?}"
            );
        }
    }
}

/// RPT-4: an EXPLICIT empty date is rejected too — it used to sort below every
/// real date and serve an empty report with a `200` (`?asOf=` returned
/// `grandTotal: {}`). The one exception is `gainSince`, whose empty value is its
/// documented "all-time gain" sentinel.
#[tokio::test]
async fn empty_date_is_400_except_the_gain_since_sentinel() {
    let journal = sample_journal();
    for (endpoint, param) in DATE_PARAMS {
        let uri = format!("{endpoint}?{param}=");
        let (status, _) = get_error(&journal, &uri).await;
        let expected = if param == "gainSince" {
            StatusCode::OK
        } else {
            StatusCode::BAD_REQUEST
        };
        assert_eq!(status, expected, "GET {uri}");
    }
    // The sentinel means "all-time", i.e. exactly the same report as omitting it.
    assert_eq!(
        body_ok(&journal, "/api/holdings?gainSince=").await,
        body_ok(&journal, "/api/holdings").await
    );
}

/// RPT-4: the unpadded and `/`- or `.`-separated spellings that `hledger 1.52`
/// accepts for `-b`/`-e` are accepted here too, and normalize to the ISO date —
/// so a URL answer agrees with `hledger -e <the same date>`, and a date that is
/// legal inside a journal is legal in a query string.
#[tokio::test]
async fn hledger_date_spellings_normalize_to_the_iso_date() {
    let journal = sample_journal();
    for (endpoint, param) in DATE_PARAMS {
        let iso = body_ok(&journal, &format!("{endpoint}?{param}=2026-06-30")).await;
        for spelling in ["2026-6-30", "2026/6/30", "2026.6.30", "2026/06/30"] {
            let uri = format!("{endpoint}?{param}={spelling}");
            assert_eq!(
                body_ok(&journal, &uri).await,
                iso,
                "GET {uri} must equal the 2026-06-30 report"
            );
        }
    }
}

/// RPT-4, with the finding's own numbers. `?asOf=2026-7-1` is a date hledger
/// accepts; it used to serve the ALL-TIME grand total ($47,871.41) with a `200`
/// because `"2026-7-1"` sorts above every real 2026 date.
#[tokio::test]
async fn hand_typed_as_of_no_longer_serves_the_all_time_total() {
    let journal = sample_journal();
    let hand_typed = body_ok(&journal, "/api/reports/balancesheet?asOf=2026-7-1").await;
    let padded = body_ok(&journal, "/api/reports/balancesheet?asOf=2026-07-01").await;
    assert_eq!(hand_typed, padded);
    assert_eq!(
        wire_ma(&hand_typed["grandTotal"])["$"],
        canon(4_834_045, 2),
        "asOf=2026-7-1 must be the 2026-07-01 total, $48,340.45"
    );
    assert_eq!(
        wire_ma(
            &body_ok(&journal, "/api/reports/balancesheet?asOf=2026-12-31").await["grandTotal"]
        )["$"],
        canon(4_787_141, 2),
        "the all-time total is a DIFFERENT number, $47,871.41 — the one the bug served"
    );
}

/// RPT-4: a garbage `end` used to grow a `2026-00` bucket (from a `"7-"` month
/// slice) whose `bucket_end` of `2026-00-00` sorts below every real date, so the
/// trailing column of every period report was silently wrong.
#[tokio::test]
async fn malformed_end_no_longer_produces_a_garbage_bucket() {
    let journal = sample_journal();
    for endpoint in [
        "/api/reports/cashflow",
        "/api/reports/networth",
        "/api/budget",
    ] {
        let uri = format!("{endpoint}?end=2026-7-1&count=3");
        let buckets = body_ok(&journal, &uri).await["buckets"].clone();
        assert_eq!(
            buckets,
            serde_json::json!(["2026-05", "2026-06", "2026-07"]),
            "GET {uri} must bucket by the real date, not 2026-00"
        );
    }
}

/// RPT-4 (related): `changeMin` used to swallow anything `Dec::parse` rejected
/// and quietly substitute the $10.00 default, silently widening the "biggest
/// change" list the caller asked to narrow.
#[tokio::test]
async fn unparseable_change_min_is_400() {
    let journal = sample_journal();
    for value in ["zzz", "1-2", "$10"] {
        let uri = format!("/api/insights?changeMin={value}");
        let (status, message) = get_error(&journal, &uri).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "GET {uri} must be a 400, not a silent $10.00 default"
        );
        assert!(message.contains("changeMin"), "{message}");
    }
    // `,` is a digit-group separator when the decimal mark is `.`, so `1,000` is
    // a well-formed 1000 and stays accepted — the finding's own example was not
    // actually a mis-parse. It is emphatically NOT the $10.00 default.
    let grouped = body_ok(&journal, "/api/insights?changeMin=1%2C000").await;
    assert_eq!(
        grouped,
        body_ok(&journal, "/api/insights?changeMin=1000").await
    );
    assert_ne!(
        grouped,
        body_ok(&journal, "/api/insights?changeMin=10").await
    );
    // Absent and explicitly empty both keep the documented $10.00 default.
    assert_eq!(
        body_ok(&journal, "/api/insights?changeMin=").await,
        body_ok(&journal, "/api/insights?changeMin=10").await
    );
}

/// SEC-1: the native report routes are same-origin only, like everything else —
/// a cross-origin `Origin` gets no `access-control-allow-origin` back, so the
/// browser will not let the page that asked read the report.
#[tokio::test]
async fn report_get_carries_no_cors_header_by_default() {
    let journal = sample_journal();
    let (status, allow_origin, _) = get_on(
        &journal,
        "/api/reports/balancesheet?asOf=2026-06-30&depth=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        allow_origin, None,
        "no CORS layer is installed unless --allow-origin names an exact origin"
    );
}
