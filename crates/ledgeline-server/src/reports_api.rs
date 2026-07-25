//! Native (non-hledger) report + budget endpoints.
//!
//! These routes expose the golden-verified `ledgeline_core::reports` engine over
//! HTTP for the SPA. Unlike the Phase-2 wire endpoints (whose bodies are
//! precomputed once from the journal), reports depend on request query params, so
//! they are computed per request from the parsed journal held in [`AppState`].
//!
//! The JSON contract is the engine's own native shape (NOT hledger's), designed
//! to map 1:1 onto `web/src/lib/reports/types.ts`:
//! - `Dec` → `{"mantissa": <string>, "places": <number>}`. The mantissa is
//!   STRING-encoded (decoded via `BigInt` on the SPA): unlike parsed amounts,
//!   COMPUTED values (e.g. holdings `marketValue = shares × price`, non-
//!   normalized) can exceed the JS safe-integer range, so a JSON number would
//!   silently lose precision.
//! - `MixedAmount` → `{"<commodity>": <Dec>, …}` (TS `Map<string, Dec>`), with
//!   zero commodities dropped (the additive-identity contract).
//! - `SectionedReport`/`PeriodReport`/`BudgetReport` use camelCase keys matching
//!   the TS interfaces (`grandTotal`, `asOf`, …).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use ledgeline_core::Dec;
use ledgeline_core::holdings::{
    Holding, HoldingsReport, HoldingsScope, HoldingsSeries, PriceSource, ScopeMode, WarningKind,
    compute_holdings, holdings_series,
};
use ledgeline_core::model::Commodity;
use ledgeline_core::reports::{
    BudgetCell, BudgetOpts, BudgetReport, Cadence, ChangeKind, ChangeRow, CostOfLiving,
    DEFAULT_EXCLUDE_DESC, InsightsOpts, InsightsReport, Interval, MetricDelta, MixedAmount,
    MoverRow, NetWorthOpts, PerfPoint, PeriodReport, ReportError, SectionedReport, Subscription,
    SubscriptionOpts, SubscriptionsReport, TopTxn, account_decls, balance_sheet, budget_report,
    cash_flow, cash_predicate, declared_types, detect_subscriptions, income_statement, insights,
    net_worth,
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
    /// STRING-encoded significand (see the module doc): computed values can
    /// exceed the JS safe-integer range, so a JSON number would lose precision.
    mantissa: String,
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
                    mantissa: dec.mantissa.to_string(),
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
// Wire representation of the insights dashboard
// ===========================================================================

/// The resolved comparison window (all camelCase ISO dates).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireInsightsPeriod {
    start: String,
    mid: String,
    end: String,
    prev_start: String,
    prev_end: String,
    curr_start: String,
    curr_end: String,
}

/// A current/previous metric with its exact change and a base-commodity percent.
#[derive(Serialize)]
struct WireMetricDelta {
    current: WireMixed,
    previous: WireMixed,
    delta: WireMixed,
    pct: Option<f64>,
}

fn wire_metric_delta(metric: &MetricDelta) -> WireMetricDelta {
    WireMetricDelta {
        current: wire_mixed(&metric.current),
        previous: wire_mixed(&metric.previous),
        delta: wire_mixed(&metric.delta),
        pct: metric.pct,
    }
}

/// Cost-of-living totals + month counts (the SPA averages for display).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireCostOfLiving {
    current_total: WireMixed,
    previous_total: WireMixed,
    months_current: u32,
    months_previous: u32,
}

fn wire_cost_of_living(col: &CostOfLiving) -> WireCostOfLiving {
    WireCostOfLiving {
        current_total: wire_mixed(&col.current_total),
        previous_total: wire_mixed(&col.previous_total),
        months_current: col.months_current,
        months_previous: col.months_previous,
    }
}

/// One period's portfolio performance in the base commodity.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WirePerfPoint {
    gain: Option<WireDec>,
    gain_pct: Option<f64>,
}

fn wire_perf_point(point: &PerfPoint) -> WirePerfPoint {
    WirePerfPoint {
        gain: wire_opt_dec(point.gain),
        gain_pct: point.gain_pct,
    }
}

