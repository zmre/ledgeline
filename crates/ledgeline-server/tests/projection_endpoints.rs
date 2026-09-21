//! The `/api/projections/{run,seed}` HTTP surface.
//!
//! Hermetic: no subprocess, no socket. Every request replays through `tower`'s
//! oneshot against `fixtures/sample.journal`.
//!
//! `projections-seed` is additionally pinned BYTE FOR BYTE by
//! `native_wire_golden.rs`, so this file does not re-assert the seed's numbers.
//! What it pins is everything a golden cannot:
//!
//! 1. **`POST /api/projections/run` projects the body, not the journal.** The
//!    scenario travels in the request precisely so that UNSAVED edits project;
//!    a run that quietly answered from the journal's own `~` rules would look
//!    right on a seeded scenario and be wrong on every edited one.
//! 2. **An oversized scenario is refused before it costs anything.** A `400`
//!    naming the limit, not a `500` after a core has been claimed for it.
//! 3. **A malformed scenario is a `400`, never a plausible zero.** A bad growth
//!    unit, a bad event date, an unknown source: each names what was wrong.
//! 4. **Both routes require the bearer token.** They read the user's finances;
//!    a report of them is not less private for being read-only.
//! 5. **No response body contains an absolute path.**

mod common;

use axum::body::Body;
use axum::http::{HeaderName, Request, StatusCode, header};
use common::{fixture_journal, fixture_journal_path};
use http_body_util::BodyExt;
use ledgeline::{AccessToken, AppState, Security, app, router_with_security, router_with_state};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn send(request: Request<Body>) -> (StatusCode, String) {
    let response = app(&fixture_journal())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn post(body: Value) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri("/api/projections/run")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request builds");
    send(request).await
}

/// `POST /api/projections/run` expecting a `200`, parsed.
async fn run(body: Value) -> Value {
    let (status, text) = post(body).await;
    assert_eq!(status, StatusCode::OK, "run should be 200: {text}");
    serde_json::from_str(&text).expect("the run body is JSON")
}

async fn get(uri: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    send(request).await
}

/// One `$4200`-a-month expense line, the smallest scenario that says anything.
fn rent_line() -> Value {
    json!({
        "id": "rent",
        "group": "rule:0",
        "account": "expenses:rent",
        "amount": {"commodity": "$", "quantity": {"mantissa": "420000", "places": 2}, "precision": 2},
        "period": {"raw": "monthly"},
        "growth": null,
        "note": "",
        "source": "journal"
    })
}

/// One ASSET row. The amount is the per-period CONTRIBUTION — `$0` here, so the
/// balance only compounds — and `role` is the discriminator, never the account.
fn asset_row(account: &str, rate: &str) -> Value {
    json!({
        "id": "brok",
        "group": "rule:1",
        "role": "asset",
        "account": account,
        "amount": {"commodity": "$", "quantity": {"mantissa": "0", "places": 2}, "precision": 2},
        "period": {"raw": "monthly"},
        "growth": {"rate": {"mantissa": rate, "places": 2}, "unit": "year"},
        "opening": null,
        "note": "",
        "source": "journal"
    })
}

fn scenario_with(lines: Vec<Value>, events: Vec<Value>) -> Value {
    json!({"name": "test", "created": null, "updated": null, "lines": lines, "events": events})
}

/// The whole `$` figure of a `{"$": {"mantissa": …, "places": …}}` wire amount.
fn dollars(mixed: &Value) -> i128 {
    let Some(dec) = mixed.get("$") else {
        return 0;
    };
    let mantissa: i128 = dec["mantissa"]
        .as_str()
        .expect("mantissa is a string")
        .parse()
        .expect("mantissa parses");
    let places = dec["places"].as_u64().expect("places is a number") as u32;
    mantissa / 10_i128.pow(places)
}

fn series(node: &Value) -> Vec<i128> {
    node["values"]
        .as_array()
        .expect("values is an array")
        .iter()
        .map(dollars)
        .collect()
}

// ---------------------------------------------------------------------------
// The run body
// ---------------------------------------------------------------------------

/// The shape in full, over a scenario the journal does not contain — which is
/// the whole point of putting it in the body.
#[tokio::test]
async fn run_projects_the_scenario_in_the_body() {
    let projection = run(json!({
        "scenario": scenario_with(vec![rent_line()], vec![]),
        "asOf": "2026-07-08",
        "interval": "monthly",
        "count": 3,
        "depth": 2
    }))
    .await;

    // The first projected bucket is the next WHOLE one after `asOf`.
    assert_eq!(
        projection["buckets"],
        json!(["2026-08", "2026-09", "2026-10"])
    );
    assert_eq!(projection["start"], "2026-08-01");

    // Net income is a `PeriodReport`, so `ReportTable` renders it unchanged —
    // in cash-flow orientation, so an expense reads negative.
    let net_income = &projection["netIncome"];
    assert_eq!(net_income["buckets"], projection["buckets"]);
    let totals: Vec<i128> = net_income["totals"]
        .as_array()
        .expect("totals is an array")
        .iter()
        .map(dollars)
        .collect();
    assert_eq!(totals, [-4200, -4200, -4200]);
    let rows = net_income["rows"].as_array().expect("rows is an array");
    assert!(
        rows.iter().any(|row| row["account"] == "expenses:rent"),
        "the scenario's own account must be a row: {rows:?}"
    );

    // Opening balances come from the REAL journal, so they are not zero…
    let opening = dollars(&projection["cash"]["opening"]);
    assert!(opening > 0, "sample.journal holds cash: {opening}");
    // …and each bucket draws down by the scenario's implied cash leg.
    assert_eq!(
        series(&projection["cash"]),
        [opening - 4200, opening - 8400, opening - 12_600]
    );
    assert!(dollars(&projection["netWorth"]["opening"]) > 0);

    // Nothing was dropped, so nothing is warned about.
    assert_eq!(projection["warnings"], json!([]));
    // sample.journal has plenty of cash, so three months of rent is no runway.
    assert_eq!(projection["runway"], Value::Null);
}

