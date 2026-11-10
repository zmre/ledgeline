//! `fixtures/matching.journal` (plan 35 Phase 4) read through the matching
//! routes, so the fixture the docs screenshots use keeps saying what its
//! header comment says: which pairs are proposed and how confidently, which
//! links collapse into transfer rows, and which Problems findings it raises.
//!
//! Read-only: every request here is a GET, so the engine runs over the
//! committed fixture directly.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ledgeline::{AppState, router_with_state};
use serde_json::Value;
use tower::ServiceExt;

/// The day the fixture is read as of (the docs screenshots pin it too).
const TODAY: &str = "2027-01-31";

fn state() -> AppState {
    AppState::from_journal_path(common::fixtures_dir().join("matching.journal"))
        .expect("the matching fixture opens")
}

async fn get(state: &AppState, uri: &str) -> Value {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
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
    assert_eq!(
        status,
        StatusCode::OK,
        "GET {uri}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).expect("JSON")
}

/// One proposal as `(a date, a file, b date, b file, clear, ambiguous)`.
type Summary = (String, String, String, String, bool, bool);

/// Every proposal, by date (the route serves them in the first half's file
/// order, which with one file per feed is not date order).
fn summarize(body: &Value) -> Vec<Summary> {
    let text = |value: &Value| value.as_str().expect("a string").to_string();
    let mut summaries = body["proposals"]
        .as_array()
        .expect("proposals")
        .iter()
        .map(|p| {
            (
                text(&p["a"]["date"]),
                text(&p["a"]["file"]),
                text(&p["b"]["date"]),
                text(&p["b"]["file"]),
                p["clear"].as_bool().expect("clear"),
                p["ambiguous"].as_bool().expect("ambiguous"),
            )
        })
        .collect::<Vec<_>>();
    summaries.sort();
    summaries
}

fn expect(a: (&str, &str), b: (&str, &str), clear: bool) -> Summary {
    (
        a.0.to_string(),
        a.1.to_string(),
        b.0.to_string(),
        b.1.to_string(),
        clear,
        !clear,
    )
}

const CHK: &str = "checking.journal";
const SAV: &str = "savings.journal";
const AMEX: &str = "amex.journal";
const BILLS: &str = "billpay.journal";

#[tokio::test]
async fn the_fixture_proposes_six_clear_pairs_two_to_check_and_one_combination() {
    let state = state();
    let body = get(&state, &format!("/api/matches?today={TODAY}")).await;
    assert_eq!(body["clearing"], true);
    assert_eq!(
        summarize(&body),
        vec![
            expect(("2026-11-05", CHK), ("2026-11-06", SAV), true),
            // Two $200 transfers four days apart: every pairing is 2 days, so
            // both proposals are ambiguous and listed with an alternative.
            expect(("2026-11-14", CHK), ("2026-11-16", SAV), false),
            expect(("2026-11-18", CHK), ("2026-11-20", SAV), false),
            expect(("2026-11-21", CHK), ("2026-11-23", AMEX), true),
            expect(("2026-12-05", CHK), ("2026-12-06", SAV), true),
            // The rejected pair (12-12, 12-14) is not proposed; each half
            // pairs elsewhere.
            expect(("2026-12-12", CHK), ("2026-12-19", SAV), true),
            expect(("2026-12-14", SAV), ("2026-12-16", CHK), true),
            // Across the year end.
            expect(("2026-12-31", CHK), ("2027-01-02", AMEX), true),
            // The bill-pay debit and its two bills (A4): always "check",
            // never ambiguous. The $480 debit's bills sum to it two ways,
            // so it is not suggested.
            (
                "2027-01-12".to_string(),
                BILLS.to_string(),
                "2027-01-12".to_string(),
                BILLS.to_string(),
                false,
                false
            ),
        ]
    );
    assert_eq!(body["counts"]["clear"], 6);
    assert_eq!(body["counts"]["check"], 3);
    assert_eq!(
        body["older"], 0,
        "everything is inside the 365-day lookback"
    );
    let proposal = |date: &str| {
        body["proposals"]
            .as_array()
            .expect("proposals")
            .iter()
            .find(|p| p["a"]["date"] == date)
            .unwrap_or_else(|| panic!("no proposal from {date}"))
    };
    let ambiguous = proposal("2026-11-14");
    assert_eq!(
        ambiguous["alternatives"].as_array().map(Vec::len),
        Some(2),
        "the tied partner (2 days, taken) and the far one (6 days): {ambiguous}"
    );
    let combination = proposal("2027-01-12");
    let members: Vec<&str> = combination["group"]
        .as_array()
        .expect("group")
        .iter()
        .map(|half| half["description"].as_str().expect("description"))
        .collect();
    assert_eq!(
        members,
        vec!["Northside Builders invoice 1042", "City Water & Power"]
    );
    let crossing = proposal("2026-12-31");
    assert_eq!(crossing["gapDays"], 2);
    assert_eq!(crossing["from"]["label"], "checking");
    assert_eq!(crossing["to"]["label"], "amex");
}

#[tokio::test]
async fn an_imports_scope_keeps_only_pairs_with_a_new_half() {
    let state = state();
    // As if the last two Amex transactions had just been imported: only the
    // year-end payment's pair has a half among them (the other is a broken
    // link, so never a candidate).
    let body = get(
        &state,
        &format!("/api/matches?today={TODAY}&since=matching/amex.journal:2"),
    )
    .await;
    assert_eq!(
        summarize(&body),
        vec![expect(("2026-12-31", CHK), ("2027-01-02", AMEX), true)]
    );
}

#[tokio::test]
async fn the_two_links_collapse_and_the_broken_one_does_not() {
    let state = state();
    let summary = get(&state, &format!("/api/journal/summary?today={TODAY}")).await;
    assert_eq!(summary["transfers"], 2);
    // 42 transactions, two pairs shown as one row each.
    assert_eq!(summary["total"], 40);

    let page = get(
        &state,
        &format!("/api/journal/rows?today={TODAY}&order=asc"),
    )
    .await;
    let transfers: Vec<(&str, &str, &str)> = page["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| {
            let transfer = row.get("transfer")?;
            Some((
                transfer["matchId"].as_str()?,
                transfer["from"]["label"].as_str()?,
                transfer["to"]["label"].as_str()?,
            ))
        })
        .collect();
    assert_eq!(
        transfers,
        vec![
            ("a1b2c3", "checking", "savings"),
            ("k7f3a2", "checking", "amex")
        ]
    );
}