/// A leaf-account change row (Boxes 7 & 9). `kind` is `"changed" | "new" | "ended"`.
#[derive(Serialize)]
struct WireChangeRow {
    account: String,
    current: WireDec,
    previous: WireDec,
    delta: WireDec,
    pct: Option<f64>,
    kind: &'static str,
}

fn wire_change_row(row: &ChangeRow) -> WireChangeRow {
    WireChangeRow {
        account: row.account.clone(),
        current: wire_dec(row.current),
        previous: wire_dec(row.previous),
        delta: wire_dec(row.delta),
        pct: row.pct,
        kind: match row.kind {
            ChangeKind::Changed => "changed",
            ChangeKind::Ended => "ended",
        },
    }
}

/// A stock mover row (Box 8).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireMoverRow {
    symbol: String,
    name: String,
    gain: Option<WireDec>,
    gain_pct: Option<f64>,
    /// The window-start value fell back to purchase cost (no market price) — the
    /// move then approximates the all-time gain. Surfaced as a caveat in the UI.
    start_estimated: bool,
}

fn wire_mover(row: &MoverRow) -> WireMoverRow {
    WireMoverRow {
        symbol: row.symbol.clone(),
        name: row.name.clone(),
        gain: wire_opt_dec(row.gain),
        gain_pct: row.gain_pct,
        start_estimated: row.start_estimated,
    }
}

/// A top-transaction row (Box 10).
#[derive(Serialize)]
struct WireTopTxn {
    index: u32,
    date: String,
    description: String,
    amount: WireDec,
}

fn wire_top_txn(txn: &TopTxn) -> WireTopTxn {
    WireTopTxn {
        index: txn.index,
        date: txn.date.clone(),
        description: txn.description.clone(),
        amount: wire_dec(txn.amount),
    }
}

/// The full insights dashboard payload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireInsightsReport {
    period: WireInsightsPeriod,
    base: String,
    /// Earliest transaction date in the journal — lets the SPA warn when the
    /// previous period is only partly covered by the data.
    journal_start: Option<String>,
    revenue: WireMetricDelta,
    expenses: WireMetricDelta,
    net_worth: WireMetricDelta,
    cost_of_living: WireCostOfLiving,
    investment: WireInvestmentPerf,
    cash_balance: WireMetricDelta,
    expense_changes: Vec<WireChangeRow>,
    revenue_changes: Vec<WireChangeRow>,
    movers: Vec<WireMoverRow>,
    top_txns: Vec<WireTopTxn>,
}

/// Investment performance for both periods.
#[derive(Serialize)]
struct WireInvestmentPerf {
    current: WirePerfPoint,
    previous: WirePerfPoint,
}

fn wire_insights(report: &InsightsReport) -> WireInsightsReport {
    WireInsightsReport {
        period: WireInsightsPeriod {
            start: report.period.start.clone(),
            mid: report.period.mid.clone(),
            end: report.period.end.clone(),
            prev_start: report.period.prev_start.clone(),
            prev_end: report.period.prev_end.clone(),
            curr_start: report.period.curr_start.clone(),
            curr_end: report.period.curr_end.clone(),
        },
        base: report.base.clone(),
        journal_start: report.journal_start.clone(),
        revenue: wire_metric_delta(&report.revenue),
        expenses: wire_metric_delta(&report.expenses),
        net_worth: wire_metric_delta(&report.net_worth),
        cost_of_living: wire_cost_of_living(&report.cost_of_living),
        investment: WireInvestmentPerf {
            current: wire_perf_point(&report.investment.current),
            previous: wire_perf_point(&report.investment.previous),
        },
        cash_balance: wire_metric_delta(&report.cash_balance),
        expense_changes: report.expense_changes.iter().map(wire_change_row).collect(),
        revenue_changes: report.revenue_changes.iter().map(wire_change_row).collect(),
        movers: report.movers.iter().map(wire_mover).collect(),
        top_txns: report.top_txns.iter().map(wire_top_txn).collect(),
    }
}

