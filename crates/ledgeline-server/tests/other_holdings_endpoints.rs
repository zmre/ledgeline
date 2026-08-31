//! End-to-end HTTP tests for the native `/api/holdings/other[/series]`
//! endpoints (`plans/14-other-holdings.md`).
//!
//! Drives the real axum `Router` through `tower`'s `oneshot` (no sockets) over
//! `fixtures/reports/other-holdings.journal`, asserting the wire JSON contract:
//! the account-keyed row shape, the null-keeping nulls, and the fact that the
//! trend response is byte-shaped like `/api/holdings/series` so one SPA decoder
//! serves both. The engine itself is verified in `ledgeline-core`'s
//! `tests/other_holdings.rs`; these tests pin the JSON.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ledgeline::app;
use ledgeline_core::{Journal, parse_journal};
use serde_json::Value;
use tower::ServiceExt;

/// Every posting and price in the fixture is ≤ 2026-06-30.
const AS_OF: &str = "2026-06-30";

fn other_journal() -> Journal {
    let path = common::fixtures_dir().join("reports/other-holdings.journal");
    let text = std::fs::read_to_string(&path).expect("other-holdings.journal readable");
    parse_journal(&text, &path.to_string_lossy()).expect("journal parses")
}

async fn get_on(journal: &Journal, uri: &str) -> (StatusCode, Value) {
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
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes();
    let body = if status == StatusCode::OK {
        serde_json::from_slice(&bytes).expect("body is JSON")
    } else {
        Value::Null
    };
    (status, body)
}

async fn body_ok(journal: &Journal, uri: &str) -> Value {
    let (status, body) = get_on(journal, uri).await;
    assert_eq!(status, StatusCode::OK, "GET {uri} should be 200 OK");
    body
}

/// Canonical `(mantissa, places)` (trailing zeros stripped), so `{45500000,2}`
/// and `{455000,0}` compare equal.
fn canon(value: &Value) -> (i128, u64) {
    let mut mantissa: i128 = value["mantissa"]
        .as_str()
        .expect("mantissa string")
        .parse()
        .expect("mantissa");
    let mut places = value["places"].as_u64().expect("places");
    while places > 0 && mantissa % 10 == 0 {
        mantissa /= 10;
        places -= 1;
    }
    (mantissa, places)
}

fn row<'a>(body: &'a Value, account: &str) -> &'a Value {
    body["holdings"]
        .as_array()
        .expect("holdings array")
        .iter()
        .find(|h| h["account"] == account)
        .unwrap_or_else(|| panic!("row {account} in body"))
}

