//! Native (non-hledger) report + budget endpoints.
//!
//! These routes expose the golden-verified `ledgeline_core::reports` engine over
//! HTTP for the SPA. Unlike the Phase-2 wire endpoints (whose bodies are
//! precomputed once from the journal), reports depend on request query params, so
//! they are computed per request from the parsed journal held in [`AppState`].
//!
//! The JSON contract is the engine's own native shape (NOT hledger's), designed
//! to map 1:1 onto `web/src/lib/reports/types.ts`:
//! - `Dec` → `{"mantissa": <number>, "places": <number>}` (matching the existing
//!   wire `decimalMantissa`; realistic money mantissas stay within JS
//!   safe-integer range, which the frontend already guards).
//! - `MixedAmount` → `{"<commodity>": <Dec>, …}` (TS `Map<string, Dec>`), with
//!   zero commodities dropped (the additive-identity contract).
//! - `SectionedReport`/`PeriodReport`/`BudgetReport` use camelCase keys matching
//!   the TS interfaces (`grandTotal`, `asOf`, …).

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    BudgetCell, BudgetOpts, BudgetReport, Interval, MixedAmount, PeriodReport, PriceDb,
    ReportError, SectionedReport, account_decls, balance_sheet, budget_report, cash_flow,
    cash_predicate, income_statement, net_worth,
};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Account-depth clamp default (mirrors `ReportParams` in `web/.../params.ts`).
const DEFAULT_DEPTH: usize = 2;
/// Lookback bucket-count default (mirrors `ReportParams`).
const DEFAULT_COUNT: usize = 12;

// ===========================================================================
// Wire representation of the report result types
// ===========================================================================

/// An exact decimal on the wire: `mantissa / 10^places`.
#[derive(Serialize)]
struct WireDec {
    mantissa: i128,
    places: u32,
}

/// A commodity-keyed bag of exact quantities → the SPA `Map<string, Dec>`. Zero
/// commodities are dropped, matching the engine's zero-free result contract.
type WireMixed = BTreeMap<String, WireDec>;

fn wire_mixed(ma: &MixedAmount) -> WireMixed {
    ma.iter()
        .filter(|(_, dec)| !dec.is_zero())
        .map(|(commodity, dec)| {
            (
                commodity.0.clone(),
                WireDec {
                    mantissa: dec.mantissa,
                    places: dec.places,
                },
            )
        })
        .collect()
}

/// One row of a sectioned report.
#[derive(Serialize)]
struct WireReportRow {
    account: String,
    depth: usize,
    own: WireMixed,
    inclusive: WireMixed,
}

/// A titled group of rows plus its subtree total.
#[derive(Serialize)]
struct WireSection {
    title: String,
    rows: Vec<WireReportRow>,
    total: WireMixed,
}

/// Balance sheet / income statement.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireSectionedReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    as_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    sections: Vec<WireSection>,
    grand_total: WireMixed,
}

fn wire_sectioned(report: &SectionedReport) -> WireSectionedReport {
    WireSectionedReport {
        as_of: report.as_of.clone(),
        from: report.from.clone(),
        to: report.to.clone(),
        sections: report
            .sections
            .iter()
            .map(|section| WireSection {
                title: section.title.clone(),
                rows: section
                    .rows
                    .iter()
                    .map(|row| WireReportRow {
                        account: row.account.clone(),
                        depth: row.depth,
                        own: wire_mixed(&row.own),
                        inclusive: wire_mixed(&row.inclusive),
                    })
                    .collect(),
                total: wire_mixed(&section.total),
            })
            .collect(),
        grand_total: wire_mixed(&report.grand_total),
    }
}

/// One row of a period report: one `MixedAmount` per bucket.
#[derive(Serialize)]
struct WirePeriodRow {
    account: String,
    depth: usize,
    values: Vec<WireMixed>,
}

/// Extra result info (currently only unpriced commodities in net worth).
#[derive(Serialize)]
struct WireReportMeta {
    unpriced: Vec<String>,
}

/// Cash flow / net worth: one column per bucket, oldest → newest.
#[derive(Serialize)]
pub(crate) struct WirePeriodReport {
    buckets: Vec<String>,
    rows: Vec<WirePeriodRow>,
    totals: Vec<WireMixed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<WireReportMeta>,
}

