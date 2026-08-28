//! Cross-endpoint reconciliation over the wire (CLEANUP.md RPT-5).
//!
//! The bulk of the invariant suite lives at the core level
//! (`ledgeline-core/tests/report_invariants.rs`), where `depth`, `interval` and
//! `value_in` can be varied freely and compared as exact `MixedAmount`s. This
//! file covers the part that only the HTTP layer can break: that the SAME
//! relationships still hold between the numbers the endpoints actually SERVE,
//! after query parsing, defaulting and JSON encoding.
//!
//! Every assertion here is exact — wire decimals are compared as canonical
//! `(mantissa, places)` pairs, never as floats or formatted text.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ledgeline_core::{Dec, Journal};
use ledgeline::app;
use serde_json::Value;
use std::collections::BTreeMap;
use tower::ServiceExt;

// ---- HTTP driver (same shape as report_endpoints.rs) ----

async fn body_ok(journal: &Journal, uri: &str) -> Value {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builds");
    let response = app(journal)
        .oneshot(request)
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK, "GET {uri} should be 200");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("body is JSON")
}

// ---- exact wire arithmetic ----

/// A wire mixed amount as exact decimals per commodity.
type Wire = BTreeMap<String, Dec>;

/// Canonical `(mantissa, places)` per commodity, trailing zeros stripped and
/// zero commodities dropped — the comparison form used by every golden suite.
type Canon = BTreeMap<String, (i128, u32)>;

fn parse_wire(value: &Value) -> Wire {
    value
        .as_object()
        .expect("mixed amount is an object")
        .iter()
        .map(|(commodity, dec)| {
            let mantissa: i128 = dec["mantissa"]
                .as_str()
                .expect("mantissa string")
                .parse()
                .expect("mantissa parses");
            let places = u32::try_from(dec["places"].as_u64().expect("places")).expect("places");
            (commodity.clone(), Dec::new(mantissa, places))
        })
        .collect()
}

fn canon(wire: &Wire) -> Canon {
    wire.iter()
        .filter(|(_, dec)| !dec.is_zero())
        .map(|(commodity, dec)| {
            let normalized = dec.normalized();
            (commodity.clone(), (normalized.mantissa, normalized.places))
        })
        .collect()
}

fn wire_add(a: &Wire, b: &Wire) -> Wire {
    let mut out = a.clone();
    for (commodity, dec) in b {
        let sum = out
            .get(commodity)
            .map_or(Ok(*dec), |prev| prev.add(*dec))
            .expect("wire add must not overflow");
        out.insert(commodity.clone(), sum);
    }
    out
}

fn wire_neg(a: &Wire) -> Wire {
    a.iter()
        .map(|(commodity, dec)| (commodity.clone(), dec.neg().expect("neg")))
        .collect()
}

fn wire_sub(a: &Wire, b: &Wire) -> Wire {
    wire_add(a, &wire_neg(b))
}

fn section<'a>(body: &'a Value, title: &str) -> &'a Value {
    body["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["title"] == title)
        .unwrap_or_else(|| panic!("section {title} exists"))
}

fn journal() -> Journal {
    common::fixture_journal()
}

// ===========================================================================

/// A sectioned report's `grandTotal` is exactly the net of its two section
/// totals, as SERVED — the wire's own restatement of the sign convention.
#[tokio::test]
async fn served_grand_totals_are_the_net_of_the_served_section_totals() {
    let journal = journal();

    for as_of in ["2024-12-31", "2025-02-14", "2026-06-30"] {
        let bs = body_ok(
            &journal,
            &format!("/api/reports/balancesheet?asOf={as_of}&depth=3"),
        )
        .await;
        let assets = parse_wire(&section(&bs, "Assets")["total"]);
        let liabilities = parse_wire(&section(&bs, "Liabilities")["total"]);
        assert_eq!(
            canon(&parse_wire(&bs["grandTotal"])),
            canon(&wire_sub(&assets, &liabilities)),
            "bs grandTotal == Assets − Liabilities (displayed) at {as_of}"
        );

        let is = body_ok(
            &journal,
            &format!("/api/reports/incomestatement?from=2024-01-01&to={as_of}&depth=3"),
        )
        .await;
        let revenues = parse_wire(&section(&is, "Revenues")["total"]);
        let expenses = parse_wire(&section(&is, "Expenses")["total"]);
        assert_eq!(
            canon(&parse_wire(&is["grandTotal"])),
            canon(&wire_sub(&revenues, &expenses)),
            "is grandTotal == Revenues − Expenses (displayed) at {as_of}"
        );
    }
}

