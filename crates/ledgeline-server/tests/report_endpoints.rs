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

fn report_fixture_journal(name: &str) -> Journal {
    let path = fixtures_dir().join("reports").join(name);
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

/// Commodity-wise sum of two wire `MixedAmount`s, with exact `Dec` math.
///
/// The canonical form [`wire_ma`] produces cannot be added directly — it has
/// already dropped the scale the addition needs — so this reads the raw
/// `(mantissa, places)` pairs, adds, and canonicalizes at the end. Used to check
/// `A == L + E` from exact values, never from displayed ones.
fn add_wire_ma(a: &Value, b: &Value) -> Canon {
    let mut sum: BTreeMap<String, Dec> = BTreeMap::new();
    for value in [a, b] {
        for (commodity, dec) in value.as_object().expect("mixed amount is an object") {
            let mantissa: i128 = dec["mantissa"]
                .as_str()
                .expect("mantissa string")
                .parse()
                .expect("mantissa");
            let places = u32::try_from(dec["places"].as_u64().expect("places")).unwrap();
            let addend = Dec::new(mantissa, places);
            sum.entry(commodity.clone())
                .and_modify(|prev| *prev = prev.add(addend).expect("no overflow"))
                .or_insert(addend);
        }
    }
    sum.into_iter()
        .map(|(commodity, dec)| (commodity, canon(dec.mantissa, dec.places)))
        .filter(|(_, (mantissa, _))| *mantissa != 0)
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
// Grouped balance sheet — vs the hledger CLI (`bs -V`, `bse -B`, `is -B`)
// ===========================================================================

/// `hledger -f fixtures/sample.journal bs -V -e 2026-07-09` reports
/// `$548,112.62, 5.0 GLD, -2.0 TSLA` of assets against `$336,531.15` of
/// liabilities, and `bse -B` / `is -B` agree on a Net of `$35,498.91,
/// -933,25 EUR`. The whole point of the report is that those reconcile, so
/// `check` must be `{}`.
///
/// The `$` figures here are the UNROUNDED ones; hledger's CLI displays them to
/// two places, and `bs -V -c '$1000.0000'` prints the `$548112.6150` /
/// `$211581.4650` asserted below.
#[tokio::test]
async fn balancesheet_grouped_matches_the_hledger_cli() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=3&value=market",
    )
    .await;

    assert_eq!(body["asOf"], "2026-07-08");
    assert_eq!(body["base"], "$");
    assert_eq!(body["value"], "market");
    assert_eq!(
        body["check"],
        serde_json::json!({}),
        "a balanced journal must report an EMPTY check"
    );
    assert_eq!(body["meta"]["unpriced"], serde_json::json!(["GLD", "TSLA"]));

    let sections = body["sections"].as_array().expect("three sections");
    assert_eq!(
        sections
            .iter()
            .map(|section| (
                section["kind"].as_str().unwrap(),
                section["title"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            ("assets", "Assets"),
            ("liabilities", "Liabilities"),
            ("equity", "Equity"),
        ]
    );

    // `bs -V`: assets, then liabilities.
    assert_eq!(
        wire_ma(&sections[0]["total"]),
        Canon::from([
            ("$".to_string(), canon(5_481_126_150, 4)),
            ("GLD".to_string(), canon(5, 0)),
            ("TSLA".to_string(), canon(-2, 0)),
        ])
    );
    assert_eq!(
        wire_ma(&sections[1]["total"]),
        Canon::from([("$".to_string(), canon(33_653_115, 2))])
    );
    // `bs -V` Net: assets − liabilities.
    assert_eq!(
        wire_ma(&body["netWorth"]),
        Canon::from([
            ("$".to_string(), canon(2_115_814_650, 4)),
            ("GLD".to_string(), canon(5, 0)),
            ("TSLA".to_string(), canon(-2, 0)),
        ])
    );
    // A == L + E, from exact values rather than displayed ones. Note the
    // displayed group subtotals need NOT visibly add up. hledger's own figures
    // for the four asset groups are $49,059.99 + $10,552.62 + $468,000.00 +
    // $20,500.00, which reads as $548,112.61 — while the exact total is
    // $548,112.6150 and displays as $548,112.62. The half-cent lives in
    // Investments, whose true value is $10,552.6250.
    assert_eq!(
        wire_ma(&sections[0]["total"]),
        add_wire_ma(&sections[1]["total"], &sections[2]["total"]),
        "assets must equal liabilities plus equity"
    );
}

/// The group shape the SPA renders: names, provenance and the two synthetic
/// equity lines, with `rows` empty on the computed ones.
#[tokio::test]
async fn balancesheet_grouped_reports_groups_and_their_provenance() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=3&value=market",
    )
    .await;

    let groups = |index: usize| -> Vec<(String, String)> {
        body["sections"][index]["groups"]
            .as_array()
            .expect("groups array")
            .iter()
            .map(|group| {
                (
                    group["name"].as_str().unwrap().to_string(),
                    group["source"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    // `Property` is tag-sourced (`bsgroup: Property` on `assets:property:home`,
    // which holds `HOME` and would otherwise be filed under Investments by the
    // commodity rule); `Vehicles` is segment-sourced, the car holding only `$`.
    assert_eq!(
        groups(0),
        [
            ("Cash and cash equivalents".to_string(), "type".to_string()),
            ("Investments".to_string(), "commodity".to_string()),
            ("Property".to_string(), "tag".to_string()),
            ("Vehicles".to_string(), "segment".to_string()),
        ]
    );
    assert_eq!(
        groups(1),
        [
            ("Credit cards".to_string(), "segment".to_string()),
            ("Mortgage".to_string(), "segment".to_string()),
        ]
    );
    assert_eq!(
        groups(2),
        [
            ("Opening".to_string(), "segment".to_string()),
            ("Transfers".to_string(), "segment".to_string()),
            ("Retained earnings".to_string(), "computed".to_string()),
            ("Valuation adjustment".to_string(), "computed".to_string()),
        ]
    );

    // `is -B` Net, on the retained-earnings line.
    let equity = body["sections"][2]["groups"].as_array().unwrap();
    let retained = &equity[2];
    assert_eq!(
        wire_ma(&retained["total"]),
        Canon::from([
            ("$".to_string(), canon(3_549_891, 2)),
            ("EUR".to_string(), canon(-93_325, 2)),
        ])
    );
    assert!(
        retained["rows"].as_array().unwrap().is_empty(),
        "a computed line stands for no accounts"
    );

    // Rows carry the same shape as the flat report's.
    let cash_rows = body["sections"][0]["groups"][0]["rows"]
        .as_array()
        .expect("rows array");
    assert_eq!(cash_rows[0]["account"], "assets:bank");
    assert_eq!(cash_rows[0]["depth"], 2);
    assert_eq!(cash_rows[0]["own"], serde_json::json!({}));
    assert_eq!(
        wire_ma(&cash_rows[1]["inclusive"]),
        Canon::from([("$".to_string(), canon(2_829_281, 2))]),
        "assets:bank:checking, `hledger bal` $28,292.81"
    );
}

/// `value=cost` reproduces `bse -B` exactly — including its DECLARED equity of
/// `$126,550.00 + 5.0 GLD`, which is the figure the identity needs (an unvalued
/// `bal type:E` says `$127,550.00` and would throw the check off by $1,000 and
/// 5 GLD). At cost nothing is unbooked, so there is no valuation-adjustment line
/// and no single base commodity.
///
/// `hledger -f fixtures/sample.journal bse -B -e 2026-07-09`:
/// ```text
///  Assets  $498,580.06, -933,25 EUR, 5.0 GLD
///  Equity  $126,550.00, 5.0 GLD
/// ```
/// The house enters at its `1 HOME @ $420,000.00` cost here, not the
/// 2026-06-30 price.
#[tokio::test]
async fn balancesheet_grouped_at_cost_matches_hledger_bse_b() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=4&value=cost",
    )
    .await;

    assert_eq!(body["value"], "cost");
    assert_eq!(body["base"], Value::Null);
    assert_eq!(body["check"], serde_json::json!({}));
    assert_eq!(body["meta"]["unpriced"], serde_json::json!([]));

    assert_eq!(
        wire_ma(&body["sections"][0]["total"]),
        Canon::from([
            ("$".to_string(), canon(49_858_006, 2)),
            ("EUR".to_string(), canon(-93_325, 2)),
            ("GLD".to_string(), canon(5, 0)),
        ])
    );

    // Declared equity = every non-computed group: $126,550.00 + 5.0 GLD.
    let equity: Vec<&Value> = body["sections"][2]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|group| group["source"] != "computed")
        .collect();
    assert_eq!(
        wire_ma(&equity[0]["total"]),
        Canon::from([("$".to_string(), canon(12_655_000, 2))])
    );
    assert_eq!(
        wire_ma(&equity[1]["total"]),
        Canon::from([("GLD".to_string(), canon(5, 0))])
    );
    assert!(
        !body["sections"][2]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|group| group["name"] == "Valuation adjustment"),
        "nothing is unbooked at cost"
    );
}

/// `value=none` is `hledger bse` unvalued: share counts, no base, and the
/// identity still holding because the revaluation line books the cost residue.
///
/// `hledger -f fixtures/sample.journal bse -e 2026-07-09` Assets:
/// ```text
///  $68,902.56, 19.5000 AAPL, 566,75 EUR, 5.0 GLD, 1.0 HOME, -2.0 TSLA, 17.0 VTI
/// ```
/// The house is the bare `1.0 HOME` it was booked as — no `P` directive is
/// consulted — while the car is `$20,500.00`, its depreciation being postings
/// that land on every basis.
#[tokio::test]
async fn balancesheet_grouped_unvalued_matches_hledger_bse() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=4&value=none",
    )
    .await;

    assert_eq!(body["value"], "none");
    assert_eq!(body["base"], Value::Null);
    assert_eq!(body["check"], serde_json::json!({}));
    assert_eq!(
        wire_ma(&body["sections"][0]["total"]),
        Canon::from([
            ("$".to_string(), canon(6_890_256, 2)),
            ("AAPL".to_string(), canon(195, 1)),
            ("EUR".to_string(), canon(56_675, 2)),
            ("GLD".to_string(), canon(5, 0)),
            ("HOME".to_string(), canon(1, 0)),
            ("TSLA".to_string(), canon(-2, 0)),
            ("VTI".to_string(), canon(17, 0)),
        ])
    );
}