fn wire_period(report: &PeriodReport) -> WirePeriodReport {
    WirePeriodReport {
        buckets: report.buckets.clone(),
        rows: report
            .rows
            .iter()
            .map(|row| WirePeriodRow {
                account: row.account.clone(),
                depth: row.depth,
                values: row.values.iter().map(wire_mixed).collect(),
            })
            .collect(),
        totals: report.totals.iter().map(wire_mixed).collect(),
        meta: report.meta.as_ref().map(|meta| WireReportMeta {
            unpriced: meta.unpriced.iter().map(|c| c.0.clone()).collect(),
        }),
    }
}

/// One account × bucket budget cell. `goal` is `null` when the account has no
/// goal (e.g. `<unbudgeted>`); an empty object `{}` is a budgeted account with no
/// goal in that bucket.
#[derive(Serialize)]
struct WireBudgetCell {
    actual: WireMixed,
    goal: Option<WireMixed>,
}

fn wire_budget_cell(cell: &BudgetCell) -> WireBudgetCell {
    WireBudgetCell {
        actual: wire_mixed(&cell.actual),
        goal: cell.goal.as_ref().map(wire_mixed),
    }
}

/// One budget row: an account and its per-bucket cells.
#[derive(Serialize)]
struct WireBudgetRow {
    account: String,
    depth: usize,
    cells: Vec<WireBudgetCell>,
}

/// A budget report: bucket keys, rows, and a grand-total row of cells.
#[derive(Serialize)]
pub(crate) struct WireBudgetReport {
    buckets: Vec<String>,
    rows: Vec<WireBudgetRow>,
    totals: Vec<WireBudgetCell>,
}

fn wire_budget(report: &BudgetReport) -> WireBudgetReport {
    WireBudgetReport {
        buckets: report.buckets.clone(),
        rows: report
            .rows
            .iter()
            .map(|row| WireBudgetRow {
                account: row.account.clone(),
                depth: row.depth,
                cells: row.cells.iter().map(wire_budget_cell).collect(),
            })
            .collect(),
        totals: report.totals.iter().map(wire_budget_cell).collect(),
    }
}

// ===========================================================================
// Query params, defaults, and helpers
// ===========================================================================

/// Current UTC date as `YYYY-MM-DD`, from the system clock.
///
/// The report engine is deliberately clock-free (see `reports::periods`);
/// "today" is a server-side request default only, so it lives here rather than
/// in `ledgeline-core`, and needs no third-party date dependency.
fn today_utc() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's `civil_from_days` (day 0 = 1970-01-01) — a dependency-free
/// proleptic-Gregorian conversion, used solely for the "today" default.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe.div_euclid(1_460) + doe.div_euclid(36_524) - doe.div_euclid(146_096))
        .div_euclid(365);
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe.div_euclid(4) - yoe.div_euclid(100));
    let mp = (5 * doy + 2).div_euclid(153);
    let day = doy - (153 * mp + 2).div_euclid(5) + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}

/// Parse a report interval, defaulting to monthly when absent. Returns a `400`
/// tuple for an unrecognized value.
fn parse_interval(raw: Option<&str>) -> Result<Interval, ApiError> {
    match raw {
        None => Ok(Interval::Monthly),
        Some("daily") => Ok(Interval::Daily),
        Some("weekly") => Ok(Interval::Weekly),
        Some("monthly") => Ok(Interval::Monthly),
        Some("quarterly") => Ok(Interval::Quarterly),
        Some("yearly") => Ok(Interval::Yearly),
        Some(other) => Err((
            StatusCode::BAD_REQUEST,
            format!("unknown interval '{other}' (expected daily|weekly|monthly|quarterly|yearly)"),
        )),
    }
}

/// An HTTP error: a status plus a human-readable message.
type ApiError = (StatusCode, String);

/// Map a report-engine error onto an HTTP status: a bad bucket key is a client
/// error (`400`); a decimal overflow is an internal error (`500`). Both are
/// unreachable for realistic journals, but neither is unwrapped.
fn report_error(err: &ReportError) -> ApiError {
    match err {
        ReportError::InvalidBucketKey(_) => (StatusCode::BAD_REQUEST, err.to_string()),
        ReportError::Decimal(_) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

/// `?asOf=&depth=` — balance sheet.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BalanceSheetQuery {
    as_of: Option<String>,
    depth: Option<usize>,
}

/// `?from=&to=&depth=` — income statement.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IncomeStatementQuery {
    from: Option<String>,
    to: Option<String>,
    depth: Option<usize>,
}