/// **The asset wire, end to end.** A `role: "asset"` row with a rate and no
/// contribution moves NET WORTH and leaves cash and net income exactly where an
/// identical scenario without the row leaves them.
///
/// The balance it compounds is the real journal's — `assets:broker:taxable` in
/// `fixtures/sample.journal` — so this also proves the seed of the running
/// balance reaches the wire, which no engine unit test can (they run over
/// hand-built transactions).
#[tokio::test]
async fn an_asset_row_grows_net_worth_and_leaves_cash_alone() {
    let window = |lines: Vec<Value>| {
        json!({
            "scenario": scenario_with(lines, vec![]),
            "asOf": "2026-07-08",
            "interval": "yearly",
            "count": 3,
            "depth": 2
        })
    };
    let without = run(window(vec![rent_line()])).await;
    let with = run(window(vec![
        rent_line(),
        asset_row("assets:broker:taxable", "7"),
    ]))
    .await;

    // Cash, net income and the runway are IDENTICAL: appreciation is not money
    // in the bank and is not a P&L event.
    assert_eq!(with["cash"], without["cash"]);
    assert_eq!(with["netIncome"], without["netIncome"]);
    assert_eq!(with["runway"], without["runway"]);
    // The openings are the journal's, and the asset row does not restate them.
    assert_eq!(with["netWorth"]["opening"], without["netWorth"]["opening"]);

    // Net worth, though, pulls away — by the growth alone, and only from the
    // first anniversary of the projection's start (2027-01-01).
    let (grew, flat) = (series(&with["netWorth"]), series(&without["netWorth"]));
    assert_eq!(grew[0], flat[0], "the first year holds no anniversary");
    assert!(
        grew[1] > flat[1] && grew[2] > flat[2],
        "{grew:?} vs {flat:?}"
    );
    // Compounding, not linear: the second step is bigger than the first.
    assert!(
        (grew[2] - flat[2]) - (grew[1] - flat[1]) > grew[1] - flat[1],
        "growth must compound on the new base: {grew:?} vs {flat:?}"
    );
    assert_eq!(with["warnings"], json!([]));

    // The PER-ROW ATTRIBUTION (plan 23, Phase 3). The Balance column and the
    // net-worth breakdown read this, and both have to agree with the curve
    // above — so it is the same walk's figures, keyed by the source rule.
    assert_eq!(
        without["assets"],
        json!([]),
        "a flow row is not an asset row"
    );
    let rows = with["assets"].as_array().expect("assets is an array");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["group"], "rule:1");
    assert_eq!(rows[0]["account"], "assets:broker:taxable");
    // The journal's real balance for the subtree — the figure the table greys
    // out, which is not on the scenario wire and could not be.
    let held = dollars(&rows[0]["journalOpening"]);
    assert!(held > 0, "sample.journal holds a taxable brokerage: {held}");
    // No override, so the row compounded exactly the journal's figure…
    assert_eq!(rows[0]["opening"], rows[0]["journalOpening"]);
    // …and its attributed growth IS the gap the net-worth series opened up.
    assert_eq!(dollars(&rows[0]["growth"]), grew[2] - flat[2]);
}

/// A CONTRIBUTION on an asset row implies its cash outflow through the same
/// residual every other row gets, and is net-worth neutral. No second path.
#[tokio::test]
async fn an_asset_contribution_moves_cash_and_not_net_worth() {
    let mut row = asset_row("assets:broker:taxable", "0");
    row["amount"]["quantity"] = json!({"mantissa": "100000", "places": 2});
    row["growth"] = Value::Null;
    let body = |lines: Vec<Value>| json!({"scenario": scenario_with(lines, vec![]), "asOf": "2026-07-08", "count": 3});
    let without = run(body(vec![])).await;
    let with = run(body(vec![row])).await;

    let opening = dollars(&with["cash"]["opening"]);
    assert_eq!(
        series(&with["cash"]),
        [opening - 1000, opening - 2000, opening - 3000],
        "a $1000 monthly contribution leaves cash at $1000 a month"
    );
    // …and net worth does not move at all: the money changed accounts.
    assert_eq!(with["netWorth"], without["netWorth"]);
    assert_eq!(with["netIncome"]["rows"], json!([]));
}

/// **A non-asset account is a WARNING, not a silent model.** Liabilities are
/// out of scope (a mortgage needs principal-versus-interest, which the journal
/// cannot supply), and so is everything else that is not an asset.
#[tokio::test]
async fn an_asset_row_on_a_non_asset_account_is_warned_about() {
    for account in ["liabilities:mortgage", "expenses:housing:rent"] {
        let projection = run(json!({
            "scenario": scenario_with(vec![asset_row(account, "7")], vec![]),
            "asOf": "2026-07-08",
            "interval": "yearly",
            "count": 3
        }))
        .await;
        let warnings = projection["warnings"]
            .as_array()
            .expect("warnings is an array");
        assert!(
            warnings.iter().any(|w| w
                .as_str()
                .is_some_and(|w| w.contains(account) && w.contains("asset"))),
            "{account}: {warnings:?}"
        );
        // …and it contributes nothing: every bucket is the opening balance.
        let opening = dollars(&projection["netWorth"]["opening"]);
        assert_eq!(series(&projection["netWorth"]), [opening; 3]);
    }
}