// ===========================================================================
// Wire representation of the subscriptions report
// ===========================================================================

/// One detected recurring charge. `cadence` is `"monthly" | "annual"`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSubscription {
    payee: String,
    cadence: &'static str,
    typical_amount: WireDec,
    annualized_cost: WireDec,
    occurrences: usize,
    first_seen: String,
    last_seen: String,
    next_expected: String,
    accounts: Vec<String>,
    /// Hand-added via a `subscription:true` tag rather than detected.
    manual: bool,
}

fn wire_subscription(subscription: &Subscription) -> WireSubscription {
    WireSubscription {
        payee: subscription.payee.clone(),
        cadence: match subscription.cadence {
            Cadence::Monthly => "monthly",
            Cadence::Annual => "annual",
        },
        typical_amount: wire_dec(subscription.typical_amount),
        annualized_cost: wire_dec(subscription.annualized_cost),
        occurrences: subscription.occurrences,
        first_seen: subscription.first_seen.clone(),
        last_seen: subscription.last_seen.clone(),
        next_expected: subscription.next_expected.clone(),
        accounts: subscription.accounts.clone(),
        manual: subscription.manual,
    }
}

/// Detected subscriptions split by cadence, each sorted by annual cost desc.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireSubscriptionsReport {
    as_of: String,
    lookback_start: String,
    monthly: Vec<WireSubscription>,
    annual: Vec<WireSubscription>,
}

fn wire_subscriptions(report: &SubscriptionsReport) -> WireSubscriptionsReport {
    WireSubscriptionsReport {
        as_of: report.as_of.clone(),
        lookback_start: report.lookback_start.clone(),
        monthly: report.monthly.iter().map(wire_subscription).collect(),
        annual: report.annual.iter().map(wire_subscription).collect(),
    }
}

// ===========================================================================
// Wire representation of the holdings result types
// ===========================================================================

fn wire_dec(dec: Dec) -> WireDec {
    WireDec {
        mantissa: dec.mantissa.to_string(),
        places: dec.places,
    }
}

fn wire_opt_dec(dec: Option<Dec>) -> Option<WireDec> {
    dec.map(wire_dec)
}

/// A holding's resolved price → `{qty, date, source}` (`source` kebab-free:
/// `"directive"` | `"cost"`).
#[derive(Serialize)]
struct WireHoldingPrice {
    qty: WireDec,
    date: String,
    source: &'static str,
}

/// One holding row. Null-valued keys (basis/price/gain/…) are kept (not omitted),
/// matching the TS `Holding` shape.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireHolding {
    symbol: String,
    name: String,
    accounts: Vec<String>,
    shares: WireDec,
    basis: Option<WireDec>,
    first_basis_date: Option<String>,
    price: Option<WireHoldingPrice>,
    market_value: Option<WireDec>,
    gain: Option<WireDec>,
    gain_pct: Option<f64>,
}

fn wire_holding(holding: &Holding) -> WireHolding {
    WireHolding {
        symbol: holding.symbol.clone(),
        name: holding.name.clone(),
        accounts: holding.accounts.clone(),
        shares: wire_dec(holding.shares),
        basis: wire_opt_dec(holding.basis),
        first_basis_date: holding.first_basis_date.clone(),
        price: holding.price.as_ref().map(|price| WireHoldingPrice {
            qty: wire_dec(price.qty),
            date: price.date.clone(),
            source: match price.source {
                PriceSource::Directive => "directive",
                PriceSource::Cost => "cost",
            },
        }),
        market_value: wire_opt_dec(holding.market_value),
        gain: wire_opt_dec(holding.gain),
        gain_pct: holding.gain_pct,
    }
}

/// A scope-local warning → `{symbol, kind, message}` (`kind` kebab-case, matching
/// the TS union: `"missing-basis"` | `"negative-shares"` | `"unpriced"`).
#[derive(Serialize)]
struct WireWarning {
    symbol: String,
    kind: &'static str,
    message: String,
}