#[tokio::test]
async fn problems_find_one_item_in_transit_and_the_broken_link() {
    let state = state();
    let body = get(&state, &format!("/api/diagnostics?today={TODAY}")).await;
    let findings = |rule: &str| -> Vec<String> {
        body["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .filter(|finding| finding["rule"] == rule)
            .map(|finding| finding["message"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    // The bill-pay debit and its bills are in a suggested combination (A5);
    // the ambiguous $480 is not yet ten days old.
    let in_transit = findings("in-transit");
    assert_eq!(in_transit.len(), 1, "{in_transit:?}");
    assert!(in_transit[0].contains("2026-12-28"), "{in_transit:?}");
    // One finding per transaction carrying the broken id.
    let broken = findings("broken-match");
    assert_eq!(broken.len(), 2, "{broken:?}");
    assert!(
        broken.iter().all(|message| message.contains("zz9k1m")),
        "{broken:?}"
    );
    assert!(findings("unusual-clearing").is_empty());
}

#[tokio::test]
async fn the_ambiguous_bill_pay_debit_offers_its_bills_to_pick_by_hand() {
    let state = state();
    // #38: the $480 debit (34 transactions before billpay.journal, then 3).
    let body = get(&state, &format!("/api/matches/for/38?today={TODAY}")).await;
    assert_eq!(body["transaction"]["description"], "BILL.COM PAYABLES");
    let debit = &body["postings"][0];
    assert_eq!(debit["exact"], serde_json::json!([]));
    assert_eq!(debit["combinations"], serde_json::json!([]));
    let nearby: Vec<u64> = debit["nearby"]
        .as_array()
        .expect("nearby")
        .iter()
        .map(|half| half["index"].as_u64().expect("index"))
        .collect();
    assert_eq!(nearby, vec![39, 40, 41, 42]);
    assert_eq!(body["links"], serde_json::json!([]));
}