/// `opening` is an OVERRIDE, and only its difference from the journal's own
/// figure reaches the series — plus a warning, because a chart that silently
/// started somewhere the balance sheet does not would be lying.
#[tokio::test]
async fn an_opening_override_adjusts_once_and_says_so() {
    let mut row = asset_row("assets:broker:taxable", "0");
    row["growth"] = Value::Null;
    row["opening"] = json!({"commodity": "$", "quantity": {"mantissa": "100000000", "places": 2}, "precision": 2});
    let body = |lines: Vec<Value>| json!({"scenario": scenario_with(lines, vec![]), "asOf": "2026-07-08", "count": 2});
    let without = run(body(vec![])).await;
    let with = run(body(vec![row])).await;

    // The OPENING figure itself is the journal's — an override restates the
    // row's balance, not the report's starting point.
    assert_eq!(with["netWorth"]["opening"], without["netWorth"]["opening"]);
    // Every bucket is shifted by the same one-off difference…
    let (adjusted, plain) = (series(&with["netWorth"]), series(&without["netWorth"]));
    let shift = adjusted[0] - plain[0];
    assert_eq!(
        adjusted[1] - plain[1],
        shift,
        "applied once, not per bucket"
    );
    assert_ne!(shift, 0);
    // …cash is untouched, or a brokerage estimate would move the runway…
    assert_eq!(with["cash"], without["cash"]);
    // …and it is named.
    assert!(
        with["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|w| w.as_str().is_some_and(|w| w.contains("opening balance"))),
        "{:?}",
        with["warnings"]
    );
    // …and BOTH balances are on the wire, so the table can show the override
    // beside the ledger's own figure rather than instead of it.
    let attributed = &with["assets"][0];
    assert_eq!(dollars(&attributed["opening"]), 1_000_000);
    assert_ne!(
        attributed["journalOpening"], attributed["opening"],
        "the journal's figure must survive an override"
    );
    // The adjustment is not appreciation: this row states no rate at all.
    assert_eq!(dollars(&attributed["growth"]), 0);
}

/// An absent `role` reads as `flow`, which is what every body written before
/// asset rows existed means — and is the only default on this wire, because a
/// scenario that lost the key would otherwise model a compounding balance as an
/// outflow the size of its contribution.
#[tokio::test]
async fn an_absent_role_reads_as_a_flow_line() {
    let mut legacy = rent_line();
    assert!(legacy.as_object_mut().unwrap().remove("role").is_none());
    let projection = run(json!({
        "scenario": scenario_with(vec![legacy], vec![]),
        "asOf": "2026-07-08",
        "count": 3
    }))
    .await;
    let opening = dollars(&projection["cash"]["opening"]);
    assert_eq!(
        series(&projection["cash"]),
        [opening - 4200, opening - 8400, opening - 12_600]
    );
}

/// An empty scenario still projects: the openings are real, and every bucket
/// simply repeats them. A tab that opened on a blank table must not 500.
#[tokio::test]
async fn an_empty_scenario_projects_the_openings_forward() {
    let projection = run(json!({
        "scenario": scenario_with(vec![], vec![]),
        "asOf": "2026-07-08",
        "count": 2
    }))
    .await;
    let opening = dollars(&projection["cash"]["opening"]);
    assert_eq!(series(&projection["cash"]), [opening, opening]);
    assert_eq!(projection["netIncome"]["rows"], json!([]));
    assert_eq!(projection["runway"], Value::Null);
}

/// A dated event, and the runway it causes. The `runway` object carries the
/// bucket's key and label beside its index, so the sentence above the chart
/// does not have to index back into `buckets` and hope the two agree.
#[tokio::test]
async fn a_large_event_produces_a_runway_naming_its_bucket() {
    let projection = run(json!({
        "scenario": scenario_with(vec![], vec![json!({
            "id": "buyout",
            "date": "2026-09-15",
            "description": "buy the building",
            "postings": [{
                "account": "expenses:building",
                "amount": {"commodity": "$", "quantity": {"mantissa": "99999999", "places": 0}, "precision": 0}
            }]
        })]),
        "asOf": "2026-07-08",
        "count": 4
    }))
    .await;
    let runway = &projection["runway"];
    assert_eq!(runway["bucket"], 1);
    assert_eq!(runway["bucketKey"], "2026-09");
    assert_eq!(runway["label"], "Sep 2026");
    assert_eq!(runway["date"], "2026-09-30");
    assert_eq!(runway["periods"], 2);
}

/// A period the engine cannot enumerate is a WARNING and a zero contribution —
/// never a `500`, and never a silent zero.
#[tokio::test]
async fn an_unsupported_period_is_warned_about_rather_than_failing() {
    let mut line = rent_line();
    line["period"] = json!({"raw": "every weekday"});
    let projection = run(json!({
        "scenario": scenario_with(vec![line], vec![]),
        "asOf": "2026-07-08",
        "count": 2
    }))
    .await;
    let warnings = projection["warnings"]
        .as_array()
        .expect("warnings is an array");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0]
            .as_str()
            .is_some_and(|w| w.contains("every weekday")),
        "the warning must quote what the user wrote: {warnings:?}"
    );
    assert_eq!(projection["netIncome"]["rows"], json!([]));
}

/// `period.raw` is re-parsed through the JOURNAL's grammar, and the derived
/// fields the client is handed back are the parse's, not the request's — so a
/// bounded segment round-trips as a bounded segment.
#[tokio::test]
async fn a_bounded_period_round_trips_through_the_journals_grammar() {
    let mut line = rent_line();
    line["period"] = json!({"raw": "monthly from 2026-09-01 to 2026-10-01"});
    let projection = run(json!({
        "scenario": scenario_with(vec![line], vec![]),
        "asOf": "2026-07-08",
        "count": 3
    }))
    .await;
    // `to` is EXCLUSIVE, so only September fires.
    let totals: Vec<i128> = projection["netIncome"]["totals"]
        .as_array()
        .unwrap()
        .iter()
        .map(dollars)
        .collect();
    assert_eq!(totals, [0, -4200, 0]);
}