/// Portfolio totals: `marketValue` always present; `basis`/`gain`/`gainPct`
/// null when refused.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireHoldingsTotals {
    market_value: WireDec,
    basis: Option<WireDec>,
    gain: Option<WireDec>,
    gain_pct: Option<f64>,
}

/// The full holdings report.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireHoldingsReport {
    as_of: String,
    base: String,
    holdings: Vec<WireHolding>,
    totals: WireHoldingsTotals,
    top_gainers: Vec<WireHolding>,
    top_losers: Vec<WireHolding>,
    warnings: Vec<WireWarning>,
}

fn wire_holdings(report: &HoldingsReport) -> WireHoldingsReport {
    WireHoldingsReport {
        as_of: report.as_of.clone(),
        base: report.base.clone(),
        holdings: report.holdings.iter().map(wire_holding).collect(),
        totals: WireHoldingsTotals {
            market_value: wire_dec(report.totals.market_value),
            basis: wire_opt_dec(report.totals.basis),
            gain: wire_opt_dec(report.totals.gain),
            gain_pct: report.totals.gain_pct,
        },
        top_gainers: report.top_gainers.iter().map(wire_holding).collect(),
        top_losers: report.top_losers.iter().map(wire_holding).collect(),
        warnings: report
            .warnings
            .iter()
            .map(|warning| WireWarning {
                symbol: warning.symbol.clone(),
                kind: match warning.kind {
                    WarningKind::MissingBasis => "missing-basis",
                    WarningKind::NegativeShares => "negative-shares",
                    WarningKind::Unpriced => "unpriced",
                },
                message: warning.message.clone(),
            })
            .collect(),
    }
}

/// One point of the holdings trend.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireHoldingsPoint {
    date: String,
    bucket: String,
    label: String,
    market_value: WireDec,
    basis: Option<WireDec>,
}

/// The holdings-over-time series.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireHoldingsSeries {
    base: String,
    points: Vec<WireHoldingsPoint>,
    has_basis: bool,
}