/// `?end=&interval=&count=&depth=` — cash flow.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CashFlowQuery {
    end: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    depth: Option<usize>,
}

/// `?end=&interval=&count=&valueIn=` — net worth.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetWorthQuery {
    end: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    value_in: Option<String>,
}

/// `?end=&interval=&count=&depth=&budgetDesc=` — budget report.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BudgetQuery {
    end: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    depth: Option<usize>,
    budget_desc: Option<String>,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `GET /api/reports/balancesheet` — assets + liabilities as of a date.
pub(crate) async fn balancesheet(
    State(state): State<AppState>,
    Query(query): Query<BalanceSheetQuery>,
) -> Result<Json<WireSectionedReport>, ApiError> {
    let as_of = query.as_of.unwrap_or_else(today_utc);
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);
    let report = balance_sheet(&state.journal.transactions, &as_of, depth)
        .map_err(|err| report_error(&err))?;
    Ok(Json(wire_sectioned(&report)))
}

/// `GET /api/reports/incomestatement` — revenues + expenses over a range.
pub(crate) async fn incomestatement(
    State(state): State<AppState>,
    Query(query): Query<IncomeStatementQuery>,
) -> Result<Json<WireSectionedReport>, ApiError> {
    let today = today_utc();
    let from = query
        .from
        .unwrap_or_else(|| format!("{}-01-01", &today[..4]));
    let to = query.to.unwrap_or(today);
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);
    let report = income_statement(&state.journal.transactions, &from, &to, depth)
        .map_err(|err| report_error(&err))?;
    Ok(Json(wire_sectioned(&report)))
}

/// `GET /api/reports/cashflow` — per-bucket cash-like-asset changes. The cash
/// predicate honors the journal's declared account types (same as the SPA).
pub(crate) async fn cashflow(
    State(state): State<AppState>,
    Query(query): Query<CashFlowQuery>,
) -> Result<Json<WirePeriodReport>, ApiError> {
    let end = query.end.unwrap_or_else(today_utc);
    let interval = parse_interval(query.interval.as_deref())?;
    let count = query.count.unwrap_or(DEFAULT_COUNT);
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);

    let decls = account_decls(&state.journal);
    let predicate = cash_predicate(&decls);
    let is_cash: &dyn Fn(&str) -> bool = &predicate;
    let report = cash_flow(
        &state.journal.transactions,
        &end,
        interval,
        count,
        depth,
        Some(is_cash),
    )
    .map_err(|err| report_error(&err))?;
    Ok(Json(wire_period(&report)))
}

/// `GET /api/reports/networth` — market-valued net worth per bucket. Prices come
/// from the journal's `P` directives; `valueIn` overrides the target commodity.
pub(crate) async fn networth(
    State(state): State<AppState>,
    Query(query): Query<NetWorthQuery>,
) -> Result<Json<WirePeriodReport>, ApiError> {
    let end = query.end.unwrap_or_else(today_utc);
    let interval = parse_interval(query.interval.as_deref())?;
    let count = query.count.unwrap_or(DEFAULT_COUNT);
    let value_in = query
        .value_in
        .filter(|symbol| !symbol.is_empty())
        .map(Commodity);

    let prices = PriceDb::build(&state.journal.prices);
    let report = net_worth(
        &state.journal.transactions,
        &prices,
        &end,
        interval,
        count,
        value_in,
    )
    .map_err(|err| report_error(&err))?;
    Ok(Json(wire_period(&report)))
}

/// `GET /api/budget` — actuals vs. periodic-rule goals per bucket.
pub(crate) async fn budget(
    State(state): State<AppState>,
    Query(query): Query<BudgetQuery>,
) -> Result<Json<WireBudgetReport>, ApiError> {
    let end = query.end.unwrap_or_else(today_utc);
    let interval = parse_interval(query.interval.as_deref())?;
    let count = query.count.unwrap_or(DEFAULT_COUNT);
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);
    let budget_desc = query
        .budget_desc
        .as_deref()
        .filter(|pattern| !pattern.is_empty());

    let opts = BudgetOpts {
        end: &end,
        interval,
        count,
        depth,
        budget_desc,
    };
    let report = budget_report(
        &state.journal.transactions,
        &state.journal.periodic_transactions,
        &opts,
    )
    .map_err(|err| report_error(&err))?;
    Ok(Json(wire_budget(&report)))
}