/// The derived period fields go OUT beside `raw`, and a client may hand the
/// whole object back without stripping them.
#[tokio::test]
async fn the_seed_emits_derived_period_fields_that_a_run_accepts_back() {
    let (status, text) = get("/api/projections/seed?end=2026-07-08&count=12&depth=2").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let seed: Value = serde_json::from_str(&text).expect("the seed body is JSON");

    let first = &seed["lines"][0];
    assert_eq!(first["period"]["raw"], "monthly");
    assert_eq!(first["period"]["simple"], "monthly");
    assert_eq!(first["period"]["from"], Value::Null);
    assert_eq!(first["period"]["to"], Value::Null);
    assert_eq!(first["source"], "unbudgeted");
    // `created`/`updated` are present-and-null, not omitted.
    assert!(seed.as_object().unwrap().contains_key("created"));
    assert_eq!(seed["created"], Value::Null);

    // The whole scenario goes straight back into a run, unedited.
    let projection = run(json!({
        "scenario": seed,
        "asOf": "2026-07-08",
        "count": 3
    }))
    .await;
    assert_eq!(projection["buckets"].as_array().unwrap().len(), 3);
    assert!(dollars(&projection["cash"]["opening"]) > 0);

    // sample.journal spends in EUR as well as `$`, so the seed does too — and
    // the projection SAYS so rather than quietly adding the two together. This
    // is the warning working, not a defect in the round trip.
    let warnings = projection["warnings"]
        .as_array()
        .expect("warnings is an array");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0]
            .as_str()
            .is_some_and(|w| w.contains("EUR") && w.contains("without conversion")),
        "{warnings:?}"
    );
}