#[tokio::test]
async fn other_holdings_report_shape_and_rows() {
    let journal = other_journal();
    let body = body_ok(&journal, &format!("/api/holdings/other?asOf={AS_OF}")).await;

    assert_eq!(body["asOf"], AS_OF);
    assert_eq!(body["base"], "$");

    // Value desc. Cash (type:C), the brokerage (holds VTI) and the petty
    // receivable (`holdings: none`) are all absent.
    let accounts: Vec<&str> = body["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["account"].as_str().unwrap())
        .collect();
    assert_eq!(
        accounts,
        [
            "assets:property:house",
            "assets:partners:acme",
            "assets:vehicles:van",
        ]
    );

    // The house: commodity-booked, tagged onto this tab, revalued by `P`.
    let house = row(&body, "assets:property:house");
    assert_eq!(house["name"], "Family home");
    assert_eq!(canon(&house["value"]), (455_000, 0));
    assert_eq!(canon(&house["cost"]), (400_000, 0));
    assert_eq!(canon(&house["change"]), (55_000, 0));
    assert!((house["changePct"].as_f64().unwrap() - 13.75).abs() < 1e-9);
    // `commodities` is the balance AS WRITTEN, so the UI can print "1 HOUSE".
    assert_eq!(canon(&house["commodities"]["HOUSE"]), (1, 0));

    // The van: dollar-booked and depreciated. Cost IS value, so the all-time
    // change is a real zero — present, not null.
    let van = row(&body, "assets:vehicles:van");
    assert_eq!(van["name"], "Delivery van");
    assert_eq!(canon(&van["value"]), (24_500, 0));
    assert_eq!(canon(&van["change"]), (0, 0));
    assert!(
        van["commodities"]["HOUSE"].is_null(),
        "a dollar-booked asset carries no security"
    );

    let totals = &body["totals"];
    assert_eq!(canon(&totals["value"]), (554_500, 0));
    assert_eq!(canon(&totals["cost"]), (499_500, 0));
    assert_eq!(canon(&totals["change"]), (55_000, 0));

    assert!(
        body["warnings"]
            .as_array()
            .expect("warnings array")
            .is_empty(),
        "everything in this fixture prices"
    );
}

/// Nulls are KEPT, not omitted — the SPA's decoder reads the keys.
#[tokio::test]
async fn nullable_keys_are_present() {
    let journal = other_journal();
    let body = body_ok(&journal, &format!("/api/holdings/other?asOf={AS_OF}")).await;
    let acme = row(&body, "assets:partners:acme");
    for key in ["value", "cost", "change", "changePct"] {
        assert!(
            acme.get(key).is_some(),
            "`{key}` must be present even when null"
        );
    }
}

/// `gainSince` means on this tab what it means on the Stocks tab: change is
/// measured against the account's value at the window start.
#[tokio::test]
async fn gain_since_switches_the_reference_to_the_window_opening() {
    let journal = other_journal();
    let body = body_ok(
        &journal,
        &format!("/api/holdings/other?asOf={AS_OF}&gainSince=2026-04-01"),
    )
    .await;

    // $430,000 on 2026-04-01 → $455,000 on 2026-06-30.
    assert_eq!(
        canon(&row(&body, "assets:property:house")["change"]),
        (25_000, 0)
    );
    // Depreciation inside the window is a real loss.
    assert_eq!(
        canon(&row(&body, "assets:vehicles:van")["change"]),
        (-7_500, 0)
    );
}

/// The scope bar drives both tabs identically.
#[tokio::test]
async fn exclude_mode_drops_the_account_and_shrinks_the_totals() {
    let journal = other_journal();
    let body = body_ok(
        &journal,
        &format!("/api/holdings/other?asOf={AS_OF}&mode=exclude&accounts=assets:property:house"),
    )
    .await;

    let accounts: Vec<&str> = body["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["account"].as_str().unwrap())
        .collect();
    assert_eq!(accounts, ["assets:partners:acme", "assets:vehicles:van"]);
    assert_eq!(canon(&body["totals"]["value"]), (99_500, 0));
}

/// The trend response is the SAME wire shape as `/api/holdings/series` — that is
/// the whole reason the SPA needs no second decoder or chart component.
#[tokio::test]
async fn other_series_matches_the_stock_series_shape() {
    let journal = other_journal();
    let body = body_ok(
        &journal,
        &format!("/api/holdings/other/series?asOf={AS_OF}&interval=monthly&count=5"),
    )
    .await;

    assert_eq!(body["base"], "$");
    assert_eq!(body["hasBasis"], true);
    let points = body["points"].as_array().expect("points array");
    assert_eq!(points.len(), 5);

    // Every key the STOCK series emits, and no others — compared against a live
    // response from the other endpoint rather than a hand-copied list, so the
    // two cannot drift apart without this failing.
    let stocks = body_ok(
        &journal,
        &format!("/api/holdings/series?asOf={AS_OF}&interval=monthly&count=5"),
    )
    .await;
    let keys = |value: &Value| -> Vec<String> {
        value.as_object().expect("object").keys().cloned().collect()
    };
    assert_eq!(keys(&body), keys(&stocks), "series envelope");
    assert_eq!(
        keys(&points[0]),
        keys(&stocks["points"].as_array().expect("stock points")[0]),
        "series point"
    );

    assert_eq!(points[0]["bucket"], "2026-02");
    assert_eq!(points[4]["bucket"], "2026-06");
    assert_eq!(points[4]["date"], AS_OF);
    assert_eq!(canon(&points[4]["marketValue"]), (554_500, 0));
}

// ===========================================================================
// `valueIn` — admitted against THIS tab's rows (HOLD-3, Other-scoped)
// ===========================================================================

/// A journal whose two tabs are priced through DISJOINT commodities: the stock
/// (VTI) only in `$` via its cost, the house (`holdings: other`) only in EUR
/// via an explicit `P` — so a stocks-scoped admission test and an Other-scoped
/// one answer in exactly opposite directions.
fn split_priced_journal() -> Journal {
    let text = "\
account assets:broker  ; type: A
account assets:house   ; type: A, holdings: other

P 2026-06-01 HOUSE 250000.00 EUR

2026-01-05 buy VTI
    assets:broker    10 VTI @ $100.00
    equity:opening

2026-01-10 buy house
    assets:house    1 HOUSE
    equity:opening
";
    parse_journal(text, "split-priced.journal").expect("journal parses")
}

/// `valueIn` is admitted against the rows THIS endpoint serves, not the Stocks
/// tab's portfolio. EUR prices every Other row here and no stock: the old
/// stocks-scoped test answered 400 for a report that values perfectly well.
#[tokio::test]
async fn value_in_is_validated_against_other_rows_not_the_stock_portfolio() {
    let journal = split_priced_journal();

    let body = body_ok(
        &journal,
        &format!("/api/holdings/other?asOf={AS_OF}&valueIn=EUR"),
    )
    .await;
    assert_eq!(body["base"], "EUR");
    assert_eq!(canon(&row(&body, "assets:house")["value"]), (250_000, 0));

    // The series endpoint shares the admission test.
    let series = body_ok(
        &journal,
        &format!("/api/holdings/other/series?asOf={AS_OF}&valueIn=EUR&count=1"),
    )
    .await;
    assert_eq!(series["base"], "EUR");

    // The mirror image: `$` prices the stock and NO Other row, so this tab
    // refuses it rather than serving an all-null table over a zero total…
    let (status, _) = get_on(
        &journal,
        &format!("/api/holdings/other?asOf={AS_OF}&valueIn=$"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // …while the stocks endpoints keep their own admission test byte-for-byte:
    // `$` (prices VTI) is served, EUR (prices no stock) is still refused.
    let stocks = body_ok(&journal, &format!("/api/holdings?asOf={AS_OF}&valueIn=$")).await;
    assert_eq!(stocks["base"], "$");
    let (status, _) = get_on(&journal, &format!("/api/holdings?asOf={AS_OF}&valueIn=EUR")).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the stocks admission test still measures the stock portfolio"
    );
}

/// A journal with ONLY Other holdings: the stocks-scoped held-set is empty, and
/// its vacuous accept used to serve `valueIn=XYZZY` a 200 with `base:"XYZZY"`,
/// zero totals and every row null — the exact plausible-zero HOLD-3 exists to
/// prevent. Both endpoints refuse it; a commodity that DOES price the rows is
/// still served.
#[tokio::test]
async fn a_value_in_pricing_no_other_row_is_refused_even_with_no_stocks_at_all() {
    let text = "\
account assets:house   ; type: A, holdings: other

P 2026-06-01 HOUSE 250000.00 EUR

2026-01-10 buy house
    assets:house    1 HOUSE
    equity:opening
";
    let journal = parse_journal(text, "other-only.journal").expect("journal parses");
    for route in ["/api/holdings/other", "/api/holdings/other/series"] {
        let (status, _) = get_on(&journal, &format!("{route}?asOf={AS_OF}&valueIn=XYZZY")).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{route}?valueIn=XYZZY must be refused, not answered with zeros"
        );
    }
    let body = body_ok(
        &journal,
        &format!("/api/holdings/other?asOf={AS_OF}&valueIn=EUR"),
    )
    .await;
    assert_eq!(body["base"], "EUR");
    assert_eq!(canon(&body["totals"]["value"]), (250_000, 0));
}

/// A misspelt `holdings:` code is named in the Problems drawer rather than
/// refused, and the tab still answers.
///
/// The refusal was the worst of the four, because `compute_holdings` is not one
/// tab: it feeds BOTH Holdings tabs, the Insights tab, and the drawer's own
/// three `stock-*` findings. One typo emptied all of them at once — including
/// the drawer that was supposed to explain it, since
/// `journal_to_stock_diagnostics` answers a failed computation with an empty
/// vector.
#[tokio::test]
async fn an_unknown_holdings_class_is_a_diagnostic_and_the_tab_still_serves() {
    let text = "\
account assets:house  ; type: A, holdings: real-estate

2026-01-01 buy
    assets:house   $100.00
    equity:opening
";
    let journal = parse_journal(text, "bad-holdings.journal").expect("journal parses");

    let (status, _) = get_on(&journal, &format!("/api/holdings/other?asOf={AS_OF}")).await;
    assert_eq!(status, StatusCode::OK, "the Other tab still answers");

    let (status, _) = get_on(&journal, &format!("/api/holdings?asOf={AS_OF}")).await;
    assert_eq!(status, StatusCode::OK, "and so does the Stocks tab");

    let (status, body) = get_on(&journal, "/api/diagnostics").await;
    assert_eq!(status, StatusCode::OK);
    let found: Vec<&serde_json::Value> = body["diagnostics"]
        .as_array()
        .expect("an array")
        .iter()
        .filter(|d| d["rule"] == "account-tag")
        .collect();
    assert_eq!(found.len(), 1, "{body}");
    assert_eq!(found[0]["account"], "assets:house");
    let message = found[0]["message"].as_str().expect("a message");
    for expected in ["assets:house", "real-estate", "stocks", "other", "none"] {
        assert!(message.contains(expected), "{message}");
    }
}