/// `?depth=` must not move any total. hledger's totals are depth-independent,
/// and the endpoints' `?depth=0` ("totals only") case is exactly where RPT-4
/// found them reading zero.
#[tokio::test]
async fn served_totals_do_not_move_with_depth() {
    let journal = journal();
    let as_of = "2026-06-30";

    let baseline = body_ok(
        &journal,
        &format!("/api/reports/balancesheet?asOf={as_of}&depth=1"),
    )
    .await;
    let baseline_total = canon(&parse_wire(&baseline["grandTotal"]));

    let networth_baseline = body_ok(
        &journal,
        &format!("/api/reports/networth?end={as_of}&interval=monthly&count=1&depth=1"),
    )
    .await;
    let networth_total = canon(&parse_wire(&networth_baseline["totals"][0]));

    for depth in [0_usize, 1, 2, 3, 4, 5, 6] {
        let bs = body_ok(
            &journal,
            &format!("/api/reports/balancesheet?asOf={as_of}&depth={depth}"),
        )
        .await;
        assert_eq!(
            canon(&parse_wire(&bs["grandTotal"])),
            baseline_total,
            "bs grandTotal at depth {depth}"
        );

        let nw = body_ok(
            &journal,
            &format!("/api/reports/networth?end={as_of}&interval=monthly&count=1&depth={depth}"),
        )
        .await;
        assert_eq!(
            canon(&parse_wire(&nw["totals"][0])),
            networth_total,
            "net worth total at depth {depth}"
        );
    }
}

/// Every served parent row equals its own postings plus its served children —
/// `aggregate::roll_up` as the SPA sees it.
#[tokio::test]
async fn served_rows_roll_up_from_their_served_children() {
    let journal = journal();
    let depth = 4;
    let bs = body_ok(
        &journal,
        &format!("/api/reports/balancesheet?asOf=2026-06-30&depth={depth}"),
    )
    .await;

    for title in ["Assets", "Liabilities"] {
        let rows: BTreeMap<String, (Wire, Wire)> = section(&bs, title)["rows"]
            .as_array()
            .expect("rows array")
            .iter()
            .map(|row| {
                (
                    row["account"].as_str().expect("account").to_string(),
                    (parse_wire(&row["own"]), parse_wire(&row["inclusive"])),
                )
            })
            .collect();
        assert!(!rows.is_empty(), "{title} has rows");

        for (account, (own, inclusive)) in &rows {
            let account_depth = account.split(':').count();
            if account_depth >= depth {
                continue; // children clamped away
            }
            let prefix = format!("{account}:");
            let children = rows
                .iter()
                .filter(|(name, _)| {
                    name.starts_with(&prefix) && name.split(':').count() == account_depth + 1
                })
                .fold(Wire::new(), |acc, (_, (_, child))| wire_add(&acc, child));
            assert_eq!(
                canon(inclusive),
                canon(&wire_add(own, &children)),
                "{title}/{account}: inclusive != own + Σ children"
            );
        }
    }
}