/// A date is NORMALIZED, not merely validated — `2026-9-1` is what hledger's
/// own `-b`/`-e` accept, and it must reach the bucket math as `2026-09-01`
/// rather than as a string that sorts above `2026-12-31` (RPT-4).
#[tokio::test]
async fn an_unpadded_event_date_is_normalized_rather_than_refused() {
    let projection = run(json!({
        "scenario": scenario_with(vec![], vec![json!({
            "id": "e",
            "date": "2026-9-1",
            "description": "unpadded",
            "postings": [{
                "account": "expenses:legal",
                "amount": {"commodity": "$", "quantity": {"mantissa": "45000", "places": 0}, "precision": 0}
            }]
        })]),
        "asOf": "2026-07-08",
        "count": 3
    }))
    .await;
    assert_eq!(
        projection["buckets"],
        json!(["2026-08", "2026-09", "2026-10"])
    );
    let totals: Vec<i128> = projection["netIncome"]["totals"]
        .as_array()
        .unwrap()
        .iter()
        .map(dollars)
        .collect();
    // September, not a garbage `2026-00` bucket and not nothing at all.
    assert_eq!(totals, [0, -45_000, 0]);
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// An oversized scenario is a `400` naming the limit, refused before a
/// `compute` slot is claimed for it.
///
/// Each line here is spelled as small as the shape allows, so the body stays
/// well inside [the route's byte limit] and this really does test the COUNT
/// guard. A test that tripped the byte limit instead would pass for the wrong
/// reason and stop noticing if the count guard were removed.
///
/// [the route's byte limit]: a_body_past_the_route_limit_is_refused
#[tokio::test]
async fn an_oversized_scenario_is_a_400() {
    let tiny = |index: usize| {
        json!({
            "id": index.to_string(),
            "group": "g",
            "account": "e:x",
            "amount": {"commodity": "$", "quantity": {"mantissa": "1", "places": 0}},
            "period": {"raw": "monthly"}
        })
    };
    let many: Vec<Value> = (0..1001).map(tiny).collect();
    let body_text = json!({"scenario": scenario_with(many, vec![])}).to_string();
    assert!(
        body_text.len() < 200_000,
        "the count guard, not the byte guard, is what this asserts: {} bytes",
        body_text.len()
    );

    let (status, body) = post(serde_json::from_str(&body_text).expect("valid JSON")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("1000"), "the limit must be named: {body}");
    assert!(body.contains("1001"), "and so must what was sent: {body}");

    // …and the same for events, and for one event's postings.
    let events: Vec<Value> = (0..1001)
        .map(|index| json!({"id": format!("e{index}"), "date": "2026-09-01", "postings": []}))
        .collect();
    let (status, body) = post(json!({"scenario": scenario_with(vec![], events)})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("events"), "{body}");

    let postings: Vec<Value> = (0..101)
        .map(|_| {
            json!({
                "account": "expenses:x",
                "amount": {"commodity": "$", "quantity": {"mantissa": "1", "places": 0}}
            })
        })
        .collect();
    let fat = json!({"id": "fat", "date": "2026-09-01", "postings": postings});
    let (status, body) = post(json!({"scenario": scenario_with(vec![], vec![fat])})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("postings"), "{body}");
    assert!(
        body.contains("'fat'"),
        "the offending event is named: {body}"
    );
}

/// A body past the route's own 256 KiB limit is refused — and as a `400`, the
/// same class every other malformed body here gets, rather than the `413` axum
/// would produce on its own.
///
/// The body is deliberately VALID JSON carrying ONE line, so the only thing that
/// can refuse it is the byte limit: without the route's `DefaultBodyLimit` this
/// is well under axum's 2 MiB default and would be a `200`.
#[tokio::test]
async fn a_body_past_the_route_limit_is_refused() {
    let mut line = rent_line();
    line["note"] = json!("x".repeat(400 * 1024));
    let body = json!({"scenario": scenario_with(vec![line], vec![])}).to_string();
    assert!(body.len() > 256 * 1024 && body.len() < 2 * 1024 * 1024);

    let request = Request::builder()
        .method("POST")
        .uri("/api/projections/run")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("request builds");
    let (status, _) = send(request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Every malformed field is a `400` that says what was wrong — never a
/// plausible-looking zero.
#[tokio::test]
async fn malformed_scenario_fields_are_named_in_the_400() {
    let cases: Vec<(Value, &str)> = vec![
        // An unknown growth unit.
        (
            {
                let mut line = rent_line();
                line["growth"] =
                    json!({"rate": {"mantissa": "3", "places": 2}, "unit": "fortnight"});
                scenario_with(vec![line], vec![])
            },
            "fortnight",
        ),
        // An unknown line source.
        (
            {
                let mut line = rent_line();
                line["source"] = json!("guessed");
                scenario_with(vec![line], vec![])
            },
            "guessed",
        ),
        // An unknown ROLE. The two roles project entirely different numbers, so
        // a value this server does not recognize is a refusal rather than a
        // fallback to either of them.
        (
            {
                let mut line = rent_line();
                line["role"] = json!("stock");
                scenario_with(vec![line], vec![])
            },
            "stock",
        ),
        // An opening balance on a FLOW line: a field the engine reads from
        // nowhere and the file cannot write, so it is named rather than dropped.
        (
            {
                let mut line = rent_line();
                line["opening"] = json!({
                    "commodity": "$",
                    "quantity": {"mantissa": "100", "places": 0},
                    "precision": 0
                });
                scenario_with(vec![line], vec![])
            },
            "opening balance",
        ),
        // An empty account.
        (
            {
                let mut line = rent_line();
                line["account"] = json!("   ");
                scenario_with(vec![line], vec![])
            },
            "account",
        ),
        // An empty commodity.
        (
            {
                let mut line = rent_line();
                line["amount"]["commodity"] = json!("");
                scenario_with(vec![line], vec![])
            },
            "commodity",
        ),
        // RPT-4: an event date reaches the same bucket math a `?end=` param
        // does, so it gets the same validation. A day that does not exist, and
        // a string that is not a date at all, are both refused — the second
        // would otherwise sort below every real date and bucket as `0000-00`.
        (
            scenario_with(
                vec![],
                vec![json!({"id": "e", "date": "2026-02-30", "postings": []})],
            ),
            "event date",
        ),
        (
            scenario_with(
                vec![],
                vec![json!({"id": "e", "date": "garbage", "postings": []})],
            ),
            "event date",
        ),
        (
            scenario_with(vec![], vec![json!({"id": "e", "date": "", "postings": []})]),
            "event date",
        ),
    ];
    for (scenario, expected) in cases {
        let (status, body) = post(json!({"scenario": scenario})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{scenario}");
        assert!(
            body.contains(expected),
            "the 400 must name '{expected}': {body}"
        );
    }
}

/// The window params are resolved through the SAME `Window` the cash-flow
/// report uses, so an interval or count it rejects is rejected here identically
/// — and the date error names the field the caller actually sent.
#[tokio::test]
async fn window_params_are_refused_exactly_as_the_other_reports_refuse_them() {
    let with = |extra: Value| {
        let mut body = json!({"scenario": scenario_with(vec![], vec![])});
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        body
    };

    let (status, body) = post(with(json!({"interval": "fortnightly"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("fortnightly"), "{body}");

    let (status, _) = post(with(json!({"count": 0}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(with(json!({"count": 100_000}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The date error names `asOf`, which is what the caller sent — not `end`.
    let (status, body) = post(with(json!({"asOf": "nonsense"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("asOf"), "{body}");

    // A field nobody sent is a 400, not a silently different projection.
    let (status, _) = post(with(json!({"budgetDesc": "rent"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The seed refuses the same window params, through the same resolver.
#[tokio::test]
async fn the_seed_refuses_a_bad_window() {
    for uri in [
        "/api/projections/seed?end=nonsense",
        "/api/projections/seed?end=2026-07-08&count=0",
        "/api/projections/seed?end=2026-07-08&count=99999",
    ] {
        let (status, body) = get(uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
    }
}

// ---------------------------------------------------------------------------
// The token guard, and paths
// ---------------------------------------------------------------------------

/// Both routes read the user's finances. Below the `route_layer` in `lib.rs`
/// they would do it with no bearer token at all; this is the test that fails if
/// they are ever moved.
#[tokio::test]
async fn every_projection_route_requires_the_token() {
    const PORT: u16 = 5099;
    const HOST: &str = "127.0.0.1:5099";
    let state = AppState::from_journal_path(fixture_journal_path()).expect("the fixture opens");
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");

    let probe = |method: &'static str, uri: &'static str, auth: Option<&'static str>| {
        let state = state.clone();
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
                    json!({"scenario": scenario_with(vec![], vec![])}).to_string(),
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
        ("POST", "/api/projections/run"),
        ("GET", "/api/projections/seed"),
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
    let root = fixture_journal_path()
        .parent()
        .expect("the fixture has a directory")
        .to_string_lossy()
        .into_owned();

    let (_, seed) = get("/api/projections/seed?end=2026-07-08").await;
    let (_, projection) = post(json!({
        "scenario": scenario_with(vec![rent_line()], vec![]),
        "asOf": "2026-07-08",
        "count": 2
    }))
    .await;
    for body in [seed, projection] {
        assert!(
            !body.contains(&root),
            "a response leaked the journal's directory"
        );
    }
}

// ===========================================================================
// Scenario FILES — `GET /api/projections`, `GET`/`PUT /api/projections/{*id}`
// ===========================================================================
//
// A real directory tree and a real editing-enabled state, because everything
// worth asserting here is about bytes on a disk: what the scan offers, what a
// save leaves alone, and which of the five refusals a given request gets.
//
// The pure guards (the caps, the symlink refusal, containment, the serializer's
// round trip) are pinned in `ledgeline-core`'s own `tests/projection_files.rs`
// against a filesystem. What is pinned HERE is the wire: the status codes, the
// optimistic-concurrency contract, and decision 5.

/// A projection file the tree starts with.
const PLAN: &str = "\
; Ledgeline projection
; projection: Plan of record
; created: 2026-01-01
; updated: 2026-01-01

; the user's own note, which a save must not eat
~ monthly  plan
    (expenses:rent)      $4200.00
    (revenues:salary)  $-12000.00
";

/// A journal the MAIN journal includes, so decision 5 has something to refuse.
const INCLUDED: &str = "~ monthly  included plan\n    (expenses:included)  $1.00\n";

/// The main journal every file test is rooted at. It `include`s `budget.journal`,
/// which is what makes that file un-writable through this surface.
const MAIN: &str = "\
account assets:cash    ; type: A
account expenses:rent  ; type: X
account revenues:salary  ; type: R

include budget.journal

2026-01-05 opening
    assets:cash      $10000.00
    equity:opening  $-10000.00
";

static FILE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A temp journal directory plus an editing-enabled state bound to its journal.
struct Tree {
    dir: PathBuf,
    state: AppState,
}

impl Tree {
    fn new(files: &[(&str, &str)]) -> Self {
        let seq = FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!(
                "ledgeline-projection-endpoints/{}-{seq}",
                std::process::id()
            ))
            .to_path_buf();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let journal = dir.join("main.journal");
        std::fs::write(&journal, MAIN).expect("write journal");
        std::fs::write(dir.join("budget.journal"), INCLUDED).expect("write include");
        for (relative, contents) in files {
            let path = dir.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("temp subdir");
            }
            std::fs::write(&path, contents).expect("write file");
        }
        let state = AppState::from_journal_path(&journal).expect("editor opens");
        Self { dir, state }
    }

    /// The default shape: one projection file, one plain journal beside it.
    fn standard() -> Self {
        Self::new(&[
            ("plans/projection-plan.journal", PLAN),
            ("notes.journal", "; nothing recurring here\n"),
        ])
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.dir.join(relative)).expect("read file")
    }

    fn exists(&self, relative: &str) -> bool {
        self.dir.join(relative).is_file()
    }

    /// Every spelling of this tree's own directory that could leak into a
    /// response: the path as constructed and its canonical form (which on macOS
    /// gains a `/private` prefix).
    fn secret_paths(&self) -> Vec<String> {
        let mut paths = vec![self.dir.to_string_lossy().into_owned()];
        if let Ok(canonical) = self.dir.canonicalize() {
            paths.push(canonical.to_string_lossy().into_owned());
        }
        paths
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One request against a fresh router over `state`. Clones share the editor,
/// the snapshot and the write mutex, so effects persist between calls.
async fn file_request(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(value) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&value).expect("serialize")))
            .expect("request builds"),
        None => builder.body(Body::empty()).expect("request builds"),
    };
    let response = router_with_state(state.clone())
        .oneshot(request)
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = BodyExt::collect(response.into_body())
        .await
        .expect("body collects")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

async fn file_json(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, bytes) = file_request(state, method, uri, body).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn file_text(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let (status, bytes) = file_request(state, method, uri, body).await;
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

const PLAN_URI: &str = "/api/projections/plans/projection-plan.journal";

/// `GET` the plan file and hand back its revision plus its scenario.
async fn open_plan(state: &AppState) -> (String, Value) {
    let (status, doc) = file_json(state, "GET", PLAN_URI, None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    (
        doc["revision"].as_str().expect("revision").to_string(),
        doc["scenario"].clone(),
    )
}

// ---------------------------------------------------------------------------
// The listing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_index_lists_every_journal_projections_first_and_says_which_are_writable() {
    let tree = Tree::standard();
    let (status, index) = file_json(&tree.state, "GET", "/api/projections", None).await;
    assert_eq!(status, StatusCode::OK, "{index}");

    let files = index["files"].as_array().expect("files");
    let ids: Vec<&str> = files.iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        vec![
            "plans/projection-plan.journal",
            "budget.journal",
            "main.journal",
            "notes.journal",
        ]
    );

    let plan = &files[0];
    assert_eq!(plan["isProjection"], json!(true));
    assert_eq!(plan["name"], json!("Plan of record"));
    assert_eq!(plan["created"], json!("2026-01-01"));
    assert_eq!(plan["label"], json!("plan"));
    assert_eq!(plan["writable"], json!(true));

    // DECISION 5, on the listing: a file the main journal is parsed from is
    // shown (so it can be loaded) and flagged un-writable.
    for id in ["budget.journal", "main.journal"] {
        let file = files.iter().find(|f| f["id"] == json!(id)).expect(id);
        assert_eq!(file["writable"], json!(false), "{id} must not be writable");
    }
    // A journal the main one does not include IS writable, even without the
    // `projection-` prefix: "really any journal file … could be used".
    let notes = files
        .iter()
        .find(|f| f["id"] == json!("notes.journal"))
        .expect("notes");
    assert_eq!(notes["writable"], json!(true));
    assert_eq!(notes["isProjection"], json!(false));
    assert_eq!(notes["name"], Value::Null, "its header is not read");

    assert_eq!(index["truncated"], json!(false));
    assert_eq!(index["editable"], json!(true));
    // The Save As dialog's directory list — relative, root first.
    assert_eq!(index["directories"], json!(["", "plans"]));
    assert!(
        !index["rootLabel"]
            .as_str()
            .expect("rootLabel")
            .contains('/'),
        "the heading is a component, never a path"
    );
}

#[tokio::test]
async fn no_file_response_contains_an_absolute_path() {
    // Layer 5: a dialog is a fine oracle for "does this directory exist".
    let tree = Tree::standard();
    let (_, index) = file_text(&tree.state, "GET", "/api/projections", None).await;
    let (_, doc) = file_text(&tree.state, "GET", PLAN_URI, None).await;
    let (_, missing) = file_text(
        &tree.state,
        "GET",
        "/api/projections/nope/missing.journal",
        None,
    )
    .await;
    let (_, malformed) = file_text(
        &tree.state,
        "GET",
        "/api/projections/../../etc/passwd.journal",
        None,
    )
    .await;
    for body in [index, doc, missing, malformed] {
        for secret in tree.secret_paths() {
            assert!(!body.contains(&secret), "a response leaked {secret}");
        }
    }
}

// ---------------------------------------------------------------------------
// Reading one file
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_projection_file_reads_back_as_the_scenario_it_states() {
    let tree = Tree::standard();
    let (revision, scenario) = open_plan(&tree.state).await;
    assert!(!revision.is_empty(), "a real file has a real revision");

    assert_eq!(scenario["name"], json!("Plan of record"));
    assert_eq!(scenario["created"], json!("2026-01-01"));
    let lines = scenario["lines"].as_array().expect("lines");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["account"], json!("expenses:rent"));
    assert_eq!(lines[0]["group"], json!("rule:0"));
    assert_eq!(lines[0]["period"]["raw"], json!("monthly"));
    // Revenue keeps the journal's own sign on this wire.
    assert_eq!(
        lines[1]["amount"]["quantity"]["mantissa"],
        json!("-1200000")
    );
    assert_eq!(scenario["events"], json!([]));
}

#[tokio::test]
async fn a_journal_the_main_one_includes_can_be_read_but_never_written() {
    // The ask: "We should default to showing the last loaded and if there isn't
    // one, default to using the active budget file." So a `GET` works.
    let tree = Tree::standard();
    let (status, doc) =
        file_json(&tree.state, "GET", "/api/projections/budget.journal", None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["writable"], json!(false));
    assert_eq!(
        doc["scenario"]["lines"][0]["account"],
        json!("expenses:included")
    );

    // …and the `PUT` is refused, with a sentence that says why rather than a
    // `404` that pretends the file is not there.
    let (status, message) = file_text(
        &tree.state,
        "PUT",
        "/api/projections/budget.journal",
        Some(json!({"revision": doc["revision"], "scenario": doc["scenario"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert!(message.contains("plan of record"), "{message}");
    assert_eq!(tree.read("budget.journal"), INCLUDED, "nothing was written");
}

#[tokio::test]
async fn a_malformed_id_is_a_400_and_a_missing_one_a_404_whatever_is_on_disk() {
    let tree = Tree::standard();
    // Shape first, before any filesystem call — so the answer cannot depend on
    // what is there, which is what stops the route being an existence oracle.
    for id in [
        "../../etc/passwd.journal",
        "plans/../../escape.journal",
        "plans/projection-plan.rules",
        "a:b.journal",
    ] {
        let (status, body) =
            file_text(&tree.state, "GET", &format!("/api/projections/{id}"), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "for `{id}`: {body}");
    }
    // Well-formed but not in the scanned set: the same `404` for every cause.
    for id in [
        "plans/projection-nope.journal",
        "nowhere/at/all.journal",
        "node_modules/skipped.journal",
    ] {
        let (status, body) =
            file_text(&tree.state, "GET", &format!("/api/projections/{id}"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "for `{id}`: {body}");
    }
}

// ---------------------------------------------------------------------------
// Saving: update
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_save_rewrites_only_the_rule_it_changed() {
    let tree = Tree::standard();
    let (revision, mut scenario) = open_plan(&tree.state).await;
    scenario["lines"][0]["amount"]["quantity"]["mantissa"] = json!("450000");

    let (status, doc) = file_json(
        &tree.state,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": revision, "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_ne!(doc["revision"], json!(revision), "the revision moves");

    let after = tree.read("plans/projection-plan.journal");
    // The user's own comment survives, and so does the `created:` date.
    assert!(
        after.contains("; the user's own note, which a save must not eat\n"),
        "{after}"
    );
    assert!(after.contains("; created: 2026-01-01\n"), "{after}");
    // Within one block the amounts land in one column, so the two lines of
    // this rule are laid out against the longer of the two accounts.
    assert!(
        after.contains("    (expenses:rent)    $4500.00\n"),
        "{after}"
    );
    // `updated:` is the SERVER's today, never the client's — a timestamp a
    // caller supplies is a timestamp that can say anything.
    assert!(
        !after.contains("; updated: 2026-01-01\n"),
        "updated: was not touched:\n{after}"
    );
    // The other line of the same rule is untouched in content.
    assert!(
        after.contains("    (revenues:salary)  $-12000.00\n"),
        "{after}"
    );
}

#[tokio::test]
async fn a_save_that_changes_nothing_writes_nothing() {
    let tree = Tree::standard();
    let (revision, scenario) = open_plan(&tree.state).await;
    // `updated:` moves to today on every save, so a genuinely byte-identical
    // result needs the file to already carry today's date. Save once to get
    // there, then save again and require the bytes not to move.
    let (status, doc) = file_json(
        &tree.state,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": revision, "scenario": scenario.clone()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let settled = tree.read("plans/projection-plan.journal");

    let (revision, scenario) = open_plan(&tree.state).await;
    let (status, doc) = file_json(
        &tree.state,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": revision.clone(), "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(
        tree.read("plans/projection-plan.journal"),
        settled,
        "a no-op must not rewrite the file"
    );
    assert_eq!(
        doc["revision"],
        json!(revision),
        "and the revision must not move either"
    );
}

#[tokio::test]
async fn a_stale_revision_is_a_409_and_writes_nothing() {
    let tree = Tree::standard();
    let (_, scenario) = open_plan(&tree.state).await;
    let before = tree.read("plans/projection-plan.journal");

    for revision in ["0-deadbeef", "not-a-token"] {
        let (status, message) = file_text(
            &tree.state,
            "PUT",
            PLAN_URI,
            Some(json!({"revision": revision, "scenario": scenario})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(message.contains("changed on disk"), "{message}");
    }
    assert_eq!(tree.read("plans/projection-plan.journal"), before);
}

#[tokio::test]
async fn the_revision_a_save_returns_is_the_one_the_next_save_needs() {
    // The optimistic-concurrency loop, end to end: save, take the revision the
    // response gave, save again with it. A server that returned a revision from
    // a re-read rather than from what it wrote would break here the moment
    // anything else touched the file.
    let tree = Tree::standard();
    let (revision, mut scenario) = open_plan(&tree.state).await;
    scenario["lines"][0]["amount"]["quantity"]["mantissa"] = json!("450000");
    let (status, first) = file_json(
        &tree.state,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": revision, "scenario": scenario.clone()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");

    scenario["lines"][0]["amount"]["quantity"]["mantissa"] = json!("460000");
    let (status, second) = file_json(
        &tree.state,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": first["revision"], "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert!(
        tree.read("plans/projection-plan.journal")
            .contains("$4600.00"),
        "{}",
        tree.read("plans/projection-plan.journal")
    );
}

// ---------------------------------------------------------------------------
// Saving: create
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_revision_creates_a_new_file() {
    let tree = Tree::standard();
    let (_, scenario) = open_plan(&tree.state).await;
    let mut fresh = scenario;
    fresh["name"] = json!("Series A");
    // `created` is the SERVER's, so a client that sends one is ignored on a
    // new file — this one deliberately sends the loaded file's.
    let uri = "/api/projections/plans/projection-series-a.journal";

    let (status, doc) = file_json(
        &tree.state,
        "PUT",
        uri,
        Some(json!({"revision": "", "scenario": fresh})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["id"], json!("plans/projection-series-a.journal"));
    assert_eq!(doc["scenario"]["name"], json!("Series A"));
    assert!(doc["writable"].as_bool().expect("writable"));

    let written = tree.read("plans/projection-series-a.journal");
    assert!(
        written.starts_with("; Ledgeline projection\n; projection: Series A\n"),
        "{written}"
    );
    assert!(
        written.contains("    (expenses:rent)    $4200.00\n"),
        "{written}"
    );

    // And it now appears in the listing, first, because it is a `projection-*`.
    let (_, index) = file_json(&tree.state, "GET", "/api/projections", None).await;
    assert_eq!(
        index["files"][0]["id"],
        json!("plans/projection-plan.journal")
    );
    assert_eq!(
        index["files"][1]["id"],
        json!("plans/projection-series-a.journal")
    );
}

#[tokio::test]
async fn creating_over_an_existing_file_is_refused_and_leaves_it_alone() {
    // The refusal is the KERNEL's (`O_EXCL`), not a check-then-write. The
    // Save As dialog shows it as "a file already exists there".
    let tree = Tree::standard();
    let (_, scenario) = open_plan(&tree.state).await;
    let before = tree.read("plans/projection-plan.journal");

    let (status, message) = file_text(
        &tree.state,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": "", "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{message}");
    assert!(message.contains("already exists"), "{message}");
    assert_eq!(tree.read("plans/projection-plan.journal"), before);
}

#[tokio::test]
async fn a_create_never_makes_a_directory() {
    // "The Save As dialog offers the directories the scan found; making new
    // ones is the user's job." A missing directory answers the ordinary `404`,
    // because "that directory is not there" is a fact about the filesystem.
    let tree = Tree::standard();
    let (_, scenario) = open_plan(&tree.state).await;
    let (status, message) = file_text(
        &tree.state,
        "PUT",
        "/api/projections/brand/new/projection-x.journal",
        Some(json!({"revision": "", "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{message}");
    assert!(!tree.exists("brand/new/projection-x.journal"));
    assert!(!tree.dir.join("brand").exists(), "no directory was created");
}

#[tokio::test]
async fn a_scenario_the_writer_refuses_is_a_400_and_writes_nothing() {
    let tree = Tree::standard();
    let (_, scenario) = open_plan(&tree.state).await;
    let uri = "/api/projections/projection-refused.journal";

    for (field, value) in [
        // hledger ends a tag's value at a comma, so this name would read back
        // truncated — and the name is also the filename.
        ("name", json!("Plan B, revised")),
    ] {
        let mut bad = scenario.clone();
        bad[field] = value;
        let (status, message) = file_text(
            &tree.state,
            "PUT",
            uri,
            Some(json!({"revision": "", "scenario": bad})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
        assert!(message.contains("comma"), "{message}");
    }

    // An account name that would split at a double space, which hledger would
    // read as the start of an amount.
    let mut bad = scenario;
    bad["lines"][0]["account"] = json!("expenses:a  b");
    let (status, message) = file_text(
        &tree.state,
        "PUT",
        uri,
        Some(json!({"revision": "", "scenario": bad})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert!(
        !tree.exists("projection-refused.journal"),
        "nothing was written"
    );
}

#[tokio::test]
async fn a_read_only_server_refuses_to_save_at_all() {
    // No editor bound means no write surface, and the SPA turns the `501` into
    // the same "start the Ledgeline engine" it shows everywhere else.
    let tree = Tree::standard();
    let text = std::fs::read_to_string(tree.dir.join("main.journal")).expect("read journal");
    let journal =
        ledgeline_core::parse_journal(&text, &tree.dir.join("main.journal").to_string_lossy())
            .expect("journal parses");
    let read_only = AppState::from_journal(&journal);

    let (_, scenario) = open_plan(&tree.state).await;
    let (status, message) = file_text(
        &read_only,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": "", "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{message}");
}

// ---------------------------------------------------------------------------
// The token guard, for the three file routes
// ---------------------------------------------------------------------------

/// `PUT /api/projections/{*id}` is a write primitive over a file in the user's
/// journal directory. Below the `route_layer` in `lib.rs` it would be reachable
/// with no bearer token at all; this is the test that fails if it is ever moved.
#[tokio::test]
async fn every_projection_file_route_requires_the_token() {
    const PORT: u16 = 5098;
    const HOST: &str = "127.0.0.1:5098";
    let state = AppState::from_journal_path(fixture_journal_path()).expect("the fixture opens");
    let token = AccessToken::parse("integration-test-token").expect("well-formed token");

    let probe = |method: &'static str, uri: &'static str, auth: Option<&'static str>| {
        let state = state.clone();
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
                    json!({"revision": "", "scenario": scenario_with(vec![], vec![])}).to_string(),
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
        ("GET", "/api/projections"),
        ("GET", "/api/projections/sample.journal"),
        ("PUT", "/api/projections/sample.journal"),
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
