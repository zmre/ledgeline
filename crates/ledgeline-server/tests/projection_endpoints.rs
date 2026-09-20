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
use ledgeline::{AccessToken, AppState, Security, app, router_with_security};
use serde_json::{Value, json};
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