/// Defaults: today's date, market, and NO depth clamp. Only `depth` and `value`
/// are checkable without a clock, so those are what is pinned.
///
/// An omitted `depth` is unlimited rather than some default level, which is what
/// the SPA relies on: it stopped sending the param when the balance sheet's
/// depth slider was removed, and expanding a group has to show the whole group.
/// `depth=0` already means totals-only, so unlimited cannot be a number at all.
#[tokio::test]
async fn balancesheet_grouped_defaults_to_no_clamp_and_market() {
    let journal = sample_journal();
    let defaulted = body_ok(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08",
    )
    .await;
    // `sample.journal`'s deepest account is 4 segments, so anything at or past
    // that is already unclamped — and the basis defaults to market.
    let explicit = body_ok(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=100&value=market",
    )
    .await;
    assert_eq!(defaulted, explicit);

    // ... and it really is deeper than the 3 the slider used to ask for, so the
    // assertion above is not vacuous.
    let clamped = body_ok(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=3",
    )
    .await;
    assert_ne!(defaulted, clamped);
    assert_eq!(
        clamped["netWorth"], defaulted["netWorth"],
        "only the rows move with depth, never a total (RPT-1/RPT-4)"
    );
}

/// `valueIn` moves the whole report into another commodity. hledger reverses the
/// `P … EUR $1.16` edge to price `$` in EUR, so everything converts — and the
/// identity survives the change of unit.
#[tokio::test]
async fn balancesheet_grouped_honors_value_in() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=3&value=market&valueIn=EUR",
    )
    .await;
    assert_eq!(body["base"], "EUR");
    assert_eq!(body["check"], serde_json::json!({}));
    assert!(
        body["sections"][0]["total"].get("EUR").is_some(),
        "assets are reported in EUR: {}",
        body["sections"][0]["total"]
    );
}