fn wire_holdings_series(series: &HoldingsSeries) -> WireHoldingsSeries {
    WireHoldingsSeries {
        base: series.base.clone(),
        points: series
            .points
            .iter()
            .map(|point| WireHoldingsPoint {
                date: point.date.clone(),
                bucket: point.bucket.clone(),
                label: point.label.clone(),
                market_value: wire_dec(point.market_value),
                basis: wire_opt_dec(point.basis),
            })
            .collect(),
        has_basis: series.has_basis,
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

/// Parse a holdings scope mode, defaulting to `include` when absent. Returns a
/// `400` tuple for an unrecognized value.
fn parse_mode(raw: Option<&str>) -> Result<ScopeMode, ApiError> {
    match raw {
        None | Some("include") => Ok(ScopeMode::Include),
        Some("exclude") => Ok(ScopeMode::Exclude),
        Some(other) => Err((
            StatusCode::BAD_REQUEST,
            format!("unknown mode '{other}' (expected include|exclude)"),
        )),
    }
}

/// Split a comma-separated `accounts` param into a set of subtree roots, trimming
/// whitespace and dropping empties. `None`/empty ⇒ the empty set = all accounts.
fn parse_accounts(raw: Option<&str>) -> BTreeSet<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// The default cost-of-living exclusion list when the request omits `exclude`:
/// tax accounts. (Mortgage principal and investment/savings transfers are not
/// expense postings, so they never enter the expense total in the first place.)
/// Overridable per request; a config file will supersede this later.
const DEFAULT_COST_EXCLUDE: &[&str] = &["expenses:tax", "expenses:taxes"];

/// Split a comma-separated exclusion param, falling back to `defaults` when the
/// param is absent. An explicit empty value (`exclude=`) means "exclude
/// nothing" — distinct from omitting it, which keeps the defaults.
fn parse_csv(raw: Option<&str>, defaults: &[&str]) -> Vec<String> {
    match raw {
        None => defaults.iter().map(|item| (*item).to_string()).collect(),
        Some(value) => value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

/// The default comparison-span start when the request omits `start`: the first
/// day of the month 24 months before `end` (a trailing two-year "Year-over-
/// year" window). The SPA normally sends explicit month-aligned dates.
fn default_insights_start(end: &str) -> String {
    let year: i64 = end.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(1970);
    let month: i64 = end.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(1);
    let index = year * 12 + (month - 1) - 24;
    let start_year = index.div_euclid(12);
    let start_month = index.rem_euclid(12) + 1;
    format!("{start_year:04}-{start_month:02}-01")
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

/// `?end=&interval=&count=&depth=&valueIn=` — net worth.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetWorthQuery {
    end: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
    depth: Option<usize>,
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

/// `?start=&end=&exclude=` — insights dashboard.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InsightsQuery {
    start: Option<String>,
    end: Option<String>,
    /// Comma-separated account-name prefixes excluded from cost of living.
    exclude: Option<String>,
    /// Minimum base-commodity magnitude for a "biggest change" row (e.g. `10`).
    change_min: Option<String>,
}

/// `?asOf=&lookback=&minMonthly=&minAnnual=` — subscription detection.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubscriptionsQuery {
    as_of: Option<String>,
    /// Months of history to scan (default 24).
    lookback: Option<i64>,
    /// Charges needed before a monthly cadence is believed (default 5).
    min_monthly: Option<usize>,
    /// Charges needed before an annual cadence is believed (default 2).
    min_annual: Option<usize>,
    /// Months a charge may go unseen — measured against its funding account's
    /// own latest activity — before it counts as cancelled (default 3).
    stale_months: Option<i64>,
    /// Comma-separated case-insensitive description substrings to exclude
    /// (default [`DEFAULT_EXCLUDE_DESC`]). An explicit empty value excludes
    /// nothing.
    exclude_desc: Option<String>,
}

/// `?asOf=&accounts=&mode=&gainSince=` — holdings snapshot.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HoldingsQuery {
    as_of: Option<String>,
    accounts: Option<String>,
    mode: Option<String>,
    /// Gain-measurement window start (`YYYY-MM-DD`). Absent/empty = all-time
    /// average-cost gain (unchanged). When set, `gain`/`gainPct` (and totals +
    /// gainers/losers) become `marketValue(asOf) − valueAtStart`; `basis` stays
    /// all-time. See [`holdings`] for the full contract.
    gain_since: Option<String>,
}

/// `?asOf=&accounts=&mode=&interval=&count=` — holdings trend.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HoldingsSeriesQuery {
    as_of: Option<String>,
    accounts: Option<String>,
    mode: Option<String>,
    interval: Option<String>,
    count: Option<usize>,
}

// ===========================================================================
// Handlers
// ===========================================================================

/// `GET /api/reports/balancesheet` — assets + liabilities as of a date.
pub(crate) async fn balancesheet(
    State(state): State<AppState>,
    Query(query): Query<BalanceSheetQuery>,
) -> Result<Json<WireSectionedReport>, ApiError> {
    let snapshot = state.snapshot();
    let as_of = query.as_of.unwrap_or_else(today_utc);
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);
    let declared = declared_types(&account_decls(&snapshot.journal));
    let report = balance_sheet(&snapshot.journal.transactions, &as_of, depth, &declared)
        .map_err(|err| report_error(&err))?;
    Ok(Json(wire_sectioned(&report)))
}

/// `GET /api/reports/incomestatement` — revenues + expenses over a range.
pub(crate) async fn incomestatement(
    State(state): State<AppState>,
    Query(query): Query<IncomeStatementQuery>,
) -> Result<Json<WireSectionedReport>, ApiError> {
    let snapshot = state.snapshot();
    let today = today_utc();
    let from = query
        .from
        .unwrap_or_else(|| format!("{}-01-01", &today[..4]));
    let to = query.to.unwrap_or(today);
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);
    let declared = declared_types(&account_decls(&snapshot.journal));
    let report = income_statement(&snapshot.journal.transactions, &from, &to, depth, &declared)
        .map_err(|err| report_error(&err))?;
    Ok(Json(wire_sectioned(&report)))
}

