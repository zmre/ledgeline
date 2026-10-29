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

/// The `$` figure exactly, at twelve decimal places.
fn exact(mixed: &Value) -> i128 {
    let Some(dec) = mixed.get("$") else {
        return 0;
    };
    let mantissa: i128 = dec["mantissa"]
        .as_str()
        .expect("mantissa")
        .parse()
        .expect("parses");
    let places = dec["places"].as_u64().expect("places") as u32;
    mantissa * 10_i128.pow(12 - places)
}

/// A journal row posting's first amount as `(mantissa, places)`: rows write a
/// quantity as `{m, p}`.
fn posting_qty(posting: &Value) -> (i128, u32) {
    let qty = &posting["amounts"][0]["qty"];
    let mantissa: i128 = qty["m"].as_str().expect("m").parse().expect("m parses");
    let places = u32::try_from(qty["p"].as_u64().expect("p")).expect("p fits");
    (mantissa, places)
}

/// A row's `values`, each `$` figure exactly ([`exact`]).
fn exact_series(node: &Value) -> Vec<i128> {
    node["values"]
        .as_array()
        .expect("values is an array")
        .iter()
        .map(exact)
        .collect()
}

fn series(node: &Value) -> Vec<i128> {
    node["values"]
        .as_array()
        .expect("values is an array")
        .iter()
        .map(dollars)
        .collect()
}

/// The index of the first forecast column in the statements.
fn projected_from(projection: &Value) -> usize {
    projection["projectedFrom"]
        .as_u64()
        .expect("projectedFrom is a number") as usize
}

/// The FORECAST columns of a statement array: everything from the run's
/// `projectedFrom` on.
fn projected<'a>(projection: &Value, columns: &'a Value) -> &'a [Value] {
    &columns.as_array().expect("a column array")[projected_from(projection)..]
}

/// Projected net income per forecast period, whole dollars — the income
/// statement's own `netIncome` row.
fn net_income(projection: &Value) -> Vec<i128> {
    projected(
        projection,
        &projection["incomeStatement"]["netIncome"]["values"],
    )
    .iter()
    .map(dollars)
    .collect()
}

/// Net worth at the end of each forecast period, EXACT — the balance sheet's
/// own `netWorth`.
fn net_worth(projection: &Value) -> Vec<i128> {
    projected(projection, &projection["balanceSheet"]["netWorth"])
        .iter()
        .map(exact)
        .collect()
}