/// `check` and `balanced` are two different facts and the wire carries both.
///
/// `bs-cost-dust.journal` is valid — hledger's `check` passes on it — yet
/// `26.2690 VTI @ $289.7713` costs `$7,612.00227970` and no cash posting can
/// carry the surplus digits. The residual must reach the client EXACTLY (it is
/// what a warning would have to quote) while the verdict says the journal is
/// fine, so nothing downstream re-derives the ✓/✗ from `check` itself.
#[tokio::test]
async fn balancesheet_grouped_sends_the_verdict_beside_the_exact_residual() {
    let balanced = body_ok(
        &sample_journal(),
        "/api/reports/balancesheet/grouped?asOf=2026-07-08",
    )
    .await;
    assert_eq!(balanced["check"], serde_json::json!({}));
    assert_eq!(balanced["balanced"], serde_json::json!(true));

    let dusty = body_ok(
        &report_fixture_journal("bs-cost-dust.journal"),
        "/api/reports/balancesheet/grouped?asOf=2026-12-31",
    )
    .await;
    assert_eq!(
        wire_ma(&dusty["check"]),
        Canon::from([("$".to_string(), canon(22797, 7))]),
        "the exact residual survives to the wire"
    );
    assert_eq!(
        dusty["balanced"],
        serde_json::json!(true),
        "sub-cent cost dust is not an imbalance"
    );

    let broken = body_ok(
        &report_fixture_journal("errors/bs-unbalanced.journal"),
        "/api/reports/balancesheet/grouped?asOf=2026-12-31",
    )
    .await;
    assert_eq!(
        wire_ma(&broken["check"]),
        Canon::from([("$".to_string(), canon(1000, 2))])
    );
    assert_eq!(
        broken["balanced"],
        serde_json::json!(false),
        "a real $10.00 imbalance must still be reported as one"
    );
}