/// `GET /api/reports/cashflow` — per-bucket cash-like-asset changes. The cash
/// predicate honors the journal's declared account types (same as the SPA).
pub(crate) async fn cashflow(
    State(state): State<AppState>,
    Query(query): Query<CashFlowQuery>,
) -> Result<Json<WirePeriodReport>, ApiError> {
    let snapshot = state.snapshot();
    let end = query.end.unwrap_or_else(today_utc);
    let interval = parse_interval(query.interval.as_deref())?;
    let count = query.count.unwrap_or(DEFAULT_COUNT);
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);

    let decls = account_decls(&snapshot.journal);
    let predicate = cash_predicate(&decls);
    let is_cash: &dyn Fn(&str) -> bool = &predicate;
    let report = cash_flow(
        &snapshot.journal.transactions,
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
/// from the journal's explicit `P` directives PLUS prices inferred from `@`/`@@`
/// cost annotations (hledger `--infer-market-prices`); `depth` clamps the account
/// rows; `valueIn` overrides the target commodity.
pub(crate) async fn networth(
    State(state): State<AppState>,
    Query(query): Query<NetWorthQuery>,
) -> Result<Json<WirePeriodReport>, ApiError> {
    let snapshot = state.snapshot();
    let end = query.end.unwrap_or_else(today_utc);
    let interval = parse_interval(query.interval.as_deref())?;
    let count = query.count.unwrap_or(DEFAULT_COUNT);
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH);
    let value_in = query
        .value_in
        .filter(|symbol| !symbol.is_empty())
        .map(Commodity);

    let declared = declared_types(&account_decls(&snapshot.journal));
    let report = net_worth(
        &snapshot.journal.transactions,
        &snapshot.journal.prices,
        &NetWorthOpts {
            end: &end,
            interval,
            count,
            depth,
            value_in,
            declared: &declared,
        },
    )
    .map_err(|err| report_error(&err))?;
    Ok(Json(wire_period(&report)))
}

/// `GET /api/budget` — actuals vs. periodic-rule goals per bucket.
pub(crate) async fn budget(
    State(state): State<AppState>,
    Query(query): Query<BudgetQuery>,
) -> Result<Json<WireBudgetReport>, ApiError> {
    let snapshot = state.snapshot();
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
        &snapshot.journal.transactions,
        &snapshot.journal.periodic_transactions,
        &opts,
    )
    .map_err(|err| report_error(&err))?;
    Ok(Json(wire_budget(&report)))
}

/// `GET /api/insights` — the period-over-period dashboard. `start`/`end` bound
/// the whole comparison span (default: a trailing 24 months ending today); the
/// engine splits it at its midpoint into a previous and current period.
/// `exclude` overrides the cost-of-living exclusion list.
pub(crate) async fn insights_report(
    State(state): State<AppState>,
    Query(query): Query<InsightsQuery>,
) -> Result<Json<WireInsightsReport>, ApiError> {
    let snapshot = state.snapshot();
    let end = query.end.unwrap_or_else(today_utc);
    let start = query.start.unwrap_or_else(|| default_insights_start(&end));
    let cost_exclude = parse_csv(query.exclude.as_deref(), DEFAULT_COST_EXCLUDE);
    // Default "biggest change" floor: $10 in the base commodity; a malformed
    // value falls back to the default rather than erroring.
    let change_min = query
        .change_min
        .as_deref()
        .and_then(|raw| Dec::parse(raw, '.').ok())
        .unwrap_or_else(|| Dec::new(1000, 2));
    let opts = InsightsOpts {
        start: &start,
        end: &end,
        cost_exclude: &cost_exclude,
        change_min,
    };
    let report = insights(&snapshot.journal, &opts).map_err(|err| report_error(&err))?;
    Ok(Json(wire_insights(&report)))
}