/// The period's served net income is exactly the served change in the balance
/// sheet, over a window with no equity postings and no cost conversions.
///
/// ```text
/// $ hledger -f fixtures/sample.journal bal type:AL -e 2026-03-01 --depth 1 → $41,188.44
/// $ hledger -f fixtures/sample.journal bal type:AL -e 2026-02-01 --depth 1 → $39,245.09
/// $ hledger -f fixtures/sample.journal is -b 2026-02-01 -e 2026-03-01      → Net: $1,943.35
/// ```
#[tokio::test]
async fn served_income_statement_net_equals_the_served_balance_sheet_delta() {
    let journal = journal();

    let opening = body_ok(
        &journal,
        "/api/reports/balancesheet?asOf=2026-01-31&depth=1",
    )
    .await;
    let closing = body_ok(
        &journal,
        "/api/reports/balancesheet?asOf=2026-02-28&depth=1",
    )
    .await;
    let is = body_ok(
        &journal,
        "/api/reports/incomestatement?from=2026-02-01&to=2026-02-28&depth=1",
    )
    .await;

    let delta = wire_sub(
        &parse_wire(&closing["grandTotal"]),
        &parse_wire(&opening["grandTotal"]),
    );
    assert_eq!(
        canon(&delta),
        canon(&parse_wire(&is["grandTotal"])),
        "served bs delta == served is net over 2026-02"
    );
    // Tie to the hledger-verified figure so a shared sign error cannot pass.
    assert_eq!(
        canon(&delta),
        Canon::from([("$".to_string(), (194_335, 2))]),
        "…and it is $1,943.35"
    );
}

/// Served cash-flow buckets sum across intervals when they cover the same range
/// — including a mid-month `end`, where both the last monthly bucket and the
/// single yearly bucket truncate at `end`.
#[tokio::test]
async fn served_monthly_buckets_sum_to_the_served_yearly_bucket() {
    let journal = journal();

    for (end, months) in [("2026-06-30", 6), ("2026-02-14", 2), ("2024-12-31", 12)] {
        let monthly = body_ok(
            &journal,
            &format!("/api/reports/cashflow?end={end}&interval=monthly&count={months}&depth=1"),
        )
        .await;
        let yearly = body_ok(
            &journal,
            &format!("/api/reports/cashflow?end={end}&interval=yearly&count=1&depth=1"),
        )
        .await;

        let summed = monthly["totals"]
            .as_array()
            .expect("totals array")
            .iter()
            .fold(Wire::new(), |acc, total| wire_add(&acc, &parse_wire(total)));
        assert_eq!(
            canon(&summed),
            canon(&parse_wire(&yearly["totals"][0])),
            "Σ {months} served monthly buckets == the served yearly bucket at {end}"
        );
    }
}

/// PINNED, DELIBERATE DIVERGENCE (CLEANUP.md "INFO"), as served.
///
/// `hledger bal -M -e 2026-02-16` WIDENS its final bucket to 2026-02-28;
/// `/api/reports/cashflow?end=2026-02-16` truncates it at `end`. The gap is the
/// 2026-02-27 salary deposit.
///
/// ```text
/// $ hledger -f fixtures/sample.journal bal type:C -M -e 2026-02-16 --depth 1
///     2026-02 column: $1,923.16     # widened to the whole month
/// $ hledger -f fixtures/sample.journal bal type:C -b 2026-02-01 -e 2026-02-17 --depth 1
///     $-2,276.84                    # truncated at 2026-02-16 — what we serve
/// ```
#[tokio::test]
async fn served_interval_reports_truncate_the_final_bucket_at_end() {
    let journal = journal();

    let truncated = body_ok(
        &journal,
        "/api/reports/cashflow?end=2026-02-16&interval=monthly&count=2&depth=1",
    )
    .await;
    assert_eq!(truncated["buckets"][1], "2026-02");
    assert_eq!(
        canon(&parse_wire(&truncated["totals"][1])),
        Canon::from([("$".to_string(), (-227_684, 2))]),
        "final bucket is 2026-02-01..2026-02-16 → $-2,276.84"
    );

    let whole_month = body_ok(
        &journal,
        "/api/reports/cashflow?end=2026-02-28&interval=monthly&count=1&depth=1",
    )
    .await;
    assert_eq!(
        canon(&parse_wire(&whole_month["totals"][0])),
        Canon::from([("$".to_string(), (192_316, 2))]),
        "the whole month is $1,923.16 — the $4,200.00 salary lands 2026-02-27"
    );
}