/// The new route validates `depth`, which the older ones do not. `0` is
/// hledger's totals-only and stays legal; past the ceiling is a 400 naming the
/// range, and a non-numeric value is a 400 from the extractor.
#[tokio::test]
async fn balancesheet_grouped_validates_depth_and_value() {
    let journal = sample_journal();

    let (status, _, body) = get_on(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=0",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "depth 0 is `--depth 0`, not an error"
    );
    assert_eq!(
        body["check"],
        serde_json::json!({}),
        "totals survive depth 0 (RPT-4)"
    );

    let (status, message) = get_error(
        &journal,
        "/api/reports/balancesheet/grouped?asOf=2026-07-08&depth=1000000",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("depth 1000000"), "{message}");
    assert!(message.contains("out of range"), "{message}");

    for uri in [
        "/api/reports/balancesheet/grouped?depth=lots",
        "/api/reports/balancesheet/grouped?depth=-1",
        "/api/reports/balancesheet/grouped?value=fair",
    ] {
        let (status, _, _) = get_on(&journal, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "GET {uri}");
    }
    let (_, message) = get_error(&journal, "/api/reports/balancesheet/grouped?value=fair").await;
    assert!(message.contains("market|cost|none"), "{message}");
}

/// The new route must not disturb the old one, whose bytes are pinned by
/// `fixtures/native/v1/balancesheet.json` and by the hledger golden above.
#[tokio::test]
async fn the_flat_balancesheet_is_unchanged_by_the_grouped_one() {
    let journal = sample_journal();
    let flat = body_ok(
        &journal,
        "/api/reports/balancesheet?asOf=2026-07-08&depth=2",
    )
    .await;
    assert_eq!(
        flat,
        golden("native/v1", "balancesheet.json"),
        "/api/reports/balancesheet must stay byte-identical to its committed golden"
    );
    // It still answers the flat shape, with no grouped keys leaking in.
    assert!(flat.get("groups").is_none());
    assert!(flat.get("check").is_none());
    assert_eq!(flat["sections"].as_array().unwrap().len(), 2);
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
// Grouped income statement (plans/13-income-statement-redesign.md)
// ===========================================================================
//
// Every figure below was read off `hledger 1.52` in the dev shell, never off our
// own output. `-e` is EXCLUSIVE there and our `to` is INCLUSIVE, so
// `to=2026-07-08` is checked against `-e 2026-07-09`.

/// `(kind, title)` per box, in presentation order.
fn is_boxes(body: &Value) -> Vec<(String, String)> {
    body["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .map(|section| {
            (
                section["kind"].as_str().unwrap().to_string(),
                section["title"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// The named box.
fn is_box<'a>(body: &'a Value, kind: &str) -> &'a Value {
    body["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|section| section["kind"] == kind)
        .unwrap_or_else(|| panic!("box {kind} in {:?}", is_boxes(body)))
}

/// `(kind, label)` for every subtotal on the statement, in presentation order.
fn is_subtotals(body: &Value) -> Vec<(String, String)> {
    body["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .flat_map(|section| section["trailing"].as_array().expect("trailing array"))
        .map(|subtotal| {
            (
                subtotal["kind"].as_str().unwrap().to_string(),
                subtotal["label"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// The named subtotal's `Amounts`.
fn is_subtotal<'a>(body: &'a Value, kind: &str) -> &'a Value {
    body["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .flat_map(|section| section["trailing"].as_array().expect("trailing array"))
        .find(|subtotal| subtotal["kind"] == kind)
        .map(|subtotal| &subtotal["total"])
        .unwrap_or_else(|| panic!("subtotal {kind} in {:?}", is_subtotals(body)))
}

/// `$x.yz` in cents, canonicalized for comparison against a wire `Amounts`.
fn cents(value: i128) -> Canon {
    if value == 0 {
        return Canon::new();
    }
    Canon::from([("$".to_string(), canon(value, 2))])
}

/// Both columns of an `Amounts` at once, in cents. `prior: None` asserts the key
/// is ABSENT, which is what `compare=none` must produce.
#[track_caller]
fn assert_amounts(amounts: &Value, current: i128, prior: Option<i128>, what: &str) {
    assert_eq!(
        wire_ma(&amounts["current"]),
        cents(current),
        "{what} (current)"
    );
    match prior {
        Some(want) => assert_eq!(wire_ma(&amounts["prior"]), cents(want), "{what} (prior)"),
        None => assert!(
            amounts.get("prior").is_none(),
            "{what}: `prior` must be ABSENT, not null — got {:?}",
            amounts.get("prior")
        ),
    }
}

/// The default shape on an UNTAGGED journal: two boxes, no ladder, no jargon.
///
/// `hledger -f fixtures/sample.journal is -V -b 2026-01-01 -e 2026-07-09 --depth 2`
/// ```text
///  Revenues  $34,010.00   Expenses  $28,626.48   Net:  $5,383.52
/// ```
/// and the prior window (`2025-06-26..2025-12-31`, the preceding 188 days):
/// ```text
///  Revenues  $39,397.50   Expenses  $28,516.71   Net:  $10,880.79
/// ```
/// The two windows take one `expenses:depreciation` entry each — `$3,500.00`
/// (2026-06-30) current, `$4,000.00` (2025-06-30) prior — so a boundary
/// off-by-one would move a four-figure sum across the columns.
#[tokio::test]
async fn incomestatement_grouped_matches_the_hledger_cli() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08",
    )
    .await;

    assert_eq!(body["from"], "2026-01-01");
    assert_eq!(body["to"], "2026-07-08");
    assert_eq!(body["base"], "$");
    assert_eq!(body["value"], "market", "the default basis, echoed back");
    assert_eq!(
        body["multiStep"], false,
        "an untagged journal asks for no ladder"
    );
    assert_eq!(body["meta"]["unpriced"], serde_json::json!([]));
    assert_eq!(
        body["prior"],
        serde_json::json!({"from": "2025-06-26", "to": "2025-12-31"}),
        "compare=previous defaults on, over the preceding equal-length window"
    );

    assert_eq!(
        is_boxes(&body),
        [
            ("revenue".to_string(), "Revenue".to_string()),
            // Not "Operating expenses" — there is nothing to be operating as
            // distinct from.
            ("opex".to_string(), "Expenses".to_string()),
        ]
    );
    assert!(is_subtotals(&body).is_empty(), "no rungs on a simple book");

    assert_amounts(
        &is_box(&body, "revenue")["total"],
        3_401_000,
        Some(3_939_750),
        "Revenue",
    );
    assert_amounts(
        &is_box(&body, "opex")["total"],
        2_862_648,
        Some(2_851_671),
        "Expenses",
    );
    assert_amounts(&body["netIncome"], 538_352, Some(1_088_079), "Net income");
}

/// The group shape the SPA renders, with the plan's own pinned figures.
///
/// `hledger … is -V -b 2026-01-01 -e 2026-07-09 --depth 2` per account:
/// Salary `$33,960.00`, Dividends `$50.00`; Depreciation `$3,500.00`, Food
/// `$1,654.38`, Housing `$13,125.00`, Taxes `$8,760.00`, Transport `$186.54`,
/// Travel `$656.40`, Unknown `$75.00`, Utilities `$669.16`.
///
/// `Depreciation` takes its line from the same untagged second-segment rule as
/// every neighbour — `expenses:depreciation` declares nothing but `type: X` —
/// and sorts first because the list is alphabetical, as hledger's rows are.
#[tokio::test]
async fn incomestatement_grouped_reports_groups_and_their_provenance() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08&compare=none",
    )
    .await;

    let groups = |kind: &str| -> Vec<(String, String)> {
        is_box(&body, kind)["groups"]
            .as_array()
            .expect("groups array")
            .iter()
            .map(|group| {
                (
                    group["name"].as_str().unwrap().to_string(),
                    group["source"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    assert_eq!(
        groups("revenue"),
        [
            ("Dividends".to_string(), "segment".to_string()),
            ("Salary".to_string(), "segment".to_string()),
        ]
    );
    assert_eq!(
        groups("opex")
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        [
            "Depreciation",
            "Food",
            "Housing",
            "Taxes",
            "Transport",
            "Travel",
            "Unknown",
            "Utilities",
        ]
    );

    let group = |kind: &str, name: &str| -> &Value {
        is_box(&body, kind)["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["name"] == name)
            .unwrap_or_else(|| panic!("group {name}"))
    };
    for (kind, name, value) in [
        ("revenue", "Salary", 3_396_000),
        ("revenue", "Dividends", 5_000),
        ("opex", "Depreciation", 350_000),
        ("opex", "Food", 165_438),
        ("opex", "Housing", 1_312_500),
        ("opex", "Taxes", 876_000),
        ("opex", "Transport", 18_654),
        ("opex", "Travel", 65_640),
        ("opex", "Unknown", 7_500),
        ("opex", "Utilities", 66_916),
    ] {
        assert_amounts(&group(kind, name)["total"], value, None, name);
    }

    // Rows are the group's accounts at full depth — there is no depth control on
    // this report.
    assert_eq!(
        group("opex", "Travel")["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row["account"].as_str().unwrap(), row["depth"].as_u64()))
            .collect::<Vec<_>>(),
        [
            ("expenses:travel:flights", Some(3)),
            ("expenses:travel:lodging", Some(3)),
        ]
    );
}

/// `compare=none` must leave `prior` ABSENT — key by key, everywhere — so a
/// client cannot read "not compared" as "the prior period was empty".
#[tokio::test]
async fn incomestatement_grouped_omits_prior_entirely_without_a_comparison() {
    let journal = sample_journal();
    let body = body_ok(
        &journal,
        "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08&compare=none",
    )
    .await;

    // The top-level window is an EXPLICIT null: it is the switch the client
    // reads to decide whether to demand a prior figure everywhere else, and a
    // switch has to be present to be read.
    assert_eq!(body["prior"], Value::Null, "no prior WINDOW");
    assert!(body.as_object().unwrap().contains_key("prior"));
    // Every FIGURE, by contrast, omits the key entirely — so a missing one is
    // caught rather than defaulting to an empty period.
    assert!(body["netIncome"].get("prior").is_none());
    for section in body["sections"].as_array().unwrap() {
        assert!(section["total"].get("prior").is_none());
        for subtotal in section["trailing"].as_array().unwrap() {
            assert!(subtotal["total"].get("prior").is_none());
        }
        for group in section["groups"].as_array().unwrap() {
            assert!(group["total"].get("prior").is_none());
            for row in group["rows"].as_array().unwrap() {
                assert!(row["amounts"].get("prior").is_none(), "{}", row["account"]);
            }
        }
    }
    // The current column is untouched by dropping the comparison.
    let compared = body_ok(
        &journal,
        "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08&compare=previous",
    )
    .await;
    assert_eq!(
        wire_ma(&body["netIncome"]["current"]),
        wire_ma(&compared["netIncome"]["current"])
    );
}

/// The tagged book: all seven boxes, the full ladder, kebab-case subtotal codes,
/// and a `other` box that prints NEGATIVE.
///
/// hledger 1.52 over `fixtures/reports/is-sections.journal`, 2026:
/// ```text
/// bal revenue                                        $-150,000.00
/// bal cogs                                             $22,500.00
/// bal 'acct:^expenses:(salaries|marketing|rent)'       $101,000.00
/// bal expenses:depreciation                             $6,000.00
/// bal 'acct:^(income:grants|expenses:lawsuit)'           $3,000.00
/// bal expenses:interest                                 $3,000.00
/// bal expenses:taxes                                    $6,200.00
/// is                                          Net:      $8,300.00
/// ```
#[tokio::test]
async fn incomestatement_grouped_renders_the_full_ladder_when_tagged() {
    let journal = report_fixture_journal("is-sections.journal");
    let body = body_ok(
        &journal,
        "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-12-31&value=none&compare=none",
    )
    .await;

    assert_eq!(body["multiStep"], true);
    assert_eq!(body["base"], Value::Null, "unvalued has no base commodity");
    assert_eq!(
        is_boxes(&body),
        [
            ("revenue".to_string(), "Revenue".to_string()),
            ("cogs".to_string(), "Cost of revenue".to_string()),
            // Retitled by the ladder; same box, same accounts.
            ("opex".to_string(), "Operating expenses".to_string()),
            (
                "depreciation".to_string(),
                "Depreciation & amortization".to_string()
            ),
            ("other".to_string(), "Other income & expense".to_string()),
            ("interest".to_string(), "Interest".to_string()),
            ("tax".to_string(), "Income taxes".to_string()),
        ]
    );
    assert_eq!(
        is_subtotals(&body),
        [
            ("grossProfit".to_string(), "Gross profit".to_string()),
            // EBITDA sits ABOVE the D&A box, so each rung is a running total of
            // everything printed above it.
            ("ebitda".to_string(), "EBITDA".to_string()),
            (
                "operatingIncome".to_string(),
                "Operating income".to_string()
            ),
            (
                "pretaxIncome".to_string(),
                "Income before taxes".to_string()
            ),
        ]
    );

    for (kind, value, what) in [
        ("revenue", 15_000_000, "Revenue"),
        ("cogs", 2_250_000, "Cost of revenue"),
        ("opex", 10_100_000, "Operating expenses"),
        ("depreciation", 600_000, "D&A"),
        // The mixed box is signed: $5,000 of grants against an $8,000
        // settlement is a drag on income, and it says so.
        ("other", -300_000, "Other income & expense"),
        ("interest", 300_000, "Interest"),
        ("tax", 620_000, "Income taxes"),
    ] {
        assert_amounts(&is_box(&body, kind)["total"], value, None, what);
    }
    for (kind, value) in [
        ("grossProfit", 12_750_000),
        ("ebitda", 2_650_000),
        ("operatingIncome", 2_050_000),
        ("pretaxIncome", 1_450_000),
    ] {
        assert_amounts(is_subtotal(&body, kind), value, None, kind);
    }
    assert_amounts(&body["netIncome"], 830_000, None, "Net income");

    // `isgroup:` merges two accounts with no common ancestor onto one line:
    // `hledger … bal 'acct:^(expenses:marketing:ads|expenses:salaries:sales)$'` → $32,000.00
    let growth = is_box(&body, "opex")["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["name"] == "Growth")
        .expect("the tagged Growth line");
    assert_eq!(growth["source"], "tag");
    assert_amounts(&growth["total"], 3_200_000, None, "Growth");
    assert_eq!(
        growth["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["account"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["expenses:marketing:ads", "expenses:salaries:sales"]
    );
}

/// The union merge over the wire: a line present in only one period arrives with
/// a zero (an EMPTY mixed amount) on the other side rather than being dropped.
///
/// ```text
/// bal -b 2026-01-01 -e 2027-01-01 expenses:marketing:events            0
/// bal -b 2025-01-01 -e 2026-01-01 expenses:marketing:events    $4,000.00
/// is  -b 2025-01-01 -e 2026-01-01                       Net:   $-4,300.00
/// ```
#[tokio::test]
async fn incomestatement_grouped_keeps_a_line_that_exists_in_only_one_period() {
    let journal = report_fixture_journal("is-sections.journal");
    let body = body_ok(
        &journal,
        "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-12-31&value=none",
    )
    .await;
    assert_eq!(
        body["prior"],
        serde_json::json!({"from": "2025-01-01", "to": "2025-12-31"})
    );

    let marketing = is_box(&body, "opex")["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["name"] == "Marketing")
        .expect("a 2025-only line must still be on the page");
    assert_amounts(&marketing["total"], 0, Some(400_000), "Marketing");
    assert_eq!(
        marketing["total"]["current"],
        serde_json::json!({}),
        "a zero column is the EMPTY mixed amount, matching the engine's contract"
    );
    // And the prior column still ties out to hledger's own prior net income —
    // which is precisely what a dropped line would break.
    assert_amounts(&body["netIncome"], 830_000, Some(-430_000), "Net income");
}

/// `value=cost` and `valueIn=` reach the engine, and the response says which
/// basis produced its numbers.
///
/// `hledger -f fixtures/sample.journal is -B -b 2024-07-01 -e 2026-07-09`
/// Net: `$35,498.91, -933,25 EUR` — the at-cost net income that IS the balance
/// sheet's Retained earnings line, and which `bse -B` reports as the same Net.
#[tokio::test]
async fn incomestatement_grouped_honors_value_and_value_in() {
    let journal = sample_journal();
    let at_cost = body_ok(
        &journal,
        "/api/reports/incomestatement/grouped?from=2024-07-01&to=2026-07-08&value=cost&compare=none",
    )
    .await;
    assert_eq!(at_cost["value"], "cost");
    assert_eq!(at_cost["base"], Value::Null);
    assert_eq!(
        wire_ma(&at_cost["netIncome"]["current"]),
        Canon::from([
            ("$".to_string(), canon(3_549_891, 2)),
            ("EUR".to_string(), canon(-93_325, 2)),
        ]),
        "at-cost net income == the grouped balance sheet's Retained earnings"
    );
    assert_eq!(
        wire_ma(&at_cost["netIncome"]["current"]),
        wire_ma(
            &body_ok(
                &journal,
                "/api/reports/balancesheet/grouped?asOf=2026-07-08&value=cost"
            )
            .await["sections"][2]["groups"][2]["total"]
        ),
        "the two statements tie out on the same number"
    );

    let in_eur = body_ok(
        &journal,
        "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08&valueIn=EUR&compare=none",
    )
    .await;
    assert_eq!(in_eur["base"], "EUR");
    assert_eq!(in_eur["value"], "market");
    assert!(
        wire_ma(&in_eur["netIncome"]["current"]).contains_key("EUR"),
        "valueIn must actually retarget the valuation"
    );
}

/// The new route validates `value` and `compare`, rejecting rather than
/// defaulting — and takes no `depth` at all, which is silently ignored like any
/// unknown param rather than changing the answer.
#[tokio::test]
async fn incomestatement_grouped_validates_value_and_compare() {
    let journal = sample_journal();

    for uri in [
        "/api/reports/incomestatement/grouped?value=fair",
        "/api/reports/incomestatement/grouped?compare=yoy",
        "/api/reports/incomestatement/grouped?compare=",
    ] {
        let (status, _, _) = get_on(&journal, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "GET {uri}");
    }
    let (_, message) =
        get_error(&journal, "/api/reports/incomestatement/grouped?compare=yoy").await;
    assert!(message.contains("previous|none"), "{message}");
    let (_, message) = get_error(&journal, "/api/reports/incomestatement/grouped?value=fair").await;
    assert!(message.contains("market|cost|none"), "{message}");

    // No depth on this report: passing one cannot change the answer.
    assert_eq!(
        body_ok(
            &journal,
            "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08&depth=1"
        )
        .await,
        body_ok(
            &journal,
            "/api/reports/incomestatement/grouped?from=2026-01-01&to=2026-07-08"
        )
        .await
    );
}

/// **The cautionary tale, over HTTP.** A journal whose `issection:` is misspelt
/// must fail loudly instead of serving a statement with a box reading zero.
#[tokio::test]
async fn a_bad_issection_tag_is_a_400_naming_the_account_and_the_alternatives() {
    let text = std::fs::read_to_string(fixtures_dir().join("reports").join("is-sections.journal"))
        .expect("read is-sections.journal")
        .replace("issection: cogs", "issection: cost-of-goods-sold");
    let journal =
        parse_journal(&text, "is-sections-typo.journal").expect("the FILE is still valid");

    let (status, message) = get_error(&journal, "/api/reports/incomestatement/grouped").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    for expected in ["cogs", "cost-of-goods-sold", "revenue", "depreciation"] {
        assert!(message.contains(expected), "{message}");
    }
    // Every OTHER report is unaffected — the tag is this statement's alone.
    for uri in [
        "/api/reports/incomestatement",
        "/api/reports/balancesheet/grouped",
        "/api/reports/networth",
    ] {
        let (status, _, _) = get_on(&journal, uri).await;
        assert_eq!(status, StatusCode::OK, "GET {uri}");
    }
}

/// The new route must not disturb the old one, whose bytes are pinned by
/// `fixtures/native/v1/incomestatement.json` and by the hledger golden above.
#[tokio::test]
async fn the_flat_incomestatement_is_unchanged_by_the_grouped_one() {
    let journal = sample_journal();
    let flat = body_ok(
        &journal,
        "/api/reports/incomestatement?from=2026-01-01&to=2026-07-08&depth=2",
    )
    .await;
    assert_eq!(
        flat,
        golden("native/v1", "incomestatement.json"),
        "/api/reports/incomestatement must stay byte-identical to its committed golden"
    );
    // It still answers the flat shape, with no grouped keys leaking in.
    assert!(flat.get("netIncome").is_none());
    assert!(flat.get("multiStep").is_none());
    assert!(flat.get("prior").is_none());
    assert_eq!(flat["sections"].as_array().unwrap().len(), 2);
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
        "/api/reports/balancesheet/grouped",
        "/api/reports/incomestatement",
        "/api/reports/incomestatement/grouped",
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
const DATE_PARAMS: [(&str, &str); 15] = [
    ("/api/reports/balancesheet", "asOf"),
    ("/api/reports/balancesheet/grouped", "asOf"),
    ("/api/reports/incomestatement", "from"),
    ("/api/reports/incomestatement", "to"),
    ("/api/reports/incomestatement/grouped", "from"),
    ("/api/reports/incomestatement/grouped", "to"),
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
/// accepts; it used to serve the ALL-TIME grand total (`-$267,628.59`) with a
/// `200` because `"2026-7-1"` sorts above every real 2026 date.
///
/// This endpoint takes no `value` parameter — it is the flat, UNVALUED
/// hledger-parity shape — so `grandTotal` is `hledger bs`'s Net with no `-V` or
/// `-B`, and its `$` component is legitimately NEGATIVE: the house sits in the
/// assets as `1.0 HOME` while the mortgage against it is `$336,000.00` of real
/// dollars. Unvalued, those two do not meet. `bs -V` nets them to a healthy
/// `$211,581.46`; this figure is not a net worth and is not read as one.
///
/// ```text
/// $ hledger -f fixtures/sample.journal bs -e 2026-07-02   (i.e. asOf 2026-07-01)
///   Net:  $-267,159.55, 19.5000 AAPL, 566,75 EUR, 5.0 GLD, 1.0 HOME, -2.0 TSLA, 17.0 VTI
/// $ hledger -f fixtures/sample.journal bs -e 2027-01-01   (i.e. asOf 2026-12-31)
///   Net:  $-267,628.59, 19.5000 AAPL, 566,75 EUR, 5.0 GLD, 1.0 HOME, -2.0 TSLA, 17.0 VTI
/// ```
/// The $469.04 between them is the July visa activity ($412.80 of flights plus
/// $56.24 of groceries) — the same gap the pre-`plans/14` figures had, so the
/// test still separates "the date I asked for" from "everything".
#[tokio::test]
async fn hand_typed_as_of_no_longer_serves_the_all_time_total() {
    let journal = sample_journal();
    let hand_typed = body_ok(&journal, "/api/reports/balancesheet?asOf=2026-7-1").await;
    let padded = body_ok(&journal, "/api/reports/balancesheet?asOf=2026-07-01").await;
    assert_eq!(hand_typed, padded);
    assert_eq!(
        wire_ma(&hand_typed["grandTotal"])["$"],
        canon(-26_715_955, 2),
        "asOf=2026-7-1 must be the 2026-07-01 total, -$267,159.55"
    );
    assert_eq!(
        wire_ma(
            &body_ok(&journal, "/api/reports/balancesheet?asOf=2026-12-31").await["grandTotal"]
        )["$"],
        canon(-26_762_859, 2),
        "the all-time total is a DIFFERENT number, -$267,628.59 — the one the bug served"
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