/// `GET /api/subscriptions` — recurring monthly/annual charges inferred from
/// the journal's expense history. Independent of the insights comparison period:
/// it always scans the trailing `lookback` months ending at `asOf`.
pub(crate) async fn subscriptions(
    State(state): State<AppState>,
    Query(query): Query<SubscriptionsQuery>,
) -> Result<Json<WireSubscriptionsReport>, ApiError> {
    let snapshot = state.snapshot();
    let as_of = query.as_of.unwrap_or_else(today_utc);
    let defaults = SubscriptionOpts::default();
    let exclude_desc = parse_csv(query.exclude_desc.as_deref(), DEFAULT_EXCLUDE_DESC);
    let opts = SubscriptionOpts {
        as_of: &as_of,
        lookback_months: query.lookback.unwrap_or(defaults.lookback_months).max(1),
        min_monthly: query.min_monthly.unwrap_or(defaults.min_monthly).max(2),
        min_annual: query.min_annual.unwrap_or(defaults.min_annual).max(2),
        stale_months: query.stale_months.unwrap_or(defaults.stale_months).max(1),
        exclude_desc: &exclude_desc,
        ..defaults
    };
    let report =
        detect_subscriptions(&snapshot.journal, &opts).map_err(|err| report_error(&err))?;
    Ok(Json(wire_subscriptions(&report)))
}

/// `GET /api/holdings` — average-cost stock positions as of a date. `accounts`
/// is a comma-separated set of subtree roots; `mode` selects include vs. exclude.
/// Prices come from the journal's `P` directives (and cost-annotation fallbacks).
///
/// `gainSince=YYYY-MM-DD` (optional) changes the gain start. Absent or empty →
/// all-time average-cost gain, byte-identical to before. When set, each row's
/// `gain` = `marketValue − valueAtStart` and `gainPct` = `gain / valueAtStart ×
/// 100`, where `valueAtStart` is the position's market value at `gainSince`
/// (shares held then, priced as of then; `0` when not held, `null`-propagating
/// when held-but-unpriced then). `basis` stays the all-time average-cost basis;
/// `totals.gain`/`totals.gainPct` are windowed while `totals.basis` stays all-
/// time; `topGainers`/`topLosers` rank by the windowed `gainPct`. The JSON keys
/// are unchanged — only the meaning of `gain`/`gainPct` shifts.
pub(crate) async fn holdings(
    State(state): State<AppState>,
    Query(query): Query<HoldingsQuery>,
) -> Result<Json<WireHoldingsReport>, ApiError> {
    let snapshot = state.snapshot();
    let scope = HoldingsScope {
        accounts: parse_accounts(query.accounts.as_deref()),
        mode: parse_mode(query.mode.as_deref())?,
        as_of: query.as_of.unwrap_or_else(today_utc),
        gain_since: query.gain_since.filter(|start| !start.is_empty()),
    };
    let report = compute_holdings(
        &snapshot.journal.transactions,
        &snapshot.journal.prices,
        &snapshot.journal.accounts,
        &snapshot.journal.commodity_tags,
        &scope,
    )
    .map_err(|err| report_error(&err))?;
    Ok(Json(wire_holdings(&report)))
}

/// `GET /api/holdings/series` — portfolio market value (and basis) at each of the
/// last `count` period boundaries ending at `asOf`. Same scope as `/api/holdings`.
pub(crate) async fn holdings_series_report(
    State(state): State<AppState>,
    Query(query): Query<HoldingsSeriesQuery>,
) -> Result<Json<WireHoldingsSeries>, ApiError> {
    let snapshot = state.snapshot();
    let scope = HoldingsScope {
        accounts: parse_accounts(query.accounts.as_deref()),
        mode: parse_mode(query.mode.as_deref())?,
        as_of: query.as_of.unwrap_or_else(today_utc),
        // The trend tracks market value/basis only — no per-point gain window.
        gain_since: None,
    };
    let interval = parse_interval(query.interval.as_deref())?;
    let count = query.count.unwrap_or(DEFAULT_COUNT);
    let series = holdings_series(
        &snapshot.journal.transactions,
        &snapshot.journal.prices,
        &snapshot.journal.accounts,
        &snapshot.journal.commodity_tags,
        &scope,
        interval,
        count,
    )
    .map_err(|err| report_error(&err))?;
    Ok(Json(wire_holdings_series(&series)))
}