/// Every account the income statement shows a row or a group for.
fn income_accounts(projection: &Value) -> Vec<String> {
    projection["incomeStatement"]["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .flat_map(|section| section["groups"].as_array().expect("groups").clone())
        .flat_map(|group| {
            let mut names = vec![group["name"].as_str().unwrap_or_default().to_string()];
            names.extend(
                group["rows"]
                    .as_array()
                    .expect("rows")
                    .iter()
                    .map(|row| row["account"].as_str().unwrap_or_default().to_string()),
            );
            names
        })
        .collect()
}

/// One balance-sheet group's total per column (EXACT), by section and name.
fn bs_group(projection: &Value, kind: &str, name: &str) -> Vec<i128> {
    projection["balanceSheet"]["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .find(|section| section["kind"] == kind)
        .and_then(|section| {
            section["groups"]
                .as_array()
                .expect("groups")
                .iter()
                .find(|group| group["name"] == name)
        })
        .map(|group| {
            group["total"]
                .as_array()
                .expect("total")
                .iter()
                .map(exact)
                .collect()
        })
        .unwrap_or_default()
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

    // Plan 28 Decision 5: the projection starts at the first INCOMPLETE
    // period. sample.journal's last transaction is 2026-07-03, so July is
    // projected (and July 1–3 are left out of the openings).
    assert_eq!(
        projection["buckets"],
        json!(["2026-07", "2026-08", "2026-09"])
    );
    assert_eq!(projection["actualsEnd"], "2026-07-01");
    // The default lookback is two FULL actual periods (Decision 6), and the
    // statements run across both of them and then the forecast.
    assert_eq!(projection["lookback"], json!(["2026-05", "2026-06"]));
    assert_eq!(projection["projectedFrom"], 2);
    let keys = |columns: &Value| -> Vec<Value> {
        columns
            .as_array()
            .expect("columns")
            .iter()
            .map(|column| column["key"].clone())
            .collect()
    };
    let all = json!(["2026-05", "2026-06", "2026-07", "2026-08", "2026-09"]);
    assert_eq!(json!(keys(&projection["balanceSheet"]["columns"])), all);
    assert_eq!(json!(keys(&projection["incomeStatement"]["buckets"])), all);
    assert_eq!(json!(keys(&projection["cashFlow"]["buckets"])), all);

    // Net income is the income statement's own, in the statement's sign: an
    // expense reads negative.
    assert_eq!(net_income(&projection), [-4200, -4200, -4200]);
    let accounts = income_accounts(&projection);
    assert!(
        accounts.iter().any(|account| account == "expenses:rent"),
        "the scenario's own account must be a row: {accounts:?}"
    );

    // Opening balances come from the REAL journal, so they are not zero…
    let opening = dollars(&projection["cash"]["opening"]);
    assert!(opening > 0, "sample.journal holds cash: {opening}");
    // …and each bucket draws down by the scenario's implied cash leg.
    assert_eq!(
        series(&projection["cash"]),
        [opening - 4200, opening - 8400, opening - 12_600]
    );
    // The balance sheet balances in every column, actuals and forecast alike.
    assert_eq!(
        projection["balanceSheet"]["balanced"],
        json!([true, true, true, true, true])
    );
    // The cash flow statement's net change in cash is the scenario's rent,
    // period by period. (Its balances are as held, at cost, where the runway
    // series is valued, so only the CHANGE is comparable in `$`.)
    assert_eq!(
        projected(&projection, &projection["cashFlow"]["netChange"]["values"])
            .iter()
            .map(dollars)
            .collect::<Vec<_>>(),
        [-4200, -4200, -4200]
    );

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
    assert_eq!(with["incomeStatement"], without["incomeStatement"]);
    // The cash flow statement shows the non-cash movement (the brokerage's
    // cash part compounds through a gains posting: an investing line against a
    // financing one), but not one cent of it reaches cash.
    for line in ["netChange", "endingCash"] {
        assert_eq!(with["cashFlow"][line], without["cashFlow"][line], "{line}");
    }
    assert_eq!(with["runway"], without["runway"]);

    // Net worth, though, pulls away — by the growth alone, and only from the
    // first anniversary of the projection's start (2027-01-01).
    let (grew, flat) = (net_worth(&with), net_worth(&without));
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
    // The brokerage holds PRICED shares (AAPL, VTI), so the row grows their
    // prices in memory (Decision 3) and the balance sheet revalues them
    // through its valuation adjustment. Its `$` cash compounds through gains
    // postings to `equity:unrealized gains` instead. Both are equity, so the
    // whole gap is equity and every column still balances.
    let forecast = |projection: &Value, name: &str| {
        bs_group(projection, "equity", name)[projected_from(projection)..].to_vec()
    };
    let (revalued, plain) = (
        forecast(&with, "Valuation adjustment"),
        forecast(&without, "Valuation adjustment"),
    );
    assert!(revalued[2] > plain[2], "{revalued:?} vs {plain:?}");
    let gains = forecast(&with, "Unrealized gains");
    assert!(
        gains.first() == Some(&0) && gains.get(2).is_some_and(|gain| *gain > gains[1]),
        "the gains account is a real equity line that grows from the first \
         anniversary: {gains:?}"
    );
    let equity = |projection: &Value| {
        projected(
            projection,
            &projection["balanceSheet"]["sections"][2]["total"],
        )
        .iter()
        .map(exact)
        .collect::<Vec<_>>()
    };
    let (equity_with, equity_without) = (equity(&with), equity(&without));
    assert_eq!(
        equity_with[2] - equity_without[2],
        grew[2] - flat[2],
        "the appreciation is all equity"
    );
    assert!(
        with["balanceSheet"]["balanced"]
            .as_array()
            .expect("balanced")
            .iter()
            .all(|verdict| verdict == true)
    );
    assert!(dollars(&rows[0]["growth"]) > 0);
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
    assert_eq!(net_worth(&with), net_worth(&without));
    assert_eq!(with["incomeStatement"], without["incomeStatement"]);
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
        // …and it contributes nothing: every period is the last actual one.
        let worth = net_worth(&projection);
        assert!(worth.iter().all(|figure| *figure == worth[0]), "{worth:?}");
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

    // The ACTUAL columns are the journal's — an override restates the row's
    // balance from the cutoff, not history.
    let actual = |projection: &Value| projection["balanceSheet"]["netWorth"][1].clone();
    assert_eq!(actual(&with), actual(&without));
    // Every forecast period is shifted by the same one-off difference…
    let (adjusted, plain) = (net_worth(&with), net_worth(&without));
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
    assert_eq!(net_income(&projection), [0, 0]);
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
    // July, August, September: the cutoff is 2026-07-01 (plan 28 Decision 5).
    assert_eq!(runway["bucket"], 2);
    assert_eq!(runway["label"], "Sep 2026");
    assert_eq!(runway["date"], "2026-09-30");
    assert_eq!(runway["periods"], 3);
}

/// A period the engine cannot enumerate is a WARNING and a zero contribution —
/// never a `500`, and never a silent zero.
#[tokio::test]
async fn an_unsupported_period_is_warned_about_rather_than_failing() {
    let mut line = rent_line();
    line["period"] = json!({"raw": "every last day of month"});
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
            .is_some_and(|w| w.contains("every last day of month")),
        "the warning must quote what the user wrote: {warnings:?}"
    );
    assert_eq!(net_income(&projection), [0, 0]);
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
    // July, August, September (plan 28 Decision 5).
    assert_eq!(net_income(&projection), [0, 0, -4200]);
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

    // sample.journal spends in EUR as well as `$`, so the seed does too. EUR
    // has a `P` price, so it is converted where figures are read, and nothing
    // is carried unconverted to warn about.
    let warnings = projection["warnings"]
        .as_array()
        .expect("warnings is an array");
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// The sample.journal false alarm (plan 28 amendment 22). The seeded scenario
/// builds cash, yet the runway used to read "Cash turns negative in Jul 2026":
/// the EUR seed lines were added unvalued against a dollar-valued opening, and
/// the `$` food line defaulted to the EUR-only Wise account. And projected
/// depreciation drained checking.
#[tokio::test]
async fn the_sample_seed_raises_no_false_runway_and_infers_counters_per_commodity() {
    let (status, text) = get("/api/projections/seed?end=2026-07-08&count=12&depth=2").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let seed: Value = serde_json::from_str(&text).expect("the seed body is JSON");
    let projection = run(json!({"scenario": seed, "asOf": "2026-07-08", "count": 12})).await;

    // Total cash never closes below zero in any commodity, so no runway.
    assert_eq!(projection["runway"], Value::Null, "{}", projection["cash"]);
    for (index, value) in projection["cash"]["values"]
        .as_array()
        .expect("cash values")
        .iter()
        .enumerate()
    {
        for (commodity, qty) in value.as_object().expect("a mixed amount") {
            assert!(
                !qty["mantissa"].as_str().unwrap_or("").starts_with('-'),
                "cash is negative in {commodity} in bucket {index}: {value}"
            );
        }
    }

    let defaults = &projection["defaults"]["lines"];
    let default_of = |group: &str| defaults[group].as_str().unwrap_or_default().to_string();
    // `$` food went on the visa, paid from checking: one hop, to checking —
    // never the EUR-only Wise account.
    assert_eq!(default_of("gap:expenses:food:$"), "assets:bank:checking");
    assert_eq!(default_of("gap:expenses:food:EUR"), "assets:bank:wise:eur");
    assert_eq!(
        default_of("gap:expenses:travel:$"),
        "assets:bank:checking",
        "{defaults}"
    );
    // Depreciation books against its contra-asset, which is not cash.
    assert_eq!(
        default_of("gap:expenses:depreciation:$"),
        "assets:vehicles:car:depreciation"
    );
    // Salary still lands where it always landed.
    assert_eq!(default_of("gap:income:salary:$"), "assets:bank:checking");
    // Withheld tax pairs only with the salary, which is not where money sits,
    // so it comes out of cash.
    assert_eq!(default_of("gap:expenses:taxes:$"), "assets:bank:checking");
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
        json!(["2026-07", "2026-08", "2026-09"])
    );
    // September, not a garbage `2026-00` bucket and not nothing at all.
    assert_eq!(net_income(&projection), [0, 0, -45_000]);
}

// ---------------------------------------------------------------------------
// The projected statements (plan 28, Phase 4)
// ---------------------------------------------------------------------------

/// `lookback` is how many full ACTUAL periods precede the forecast in the
/// statements, `0` included; `projectedFrom` is where they end. The cash
/// series and the runway count from the cutoff whatever it is.
#[tokio::test]
async fn lookback_sets_the_actual_columns_and_where_projection_starts() {
    for (lookback, keys) in [
        (0, vec!["2026-07", "2026-08"]),
        (
            3,
            vec!["2026-04", "2026-05", "2026-06", "2026-07", "2026-08"],
        ),
    ] {
        let projection = run(json!({
            "scenario": scenario_with(vec![rent_line()], vec![]),
            "asOf": "2026-07-08",
            "count": 2,
            "lookback": lookback
        }))
        .await;
        assert_eq!(projection["projectedFrom"], lookback);
        assert_eq!(
            projection["lookback"].as_array().expect("lookback").len(),
            lookback
        );
        let columns: Vec<&str> = projection["balanceSheet"]["columns"]
            .as_array()
            .expect("columns")
            .iter()
            .map(|column| column["key"].as_str().expect("key"))
            .collect();
        assert_eq!(columns, keys, "lookback {lookback}");
        assert_eq!(projection["buckets"], json!(["2026-07", "2026-08"]));
        assert_eq!(series(&projection["cash"]).len(), 2);
        // Each balance-sheet column is the sheet as of its period's LAST day.
        assert_eq!(
            projection["balanceSheet"]["columns"][lookback]["to"],
            "2026-07-31"
        );
    }
}

/// The ACTUAL columns are the Reports tab's own statements for those periods
/// — the same code, the same groups, the same valuation — and a real
/// transaction dated on or after the cutoff is in no column at all.
#[tokio::test]
async fn the_actual_columns_are_the_reports_tabs_statements() {
    let projection = run(json!({
        "scenario": scenario_with(vec![rent_line()], vec![]),
        "asOf": "2026-07-08",
        "count": 1
    }))
    .await;
    assert_eq!(projection["lookback"], json!(["2026-05", "2026-06"]));

    // The balance sheet at the end of June, column for column.
    let (status, text) = get("/api/reports/balancesheet/grouped?asOf=2026-06-30").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let june: Value = serde_json::from_str(&text).expect("JSON");
    let sheet = &projection["balanceSheet"];
    assert_eq!(sheet["base"], june["base"]);
    for (section, reports) in sheet["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .zip(june["sections"].as_array().expect("sections"))
    {
        assert_eq!(section["kind"], reports["kind"]);
        assert_eq!(section["total"][1], reports["total"], "{}", section["kind"]);
        for group in reports["groups"].as_array().expect("groups") {
            let pivoted = section["groups"]
                .as_array()
                .expect("groups")
                .iter()
                .find(|g| g["name"] == group["name"] && g["term"] == group["term"])
                .unwrap_or_else(|| panic!("{} is in the pivot", group["name"]));
            assert_eq!(pivoted["total"][1], group["total"], "{}", group["name"]);
        }
    }
    assert_eq!(sheet["netWorth"][1], june["netWorth"]);

    // The income statement's two actual months.
    let (status, text) = get(
        "/api/reports/incomestatement/grouped?from=2026-05-01&to=2026-06-30&interval=monthly&compare=none",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let months: Value = serde_json::from_str(&text).expect("JSON");
    let actual = &projection["incomeStatement"]["netIncome"]["values"];
    assert_eq!(actual[0], months["netIncome"]["values"][0]);
    assert_eq!(actual[1], months["netIncome"]["values"][1]);

    // July is projected: sample.journal's real July 1–3 postings are left
    // out (Decision 5), so July's net income is the scenario's rent alone.
    assert_eq!(net_income(&projection), [-4200]);
}

// ---------------------------------------------------------------------------
// The Accounts and Journal readings (plan 28, follow-ups 2026-10-26)
// ---------------------------------------------------------------------------

/// A run asking for the Accounts reading, over July and August with May and
/// June as actuals, at depth 3.
async fn accounts_run(empty: bool, events: Vec<Value>) -> Value {
    run(json!({
        "scenario": scenario_with(vec![rent_line()], events),
        "asOf": "2026-07-08",
        "count": 2,
        "depth": 3,
        "accounts": {"empty": empty}
    }))
    .await
}

/// The Accounts report's row for `account` and its section, if it has one.
fn account_row<'a>(projection: &'a Value, account: &str) -> Option<&'a Value> {
    account_sections(projection)
        .iter()
        .flat_map(|section| section["rows"].as_array().expect("rows"))
        .find(|row| row["account"] == account)
}

fn account_sections(projection: &Value) -> &Vec<Value> {
    projection["accounts"]["sections"]
        .as_array()
        .expect("accounts sections")
}

fn account_values(projection: &Value, account: &str) -> Vec<i128> {
    series(account_row(projection, account).unwrap_or_else(|| panic!("no {account} row")))
}

/// The readings only one tab shows are computed only when asked for.
#[tokio::test]
async fn the_accounts_and_journal_readings_are_null_unless_asked_for() {
    let projection = run(json!({
        "scenario": scenario_with(vec![rent_line()], vec![]),
        "asOf": "2026-07-08",
        "count": 2
    }))
    .await;
    assert_eq!(projection["accounts"], Value::Null);
    assert_eq!(projection["journal"], Value::Null);
}

/// **Each section reads its own way** (2026-10-28): assets and liabilities
/// are each period's BALANCE with the window's Δ last, everything else each
/// period's net CHANGE with a Total last — over `lookback ++ buckets`, the
/// real journal before the cutoff and the forecast after it.
#[tokio::test]
async fn assets_are_balances_and_flows_are_changes() {
    let projection = accounts_run(false, vec![]).await;
    let keys = json!(["2026-05", "2026-06", "2026-07", "2026-08"]);
    assert_eq!(projection["accounts"]["buckets"], keys);
    assert_eq!(projection["accounts"]["periods"][2]["label"], "Jul 2026");
    let shape: Vec<(String, String)> = account_sections(&projection)
        .iter()
        .map(|section| {
            (
                section["section"].as_str().expect("section").to_string(),
                section["reading"].as_str().expect("reading").to_string(),
            )
        })
        .collect();
    for (section, reading) in &shape {
        let want = if matches!(section.as_str(), "assets" | "liabilities") {
            "balance"
        } else {
            "change"
        };
        assert_eq!(reading, want, "{section}");
    }
    assert_eq!(shape[0].0, "assets", "{shape:?}");

    // A flow: nothing real, then $4,200 a month, and a Total.
    assert_eq!(
        account_values(&projection, "expenses:rent"),
        [0, 0, 4200, 4200]
    );
    let rent = account_row(&projection, "expenses:rent").expect("rent");
    assert_eq!(dollars(&rent["summary"]), 8400);

    // Cash is a BALANCE: each column the previous plus that period's
    // movement, and the forecast's rent comes out of it — July's balance is
    // June's less $4,200 (real July 1–3 postings are left out, Decision 5).
    let checking =
        exact_series(account_row(&projection, "assets:bank:checking").expect("checking"));
    assert!(checking[0] > 0, "a balance, not a movement: {checking:?}");
    assert_eq!(checking[2], checking[1] - 4_200_000_000_000_000);
    assert_eq!(checking[3], checking[2] - 4_200_000_000_000_000);
    // (Its last column, the Δ over the window, is pinned against hledger's
    // `balance -T` in `projection_files.rs`.)
    // Natural signs, as `hledger balance` prints them: salary is negative.
    assert!(account_values(&projection, "income:salary")[0] < 0);
}

/// **Depth rolls the rows up** like every other report's slider, within each
/// section, and never the section totals.
#[tokio::test]
async fn accounts_honour_the_depth() {
    let projection = run(json!({
        "scenario": scenario_with(vec![rent_line()], vec![]),
        "asOf": "2026-07-08",
        "count": 2,
        "depth": 1,
        "accounts": {"empty": false}
    }))
    .await;
    for section in account_sections(&projection) {
        let rows = section["rows"].as_array().expect("rows");
        assert!(rows.iter().all(|row| row["depth"] == 1), "{rows:?}");
    }
    assert_eq!(account_values(&projection, "expenses")[2..], [4200, 4200]);
}

/// **Show empty accounts** adds the declared and known accounts the window
/// never touches — what a projection misses — as all-zero rows.
#[tokio::test]
async fn empty_accounts_are_shown_only_when_asked_for() {
    let hidden = accounts_run(false, vec![]).await;
    assert!(
        account_row(&hidden, "expenses:shopping").is_none(),
        "declared, never posted: hidden by default"
    );
    let shown = accounts_run(true, vec![]).await;
    let shopping = account_row(&shown, "expenses:shopping").expect("the declared account is a row");
    assert_eq!(series(shopping), [0, 0, 0, 0]);
    for section in account_sections(&hidden) {
        for row in section["rows"].as_array().expect("rows") {
            let account = row["account"].as_str().expect("account");
            assert_eq!(account_values(&shown, account), series(row), "{account}");
        }
    }
}

/// A page from before 2026-10-28 still sends the old Change/Balance `mode`;
/// it is accepted and ignored.
#[tokio::test]
async fn an_old_page_s_accounts_mode_is_ignored() {
    let old = run(json!({
        "scenario": scenario_with(vec![rent_line()], vec![]),
        "asOf": "2026-07-08",
        "count": 2,
        "depth": 3,
        "accounts": {"mode": "change", "empty": false}
    }))
    .await;
    assert_eq!(
        old["accounts"],
        accounts_run(false, vec![]).await["accounts"]
    );
}

/// **The journal is the generated transactions only**, date-ascending, in the
/// journal view's row shape and with hledger's `generated-transaction` tag —
/// what `hledger print --forecast tag:generated` prints — each naming the
/// part of the scenario it came from.
#[tokio::test]
async fn the_journal_is_the_forecast_transactions_in_print_shape() {
    let event = json!({
        "id": "laptop",
        "date": "2026-07-20",
        "description": "new laptop",
        "postings": [{
            "account": "expenses:equipment",
            "amount": {"commodity": "$", "quantity": {"mantissa": "2000", "places": 0}, "precision": 0}
        }],
        "counter": "liabilities:cc:visa"
    });
    let projection = run(json!({
        "scenario": scenario_with(vec![rent_line()], vec![event]),
        "asOf": "2026-07-08",
        "count": 2,
        "journal": true,
        "accounts": {"mode": "change"}
    }))
    .await;
    let journal = &projection["journal"];
    assert_eq!(journal["total"], 3);
    assert_eq!(journal["truncated"], false);
    let entries = journal["entries"].as_array().expect("entries");
    let dates: Vec<&str> = entries
        .iter()
        .map(|entry| entry["txn"]["date"].as_str().expect("date"))
        .collect();
    assert_eq!(dates, ["2026-07-01", "2026-07-20", "2026-08-01"]);

    let rent = &entries[0];
    assert_eq!(rent["origin"], json!({"kind": "rule", "key": "rule:0"}));
    assert_eq!(rent["txn"]["description"], "projection");
    assert!(
        rent["txn"]["tags"]
            .as_array()
            .expect("tags")
            .contains(&json!(["generated-transaction", "~ monthly"])),
        "{}",
        rent["txn"]["tags"]
    );
    let postings = rent["txn"]["postings"].as_array().expect("postings");
    let accounts: Vec<&str> = postings
        .iter()
        .map(|posting| posting["account"].as_str().expect("account"))
        .collect();
    assert_eq!(accounts, ["expenses:rent", "assets:bank:checking"]);
    let quantity = |posting: &Value| -> String {
        posting["amounts"][0]["qty"]["m"]
            .as_str()
            .expect("mantissa")
            .to_string()
    };
    assert_eq!(quantity(&postings[0]), "420000");
    assert_eq!(quantity(&postings[1]), "-420000");

    let laptop = &entries[1];
    assert_eq!(laptop["origin"], json!({"kind": "event", "key": "laptop"}));
    assert_eq!(laptop["txn"]["description"], "new laptop");
    assert_eq!(
        laptop["txn"]["postings"][1]["account"],
        "liabilities:cc:visa"
    );

    // No real transaction is in it: none carries a source position or lacks
    // the generated tag.
    assert!(entries.iter().all(|entry| {
        entry["txn"]["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .any(|tag| tag[0] == "generated-transaction")
    }));

    // The two readings agree: the forecast columns of the Accounts report are
    // the journal's postings, summed per account and month.
    assert_eq!(
        account_values(&projection, "expenses:equipment")[2..],
        [2000, 0]
    );
    assert_eq!(
        account_values(&projection, "expenses:rent")[2..],
        [4200, 4200]
    );
}

/// An asset row's appreciation is a `growth` entry naming the row.
#[tokio::test]
async fn an_asset_rows_growth_is_named_in_the_journal() {
    let projection = run(json!({
        "scenario": scenario_with(vec![asset_row("assets:vehicles:car", "12")], vec![]),
        "asOf": "2026-07-08",
        "interval": "yearly",
        "lookback": 0,
        "count": 3,
        "journal": true
    }))
    .await;
    let entries = projection["journal"]["entries"]
        .as_array()
        .expect("entries");
    assert!(
        !entries.is_empty(),
        "the car grows: {}",
        projection["warnings"]
    );
    for entry in entries {
        assert_eq!(entry["origin"], json!({"kind": "growth", "key": "rule:1"}));
        assert_eq!(
            entry["txn"]["postings"][0]["account"],
            "assets:vehicles:car"
        );
        // The search text the Journal tab filters on: the journal's own
        // haystack, lowercase, holding the description and every account.
        let haystack = entry["haystack"].as_str().expect("a haystack");
        assert_eq!(haystack, haystack.to_lowercase());
        assert!(haystack.contains("assets:vehicles:car"), "{haystack:?}");
        let description = entry["txn"]["description"].as_str().expect("description");
        assert!(
            haystack.contains(&description.to_lowercase()),
            "{haystack:?}"
        );
    }
}

/// **An UNPRICED asset's appreciation is a posting to a gains account**
/// (Decisions 3 and 4): the asset rises, `equity:unrealized gains` rises with
/// it, the sheet balances, and neither net income nor cash moves.
#[tokio::test]
async fn an_unpriced_asset_row_grows_into_the_gains_account() {
    let body = |lines: Vec<Value>| {
        json!({
            "scenario": scenario_with(lines, vec![]),
            "asOf": "2026-07-08",
            "interval": "yearly",
            "count": 3,
            "lookback": 0
        })
    };
    let without = run(body(vec![])).await;
    let with = run(body(vec![asset_row("assets:vehicles:car", "10")])).await;

    let gains = bs_group(&with, "equity", "Unrealized gains");
    assert_eq!(gains.len(), 3, "a gains line in every column: {gains:?}");
    assert_eq!(gains[0], 0, "no anniversary inside the first year");
    assert!(gains[1] > 0 && gains[2] > gains[1], "{gains:?}");
    assert!(bs_group(&without, "equity", "Unrealized gains").is_empty());

    // The asset side moved by exactly the gains: A = L + E, by construction.
    let (grew, flat) = (net_worth(&with), net_worth(&without));
    for column in 0..3 {
        assert_eq!(
            grew[column] - flat[column],
            gains[column],
            "column {column}"
        );
    }
    assert!(
        with["balanceSheet"]["balanced"]
            .as_array()
            .expect("balanced")
            .iter()
            .all(|verdict| verdict == true)
    );
    // Paper gains are not income and not cash.
    assert_eq!(with["incomeStatement"], without["incomeStatement"]);
    assert_eq!(with["cash"], without["cash"]);
    // The default gains account is on the wire for the table to grey out.
    assert_eq!(with["defaults"]["gains"], "equity:unrealized gains");
}

/// **What the table greys out.** A line or event with no counter-account gets
/// the engine's inferred default, keyed by rule (or event id); one that names
/// its own gets nothing, because it needs no default.
#[tokio::test]
async fn the_inferred_defaults_are_on_the_wire() {
    let mut paid = rent_line();
    paid["id"] = json!("paid");
    paid["group"] = json!("rule:9");
    paid["counter"] = json!("assets:bank:savings");
    let event = json!({
        "id": "legal",
        "date": "2026-08-15",
        "description": "lawyer",
        "postings": [{
            "account": "expenses:legal",
            "amount": {"commodity": "$", "quantity": {"mantissa": "500", "places": 0}, "precision": 0}
        }]
    });
    let projection = run(json!({
        "scenario": scenario_with(vec![rent_line(), paid], vec![event]),
        "asOf": "2026-07-08",
        "count": 2
    }))
    .await;
    let defaults = &projection["defaults"];
    // sample.journal pays its rent from checking, so history picks it.
    assert_eq!(defaults["lines"], json!({"rule:0": "assets:bank:checking"}));
    assert_eq!(defaults["events"]["legal"], "assets:bank:checking");
    assert!(defaults["cash"].is_string());
    assert_eq!(defaults["gains"], "equity:unrealized gains");

    // And the explicit counter is honoured: the paid line's rent comes out of
    // savings, which is cash too, so cash drops by both rents (and the lawyer
    // in August) while the balance sheet still balances.
    let opening = dollars(&projection["cash"]["opening"]);
    assert_eq!(
        series(&projection["cash"]),
        [opening - 8400, opening - 17_300]
    );
    assert_eq!(
        projection["balanceSheet"]["balanced"],
        json!([true, true, true, true])
    );
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
        // A display precision past the parser's own scale cap (SEC-5): the
        // engine rounds to it every step and the writer pads to it.
        (
            {
                let mut line = rent_line();
                line["amount"]["precision"] = json!(65_535);
                scenario_with(vec![line], vec![])
            },
            "precision",
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
    // The statements carry `lookback + count` columns, held to the same cap.
    let (status, body) = post(with(json!({"count": 1200, "lookback": 1}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("lookback"), "{body}");
    let (status, _) = post(with(json!({"count": 1198, "lookback": 2}))).await;
    assert_eq!(status, StatusCode::OK);

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

/// One rule per period form the what-if table must edit, with and without a
/// two-space description.
const BOUNDED: &str = "\
; Ledgeline projection
; projection: Bounded
; created: 2026-01-01
; updated: 2026-01-01

~ monthly from 2026-11-01  unbudgeted — average over 2025-11-01 to 2026-10-22
    (expenses:a)  $1.00

~ monthly from 2026-11-01
    (expenses:b)  $1.00

~ monthly to 2027-06-01  ends
    (expenses:c)  $1.00

~ yearly from 2026-11-01 to 2030-01-01
    (expenses:d)  $1.00

~ every 3 months from 2026-11-01  quarterly-ish
    (expenses:e)  $1.00

~ every 15th day of month from 2026-11-01  mid-month
    (expenses:f)  $1.00

~ every last day of month  workdays
    (expenses:g)  $1.00
";

/// The report: `~ monthly from 2026-11-01  …` showed its whole `raw` in the
/// Per column and locked From/To, because `simple` was answered by
/// `PeriodSpec::interval`, which is `None` for ANY bounded rule. A bounded
/// rule on one of the five words is `simple` now; one the editor cannot spell
/// still names the words its bounds can be rewritten around (`base`); and a
/// save of an edited bound writes hledger's syntax with the description kept.
#[tokio::test]
async fn a_bounded_rule_reads_as_its_interval_and_saves_an_edited_bound() {
    let uri = "/api/projections/projection-bounded.journal";
    let tree = Tree::new(&[("projection-bounded.journal", BOUNDED)]);
    let (status, doc) = file_json(&tree.state, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let periods: Vec<Value> = doc["scenario"]["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .map(|line| line["period"].clone())
        .collect();
    let expect = |raw: &str, simple: Value, base: Value, from: Value, to: Value| json!({"raw": raw, "simple": simple, "base": base, "from": from, "to": to});
    let null = Value::Null;
    assert_eq!(
        periods,
        vec![
            expect(
                "monthly from 2026-11-01",
                json!("monthly"),
                null.clone(),
                json!("2026-11-01"),
                null.clone()
            ),
            expect(
                "monthly from 2026-11-01",
                json!("monthly"),
                null.clone(),
                json!("2026-11-01"),
                null.clone()
            ),
            expect(
                "monthly to 2027-06-01",
                json!("monthly"),
                null.clone(),
                null.clone(),
                json!("2027-06-01")
            ),
            expect(
                "yearly from 2026-11-01 to 2030-01-01",
                json!("yearly"),
                null.clone(),
                json!("2026-11-01"),
                json!("2030-01-01")
            ),
            expect(
                "every 3 months from 2026-11-01",
                null.clone(),
                json!("every 3 months"),
                json!("2026-11-01"),
                null.clone()
            ),
            expect(
                "every 15th day of month from 2026-11-01",
                null.clone(),
                json!("every 15th day of month"),
                json!("2026-11-01"),
                null.clone()
            ),
            expect(
                "every last day of month",
                null.clone(),
                null.clone(),
                null.clone(),
                null
            ),
        ]
    );

    // Edit two bounds the way the table does — `raw` rebuilt from the derived
    // fields — and hand everything else back untouched.
    let revision = doc["revision"].clone();
    let mut scenario = doc["scenario"].clone();
    scenario["lines"][0]["period"]["raw"] = json!("monthly from 2026-12-01 to 2027-12-01");
    scenario["lines"][5]["period"]["raw"] = json!("every 15th day of month from 2027-01-01");
    let (status, saved) = file_json(
        &tree.state,
        "PUT",
        uri,
        Some(json!({"revision": revision, "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let written = tree.read("projection-bounded.journal");
    for header in [
        "~ monthly from 2026-12-01 to 2027-12-01  unbudgeted — average over 2025-11-01 to 2026-10-22\n",
        "~ monthly from 2026-11-01\n",
        "~ monthly to 2027-06-01  ends\n",
        "~ every 15th day of month from 2027-01-01  mid-month\n",
        "~ every last day of month  workdays\n",
    ] {
        assert!(
            written.contains(header),
            "missing {header:?} in:\n{written}"
        );
    }
    assert_eq!(
        saved["scenario"]["lines"][0]["period"]["to"],
        json!("2027-12-01")
    );
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

#[tokio::test]
async fn an_edit_made_outside_ledgeline_is_read_by_the_next_get_with_a_new_revision() {
    // The tab notices a file edited in another program by re-reading it and
    // comparing revisions, so the engine must never answer from a cached parse:
    // the second GET has to see the new bytes AND say they are new.
    let tree = Tree::standard();
    let (before, _) = open_plan(&tree.state).await;

    let edited = tree
        .read("plans/projection-plan.journal")
        .replace("$4200.00", "$4300.00");
    std::fs::write(tree.dir.join("plans/projection-plan.journal"), edited).expect("edit");

    let (after, scenario) = open_plan(&tree.state).await;
    assert_ne!(after, before, "an edited file must carry a new revision");
    assert_eq!(
        scenario["lines"][0]["amount"]["quantity"]["mantissa"],
        json!("430000")
    );
}

#[tokio::test]
async fn a_save_from_before_an_outside_edit_is_refused_and_keeps_that_edit() {
    // The protection the tab relies on when it holds unsaved work over a file
    // somebody else changed: it keeps the OLD revision, so its save is a 409
    // and the other program's edit survives byte for byte.
    let tree = Tree::standard();
    let (revision, scenario) = open_plan(&tree.state).await;
    let edited = tree
        .read("plans/projection-plan.journal")
        .replace("$4200.00", "$4300.00");
    std::fs::write(tree.dir.join("plans/projection-plan.journal"), &edited).expect("edit");

    let (status, message) = file_text(
        &tree.state,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": revision, "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{message}");
    assert_eq!(tree.read("plans/projection-plan.journal"), edited);
}

#[tokio::test]
async fn a_file_that_does_not_parse_is_a_400_about_reading_it_not_a_500_about_writing() {
    // A hand edit with a typo in it is the user's to fix. It used to be a 500
    // saying the file "was NOT written", which read as an engine fault and
    // described a write nobody had attempted.
    let tree = Tree::new(&[(
        "projection-broken.journal",
        "~ monthly  plan\n    (expenses:rent)  $4200.00 @@@ ((\n",
    )]);
    let (status, message) = file_text(
        &tree.state,
        "GET",
        "/api/projections/projection-broken.journal",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert!(
        message.contains("cannot be opened as a projection"),
        "{message}"
    );
    assert!(message.contains("line 2"), "names where to look: {message}");
    assert!(!message.contains("NOT written"), "{message}");
    for secret in tree.secret_paths() {
        assert!(!message.contains(&secret), "leaked a path: {message}");
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
    // this rule are laid out against the longer of the two accounts. The rule
    // did not balance, so the save also UPGRADED it (plan 28): plain postings
    // and the amountless default counter-account hledger balances them with.
    assert!(after.contains("    expenses:rent    $4500.00\n"), "{after}");
    assert!(after.contains("    assets:cash\n"), "{after}");
    // `updated:` is the SERVER's today, never the client's — a timestamp a
    // caller supplies is a timestamp that can say anything.
    assert!(
        !after.contains("; updated: 2026-01-01\n"),
        "updated: was not touched:\n{after}"
    );
    // The other line of the same rule is untouched in content.
    assert!(
        after.contains("    revenues:salary  $-12000.00\n"),
        "{after}"
    );
}

/// A save takes the same scenario `run` does, so it gets the same 256 KiB body
/// limit rather than axum's 2 MiB default. The body is the valid save the test
/// above makes, padded with insignificant JSON whitespace, so the byte limit is
/// the only thing that can refuse it.
#[tokio::test]
async fn a_save_body_past_the_route_limit_is_refused() {
    let tree = Tree::standard();
    let before = tree.read("plans/projection-plan.journal");
    let (revision, mut scenario) = open_plan(&tree.state).await;
    scenario["lines"][0]["amount"]["quantity"]["mantissa"] = json!("450000");
    let save = json!({"revision": revision, "scenario": scenario}).to_string();
    let padded = format!("{save}{}", " ".repeat(400 * 1024));

    let put = |body: String| {
        Request::builder()
            .method("PUT")
            .uri(PLAN_URI)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .expect("request builds")
    };
    let status = |request: Request<Body>| router_with_state(tree.state.clone()).oneshot(request);
    let refused = status(put(padded)).await.expect("router responds");
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(tree.read("plans/projection-plan.journal"), before);
    // The same save, unpadded, goes through.
    let saved = status(put(save)).await.expect("router responds");
    assert_eq!(saved.status(), StatusCode::OK);
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
        written.contains("    expenses:rent    $4200.00\n"),
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

// ---------------------------------------------------------------------------
// Magnitudes in, signs and sections out (plan 26 B6)
// ---------------------------------------------------------------------------

/// An income line sent as a MAGNITUDE (`entry`) projects exactly as the same
/// line sent signed the way hledger writes it — so a client need not know that
/// revenue is negative.
#[tokio::test]
async fn an_entry_is_signed_by_the_accounts_type() {
    let income = |quantity: &str, entry: Option<&str>| {
        let mut line = json!({
            "id": "salary",
            "group": "rule:0",
            "account": "income:salary",
            "amount": {"commodity": "$", "quantity": {"mantissa": quantity, "places": 2}, "precision": 2},
            "period": {"raw": "monthly"},
            "growth": null,
            "note": "",
            "source": "journal"
        });
        if let Some(entry) = entry {
            line["entry"] = json!({"mantissa": entry, "places": 2});
        }
        line
    };
    let window = |line: Value| {
        json!({
            "scenario": scenario_with(vec![line], vec![]),
            "asOf": "2026-07-08",
            "interval": "monthly",
            "count": 2,
            "depth": 2
        })
    };
    let signed = run(window(income("-500000", None))).await;
    // The quantity beside an entry is ignored, whatever its sign; a NEGATIVE
    // entry is still a magnitude.
    let from_entry = run(window(income("0", Some("500000")))).await;
    let from_negative_entry = run(window(income("123", Some("-500000")))).await;
    assert_eq!(signed["incomeStatement"], from_entry["incomeStatement"]);
    assert_eq!(
        signed["incomeStatement"],
        from_negative_entry["incomeStatement"]
    );
    assert_eq!(
        net_income(&signed),
        [5000, 5000],
        "income projects as income"
    );

    // An expense entry stays positive.
    let mut rent = rent_line();
    rent["amount"]["quantity"]["mantissa"] = json!("-1");
    rent["entry"] = json!({"mantissa": "420000", "places": 2});
    let projection = run(window(rent)).await;
    assert_eq!(net_income(&projection), [-4200, -4200]);
}

/// A saved entry is WRITTEN with the journal's sign, every outbound line says
/// which section it belongs in, and a line handed straight back — `section`
/// and all — is accepted.
#[tokio::test]
async fn a_save_writes_an_entry_signed_and_lines_carry_their_section() {
    let tree = Tree::standard();
    let (revision, mut scenario) = open_plan(&tree.state).await;
    let sections: Vec<&str> = scenario["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .map(|line| line["section"].as_str().expect("section"))
        .collect();
    assert_eq!(sections, ["expense", "income"]);

    scenario["lines"][1]["entry"] = json!({"mantissa": "1300000", "places": 2});
    let (status, doc) = file_json(
        &tree.state,
        "PUT",
        PLAN_URI,
        Some(json!({"revision": revision, "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let after = tree.read("plans/projection-plan.journal");
    assert!(
        after.contains("    revenues:salary  $-13000.00\n"),
        "{after}"
    );
    assert_eq!(doc["scenario"]["lines"][1]["section"], "income");
}

// ---------------------------------------------------------------------------
// A parent mistyped as an asset over expense-typed children
// ---------------------------------------------------------------------------

/// `fixtures/projections-expense-as-asset/typo.journal`: `expenses:ga` is
/// declared `type:A` (meant `X`) while its subaccounts declare `type:X`. The
/// actual reports are right, because nothing posts to the parent. The seed
/// used to clip the subaccounts' gaps onto `expenses:ga` at depth 2, so the
/// projection booked $5,200 a month to an ASSET: it accumulated on the
/// balance sheet, became a cash-flow operating line and left the P&L.
#[tokio::test]
async fn the_seed_never_books_an_expense_to_a_parent_typed_as_an_asset() {
    let path = common::fixtures_dir().join("projections-expense-as-asset/typo.journal");
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    let journal =
        ledgeline_core::parse_journal(&text, &path.to_string_lossy()).expect("fixture parses");
    let call = async |request: Request<Body>| {
        let response = app(&journal)
            .oneshot(request)
            .await
            .expect("router responds");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes();
        serde_json::from_slice::<Value>(&body).expect("the body is JSON")
    };

    let seed = call(
        Request::builder()
            .uri("/api/projections/seed?count=12&depth=2&end=2026-10-22")
            .body(Body::empty())
            .expect("request builds"),
    )
    .await;
    let accounts: Vec<&str> = seed["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .map(|line| line["account"].as_str().expect("account"))
        .collect();
    assert_eq!(
        accounts,
        [
            "revenues:sales",
            "expenses:ga:rent",
            "expenses:ga:software",
            "expenses:ga:insurance"
        ]
    );

    let body = json!({"scenario": seed, "asOf": "2026-10-26", "count": 3, "lookback": 2});
    let run = call(
        Request::builder()
            .method("POST")
            .uri("/api/projections/run")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request builds"),
    )
    .await;
    let sheet = run["balanceSheet"].to_string();
    assert!(
        !sheet.contains("expenses"),
        "an expense on the balance sheet: {sheet}"
    );
    let flows = run["cashFlow"]["sections"].to_string();
    assert!(
        !flows.contains("expenses"),
        "an expense as a cash-flow line: {flows}"
    );
    // The projected G&A is on the P&L: $5,200 a month across the three leaves.
    let opex = run["incomeStatement"]["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .find(|section| section["kind"] == "opex")
        .expect("an opex section");
    assert_eq!(
        opex["total"]["values"][2]["$"],
        json!({"mantissa": "520000", "places": 2}),
        "{opex}"
    );
}

// ---------------------------------------------------------------------------
// Mix-and-match depth (plan 28, follow-ups 2026-10-26)
// ---------------------------------------------------------------------------

/// One monthly `$` expense line on `account`, under a rule of its own.
fn expense_line(id: &str, account: &str, mantissa: &str) -> Value {
    json!({
        "id": id,
        "group": id,
        "account": account,
        "amount": {"commodity": "$", "quantity": {"mantissa": mantissa, "places": 2}, "precision": 2},
        "period": {"raw": "monthly"},
        "growth": null,
        "note": "",
        "source": "journal"
    })
}

/// **A line on a PARENT beside lines on its leaves.** The seed starts at the
/// page's depth, but after that a user may write a line at any level — here
/// `expenses:food` itself, whose own history is empty, beside a line on its
/// child `expenses:food:groceries`. The statements must show the parent as a
/// row of its own under the Expenses group, reading as its subtree, with the
/// child's row beneath it, and classify both as expenses (the parent is undeclared, so its
/// type is inferred from `expenses`'s declared `type: X`).
#[tokio::test]
async fn a_line_on_a_parent_account_is_its_own_row_beside_its_children() {
    let projection = run(json!({
        "scenario": scenario_with(vec![
            expense_line("food", "expenses:food", "30000"),
            expense_line("groceries", "expenses:food:groceries", "20000"),
        ], vec![]),
        "interval": "monthly",
        "count": 2,
        "lookback": 0,
        "depth": 9
    }))
    .await;
    let section = projection["incomeStatement"]["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .find(|section| section["kind"] == "opex")
        .expect("an Expenses section");
    let food = section["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .find(|group| group["name"] == "Food")
        .unwrap_or_else(|| panic!("a Food group: {section}"));
    let rows: Vec<(String, u64, Vec<i128>)> = food["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| {
            (
                row["account"].as_str().expect("account").to_string(),
                row["depth"].as_u64().expect("depth"),
                row["amounts"]["values"]
                    .as_array()
                    .expect("values")
                    .iter()
                    .map(dollars)
                    .collect(),
            )
        })
        .collect();
    // The parent keeps a row of its own, and the child's $200 sits beneath it
    // one level down — neither folded into the other nor lost. The parent's row
    // reads as its whole SUBTREE, its own $300 plus the $200 beneath it, as
    // `hledger is --tree` and the balance sheet both print a parent (2026-10-28:
    // per-person wage lines under `wages-salaries` left that row at zero when
    // a row held only its own postings).
    assert_eq!(
        rows,
        [
            ("expenses:food".to_string(), 2, vec![500, 500]),
            ("expenses:food:groceries".to_string(), 3, vec![200, 200]),
        ],
        "{food}"
    );
    // The group total is both, and both are expenses: net income is -$500.
    assert_eq!(series(&food["total"]), [500, 500]);
    assert_eq!(net_income(&projection), [-500, -500]);
}

/// **The page's depth slider reaches the income statement** (2026-10-28: it
/// changed nothing there). At depth 2 the child folds into its parent, as
/// `hledger is --depth 2` prints it; at depth 3 it is its own row again. The
/// group total is the same either way.
#[tokio::test]
async fn the_depth_slider_clips_the_income_statement_rows() {
    let food_rows = |depth: u64| async move {
        let projection = run(json!({
            "scenario": scenario_with(vec![
                expense_line("food", "expenses:food", "30000"),
                expense_line("groceries", "expenses:food:groceries", "20000"),
            ], vec![]),
            "interval": "monthly",
            "count": 2,
            "lookback": 0,
            "depth": depth
        }))
        .await;
        let food = projection["incomeStatement"]["sections"]
            .as_array()
            .expect("sections")
            .iter()
            .find(|section| section["kind"] == "opex")
            .and_then(|section| {
                section["groups"]
                    .as_array()
                    .expect("groups")
                    .iter()
                    .find(|group| group["name"] == "Food")
                    .cloned()
            })
            .expect("a Food group");
        assert_eq!(series(&food["total"]), [500, 500], "depth {depth}");
        food["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|row| {
                (
                    row["account"].as_str().expect("account").to_string(),
                    series(&row["amounts"]),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        food_rows(2).await,
        [("expenses:food".to_string(), vec![500, 500])]
    );
    assert_eq!(
        food_rows(3).await,
        [
            ("expenses:food".to_string(), vec![500, 500]),
            ("expenses:food:groceries".to_string(), vec![200, 200]),
        ]
    );
}

/// **The seed names its categories at the depth it is asked for.** The tab
/// seeds at its depth slider's value (plan 28, follow-ups 2026-10-26), so the
/// starting rows have to follow it: deeper asks for finer accounts, and no
/// seeded row is ever deeper than the ask.
#[tokio::test]
async fn the_seed_names_its_categories_at_the_requested_depth() {
    let accounts_at = |depth: usize| async move {
        let (status, text) = get(&format!(
            "/api/projections/seed?end=2026-07-08&count=12&depth={depth}"
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let seed: Value = serde_json::from_str(&text).expect("the seed body is JSON");
        seed["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .filter(|line| line["source"] == "unbudgeted")
            .map(|line| line["account"].as_str().expect("account").to_string())
            .collect::<Vec<_>>()
    };
    let shallow = accounts_at(2).await;
    let deep = accounts_at(3).await;
    assert!(!shallow.is_empty() && !deep.is_empty());
    assert!(
        shallow
            .iter()
            .all(|account| account.split(':').count() <= 2),
        "{shallow:?}"
    );
    assert!(
        deep.iter().all(|account| account.split(':').count() <= 3),
        "{deep:?}"
    );
    // sample.journal's food is `expenses:food:groceries` and
    // `expenses:food:restaurants`: one row at depth 2, two at depth 3.
    assert!(
        shallow.iter().any(|account| account == "expenses:food"),
        "{shallow:?}"
    );
    assert!(
        deep.iter()
            .any(|account| account == "expenses:food:groceries"),
        "{deep:?}"
    );
    assert!(
        !deep.iter().any(|account| account == "expenses:food"),
        "{deep:?}"
    );
}

// ---------------------------------------------------------------------------
// `= QUERY` auto-posting rules (plan 30)
// ---------------------------------------------------------------------------

/// The scenario with `rules` riding on it, as the table sends them back.
fn with_rules(rules: &[&str]) -> Value {
    let mut scenario = scenario_with(vec![rent_line()], vec![]);
    scenario["autoRules"] = json!(
        rules
            .iter()
            .map(|raw| json!({"raw": raw}))
            .collect::<Vec<_>>()
    );
    scenario
}

/// **A scenario's rule generates its postings in the run** — below the
/// posting it matched, on every forecast transaction, with hledger's tag.
#[tokio::test]
async fn a_scenario_s_auto_rule_generates_its_postings_in_the_run() {
    let projection = run(json!({
        "scenario": with_rules(&[
            "= expenses:rent tag:_generated-transaction\n    expenses:rent-tax  *0.1\n    assets:checking\n",
        ]),
        "interval": "monthly",
        "count": 2,
        "lookback": 0,
        "journal": true
    }))
    .await;
    let entries = projection["journal"]["entries"]
        .as_array()
        .expect("entries");
    assert!(!entries.is_empty());
    for entry in entries {
        let postings = entry["txn"]["postings"].as_array().expect("postings");
        let at = postings
            .iter()
            .position(|posting| posting["account"] == "expenses:rent")
            .expect("the rent posting");
        let tax = &postings[at + 1];
        assert_eq!(tax["account"], "expenses:rent-tax", "{entry}");
        // $420.000 is 10% of $4200.00.
        let (mantissa, places) = posting_qty(tax);
        assert_eq!(mantissa / 10_i128.pow(places), 420, "{tax}");
        assert!(
            tax["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag[0] == "generated-posting"),
            "{tax}"
        );
    }
    // A tagged rule draws no hledger-parity warning. (Its accounts are new
    // to the sample journal, so Decision 11's typo check does speak.)
    let warnings = projection["warnings"].to_string();
    assert!(!warnings.contains("would also apply"), "{warnings}");
    assert!(!warnings.contains("refuses the file"), "{warnings}");
}

/// A percentage form, as the rule editor sends an edited rule.
fn percent_form(match_account: &str, target: &str, percent: &str, counter: Option<&str>) -> Value {
    json!({
        "matchAccount": match_account,
        "extraQuery": "",
        "postings": [{
            "account": target,
            "kind": "percent",
            "percent": {"mantissa": percent, "places": 0},
            "amount": null
        }],
        "counter": counter,
        "comment": ""
    })
}

/// An edited rule travels as its FORM: the engine renders it — anchored,
/// forecast-only, the counter as the opposite multiplier — signs it by the
/// accounts' types, and applies it like any other rule.
#[tokio::test]
async fn a_rule_sent_as_a_form_is_rendered_and_applied() {
    let mut scenario = scenario_with(vec![rent_line()], vec![]);
    scenario["autoRules"] = json!([{
        "block": null,
        "form": percent_form("expenses:rent", "expenses:rent-tax", "10", Some("assets:bank:checking"))
    }]);
    let projection = run(json!({
        "scenario": scenario,
        "interval": "monthly",
        "count": 2,
        "lookback": 0,
        "journal": true
    }))
    .await;
    let entries = projection["journal"]["entries"]
        .as_array()
        .expect("entries");
    assert!(!entries.is_empty());
    for entry in entries {
        let postings = entry["txn"]["postings"].as_array().expect("postings");
        let at = postings
            .iter()
            .position(|posting| posting["account"] == "expenses:rent")
            .expect("the rent posting");
        let (tax, cash) = (&postings[at + 1], &postings[at + 2]);
        assert_eq!(tax["account"], "expenses:rent-tax", "{entry}");
        assert_eq!(cash["account"], "assets:bank:checking", "{entry}");
        let whole = |posting: &Value| {
            let (mantissa, places) = posting_qty(posting);
            mantissa / 10_i128.pow(places)
        };
        assert_eq!((whole(tax), whole(cash)), (420, -420), "{entry}");
    }
    // Rendered forecast-only, so no parity warning; `expenses:rent-tax` is
    // new, which Decision 11 names.
    let warnings = projection["warnings"].to_string();
    assert!(!warnings.contains("would also apply"), "{warnings}");
    assert!(warnings.contains("`expenses:rent-tax`"), "{warnings}");
    assert!(!warnings.contains("`assets:bank:checking`"), "{warnings}");
}

/// A raw rule's fixed amount is read under the main journal's commodity
/// styles, as GET reads it and as hledger reads an `include`d file: `$1,500`
/// under `commodity $1,000.00` is fifteen hundred dollars, not $1.50.
#[tokio::test]
async fn a_raw_rule_s_digit_grouped_amount_books_its_full_value_in_the_run() {
    let projection = run(json!({
        "scenario": with_rules(&[
            "= expenses:rent tag:_generated-transaction\n    expenses:rent-tax  $1,500\n    assets:bank:checking\n",
        ]),
        "interval": "monthly",
        "count": 2,
        "lookback": 0,
        "journal": true
    }))
    .await;
    let entries = projection["journal"]["entries"]
        .as_array()
        .expect("entries");
    assert!(!entries.is_empty());
    for entry in entries {
        let postings = entry["txn"]["postings"].as_array().expect("postings");
        let tax = postings
            .iter()
            .find(|posting| posting["account"] == "expenses:rent-tax")
            .expect("the generated posting");
        let (mantissa, places) = posting_qty(tax);
        assert_eq!(mantissa, 1500 * 10_i128.pow(places), "{tax}");
    }
}

/// Decision 11: a rule posting to an account the journal neither declares nor
/// uses — the user's real typo — is warned about, by name, and still runs.
#[tokio::test]
async fn a_rule_posting_to_an_unknown_account_is_warned_about() {
    let projection = run(json!({
        "scenario": with_rules(&[
            "= expenses:rent tag:_generated-transaction\n    expesnses:people-costs:payroll-tax  *0.1\n    assets:bank:checking  *-0.1\n",
        ]),
        "interval": "monthly",
        "count": 2,
        "lookback": 0
    }))
    .await;
    let warnings = projection["warnings"].to_string();
    assert!(
        warnings.contains("`expesnses:people-costs:payroll-tax`")
            && warnings.contains("check the spelling"),
        "{warnings}"
    );
    assert!(!warnings.contains("`assets:bank:checking`"), "{warnings}");
}

/// A rule carries exactly one of its text and its form, and a form's fields
/// are checked before anything is rendered.
#[tokio::test]
async fn a_malformed_rule_form_is_a_400_naming_the_problem() {
    let form = percent_form("expenses:rent", "expenses:rent-tax", "10", None);
    let mut too_many = form.clone();
    too_many["postings"] = json!(vec![form["postings"][0].clone(); 51]);
    let mut wrong_kind = form.clone();
    wrong_kind["postings"][0]["kind"] = json!("share");
    let mut mixed = form.clone();
    mixed["postings"] = json!([
        form["postings"][0].clone(),
        {"account": "expenses:fee", "kind": "fixed", "percent": null,
         "amount": {"commodity": "$", "quantity": {"mantissa": "5", "places": 0}, "precision": 0}}
    ]);
    mixed["counter"] = json!("assets:bank:checking");
    let mut blank = form.clone();
    blank["matchAccount"] = json!("  ");
    for (rule, says) in [
        (
            json!({"raw": "= food\n    (x)  1\n", "form": form.clone()}),
            "exactly one of",
        ),
        (json!({"block": 0}), "exactly one of"),
        (json!({"form": too_many}), "at most 50 postings"),
        (json!({"form": wrong_kind}), "kind 'share'"),
        (json!({"form": mixed}), "not a mix"),
        (json!({"form": blank}), "needs an account to match"),
    ] {
        let mut scenario = scenario_with(vec![rent_line()], vec![]);
        scenario["autoRules"] = json!([rule]);
        let (status, text) = post(json!({
            "scenario": scenario,
            "interval": "monthly",
            "count": 2,
            "lookback": 0
        }))
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{rule}: {text}");
        assert!(text.contains(says), "{rule}: {text}");
    }
}

/// A rule with no `tag:_generated-transaction` still applies only to the
/// forecast here, and the run says what stock hledger would do with it.
#[tokio::test]
async fn an_untagged_rule_applies_to_the_forecast_and_says_hledger_would_not_stop_there() {
    let projection = run(json!({
        "scenario": with_rules(&["= expenses:rent\n    (budget:rent)  *-1\n"]),
        "interval": "monthly",
        "count": 2,
        "lookback": 0
    }))
    .await;
    assert!(
        projection["warnings"]
            .to_string()
            .contains("would also apply it to recorded ones"),
        "{}",
        projection["warnings"]
    );
}

/// A rule's text is one `=` block and nothing else: a smuggled directive, a
/// transaction, a blank line splitting the block, or two rules in one are
/// refused before the parser acts on any of it.
#[tokio::test]
async fn a_rule_that_is_not_exactly_one_block_is_refused() {
    for raw in [
        "include /etc/passwd\n",
        "= food\n    (x)  1\ninclude other.journal\n",
        "= food\n    (x)  1\n\n    (y)  2\n",
        "2026-01-01 txn\n    a  1\n    b\n",
        "= food\n    (x)  1\n= rent\n    (y)  1\n",
    ] {
        let (status, text) = post(json!({
            "scenario": with_rules(&[raw]),
            "interval": "monthly",
            "count": 2,
            "lookback": 0
        }))
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{raw:?}: {text}");
        assert!(text.contains("auto rule"), "{raw:?}: {text}");
    }
}

/// A body from before rules travelled — no `autoRules` at all — still runs.
#[tokio::test]
async fn a_body_without_auto_rules_still_runs() {
    let projection = run(json!({
        "scenario": scenario_with(vec![rent_line()], vec![]),
        "interval": "monthly",
        "count": 2,
        "lookback": 0
    }))
    .await;
    assert!(projection["incomeStatement"].is_object());
}

/// The user's payroll-tax rule on the PARENT account fires for per-person
/// CHILD accounts (2026-10-28). With two people in one `~` group (one
/// transaction holding both), Ledgeline still taxes each, but the run warns
/// that stock `hledger --auto` refuses that file — and says nothing when each
/// person has a group of their own, which hledger reads fine.
#[tokio::test]
async fn a_parent_account_rule_taxes_each_child_and_warns_when_hledger_would_refuse() {
    let person = |account: &str, group: &str, mantissa: &str| {
        let mut line = expense_line(account, account, mantissa);
        line["group"] = json!(group);
        line
    };
    let rule = "= expenses:people-costs:wages-salaries tag:_generated-transaction\n    expenses:people-costs:payroll-tax  *0.1\n    assets:checking\n";
    for (shared, warned) in [(true, true), (false, false)] {
        let mut scenario = scenario_with(
            vec![
                person(
                    "expenses:people-costs:wages-salaries:george",
                    "rule:0",
                    "600000",
                ),
                person(
                    "expenses:people-costs:wages-salaries:john",
                    if shared { "rule:0" } else { "rule:1" },
                    "500000",
                ),
            ],
            vec![],
        );
        scenario["autoRules"] = json!([{"raw": rule}]);
        let projection = run(json!({
            "scenario": scenario,
            "interval": "monthly",
            "count": 2,
            "lookback": 0,
            "journal": true
        }))
        .await;
        // Each person's wages are followed by their own 10%.
        let mut taxes: Vec<(String, i128)> = Vec::new();
        for entry in projection["journal"]["entries"]
            .as_array()
            .expect("entries")
        {
            let postings = entry["txn"]["postings"].as_array().expect("postings");
            for (at, posting) in postings.iter().enumerate() {
                if posting["account"] == "expenses:people-costs:payroll-tax" {
                    let (mantissa, places) = posting_qty(posting);
                    taxes.push((
                        postings[at - 1]["account"]
                            .as_str()
                            .expect("account")
                            .to_string(),
                        mantissa / 10_i128.pow(places),
                    ));
                }
            }
        }
        let per_person: std::collections::BTreeSet<(String, i128)> =
            taxes.iter().cloned().collect();
        assert_eq!(
            per_person.into_iter().collect::<Vec<_>>(),
            [
                (
                    "expenses:people-costs:wages-salaries:george".to_string(),
                    600
                ),
                ("expenses:people-costs:wages-salaries:john".to_string(), 500),
            ],
            "shared {shared}: {taxes:?}"
        );
        assert_eq!(taxes.len(), 4, "two people over two months: {taxes:?}");
        let says = projection["warnings"]
            .to_string()
            .contains("refuses the file");
        assert_eq!(says, warned, "shared {shared}: {}", projection["warnings"]);
    }
}

// ---------------------------------------------------------------------------
// Rule forms and blocks in scenario FILES (plan 30 Phase 4)
// ---------------------------------------------------------------------------

const RULED: &str = "\
; Ledgeline projection
; projection: Ruled
; created: 2026-01-01
; updated: 2026-01-01

= expenses:rent tag:_generated-transaction  ; rent tax
    expenses:rent-tax   *0.1
    assets:cash

~ monthly  plan
    expenses:rent    $4200.00
    assets:cash

=revenues:salary
    (liabilities:tax)  *0.05
";

const RULED_URI: &str = "/api/projections/plans/projection-ruled.journal";

/// A file's rules arrive with their block and, where the editor can state
/// one, their form — percentages positive, as the user types them.
#[tokio::test]
async fn a_file_s_rules_carry_their_block_and_their_form() {
    let tree = Tree::new(&[("plans/projection-ruled.journal", RULED)]);
    let (status, doc) = file_json(&tree.state, "GET", RULED_URI, None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let rules = doc["scenario"]["autoRules"].as_array().expect("rules");
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0]["block"], json!(0));
    assert_eq!(rules[1]["block"], json!(1));
    assert_eq!(
        rules[0]["form"],
        json!({
            "matchAccount": "expenses:rent",
            "extraQuery": "",
            "postings": [{
                "account": "expenses:rent-tax",
                "kind": "percent",
                "percent": {"mantissa": "10", "places": 0},
                "amount": null
            }],
            "counter": "assets:cash",
            "comment": "rent tax",
            "forecastOnly": true
        })
    );
    // A virtual posting is beyond the form: shown raw, locked.
    assert_eq!(rules[1]["form"], Value::Null);
}

/// Saving an edited form rewrites that rule's block alone; the untouched rule,
/// sent back as its text, keeps every byte, and so does the `~` block between.
#[tokio::test]
async fn a_save_with_an_edited_form_rewrites_only_that_rule_s_block() {
    let tree = Tree::new(&[("plans/projection-ruled.journal", RULED)]);
    let (status, doc) = file_json(&tree.state, "GET", RULED_URI, None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let mut scenario = doc["scenario"].clone();
    let mut form = scenario["autoRules"][0]["form"].clone();
    form["postings"][0]["percent"] = json!({"mantissa": "12", "places": 0});
    scenario["autoRules"][0] = json!({"block": 0, "form": form});
    let raw = scenario["autoRules"][1]["raw"].clone();
    scenario["autoRules"][1] = json!({"block": 1, "raw": raw});

    let (status, saved) = file_json(
        &tree.state,
        "PUT",
        RULED_URI,
        Some(json!({"revision": doc["revision"], "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let after = tree.read("plans/projection-ruled.journal");
    let body = |text: &str| {
        text.split_once("\n\n")
            .map(|(_, rest)| rest.to_string())
            .unwrap()
    };
    assert_eq!(
        body(&after),
        body(RULED).replace(
            "= expenses:rent tag:_generated-transaction  ; rent tax\n    expenses:rent-tax   *0.1\n    assets:cash\n",
            "= ^expenses:rent(:|$) tag:_generated-transaction  ; rent tax\n    expenses:rent-tax  *0.12\n    assets:cash        *-0.12\n",
        )
    );
    // The answer states the rule as written, block and all.
    assert_eq!(saved["scenario"]["autoRules"][0]["block"], json!(0));
    assert!(
        saved["scenario"]["autoRules"][0]["raw"]
            .as_str()
            .is_some_and(|raw| raw.contains("*0.12")),
        "{saved}"
    );
}

/// Deleting a rule removes its block; a rule with no block is appended.
#[tokio::test]
async fn a_save_deletes_and_appends_rules_by_block() {
    let tree = Tree::new(&[("plans/projection-ruled.journal", RULED)]);
    let (_, doc) = file_json(&tree.state, "GET", RULED_URI, None).await;
    let mut scenario = doc["scenario"].clone();
    let first = scenario["autoRules"][0]["raw"].clone();
    scenario["autoRules"] = json!([
        {"block": 0, "raw": first},
        {"block": null, "form": percent_form("expenses:rent", "expenses:rent-fee", "1", Some("assets:cash"))}
    ]);
    let (status, saved) = file_json(
        &tree.state,
        "PUT",
        RULED_URI,
        Some(json!({"revision": doc["revision"], "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let after = tree.read("plans/projection-ruled.journal");
    assert!(!after.contains("=revenues:salary"), "{after}");
    assert!(
        after.ends_with(
            "    assets:cash\n\n= ^expenses:rent(:|$) tag:_generated-transaction\n    expenses:rent-fee  *0.01\n    assets:cash        *-0.01\n"
        ),
        "{after}"
    );
}

// ---------------------------------------------------------------------------
// Account types a scenario FILE declares itself
// ---------------------------------------------------------------------------

/// A scenario that declares its own liability, which name inference cannot
/// classify (`hmrc` is no hledger root), and a payroll-tax rule on it. hledger
/// 1.52 `bal --auto`: per $1000 of wages, `hmrc:paye` $-100 (the liability
/// grows) and `expenses:payroll-tax` $100.
const PAYE: &str = "\
; Ledgeline projection
; projection: PAYE
; created: 2026-01-01
; updated: 2026-01-01

account hmrc:paye  ; type: L

= ^expenses:wages(:|$) tag:_generated-transaction
    hmrc:paye             *-0.1
    expenses:payroll-tax  *0.1

~ monthly  wages
    expenses:wages    $1000.00
    assets:cash
";

const PAYE_URI: &str = "/api/projections/plans/projection-paye.journal";

/// The posting to `account` in a served rule form.
fn form_posting<'a>(form: &'a Value, account: &str) -> &'a Value {
    form["postings"]
        .as_array()
        .expect("postings")
        .iter()
        .find(|posting| posting["account"] == account)
        .unwrap_or_else(|| panic!("no {account} posting in {form}"))
}

/// **A5-7.** GET reads the rule under the file's own `type: L`, so the
/// liability's percentage is served positive, as the docs tell the user to
/// type it — and a save of that form writes back the multipliers hledger
/// books, not their inverse.
#[tokio::test]
async fn a_rule_on_a_liability_the_scenario_declares_is_served_positive_and_saves_unchanged() {
    let tree = Tree::new(&[("plans/projection-paye.journal", PAYE)]);
    let (status, doc) = file_json(&tree.state, "GET", PAYE_URI, None).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    let mut scenario = doc["scenario"].clone();
    assert_eq!(scenario["accountTypes"], json!({"hmrc:paye": "liability"}));
    let form = scenario["autoRules"][0]["form"].clone();
    assert_eq!(form["counter"], "expenses:payroll-tax", "{form}");
    assert_eq!(
        form_posting(&form, "hmrc:paye")["percent"],
        json!({"mantissa": "10", "places": 0}),
        "{form}"
    );

    // The user re-saves the form exactly as served (the documented figure).
    scenario["autoRules"][0] = json!({"block": 0, "form": form});
    let (status, saved) = file_json(
        &tree.state,
        "PUT",
        PAYE_URI,
        Some(json!({"revision": doc["revision"], "scenario": scenario})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let after = tree.read("plans/projection-paye.journal");
    let multiplier = |account: &str| {
        after
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{account} ")))
            .and_then(|line| line.split_whitespace().last())
            .unwrap_or_else(|| panic!("no {account} line in {after}"))
            .to_string()
    };
    assert_eq!(multiplier("hmrc:paye"), "*-0.1", "{after}");
    assert_eq!(multiplier("expenses:payroll-tax"), "*0.1", "{after}");
    assert_eq!(
        saved["scenario"]["accountTypes"],
        json!({"hmrc:paye": "liability"})
    );
}

/// Run a 10% `hmrc:paye` rule (countered by payroll tax) over the rent line,
/// with `account_types` riding on the body, and hand back the whole-dollar
/// amounts booked to (`hmrc:paye`, `expenses:payroll-tax`) in each entry.
async fn paye_run(account_types: Option<Value>) -> Vec<(i128, i128)> {
    let mut scenario = scenario_with(vec![rent_line()], vec![]);
    scenario["autoRules"] = json!([{
        "block": null,
        "form": percent_form("expenses:rent", "hmrc:paye", "10", Some("expenses:payroll-tax"))
    }]);
    if let Some(types) = account_types {
        scenario["accountTypes"] = types;
    }
    let projection = run(json!({
        "scenario": scenario,
        "interval": "monthly",
        "count": 2,
        "lookback": 0,
        "journal": true
    }))
    .await;
    let entries = projection["journal"]["entries"]
        .as_array()
        .expect("entries");
    assert!(!entries.is_empty());
    entries
        .iter()
        .map(|entry| {
            let postings = entry["txn"]["postings"].as_array().expect("postings");
            let whole = |account: &str| {
                let posting = postings
                    .iter()
                    .find(|posting| posting["account"] == account)
                    .unwrap_or_else(|| panic!("no {account} posting in {entry}"));
                let (mantissa, places) = posting_qty(posting);
                mantissa / 10_i128.pow(places)
            };
            (whole("hmrc:paye"), whole("expenses:payroll-tax"))
        })
        .collect()
}

/// **A5-7, the run.** A body carrying the scenario's own `hmrc:paye` liability
/// reads the positive 10% as the liability GROWING — the multipliers hledger
/// books for the file — so the run and the save agree.
#[tokio::test]
async fn a_run_reads_an_edited_rule_under_the_scenario_s_own_account_types() {
    let booked = paye_run(Some(json!({"hmrc:paye": "L"}))).await;
    assert!(booked.iter().all(|pair| *pair == (-420, 420)), "{booked:?}");
}

/// The fallback, pinned: a body WITHOUT `accountTypes` (an older client) reads
/// under the main journal's types alone. `hmrc:paye` is then unclassified, and
/// an unclassified account's percentage is its multiplier as typed.
#[tokio::test]
async fn a_run_without_account_types_reads_an_unclassified_account_s_percent_as_typed() {
    let booked = paye_run(None).await;
    assert!(booked.iter().all(|pair| *pair == (420, -420)), "{booked:?}");
}

/// A type the `account` directive itself would not accept is a 400 naming it.
#[tokio::test]
async fn an_unknown_account_type_in_the_body_is_a_400() {
    let mut scenario = scenario_with(vec![rent_line()], vec![]);
    scenario["accountTypes"] = json!({"hmrc:paye": "debt"});
    let (status, text) = post(json!({"scenario": scenario})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(
        text.contains("hmrc:paye") && text.contains("debt"),
        "{text}"
    );
}
