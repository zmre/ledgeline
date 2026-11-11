//! Startup metrics on Insights — `plans/37-startup-metrics.md`.
//!
//! ARR, MRR growth, the MRR bridge, customers, NRR/GRR, burn multiple, runway,
//! gross margin and the secondary metrics behind them, each reported only when
//! the journal holds the data it needs:
//!
//! - **Tier A** (any journal): revenue, gross margin (when an account resolves
//!   to `issection: cogs`), net burn, cash and runway.
//! - **Tier B** (`kpi:` roles): MRR/ARR, usage, S&M/R&D/G&A, deferred revenue,
//!   billings, DSO/DPO.
//! - **Tier C** (`customer:` on recognised-revenue postings, ≥ 90% coverage):
//!   the bridge, customers, NRR/GRR, ARPA, ACV, concentration.
//!
//! # One pass
//!
//! Transactions are walked once. Each posting's account is classified through a
//! cache keyed by account name (type, `kpi:` role, income-statement section,
//! cash flag, account-level dimension values), and its amounts land in per-month
//! accumulators and a customer × month matrix. Everything else is derived from
//! those arrays. Net burn is the one exception: it comes from
//! [`cash_flow_statement`] so there is no second cash-flow classifier.
//!
//! # Selection and valuation
//!
//! Postings are selected and dated exactly as the income statement selects them
//! ([`effective_date`], every posting type, no status filter), so
//! Σ MRR + Σ usage + non-recurring revenue = the income statement's revenue.
//! Each month's totals are valued in the base commodity at the month's end with
//! the Insights price set (explicit `P` plus inferred); a commodity that cannot
//! be valued is left out and reported in [`SaasReport::unvalued`].
//!
//! Money stays exact [`Dec`]. Averages over a COUNT (ARPA, ACV, CAC, the 3-month
//! burn) use [`Dec::div_int`] at the inputs' scale (at least cents). Ratios of
//! money to money are `f64`, computed at the end like `MetricDelta::pct`.

use super::ReportError;
use super::account_types::{AccountType, AccountTypes, account_decls, declared_types};
use super::aggregate::effective_date;
use super::cash_flow_statement::{CfOpts, CfSectionKind, CfTags, cash_flow_statement};
use super::income_statement::{IsSectionKind, account_sections_from, section_resolver};
use super::insights::{InsightsPeriod, all_prices, split_period};
use super::kpi::{
    KpiRole, any_kpi_declared, declared_dimension_tags, declared_kpi_roles, kpi_resolver,
    nearest_dimension,
};
use super::mixed_amount::MixedAmount;
use super::periods::{Interval, add_days, bucket_end, days_between, parts};
use super::prices::{PriceDb, ValuationMeta, value_at};
use super::subscriptions::payee_of;
use crate::decimal::Dec;
use crate::model::{Commodity, Journal, Posting, Transaction};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The posting tag naming a customer.
pub const CUSTOMER_TAG: &str = "customer";
/// The posting tag carrying a stable customer key (display name from
/// [`CUSTOMER_TAG`]).
pub const CUSTOMER_ID_TAG: &str = "customer-id";
/// The row label for recurring revenue with no value for a dimension key.
pub const UNASSIGNED: &str = "(unassigned)";
/// Default minimum attributed share of recurring revenue for customer metrics.
pub const COVERAGE_MIN_DEFAULT: Dec = Dec::new(90, 2);
/// Default length of each bridge category's customer list.
pub const TOP_N_DEFAULT: usize = 5;
/// Length of [`SaasReport::top_customers`].
const TOP_CUSTOMERS: usize = 10;
/// The most months the engine walks back from the span's end (the bucket
/// cap the cash-flow statement observes, 100 years).
/// Postings older than that still count toward balances.
const MAX_MONTHS: i64 = 1200;
/// Longest runway the engine turns into a cash-out date.
const MAX_RUNWAY_MONTHS: f64 = 1200.0;

/// Inputs to [`saas_metrics`].
#[derive(Debug, Clone)]
pub struct SaasOpts<'a> {
    /// Inclusive span start — the same span as Insights.
    pub start: &'a str,
    /// Inclusive span end.
    pub end: &'a str,
    /// Dimension keys to consider for breakdowns.
    pub dims: &'a [String],
    /// Minimum attributed share of recurring revenue (0.90).
    pub coverage_min: Dec,
    /// Customers listed per bridge category (default 5).
    pub top_n: usize,
    /// Include [`SaasReport::customer_matrix`] (export detail).
    pub with_customer_matrix: bool,
    /// "Today": a month is complete only when its last day is on or before
    /// both the span's end and this date, so a span reaching into the future
    /// never measures a month that hasn't finished. `None` = the span's end.
    pub as_of: Option<&'a str>,
}

/// Why customer metrics are hidden while recurring revenue is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenReason {
    /// No recurring revenue carries a customer.
    NoCustomerTags,
    /// Some does, but less than the coverage minimum.
    LowCoverage,
    /// Recurring revenue is booked at invoice and moved to deferred revenue,
    /// and most attributed revenue sits on those clearing accounts: per-customer
    /// figures would be billings, not MRR.
    BillingClearing,
}

impl HiddenReason {
    /// The wire spelling.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::NoCustomerTags => "no-customer-tags",
            Self::LowCoverage => "low-coverage",
            Self::BillingClearing => "billing-clearing",
        }
    }
}

/// Which metrics the journal supports. The UI obeys this.
#[derive(Debug, Clone, PartialEq)]
pub struct SaasAvailability {
    /// Some account resolves to `kpi: recurring` and some complete month in
    /// the span has positive MRR.
    pub recurring: bool,
    /// `recurring`, coverage ≥ the minimum, and no billing-clearing pattern.
    pub customers: bool,
    /// Any account declaration carries a `kpi:` tag (valid or not).
    pub business: bool,
    /// Some account resolves to `issection: cogs`.
    pub cogs: bool,
    /// Dimension keys with ≥ 2 distinct values carrying recurring revenue in
    /// the span, in request order.
    pub dimensions: Vec<String>,
    /// Set when `recurring` is true and `customers` is false.
    pub customers_hidden_reason: Option<HiddenReason>,
    /// Attributed share of recurring revenue over the span's complete months,
    /// as a percent (clamped to 0–100). Set whenever `recurring` is true.
    pub coverage_pct: Option<f64>,
    /// Some of the current half's last complete month's MRR is attributed
    /// through the payee fallback.
    pub inferred_customers: bool,
    /// Plan 37 A1.2: the per-customer MRR metrics are hidden and the journal
    /// has billings, so customer counts come from billings instead
    /// ([`SaasPeriodMetrics::billing_customers`]).
    pub billing_customers: bool,
    /// Plan 37 A2: the data horizon, the last month (`YYYY-MM`) holding any
    /// recurring posting. Months after it are open and never complete. Set
    /// whenever `recurring` is true.
    pub closed_through: Option<String>,
}

/// Which burn a figure is measured against: the runway (plan 37 A1.1) and
/// the burn multiple (A3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunwayBasis {
    /// Net burn adjusted for prepayments and receivables.
    Adjusted,
    /// Raw cash net burn.
    Cash,
}

impl RunwayBasis {
    /// The wire spelling.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Adjusted => "adjusted",
            Self::Cash => "cash",
        }
    }
}

/// Customer counts from billings (plan 37 A1.2), for books whose recurring
/// revenue can't be attributed per customer. A billing is a customer-attributed
/// credit to a recurring or deferred-revenue account in a transaction with a
/// receivable or cash leg; a customer is active in month *m* when it has a
/// billing in *m − 11 … m*.
#[derive(Debug, Clone, PartialEq)]
pub struct BillingCustomers {
    /// |BilledActive(m)|.
    pub customers: u32,
    /// Against |BilledActive| in the month before the half.
    pub customer_growth_pct: Option<f64>,
    /// Customers whose first-ever billing is in the half's complete months.
    pub new_logos: u32,
    /// Customers that went 12 months without a billing during the half.
    pub lapsed_logos: u32,
    /// `lapsed_logos` ÷ |BilledActive| in the month before the half × 100.
    pub logo_churn_pct: Option<f64>,
    /// Trailing-12-month billings at *m* ÷ |BilledActive(m)|.
    pub acv: Option<Dec>,
    /// The journal starts less than 12 months before *m*, so the count may be
    /// low.
    pub window_short: bool,
    /// Some active customer's billings in the window were found by payee.
    pub inferred: bool,
}

/// One month's (or one half's summed) MRR bridge. Amounts are positive;
/// `other` is signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bridge {
    /// MRR from customers never positive before.
    pub new: Dec,
    /// Increases from customers already paying.
    pub expansion: Dec,
    /// Decreases from customers still paying.
    pub contraction: Dec,
    /// MRR lost to customers that went to zero.
    pub churn: Dec,
    /// MRR from returning customers.
    pub reactivation: Dec,
    /// Reconciling line: unattributed revenue and credits (negative months).
    pub other: Dec,
    /// Count of new customers.
    pub new_logos: u32,
    /// Count of churned customers.
    pub churned_logos: u32,
    /// Count of reactivated customers.
    pub reactivated_logos: u32,
}

impl Bridge {
    fn zero() -> Self {
        Self {
            new: Dec::zero(),
            expansion: Dec::zero(),
            contraction: Dec::zero(),
            churn: Dec::zero(),
            reactivation: Dec::zero(),
            other: Dec::zero(),
            new_logos: 0,
            churned_logos: 0,
            reactivated_logos: 0,
        }
    }

    fn add(self, other: &Self) -> Result<Self, ReportError> {
        Ok(Self {
            new: self.new.add(other.new)?,
            expansion: self.expansion.add(other.expansion)?,
            contraction: self.contraction.add(other.contraction)?,
            churn: self.churn.add(other.churn)?,
            reactivation: self.reactivation.add(other.reactivation)?,
            other: self.other.add(other.other)?,
            new_logos: self.new_logos + other.new_logos,
            churned_logos: self.churned_logos + other.churned_logos,
            reactivated_logos: self.reactivated_logos + other.reactivated_logos,
        })
    }

    /// `new + expansion − contraction − churn + reactivation`.
    fn net_categories(&self) -> Result<Dec, ReportError> {
        Ok(self
            .new
            .add(self.expansion)?
            .sub(self.contraction)?
            .sub(self.churn)?
            .add(self.reactivation)?)
    }
}

/// One calendar month of the span.
#[derive(Debug, Clone, PartialEq)]
pub struct SaasMonth {
    /// `YYYY-MM`.
    pub month: String,
    /// Only partly inside the span, unfinished by `as_of`, or open: computed,
    /// never a "latest complete month".
    pub partial: bool,
    /// After the data horizon (plan 37 A2): the books aren't closed for this
    /// month yet (no recurring revenue recognised), though it has started by
    /// `as_of`. Always also `partial`.
    pub open: bool,
    /// Recognised recurring revenue.
    pub mrr: Dec,
    /// Usage revenue.
    pub usage: Dec,
    /// Recurring revenue with no customer.
    pub unattributed: Dec,
    /// Customers with positive MRR (tier C).
    pub customers: Option<u32>,
    /// The month's bridge (tier C).
    pub bridge: Option<Bridge>,
    /// All revenue.
    pub revenue: Dec,
    /// Cost of revenue (when an account resolves to `cogs`).
    pub cogs: Option<Dec>,
    /// −(operating + investing cash flow). Positive = burning.
    pub net_burn: Dec,
    /// `net_burn + Δdeferred − ΔAR` (plan 37 A1.1): burn as if every customer
    /// paid when served. Present when a deferred-revenue or receivable account
    /// resolves.
    pub net_burn_adjusted: Option<Dec>,
    /// Cash accounts' balance at month end.
    pub cash_end: Dec,
    /// Sales & marketing spend.
    pub sm: Dec,
    /// R&D spend.
    pub rnd: Dec,
    /// G&A spend.
    pub ga: Dec,
    /// Deferred revenue owed at month end (when the role exists).
    pub deferred_end: Option<Dec>,
    /// Receivables at month end (when the role exists).
    pub ar_end: Option<Dec>,
    /// Payables owed at month end (when the role exists).
    pub ap_end: Option<Dec>,
}

/// The period metrics for one half. See the plan's table for each formula.
#[derive(Debug, Clone, PartialEq)]
pub struct SaasPeriodMetrics {
    /// The half's last complete month *m* (`YYYY-MM`), `None` when it has none.
    pub month: Option<String>,
    /// Number of complete months in the half (*k*).
    pub months: u32,
    pub mrr: Option<Dec>,
    pub arr: Option<Dec>,
    pub usage: Option<Dec>,
    pub mrr_growth_pct: Option<f64>,
    pub cmgr_pct: Option<f64>,
    pub net_new_arr: Option<Dec>,
    pub bridge: Option<Bridge>,
    pub customers: Option<u32>,
    pub customer_growth_pct: Option<f64>,
    pub logo_churn_pct: Option<f64>,
    pub arpa: Option<Dec>,
    pub acv: Option<Dec>,
    pub nrr_pct: Option<f64>,
    pub grr_pct: Option<f64>,
    pub quick_ratio: Option<f64>,
    pub top1_pct: Option<f64>,
    pub top5_pct: Option<f64>,
    pub top10_pct: Option<f64>,
    /// Total revenue over the complete months.
    pub revenue: Dec,
    pub gross_profit: Option<Dec>,
    pub gross_margin_pct: Option<f64>,
    /// Σ net burn over the complete months.
    pub net_burn: Dec,
    /// Σ adjusted net burn over the complete months (plan 37 A3), when
    /// [`SaasMonth::net_burn_adjusted`] exists.
    pub net_burn_adjusted: Option<Dec>,
    pub net_burn_avg3: Option<Dec>,
    /// The 3-month average of [`SaasMonth::net_burn_adjusted`].
    pub net_burn_adjusted_avg3: Option<Dec>,
    /// Which burn `runway_months` and `cash_out_date` use: adjusted whenever
    /// it exists. `None` when the half has no complete month.
    pub runway_basis: Option<RunwayBasis>,
    pub cash: Option<Dec>,
    pub runway_months: Option<f64>,
    pub cash_out_date: Option<String>,
    /// Σ burn over the half ÷ net new ARR, on the adjusted burn when it exists
    /// (plan 37 A3), else raw net burn. `None` when either is ≤ 0.
    pub burn_multiple: Option<f64>,
    /// Which burn `burn_multiple` uses. Present whenever it is.
    pub burn_multiple_basis: Option<RunwayBasis>,
    pub rule_of_40: Option<f64>,
    pub sm: Option<Dec>,
    pub rnd: Option<Dec>,
    pub ga: Option<Dec>,
    pub sm_pct: Option<f64>,
    pub rnd_pct: Option<f64>,
    pub ga_pct: Option<f64>,
    pub cac: Option<Dec>,
    pub cac_payback_months: Option<f64>,
    pub deferred_revenue: Option<Dec>,
    pub billings: Option<Dec>,
    pub dso_days: Option<f64>,
    pub dpo_days: Option<f64>,
    /// Customer counts from billings (only when
    /// [`SaasAvailability::billing_customers`]).
    pub billing_customers: Option<BillingCustomers>,
}

/// One value of a dimension key.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentRow {
    /// The value, or [`UNASSIGNED`].
    pub value: String,
    /// MRR at the current half's *m*.
    pub mrr_curr: Dec,
    /// MRR at the previous half's *m*.
    pub mrr_prev: Dec,
    pub customers_curr: Option<u32>,
    pub arpa: Option<Dec>,
    pub nrr_pct: Option<f64>,
    /// The current half's bridge within this value.
    pub bridge: Option<Bridge>,
}

/// One dimension key's rows.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentBreakdown {
    pub key: String,
    /// By current MRR (descending), [`UNASSIGNED`] last.
    pub rows: Vec<SegmentRow>,
}

/// A customer's contribution to one bridge category over the current half.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerMove {
    /// The customer key (`customer-id:` or `customer:`).
    pub customer: String,
    /// Display name.
    pub name: String,
    /// The customer's value of the first dimension key at the move.
    pub segment: Option<String>,
    pub amount: Dec,
    /// Attributed through the payee fallback.
    pub inferred: bool,
}

/// The current half's top customers per bridge category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCustomers {
    pub new: Vec<CustomerMove>,
    pub expansion: Vec<CustomerMove>,
    pub contraction: Vec<CustomerMove>,
    pub churn: Vec<CustomerMove>,
    pub reactivation: Vec<CustomerMove>,
}

/// One customer's share of MRR at the current half's *m*.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomerShare {
    pub customer: String,
    pub name: String,
    pub mrr: Dec,
    pub pct: f64,
}

/// One row of the export's customer × month matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerRow {
    pub customer: String,
    pub name: String,
    /// First dimension key's value in the customer's latest positive month.
    pub segment: Option<String>,
    /// C[c][m] (unclamped), aligned to [`SaasReport::months`].
    pub values: Vec<Dec>,
}

/// The startup-metrics report.
#[derive(Debug, Clone, PartialEq)]
pub struct SaasReport {
    /// The same split as Insights.
    pub period: InsightsPeriod,
    pub base: String,
    pub available: SaasAvailability,
    pub months: Vec<SaasMonth>,
    pub current: SaasPeriodMetrics,
    pub previous: SaasPeriodMetrics,
    pub segments: Vec<SegmentBreakdown>,
    /// Tier C only.
    pub bridge_customers: Option<BridgeCustomers>,
    /// Tier C only (empty otherwise).
    pub top_customers: Vec<CustomerShare>,
    pub customer_matrix: Option<Vec<CustomerRow>>,
    /// Recurring and usage revenue in commodities with no price to `base`,
    /// left out of MRR and usage.
    pub unvalued: MixedAmount,
}

// ---------------------------------------------------------------------------
// Month arithmetic
// ---------------------------------------------------------------------------

/// `year × 12 + month − 1` of an ISO date.
fn month_index(date: &str) -> i64 {
    let (year, month, _) = parts(date);
    year * 12 + month - 1
}

/// `YYYY-MM` of a month index.
fn month_key(index: i64) -> String {
    format!(
        "{:04}-{:02}",
        index.div_euclid(12),
        index.rem_euclid(12) + 1
    )
}

fn month_first(index: i64) -> String {
    format!("{}-01", month_key(index))
}

fn month_last(index: i64) -> Result<String, ReportError> {
    bucket_end(&month_key(index))
}

/// The complete months inside `[from, to]` as an inclusive index range, or
/// `None` when there are none. A month is complete when it starts on or after
/// `from` and its last day is on or before both `to` and `as_of`.
fn complete_months(
    from: &str,
    to: &str,
    as_of: Option<&str>,
) -> Result<Option<(i64, i64)>, ReportError> {
    let limit = as_of.map_or(to, |as_of| to.min(as_of));
    let first = month_index(from) + i64::from(from != month_first(month_index(from)));
    let last = month_index(limit) - i64::from(limit != month_last(month_index(limit))?);
    Ok((first <= last).then_some((first, last)))
}

/// `part / whole × 100`, `None` when `whole` ≤ 0.
fn pct(part: Dec, whole: Dec) -> Option<f64> {
    (whole > Dec::zero()).then(|| part.floating_point() / whole.floating_point() * 100.0)
}

/// `part / whole`, `None` when `whole` ≤ 0.
fn ratio(part: Dec, whole: Dec) -> Option<f64> {
    (whole > Dec::zero()).then(|| part.floating_point() / whole.floating_point())
}

/// Money divided by a count, at the money's scale (at least cents).
fn mean(total: Dec, count: u32) -> Result<Option<Dec>, ReportError> {
    if count == 0 {
        return Ok(None);
    }
    let scaled = total.rounded(total.places.max(2))?;
    Ok(Some(scaled.div_int(count)?))
}

fn positive(value: Dec) -> Dec {
    value.max(Dec::zero())
}

fn sum<'a>(values: impl IntoIterator<Item = &'a Dec>) -> Result<Dec, ReportError> {
    values
        .into_iter()
        .try_fold(Dec::zero(), |acc, value| acc.add(*value))
        .map_err(ReportError::from)
}

// ---------------------------------------------------------------------------
// Accumulation
// ---------------------------------------------------------------------------

/// Everything the pass needs to know about one account, resolved once.
#[derive(Debug, Clone)]
struct AccountInfo {
    revenue: bool,
    expense: bool,
    cash: bool,
    cogs: bool,
    role: Option<KpiRole>,
    /// Account-level value per requested dimension key (nearest declared).
    dims: Vec<Option<String>>,
}

/// A month-indexed series of exact values, zero outside its range.
#[derive(Debug, Clone)]
struct Series {
    lo: i64,
    values: Vec<Dec>,
}

impl Series {
    fn get(&self, index: i64) -> Dec {
        usize::try_from(index - self.lo)
            .ok()
            .and_then(|i| self.values.get(i).copied())
            .unwrap_or_else(Dec::zero)
    }
}

/// One customer's accumulated recurring revenue.
#[derive(Debug, Clone, Default)]
struct CustomerAcc {
    /// Display name and the date it was last seen with.
    name: (String, String),
    /// Raw (as posted, revenue negative) amounts per month offset.
    months: BTreeMap<usize, MixedAmount>,
    /// Inferred raw amounts per month offset.
    inferred_months: BTreeMap<usize, MixedAmount>,
    /// The first dimension key's value per month offset (last posting wins).
    segment: BTreeMap<usize, String>,
}

/// Raw per-month accumulators, indexed by month offset from `lo`.
struct Accumulators {
    lo: i64,
    n: usize,
    revenue: Vec<MixedAmount>,
    mrr: Vec<MixedAmount>,
    usage: Vec<MixedAmount>,
    unattributed: Vec<MixedAmount>,
    cogs: Vec<MixedAmount>,
    expenses: Vec<MixedAmount>,
    sm: Vec<MixedAmount>,
    rnd: Vec<MixedAmount>,
    ga: Vec<MixedAmount>,
    /// Balance deltas; postings before `lo` land in the opening slot.
    cash: Balance,
    deferred: Balance,
    ar: Balance,
    ap: Balance,
    customers: BTreeMap<String, CustomerAcc>,
    /// key index → value → (segment MRR per month, customer → month → raw).
    dims: Vec<BTreeMap<String, DimAcc>>,
    /// Raw attributed recurring amounts per month offset.
    attributed: Vec<MixedAmount>,
    /// The attributed part that sits on clearing postings (see
    /// [`ClearingRule`]).
    attributed_clearing: Vec<MixedAmount>,
    /// Billings per customer key (plan 37 A1.2).
    billings: BTreeMap<String, BillingAcc>,
}

/// One customer's billings: raw (credit-negative) amounts per month offset,
/// and the months found by payee.
#[derive(Debug, Clone, Default)]
struct BillingAcc {
    months: BTreeMap<usize, MixedAmount>,
    inferred_months: BTreeSet<usize>,
}

#[derive(Debug, Clone, Default)]
struct DimAcc {
    mrr: BTreeMap<usize, MixedAmount>,
    customers: BTreeMap<String, BTreeMap<usize, MixedAmount>>,
}

#[derive(Debug, Clone)]
struct Balance {
    opening: MixedAmount,
    deltas: Vec<MixedAmount>,
}

impl Balance {
    fn new(n: usize) -> Self {
        Self {
            opening: MixedAmount::new(),
            deltas: vec![MixedAmount::new(); n],
        }
    }

    fn add(&mut self, slot: Option<usize>, posting: &Posting) -> Result<(), ReportError> {
        let target = match slot {
            Some(i) => &mut self.deltas[i],
            None => &mut self.opening,
        };
        accumulate(target, posting)
    }
}

fn accumulate(target: &mut MixedAmount, posting: &Posting) -> Result<(), ReportError> {
    for amount in &posting.amounts {
        target.accumulate(&amount.commodity, amount.quantity)?;
    }
    Ok(())
}

fn accumulate_at(
    map: &mut BTreeMap<usize, MixedAmount>,
    slot: usize,
    posting: &Posting,
) -> Result<(), ReportError> {
    accumulate(map.entry(slot).or_default(), posting)
}

/// A trimmed, non-empty tag value.
fn tag<'t>(tags: &'t [(String, String)], key: &str) -> Option<&'t str> {
    tags.iter()
        .find(|(k, _)| k == key)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// Who a recurring posting's revenue belongs to.
struct Attribution {
    key: String,
    name: String,
    inferred: bool,
}

/// Per-transaction facts the payee fallback and the clearing test need.
struct TxnShape {
    recurring_postings: usize,
    /// Credits to a recurring or deferred-revenue account (billing candidates).
    billing_postings: usize,
    deferred_leg: bool,
    receivable_or_cash_leg: bool,
}

/// Which recurring postings are billing-clearing postings.
///
/// A recurring account that is debited against deferred revenue (a "move to
/// deferred") inside the span's complete months is a clearing account. A
/// clearing POSTING is a CREDIT to such an account, in one of those months, in
/// a transaction with no deferred-revenue leg: the invoice that books revenue
/// before the move takes it back out. Recognition lines (which have a deferred
/// leg) never are, so correctly booked books with the odd correction debit are
/// not mistaken for clearing books.
struct ClearingRule<'a> {
    accounts: BTreeSet<&'a str>,
    months: Option<(i64, i64)>,
}

impl ClearingRule<'_> {
    fn is_clearing(&self, account: &str, month: i64, posting: &Posting, shape: &TxnShape) -> bool {
        !shape.deferred_leg
            && credits(posting)
            && self.months.is_some_and(|(a, b)| (a..=b).contains(&month))
            && self.accounts.contains(account)
    }
}

fn attribution(
    txn: &Transaction,
    posting: &Posting,
    shape: &TxnShape,
    clearing: bool,
) -> Option<Attribution> {
    if let Some(id) = tag(&posting.tags, CUSTOMER_ID_TAG) {
        let name = tag(&posting.tags, CUSTOMER_TAG).unwrap_or(id);
        return Some(Attribution {
            key: id.to_string(),
            name: name.to_string(),
            inferred: false,
        });
    }
    if let Some(name) = tag(&posting.tags, CUSTOMER_TAG).or_else(|| tag(&txn.tags, CUSTOMER_TAG)) {
        return Some(Attribution {
            key: name.to_string(),
            name: name.to_string(),
            inferred: false,
        });
    }
    // A clearing posting is an invoice, not revenue: never a payee's MRR.
    let payee = payee_of(&txn.description);
    (!clearing
        && shape.recurring_postings == 1
        && !shape.deferred_leg
        && shape.receivable_or_cash_leg
        && !payee.is_empty())
    .then(|| Attribution {
        key: payee.to_string(),
        name: payee.to_string(),
        inferred: true,
    })
}

/// The pass over every posting dated on or before the span's end.
#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
fn accumulate_journal(
    journal: &Journal,
    end: &str,
    lo: i64,
    n: usize,
    dims: &[String],
    span_complete: Option<(i64, i64)>,
    info: &mut dyn FnMut(&str) -> AccountInfo,
) -> Result<Accumulators, ReportError> {
    let mut acc = Accumulators {
        lo,
        n,
        revenue: vec![MixedAmount::new(); n],
        mrr: vec![MixedAmount::new(); n],
        usage: vec![MixedAmount::new(); n],
        unattributed: vec![MixedAmount::new(); n],
        cogs: vec![MixedAmount::new(); n],
        expenses: vec![MixedAmount::new(); n],
        sm: vec![MixedAmount::new(); n],
        rnd: vec![MixedAmount::new(); n],
        ga: vec![MixedAmount::new(); n],
        cash: Balance::new(n),
        deferred: Balance::new(n),
        ar: Balance::new(n),
        ap: Balance::new(n),
        customers: BTreeMap::new(),
        dims: vec![BTreeMap::new(); dims.len()],
        attributed: vec![MixedAmount::new(); n],
        attributed_clearing: vec![MixedAmount::new(); n],
        billings: BTreeMap::new(),
    };
    // Each distinct account is resolved once; postings refer to it by index.
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let mut infos: Vec<AccountInfo> = Vec::new();
    let mut legs: Vec<usize> = Vec::new();
    // Pre-scan: the recurring accounts moved to deferred inside the span.
    let mut clearing = ClearingRule {
        accounts: BTreeSet::new(),
        months: span_complete,
    };
    for txn in &journal.transactions {
        resolve_legs(txn, &mut ids, &mut infos, &mut legs, info);
        let deferred_credit = txn.postings.iter().enumerate().any(|(i, posting)| {
            infos[legs[i]].role == Some(KpiRole::DeferredRevenue) && credits(posting)
        });
        if !deferred_credit {
            continue;
        }
        for (i, posting) in txn.postings.iter().enumerate() {
            let month = month_index(effective_date(txn, posting));
            if infos[legs[i]].role == Some(KpiRole::Recurring)
                && debits(posting)
                && span_complete.is_some_and(|(a, b)| (a..=b).contains(&month))
            {
                clearing.accounts.insert(posting.account.0.as_str());
            }
        }
    }

    for txn in &journal.transactions {
        resolve_legs(txn, &mut ids, &mut infos, &mut legs, info);
        let leg = |i: usize| &infos[legs[i]];
        let has_role = |role: KpiRole| (0..legs.len()).any(|i| leg(i).role == Some(role));
        let shape = TxnShape {
            recurring_postings: (0..legs.len())
                .filter(|&i| leg(i).role == Some(KpiRole::Recurring))
                .count(),
            billing_postings: txn
                .postings
                .iter()
                .enumerate()
                .filter(|(i, posting)| is_billing_role(leg(*i).role) && credits(posting))
                .count(),
            deferred_leg: has_role(KpiRole::DeferredRevenue),
            receivable_or_cash_leg: has_role(KpiRole::Receivable)
                || (0..legs.len()).any(|i| leg(i).cash),
        };

        for (i, posting) in txn.postings.iter().enumerate() {
            let leg = &infos[legs[i]];
            let date = effective_date(txn, posting);
            if date > end {
                continue;
            }
            let month = month_index(date);
            let offset = month - lo;
            let slot = usize::try_from(offset).ok().filter(|&i| i < n);
            if leg.cash {
                acc.cash.add(slot, posting)?;
            }
            match leg.role {
                Some(KpiRole::DeferredRevenue) => acc.deferred.add(slot, posting)?,
                Some(KpiRole::Receivable) => acc.ar.add(slot, posting)?,
                Some(KpiRole::Payable) => acc.ap.add(slot, posting)?,
                _ => {}
            }
            // Flows before the walked range only matter to balances.
            let Some(i) = slot else {
                continue;
            };
            if is_billing_role(leg.role) && credits(posting) && shape.receivable_or_cash_leg {
                billing(&mut acc, txn, posting, &shape, i)?;
            }
            if leg.revenue {
                accumulate(&mut acc.revenue[i], posting)?;
            }
            if leg.expense {
                accumulate(&mut acc.expenses[i], posting)?;
            }
            if leg.cogs {
                accumulate(&mut acc.cogs[i], posting)?;
            }
            match leg.role {
                Some(KpiRole::Usage) => accumulate(&mut acc.usage[i], posting)?,
                Some(KpiRole::SalesMarketing) => accumulate(&mut acc.sm[i], posting)?,
                Some(KpiRole::Rnd) => accumulate(&mut acc.rnd[i], posting)?,
                Some(KpiRole::Ga) => accumulate(&mut acc.ga[i], posting)?,
                Some(KpiRole::Recurring) => {
                    let is_clearing =
                        clearing.is_clearing(&posting.account.0, month, posting, &shape);
                    recurring_posting(
                        &mut acc,
                        &RecurringPosting {
                            txn,
                            posting,
                            leg,
                            shape: &shape,
                            date,
                            clearing: is_clearing,
                        },
                        i,
                        dims,
                    )?;
                }
                _ => {}
            }
        }
    }
    Ok(acc)
}

/// The roles a billing credits: recurring revenue or deferred revenue.
fn is_billing_role(role: Option<KpiRole>) -> bool {
    matches!(role, Some(KpiRole::Recurring | KpiRole::DeferredRevenue))
}

/// Book one billing (plan 37 A1.2): a credit to a recurring or deferred
/// revenue account in a transaction with a receivable or cash leg, attributed
/// by `customer-id:`, `customer:` (posting, then transaction), or the payee
/// when the transaction holds exactly one such credit.
fn billing(
    acc: &mut Accumulators,
    txn: &Transaction,
    posting: &Posting,
    shape: &TxnShape,
    i: usize,
) -> Result<(), ReportError> {
    let tagged = tag(&posting.tags, CUSTOMER_ID_TAG)
        .or_else(|| tag(&posting.tags, CUSTOMER_TAG))
        .or_else(|| tag(&txn.tags, CUSTOMER_ID_TAG))
        .or_else(|| tag(&txn.tags, CUSTOMER_TAG));
    let payee = payee_of(&txn.description);
    let (key, inferred) = match tagged {
        Some(key) => (key, false),
        None if shape.billing_postings == 1 && !payee.is_empty() => (payee, true),
        None => return Ok(()),
    };
    let customer = acc.billings.entry(key.to_string()).or_default();
    accumulate_at(&mut customer.months, i, posting)?;
    if inferred {
        customer.inferred_months.insert(i);
    }
    Ok(())
}

/// The last month (index) holding a recurring posting dated on or before
/// `end`, or `None` when there is none (plan 37 A2).
fn recurring_horizon(
    journal: &Journal,
    end: &str,
    kpi: &dyn Fn(&str) -> Option<KpiRole>,
) -> Option<i64> {
    let mut recurring: HashMap<&str, bool> = HashMap::new();
    journal
        .transactions
        .iter()
        .flat_map(|txn| txn.postings.iter().map(move |posting| (txn, posting)))
        .filter(|(_, posting)| {
            *recurring
                .entry(posting.account.0.as_str())
                .or_insert_with(|| kpi(&posting.account.0) == Some(KpiRole::Recurring))
        })
        .map(|(txn, posting)| effective_date(txn, posting))
        .filter(|date| *date <= end)
        .max()
        .map(month_index)
}

/// Resolve `txn`'s posting accounts to indexes into `infos`, resolving each
/// distinct account once.
fn resolve_legs<'j>(
    txn: &'j Transaction,
    ids: &mut HashMap<&'j str, usize>,
    infos: &mut Vec<AccountInfo>,
    legs: &mut Vec<usize>,
    info: &mut dyn FnMut(&str) -> AccountInfo,
) {
    legs.clear();
    for posting in &txn.postings {
        let account = posting.account.0.as_str();
        let id = *ids.entry(account).or_insert_with(|| {
            infos.push(info(account));
            infos.len() - 1
        });
        legs.push(id);
    }
}

/// True when every amount on `posting` is positive (a debit).
fn debits(posting: &Posting) -> bool {
    !posting.amounts.is_empty() && posting.amounts.iter().all(|a| a.quantity > Dec::zero())
}

/// True when every amount on `posting` is negative (a credit).
fn credits(posting: &Posting) -> bool {
    !posting.amounts.is_empty() && posting.amounts.iter().all(|a| a.quantity < Dec::zero())
}

/// One recurring posting and what the pass knows about it.
struct RecurringPosting<'a> {
    txn: &'a Transaction,
    posting: &'a Posting,
    leg: &'a AccountInfo,
    shape: &'a TxnShape,
    date: &'a str,
    /// A billing-clearing posting ([`ClearingRule`]).
    clearing: bool,
}

/// Book one recurring posting: MRR, the customer matrix and each dimension.
fn recurring_posting(
    acc: &mut Accumulators,
    item: &RecurringPosting,
    i: usize,
    dims: &[String],
) -> Result<(), ReportError> {
    let &RecurringPosting {
        txn,
        posting,
        leg,
        shape,
        date,
        clearing,
    } = item;
    accumulate(&mut acc.mrr[i], posting)?;
    let values: Vec<Option<String>> = dims
        .iter()
        .zip(&leg.dims)
        .map(|(key, declared)| {
            tag(&posting.tags, key)
                .or_else(|| tag(&txn.tags, key))
                .map(str::to_string)
                .or_else(|| declared.clone())
        })
        .collect();
    let who = attribution(txn, posting, shape, clearing);
    for ((dim, value), _) in acc.dims.iter_mut().zip(&values).zip(dims) {
        let value = value.clone().unwrap_or_else(|| UNASSIGNED.to_string());
        let slot = dim.entry(value).or_default();
        accumulate_at(&mut slot.mrr, i, posting)?;
        if let Some(who) = &who {
            accumulate_at(
                slot.customers.entry(who.key.clone()).or_default(),
                i,
                posting,
            )?;
        }
    }
    let Some(who) = who else {
        accumulate(&mut acc.unattributed[i], posting)?;
        return Ok(());
    };
    let customer = acc.customers.entry(who.key).or_default();
    if customer.name.1.as_str() <= date {
        customer.name = (who.name, date.to_string());
    }
    accumulate_at(&mut customer.months, i, posting)?;
    if who.inferred {
        accumulate_at(&mut customer.inferred_months, i, posting)?;
    }
    if let Some(Some(value)) = values.first() {
        customer.segment.insert(i, value.clone());
    }
    accumulate(&mut acc.attributed[i], posting)?;
    if clearing {
        accumulate(&mut acc.attributed_clearing[i], posting)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Valuation
// ---------------------------------------------------------------------------

/// Values month-indexed amounts in the base commodity at each month's end.
struct Valuer<'a> {
    base: &'a Commodity,
    db: &'a PriceDb,
    ends: Vec<String>,
}

impl Valuer<'_> {
    /// `ma` valued at month offset `i`'s end, sign-flipped when `flip`;
    /// unpriced commodities go to `unvalued` when given.
    fn value(
        &self,
        ma: &MixedAmount,
        i: usize,
        flip: bool,
        unvalued: Option<&mut MixedAmount>,
    ) -> Result<Dec, ReportError> {
        let mut meta = ValuationMeta::default();
        let valued = value_at(ma, self.base, self.db, &self.ends[i], Some(&mut meta))?;
        if let Some(sink) = unvalued {
            for commodity in &meta.unpriced {
                if let Some(qty) = ma.get(commodity) {
                    sink.accumulate(commodity, if flip { qty.neg()? } else { qty })?;
                }
            }
        }
        Ok(if flip { valued.neg()? } else { valued })
    }

    fn flow(&self, lo: i64, raw: &[MixedAmount], flip: bool) -> Result<Series, ReportError> {
        Ok(Series {
            lo,
            values: raw
                .iter()
                .enumerate()
                .map(|(i, ma)| self.value(ma, i, flip, None))
                .collect::<Result<_, _>>()?,
        })
    }

    fn sparse(
        &self,
        lo: i64,
        n: usize,
        raw: &BTreeMap<usize, MixedAmount>,
    ) -> Result<Series, ReportError> {
        let mut values = vec![Dec::zero(); n];
        for (&i, ma) in raw {
            values[i] = self.value(ma, i, true, None)?;
        }
        Ok(Series { lo, values })
    }

    /// Month-end balances: the running sum, valued at each month's end.
    fn balance(&self, lo: i64, raw: &Balance, flip: bool) -> Result<Series, ReportError> {
        let mut running = raw.opening.clone();
        let mut values = Vec::with_capacity(raw.deltas.len());
        for (i, delta) in raw.deltas.iter().enumerate() {
            running = running.ma_add(delta)?;
            values.push(self.value(&running, i, flip, None)?);
        }
        Ok(Series { lo, values })
    }
}

// ---------------------------------------------------------------------------
// The customer matrix and the bridge
// ---------------------------------------------------------------------------

/// Customer rows (valued, unclamped) plus the total they reconcile to.
struct Matrix {
    lo: i64,
    keys: Vec<String>,
    rows: Vec<Vec<Dec>>,
    /// First month offset each row is positive, if ever.
    first_positive: Vec<Option<usize>>,
    total: Series,
}

impl Matrix {
    fn new(lo: i64, keys: Vec<String>, rows: Vec<Vec<Dec>>, total: Series) -> Self {
        let first_positive = rows
            .iter()
            .map(|row| row.iter().position(|value| *value > Dec::zero()))
            .collect();
        Self {
            lo,
            keys,
            rows,
            first_positive,
            total,
        }
    }

    fn at(&self, row: usize, index: i64) -> Dec {
        usize::try_from(index - self.lo)
            .ok()
            .and_then(|i| self.rows[row].get(i).copied())
            .unwrap_or_else(Dec::zero)
    }

    fn customers(&self, index: i64) -> u32 {
        let count = (0..self.rows.len())
            .filter(|&row| self.at(row, index) > Dec::zero())
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Σ C⁺ at `index`.
    fn attributed_positive(&self, index: i64) -> Result<Dec, ReportError> {
        (0..self.rows.len()).try_fold(Dec::zero(), |acc, row| {
            Ok(acc.add(positive(self.at(row, index)))?)
        })
    }

    /// The bridge for month `index`, and each customer's move.
    fn bridge(&self, index: i64) -> Result<(Bridge, Vec<Move>), ReportError> {
        let mut bridge = Bridge::zero();
        let mut moves = Vec::new();
        for row in 0..self.rows.len() {
            let p = positive(self.at(row, index - 1));
            let q = positive(self.at(row, index));
            let positive_before = self.first_positive[row]
                .is_some_and(|first| i64::try_from(first).unwrap_or(i64::MAX) + self.lo < index);
            let zero = Dec::zero();
            let (category, amount) = if p == zero && q > zero {
                if positive_before {
                    bridge.reactivation = bridge.reactivation.add(q)?;
                    bridge.reactivated_logos += 1;
                    (Category::Reactivation, q)
                } else {
                    bridge.new = bridge.new.add(q)?;
                    bridge.new_logos += 1;
                    (Category::New, q)
                }
            } else if p > zero && q == zero {
                bridge.churn = bridge.churn.add(p)?;
                bridge.churned_logos += 1;
                (Category::Churn, p)
            } else if p > zero && q > p {
                let delta = q.sub(p)?;
                bridge.expansion = bridge.expansion.add(delta)?;
                (Category::Expansion, delta)
            } else if p > zero && q < p {
                let delta = p.sub(q)?;
                bridge.contraction = bridge.contraction.add(delta)?;
                (Category::Contraction, delta)
            } else {
                continue;
            };
            moves.push((row, category, amount));
        }
        let change = self.total.get(index).sub(self.total.get(index - 1))?;
        bridge.other = change.sub(bridge.net_categories()?)?;
        Ok((bridge, moves))
    }

    /// The bridge summed over `[first, last]`.
    fn bridge_over(&self, first: i64, last: i64) -> Result<Bridge, ReportError> {
        (first..=last).try_fold(Bridge::zero(), |acc, index| acc.add(&self.bridge(index)?.0))
    }

    /// `(NRR %, GRR %)` of month `index` against `index − 12`.
    fn retention(&self, index: i64) -> Result<(Option<f64>, Option<f64>), ReportError> {
        let mut base = Dec::zero();
        let mut kept = Dec::zero();
        let mut gross = Dec::zero();
        for row in 0..self.rows.len() {
            let then = self.at(row, index - 12);
            if then <= Dec::zero() {
                continue;
            }
            let now = positive(self.at(row, index));
            base = base.add(then)?;
            kept = kept.add(now)?;
            gross = gross.add(now.min(then))?;
        }
        Ok((pct(kept, base), pct(gross, base)))
    }
}

/// One customer's move in one month: its row, category and amount.
type Move = (usize, Category, Dec);

/// Billings per customer, valued, with presence and payee flags per month.
struct Billings {
    lo: i64,
    /// Valued (positive) billings per month offset.
    values: Vec<Vec<Dec>>,
    /// Month offsets with a billing, per customer.
    billed: Vec<BTreeSet<usize>>,
    /// Month offsets whose billing was found by payee, per customer.
    inferred: Vec<BTreeSet<usize>>,
}

impl Billings {
    fn offset(&self, index: i64) -> Option<usize> {
        usize::try_from(index - self.lo).ok()
    }

    /// Offsets of the 12 months ending at `m` (those inside the walked range).
    fn window(&self, m: i64) -> std::ops::RangeInclusive<usize> {
        let first = self.offset(m - 11).unwrap_or(0);
        let last = self.offset(m).unwrap_or(0);
        first..=last
    }

    fn active(&self, row: usize, m: i64) -> bool {
        self.offset(m).is_some() && self.billed[row].range(self.window(m)).next().is_some()
    }

    fn count(&self, m: i64) -> u32 {
        let count = (0..self.billed.len())
            .filter(|&row| self.active(row, m))
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    fn is_empty(&self) -> bool {
        self.billed.iter().all(BTreeSet::is_empty)
    }

    /// The half's figures: `a…m` its complete months.
    fn period(
        &self,
        a: i64,
        m: i64,
        journal_first: Option<i64>,
    ) -> Result<BillingCustomers, ReportError> {
        let rows = 0..self.billed.len();
        let customers = self.count(m);
        let before = self.count(a - 1);
        let new_logos = rows
            .clone()
            .filter(|&row| {
                self.billed[row]
                    .first()
                    .is_some_and(|&first| (a..=m).contains(&(self.lo + first as i64)))
            })
            .count();
        let lapsed_logos: usize = (a..=m)
            .map(|j| {
                rows.clone()
                    .filter(|&row| self.active(row, j - 1) && !self.active(row, j))
                    .count()
            })
            .sum();
        let lapsed_logos = u32::try_from(lapsed_logos).unwrap_or(u32::MAX);
        let window = self.window(m);
        let mut trailing = Dec::zero();
        let mut inferred = false;
        for row in rows {
            if !self.active(row, m) {
                continue;
            }
            for offset in window.clone() {
                if let Some(value) = self.values[row].get(offset) {
                    trailing = trailing.add(*value)?;
                }
            }
            inferred |= self.inferred[row].range(window.clone()).next().is_some();
        }
        Ok(BillingCustomers {
            customers,
            customer_growth_pct: (before > 0)
                .then(|| (f64::from(customers) / f64::from(before) - 1.0) * 100.0),
            new_logos: u32::try_from(new_logos).unwrap_or(u32::MAX),
            lapsed_logos,
            logo_churn_pct: (before > 0)
                .then(|| f64::from(lapsed_logos) / f64::from(before) * 100.0),
            acv: mean(trailing, customers)?,
            window_short: journal_first.is_none_or(|first| first > m - 11),
            inferred,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category {
    New,
    Expansion,
    Contraction,
    Churn,
    Reactivation,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Everything derived, shared by the period and segment computations.
struct Derived {
    as_of: Option<String>,
    journal_first: Option<i64>,
    has_recurring: bool,
    customers_available: bool,
    roles: BTreeSet<KpiRole>,
    cogs_available: bool,
    mrr: Series,
    usage: Series,
    revenue: Series,
    cogs: Series,
    expenses: Series,
    sm: Series,
    rnd: Series,
    ga: Series,
    cash: Series,
    deferred: Series,
    ar: Series,
    ap: Series,
    net_burn: Series,
    /// Plan 37 A1.1: `None` without a deferred-revenue or receivable role.
    net_burn_adjusted: Option<Series>,
    /// Plan 37 A1.2: `Some` when counts come from billings.
    billings: Option<Billings>,
    matrix: Matrix,
}

/// Compute the startup metrics over `opts`'s span.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math.
#[allow(clippy::too_many_lines)]
pub fn saas_metrics(journal: &Journal, opts: &SaasOpts) -> Result<SaasReport, ReportError> {
    let period = split_period(opts.start, opts.end)?;
    let prices = all_prices(journal)?;
    let db = PriceDb::build(&prices);
    let base = db
        .base_commodity()
        .cloned()
        .unwrap_or_else(|| Commodity("$".to_string()));

    let decls = account_decls(journal);
    let types = AccountTypes::from_declared(declared_types(&decls));
    let kpi_declared = declared_kpi_roles(&journal.accounts);
    let kpi = kpi_resolver(&kpi_declared, &types);
    let sections = account_sections_from(&journal.accounts);
    let section = section_resolver(&sections, &types);
    let dim_declared = declared_dimension_tags(&journal.accounts, opts.dims);

    // The walked range: from the earliest posting (or a year before the span,
    // for the trailing metrics) to the span's end, capped at MAX_MONTHS.
    let span_first = month_index(opts.start);
    let hi = month_index(opts.end);
    let earliest = journal
        .transactions
        .iter()
        .flat_map(|txn| txn.postings.iter().map(move |p| effective_date(txn, p)))
        .min()
        .map_or(span_first, month_index);
    let lo = earliest.min(span_first - 13).max(hi - MAX_MONTHS + 1);
    let n = usize::try_from(hi - lo + 1).unwrap_or(0);
    let journal_first = journal
        .transactions
        .iter()
        .map(|txn| txn.date.as_str())
        .min()
        .map(month_index);

    let mut seen_roles: BTreeSet<KpiRole> = kpi_declared.values().copied().collect();
    let mut cogs_available = sections.values().any(|kind| *kind == IsSectionKind::Cogs);
    let mut info = |account: &str| {
        let role = kpi(account);
        let cogs = section(account) == Some(IsSectionKind::Cogs);
        if let Some(role) = role {
            seen_roles.insert(role);
        }
        cogs_available |= cogs;
        AccountInfo {
            revenue: types.is_type(account, AccountType::Revenue),
            expense: types.is_type(account, AccountType::Expense),
            cash: types.is_cash(account),
            cogs,
            role,
            dims: opts
                .dims
                .iter()
                .map(|key| nearest_dimension(&dim_declared, account, key).map(str::to_string))
                .collect(),
        }
    };
    // Plan 37 A2: the data horizon. The last month holding any recurring
    // posting is the last month the books are closed through; later months
    // (the recognition entry isn't in yet) are open and never "complete".
    // With no recurring revenue there is no horizon.
    let horizon = recurring_horizon(journal, opts.end, &kpi);
    let as_of_limit: Option<String> = match (opts.as_of, horizon) {
        (as_of, Some(h)) => {
            let closed = month_last(h)?;
            Some(as_of.map_or(closed.clone(), |as_of| closed.min(as_of.to_string())))
        }
        (as_of, None) => as_of.map(str::to_string),
    };
    let as_of = as_of_limit.as_deref();
    let span_complete = complete_months(opts.start, opts.end, as_of)?;
    let acc = accumulate_journal(
        journal,
        opts.end,
        lo,
        n,
        opts.dims,
        span_complete,
        &mut info,
    )?;

    let valuer = Valuer {
        base: &base,
        db: &db,
        ends: (0..n)
            .map(|i| month_last(lo + i as i64))
            .collect::<Result<_, _>>()?,
    };

    // Unvalued recurring/usage revenue over the span's months.
    let mut unvalued = MixedAmount::new();
    let span_offsets = || (span_first.max(lo) - lo)..(hi - lo + 1);
    for offset in span_offsets() {
        let i = usize::try_from(offset).unwrap_or(0);
        valuer.value(&acc.mrr[i], i, true, Some(&mut unvalued))?;
        valuer.value(&acc.usage[i], i, true, Some(&mut unvalued))?;
    }
    unvalued.drop_zeros();

    let mrr = valuer.flow(lo, &acc.mrr, true)?;
    let unattributed = valuer.flow(lo, &acc.unattributed, true)?;

    let customer_keys: Vec<String> = acc.customers.keys().cloned().collect();
    let customer_rows: Vec<Vec<Dec>> = acc
        .customers
        .values()
        .map(|customer| Ok(valuer.sparse(lo, n, &customer.months)?.values))
        .collect::<Result<_, ReportError>>()?;
    let matrix = Matrix::new(lo, customer_keys, customer_rows, mrr.clone());

    let net_burn = net_burn_series(journal, opts.end, lo, hi, &valuer)?;

    // Visibility.
    let has_recurring_role = seen_roles.contains(&KpiRole::Recurring);
    let recurring = has_recurring_role
        && span_complete.is_some_and(|(a, b)| (a..=b).any(|index| mrr.get(index) > Dec::zero()));
    let (mrr_total, unattributed_total) = match span_complete {
        Some((a, b)) => (
            sum((a..=b).map(|i| mrr.get(i)).collect::<Vec<_>>().iter())?,
            sum((a..=b)
                .map(|i| unattributed.get(i))
                .collect::<Vec<_>>()
                .iter())?,
        ),
        None => (Dec::zero(), Dec::zero()),
    };
    let coverage = ratio(mrr_total.sub(unattributed_total)?, mrr_total)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let clearing = billing_clearing(&acc, &valuer, span_complete)?;
    let hidden = if !recurring {
        None
    } else if clearing {
        Some(HiddenReason::BillingClearing)
    } else if coverage <= 0.0 {
        Some(HiddenReason::NoCustomerTags)
    } else if coverage < opts.coverage_min.floating_point() {
        Some(HiddenReason::LowCoverage)
    } else {
        None
    };
    let customers_available = recurring && hidden.is_none();

    // Plan 37 A1.2: billings, when the MRR customer metrics are hidden.
    let billings = Billings {
        lo,
        values: acc
            .billings
            .values()
            .map(|customer| Ok(valuer.sparse(lo, n, &customer.months)?.values))
            .collect::<Result<_, ReportError>>()?,
        billed: acc
            .billings
            .values()
            .map(|customer| customer.months.keys().copied().collect())
            .collect(),
        inferred: acc
            .billings
            .values()
            .map(|customer| customer.inferred_months.clone())
            .collect(),
    };
    let billing_customers = recurring && !customers_available && !billings.is_empty();

    // Plan 37 A1.1: burn adjusted for prepayments and receivables.
    let deferred = valuer.balance(lo, &acc.deferred, true)?;
    let ar = valuer.balance(lo, &acc.ar, false)?;
    let net_burn_adjusted = (seen_roles.contains(&KpiRole::DeferredRevenue)
        || seen_roles.contains(&KpiRole::Receivable))
    .then(|| -> Result<Series, ReportError> {
        let values = (lo..=hi)
            .map(|index| {
                let deferred_change = deferred.get(index).sub(deferred.get(index - 1))?;
                let ar_change = ar.get(index).sub(ar.get(index - 1))?;
                Ok(net_burn.get(index).add(deferred_change)?.sub(ar_change)?)
            })
            .collect::<Result<_, ReportError>>()?;
        Ok(Series { lo, values })
    })
    .transpose()?;

    let derived = Derived {
        as_of: as_of_limit.clone(),
        journal_first,
        has_recurring: recurring,
        customers_available,
        roles: seen_roles.clone(),
        cogs_available,
        usage: valuer.flow(lo, &acc.usage, true)?,
        revenue: valuer.flow(lo, &acc.revenue, true)?,
        cogs: valuer.flow(lo, &acc.cogs, false)?,
        expenses: valuer.flow(lo, &acc.expenses, false)?,
        sm: valuer.flow(lo, &acc.sm, false)?,
        rnd: valuer.flow(lo, &acc.rnd, false)?,
        ga: valuer.flow(lo, &acc.ga, false)?,
        cash: valuer.balance(lo, &acc.cash, false)?,
        deferred,
        ar,
        ap: valuer.balance(lo, &acc.ap, true)?,
        net_burn,
        net_burn_adjusted,
        billings: billing_customers.then_some(billings),
        mrr,
        matrix,
    };

    let current = period_metrics(&derived, &period.curr_start, &period.curr_end)?;
    let previous = period_metrics(&derived, &period.prev_start, &period.prev_end)?;
    let current_months = complete_months(&period.curr_start, &period.curr_end, as_of)?;

    // Inferred share of the current half's last complete month.
    let inferred_customers = customers_available
        && current_months.is_some_and(|(_, m)| {
            usize::try_from(m - lo).is_ok_and(|i| {
                acc.customers.values().any(|customer| {
                    customer
                        .inferred_months
                        .get(&i)
                        .is_some_and(|ma| !ma.is_zero())
                })
            })
        });

    // Dimensions offered: ≥ 2 distinct assigned values carrying recurring
    // revenue in the span.
    let mut dim_series: Vec<(String, Vec<(String, Matrix)>)> = Vec::new();
    for (key, values) in opts.dims.iter().zip(&acc.dims) {
        let mut rows = Vec::new();
        for (value, dim) in values {
            let total = valuer.sparse(lo, n, &dim.mrr)?;
            let carries = (span_first..=hi).any(|index| !total.get(index).is_zero());
            if !carries {
                continue;
            }
            let keys: Vec<String> = dim.customers.keys().cloned().collect();
            let customer_rows = dim
                .customers
                .values()
                .map(|months| Ok(valuer.sparse(lo, n, months)?.values))
                .collect::<Result<_, ReportError>>()?;
            rows.push((value.clone(), Matrix::new(lo, keys, customer_rows, total)));
        }
        let assigned = rows.iter().filter(|(value, _)| value != UNASSIGNED).count();
        if recurring && assigned >= 2 {
            dim_series.push((key.clone(), rows));
        }
    }
    let dimensions: Vec<String> = dim_series.iter().map(|(key, _)| key.clone()).collect();

    let segments = dim_series
        .iter()
        .map(|(key, rows)| segment_breakdown(key, rows, &derived, &period))
        .collect::<Result<Vec<_>, ReportError>>()?;

    let names: Vec<String> = acc
        .customers
        .values()
        .map(|customer| customer.name.0.clone())
        .collect();
    // Was this customer's revenue in month `offset` attributed by payee?
    let inferred_at = |row: usize, offset: usize| -> bool {
        acc.customers
            .values()
            .nth(row)
            .and_then(|customer| customer.inferred_months.get(&offset))
            .is_some_and(|ma| !ma.is_zero())
    };
    let segment_at = |row: usize, offset: usize| -> Option<String> {
        acc.customers
            .values()
            .nth(row)
            .and_then(|customer| customer.segment.range(..=offset).next_back())
            .map(|(_, value)| value.clone())
    };

    let bridge_customers = match (customers_available, current_months) {
        (true, Some((a, b))) => Some(bridge_customers(
            &derived.matrix,
            a,
            b,
            opts.top_n,
            &names,
            &segment_at,
            &inferred_at,
        )?),
        _ => None,
    };

    let top_customers = match (customers_available, current_months) {
        (true, Some((_, m))) => top_customers(&derived.matrix, m, &names)?,
        _ => Vec::new(),
    };

    // Months of the span.
    let first_month = span_first.max(lo);
    let mut months = Vec::with_capacity(usize::try_from(hi - first_month + 1).unwrap_or(0));
    let start_first = month_first(span_first);
    let end_last = month_last(hi)?;
    for index in first_month..=hi {
        // Also partial: a month that hasn't finished by `as_of`, or that is
        // after the data horizon (open: the books aren't closed yet).
        let open = horizon.is_some_and(|h| index > h)
            && opts.as_of.map_or(opts.end, |as_of| as_of.min(opts.end))
                >= month_first(index).as_str();
        let unfinished = open
            || as_of.is_some_and(|as_of| {
                usize::try_from(index - lo)
                    .ok()
                    .and_then(|i| valuer.ends.get(i))
                    .is_some_and(|last| last.as_str() > as_of)
            });
        let partial = (index == span_first && opts.start != start_first)
            || (index == hi && opts.end != end_last)
            || unfinished;
        let (customers, bridge) = if customers_available {
            (
                Some(derived.matrix.customers(index)),
                Some(derived.matrix.bridge(index)?.0),
            )
        } else {
            (None, None)
        };
        months.push(SaasMonth {
            month: month_key(index),
            partial,
            open,
            mrr: derived.mrr.get(index),
            usage: derived.usage.get(index),
            unattributed: unattributed.get(index),
            customers,
            bridge,
            revenue: derived.revenue.get(index),
            cogs: cogs_available.then(|| derived.cogs.get(index)),
            net_burn: derived.net_burn.get(index),
            net_burn_adjusted: derived
                .net_burn_adjusted
                .as_ref()
                .map(|series| series.get(index)),
            cash_end: derived.cash.get(index),
            sm: derived.sm.get(index),
            rnd: derived.rnd.get(index),
            ga: derived.ga.get(index),
            deferred_end: seen_roles
                .contains(&KpiRole::DeferredRevenue)
                .then(|| derived.deferred.get(index)),
            ar_end: seen_roles
                .contains(&KpiRole::Receivable)
                .then(|| derived.ar.get(index)),
            ap_end: seen_roles
                .contains(&KpiRole::Payable)
                .then(|| derived.ap.get(index)),
        });
    }

    let customer_matrix = (opts.with_customer_matrix && customers_available).then(|| {
        derived
            .matrix
            .rows
            .iter()
            .enumerate()
            .map(|(row, _)| {
                let latest_positive = (first_month..=hi)
                    .rev()
                    .find(|&index| derived.matrix.at(row, index) > Dec::zero())
                    .and_then(|index| usize::try_from(index - lo).ok());
                CustomerRow {
                    customer: derived.matrix.keys[row].clone(),
                    name: names[row].clone(),
                    segment: latest_positive.and_then(|offset| segment_at(row, offset)),
                    values: (first_month..=hi)
                        .map(|index| derived.matrix.at(row, index))
                        .collect(),
                }
            })
            .collect()
    });

    Ok(SaasReport {
        period,
        base: base.0.clone(),
        available: SaasAvailability {
            recurring,
            customers: customers_available,
            business: any_kpi_declared(&journal.accounts),
            cogs: cogs_available,
            dimensions,
            customers_hidden_reason: hidden,
            coverage_pct: recurring.then_some(coverage * 100.0),
            inferred_customers: inferred_customers
                || current
                    .billing_customers
                    .as_ref()
                    .is_some_and(|billed| billed.inferred),
            billing_customers,
            closed_through: horizon.filter(|_| recurring).map(month_key),
        },
        months,
        current,
        previous,
        segments,
        bridge_customers,
        top_customers,
        customer_matrix,
        unvalued,
    })
}

/// `NetBurn(month) = −(Δcash − Financing)` for every month in `[lo, hi]`, from
/// the statement of cash flows (its own cash and financing classification).
/// Because operating + investing + financing (+ the exchange-rate effect) =
/// Δcash in the statement, this is −(operating + investing + exchange).
fn net_burn_series(
    journal: &Journal,
    end: &str,
    lo: i64,
    hi: i64,
    valuer: &Valuer,
) -> Result<Series, ReportError> {
    let statement = cash_flow_statement(
        &journal.transactions,
        &CfOpts {
            from: &month_first(lo),
            to: end,
            interval: Some(Interval::Monthly),
            depth: 1,
        },
        &CfTags::from_journal(journal),
    )?;
    let financing = statement
        .sections
        .iter()
        .find(|section| section.kind == CfSectionKind::Financing);
    let n = usize::try_from(hi - lo + 1).unwrap_or(0);
    let mut values = vec![Dec::zero(); n];
    for (i, (bucket, change)) in statement
        .buckets
        .iter()
        .zip(&statement.net_change.values)
        .enumerate()
    {
        let offset = usize::try_from(month_index(&bucket.from) - lo).unwrap_or(i);
        if offset >= n {
            continue;
        }
        let financed = financing
            .and_then(|section| section.total.values.get(i))
            .cloned()
            .unwrap_or_default();
        let operating_investing = change.ma_add(&financed.ma_neg()?)?;
        values[offset] = valuer.value(&operating_investing, offset, true, None)?;
    }
    Ok(Series { lo, values })
}

/// The billing-clearing pattern: more than half of the attributed recurring
/// revenue in the span's complete months sits on clearing postings
/// ([`ClearingRule`]).
fn billing_clearing(
    acc: &Accumulators,
    valuer: &Valuer,
    span_complete: Option<(i64, i64)>,
) -> Result<bool, ReportError> {
    let Some((a, b)) = span_complete else {
        return Ok(false);
    };
    let mut on_clearing = Dec::zero();
    let mut everywhere = Dec::zero();
    for index in a..=b {
        let Some(i) = usize::try_from(index - acc.lo).ok().filter(|&i| i < acc.n) else {
            continue;
        };
        everywhere = everywhere.add(valuer.value(&acc.attributed[i], i, true, None)?)?;
        on_clearing =
            on_clearing.add(valuer.value(&acc.attributed_clearing[i], i, true, None)?)?;
    }
    Ok(everywhere > Dec::zero() && on_clearing.add(on_clearing)? > everywhere)
}

/// The metrics for the half `[from, to]`.
#[allow(clippy::too_many_lines)]
fn period_metrics(d: &Derived, from: &str, to: &str) -> Result<SaasPeriodMetrics, ReportError> {
    let complete = complete_months(from, to, d.as_of.as_deref())?;
    let Some((a, m)) = complete else {
        return Ok(empty_period());
    };
    let k = m - a + 1;
    let before = a - 1;
    let months = u32::try_from(k).unwrap_or(u32::MAX);
    let over = |series: &Series| sum((a..=m).map(|i| series.get(i)).collect::<Vec<_>>().iter());
    let days = u32::try_from(days_between(&month_first(a), &month_last(m)?) + 1).unwrap_or(0);
    let per_days = |value: Option<f64>| value.map(|r| r * f64::from(days));

    let tier_b = d.has_recurring;
    let tier_c = d.customers_available;
    let has = |role: KpiRole| d.roles.contains(&role);

    let mrr_m = d.mrr.get(m);
    let mrr_before = d.mrr.get(before);
    let twelve = Dec::new(12, 0);
    let net_new_arr = mrr_m.sub(mrr_before)?.mul(twelve)?;

    let revenue = over(&d.revenue)?;
    let cogs = over(&d.cogs)?;
    let expenses = over(&d.expenses)?;
    let net_burn = over(&d.net_burn)?;
    let gross_profit = d.cogs_available.then(|| revenue.sub(cogs)).transpose()?;
    let gross_margin_pct = gross_profit.and_then(|gp| pct(gp, revenue));

    // 3-month average burn, fewer when the journal is younger.
    let avg_first = d.journal_first.map_or(m - 2, |first| (m - 2).max(first));
    let avg_count = u32::try_from((m - avg_first + 1).max(0)).unwrap_or(0);
    let burn3 = sum((avg_first..=m)
        .map(|i| d.net_burn.get(i))
        .collect::<Vec<_>>()
        .iter())?;
    let net_burn_avg3 = mean(burn3, avg_count)?;
    // Plan 37 A1.1: runway uses the adjusted burn whenever it exists.
    let adjusted3 = d
        .net_burn_adjusted
        .as_ref()
        .map(|series| {
            sum((avg_first..=m)
                .map(|i| series.get(i))
                .collect::<Vec<_>>()
                .iter())
        })
        .transpose()?;
    let net_burn_adjusted_avg3 = adjusted3
        .map(|total| mean(total, avg_count))
        .transpose()?
        .flatten();
    let (runway_basis, runway_burn) = match adjusted3 {
        Some(total) => (RunwayBasis::Adjusted, total),
        None => (RunwayBasis::Cash, burn3),
    };
    let cash = d.cash.get(m);
    // Runway from the exact 3-month sum, not the rounded average.
    // Out of cash already reads as zero months, never negative.
    let runway_months = (runway_burn > Dec::zero()).then(|| {
        positive(cash).floating_point() * f64::from(avg_count) / runway_burn.floating_point()
    });
    let cash_out_date = runway_months
        .filter(|r| r.is_finite() && *r >= 0.0 && *r <= MAX_RUNWAY_MONTHS)
        .map(|r| cash_out(m, r))
        .transpose()?;

    // Plan 37 A3: burn multiple on the adjusted burn when it exists.
    let net_burn_adjusted = d.net_burn_adjusted.as_ref().map(over).transpose()?;
    let (multiple_basis, multiple_burn) = match net_burn_adjusted {
        Some(total) => (RunwayBasis::Adjusted, total),
        None => (RunwayBasis::Cash, net_burn),
    };
    let burn_multiple = (tier_b && multiple_burn > Dec::zero() && net_new_arr > Dec::zero())
        .then(|| multiple_burn.floating_point() / net_new_arr.floating_point());
    let burn_multiple_basis = burn_multiple.map(|_| multiple_basis);

    let prior_revenue = sum((a - k..a)
        .map(|i| d.revenue.get(i))
        .collect::<Vec<_>>()
        .iter())?;
    let rule_of_40 = match (pct(revenue, prior_revenue), pct(net_burn.neg()?, revenue)) {
        (Some(growth), Some(fcf_margin)) => Some(growth - 100.0 + fcf_margin),
        _ => None,
    };

    let bridge = tier_c.then(|| d.matrix.bridge_over(a, m)).transpose()?;
    let customers_m = tier_c.then(|| d.matrix.customers(m));
    let customers_before = d.matrix.customers(before);
    let (nrr_pct, grr_pct) = if tier_c && d.journal_first.is_some_and(|first| m - 12 >= first) {
        d.matrix.retention(m)?
    } else {
        (None, None)
    };
    let attributed_m = d.matrix.attributed_positive(m)?;
    let arpa = match customers_m {
        Some(count) => mean(attributed_m, count)?,
        None => None,
    };
    let (acv, quick_ratio, logo_churn_pct) = match &bridge {
        Some(b) => (
            mean(b.new.mul(twelve)?, b.new_logos)?,
            ratio(
                b.new.add(b.expansion)?.add(b.reactivation)?,
                b.contraction.add(b.churn)?,
            ),
            (customers_before > 0)
                .then(|| f64::from(b.churned_logos) / f64::from(customers_before) * 100.0),
        ),
        None => (None, None, None),
    };
    let customer_growth_pct = customers_m.and_then(|count| {
        (customers_before > 0)
            .then(|| (f64::from(count) / f64::from(customers_before) - 1.0) * 100.0)
    });
    let (top1_pct, top5_pct, top10_pct) = if tier_c {
        let mut shares: Vec<Dec> = (0..d.matrix.rows.len())
            .map(|row| positive(d.matrix.at(row, m)))
            .filter(|value| *value > Dec::zero())
            .collect();
        shares.sort_by(|x, y| y.cmp(x));
        let top = |count: usize| -> Result<Option<f64>, ReportError> {
            Ok(pct(sum(shares.iter().take(count))?, mrr_m))
        };
        (top(1)?, top(5)?, top(10)?)
    } else {
        (None, None, None)
    };

    let spend = |role: KpiRole, series: &Series| -> Result<Option<Dec>, ReportError> {
        has(role).then(|| over(series)).transpose()
    };
    let sm = spend(KpiRole::SalesMarketing, &d.sm)?;
    let rnd = spend(KpiRole::Rnd, &d.rnd)?;
    let ga = spend(KpiRole::Ga, &d.ga)?;
    let cac = match (sm, &bridge) {
        (Some(sm), Some(b)) => mean(sm, b.new_logos)?,
        _ => None,
    };
    let cac_payback_months = match (sm, &bridge, gross_margin_pct) {
        (Some(sm), Some(b), Some(gm)) => {
            let monthly_gain = b.new.add(b.expansion)?.floating_point() / k as f64 * gm / 100.0;
            (monthly_gain > 0.0).then(|| sm.floating_point() / monthly_gain)
        }
        _ => None,
    };
    let deferred_revenue = has(KpiRole::DeferredRevenue).then(|| d.deferred.get(m));
    let billings = deferred_revenue
        .map(|end| revenue.add(end)?.sub(d.deferred.get(before)))
        .transpose()?;
    let dso_days = has(KpiRole::Receivable)
        .then(|| per_days(ratio(d.ar.get(m), revenue)))
        .flatten();
    let dpo_days = has(KpiRole::Payable)
        .then(|| per_days(ratio(d.ap.get(m), expenses)))
        .flatten();

    Ok(SaasPeriodMetrics {
        month: Some(month_key(m)),
        months,
        mrr: tier_b.then_some(mrr_m),
        arr: tier_b.then(|| mrr_m.mul(twelve)).transpose()?,
        usage: tier_b.then(|| d.usage.get(m)),
        mrr_growth_pct: tier_b
            .then(|| pct(mrr_m.sub(mrr_before).ok()?, mrr_before))
            .flatten(),
        cmgr_pct: (tier_b && mrr_m > Dec::zero() && mrr_before > Dec::zero()).then(|| {
            ((mrr_m.floating_point() / mrr_before.floating_point()).powf(1.0 / k as f64) - 1.0)
                * 100.0
        }),
        net_new_arr: tier_b.then_some(net_new_arr),
        bridge,
        customers: customers_m,
        customer_growth_pct,
        logo_churn_pct,
        arpa,
        acv,
        nrr_pct,
        grr_pct,
        quick_ratio,
        top1_pct,
        top5_pct,
        top10_pct,
        revenue,
        gross_profit,
        gross_margin_pct,
        net_burn,
        net_burn_adjusted,
        net_burn_avg3,
        net_burn_adjusted_avg3,
        runway_basis: Some(runway_basis),
        cash: Some(cash),
        runway_months,
        cash_out_date,
        burn_multiple,
        burn_multiple_basis,
        rule_of_40,
        sm,
        rnd,
        ga,
        sm_pct: sm.and_then(|value| pct(value, revenue)),
        rnd_pct: rnd.and_then(|value| pct(value, revenue)),
        ga_pct: ga.and_then(|value| pct(value, revenue)),
        cac,
        cac_payback_months,
        deferred_revenue,
        billings,
        dso_days,
        dpo_days,
        billing_customers: d
            .billings
            .as_ref()
            .map(|billings| billings.period(a, m, d.journal_first))
            .transpose()?,
    })
}

/// A half with no complete month: every point metric `None`, flows zero.
fn empty_period() -> SaasPeriodMetrics {
    SaasPeriodMetrics {
        month: None,
        months: 0,
        mrr: None,
        arr: None,
        usage: None,
        mrr_growth_pct: None,
        cmgr_pct: None,
        net_new_arr: None,
        bridge: None,
        customers: None,
        customer_growth_pct: None,
        logo_churn_pct: None,
        arpa: None,
        acv: None,
        nrr_pct: None,
        grr_pct: None,
        quick_ratio: None,
        top1_pct: None,
        top5_pct: None,
        top10_pct: None,
        revenue: Dec::zero(),
        gross_profit: None,
        gross_margin_pct: None,
        net_burn: Dec::zero(),
        net_burn_adjusted: None,
        net_burn_avg3: None,
        net_burn_adjusted_avg3: None,
        runway_basis: None,
        cash: None,
        runway_months: None,
        cash_out_date: None,
        burn_multiple: None,
        burn_multiple_basis: None,
        rule_of_40: None,
        sm: None,
        rnd: None,
        ga: None,
        sm_pct: None,
        rnd_pct: None,
        ga_pct: None,
        cac: None,
        cac_payback_months: None,
        deferred_revenue: None,
        billings: None,
        dso_days: None,
        dpo_days: None,
        billing_customers: None,
    }
}

/// The end of month `m` plus `runway` months, to the day: whole months land
/// on that month's last day, and the fraction is that share of the next
/// month's days, rounded.
fn cash_out(m: i64, runway: f64) -> Result<String, ReportError> {
    let whole = runway.floor();
    #[allow(clippy::cast_possible_truncation)]
    let target = m + whole as i64;
    let last = month_last(target)?;
    let next_len = days_between(&month_first(target + 1), &month_last(target + 1)?) + 1;
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let extra = ((runway - whole) * next_len as f64).round() as i64;
    Ok(add_days(&last, extra))
}

/// One dimension key's rows.
fn segment_breakdown(
    key: &str,
    rows: &[(String, Matrix)],
    d: &Derived,
    period: &InsightsPeriod,
) -> Result<SegmentBreakdown, ReportError> {
    let as_of = d.as_of.as_deref();
    let current = complete_months(&period.curr_start, &period.curr_end, as_of)?;
    let previous = complete_months(&period.prev_start, &period.prev_end, as_of)?;
    let tier_c = d.customers_available;
    let mut out = rows
        .iter()
        .map(|(value, matrix)| {
            let m = current.map(|(_, m)| m);
            let customers_curr = m.filter(|_| tier_c).map(|m| matrix.customers(m));
            let arpa = match (m, customers_curr) {
                (Some(m), Some(count)) => mean(matrix.attributed_positive(m)?, count)?,
                _ => None,
            };
            let nrr_pct = match m {
                Some(m) if tier_c && d.journal_first.is_some_and(|first| m - 12 >= first) => {
                    matrix.retention(m)?.0
                }
                _ => None,
            };
            let bridge = match current {
                Some((a, b)) if tier_c => Some(matrix.bridge_over(a, b)?),
                _ => None,
            };
            Ok(SegmentRow {
                value: value.clone(),
                mrr_curr: m.map_or_else(Dec::zero, |m| matrix.total.get(m)),
                mrr_prev: previous.map_or_else(Dec::zero, |(_, m)| matrix.total.get(m)),
                customers_curr,
                arpa,
                nrr_pct,
                bridge,
            })
        })
        .collect::<Result<Vec<_>, ReportError>>()?;
    out.sort_by(|x, y| {
        (x.value == UNASSIGNED)
            .cmp(&(y.value == UNASSIGNED))
            .then_with(|| y.mrr_curr.cmp(&x.mrr_curr))
            .then_with(|| x.value.cmp(&y.value))
    });
    Ok(SegmentBreakdown {
        key: key.to_string(),
        rows: out,
    })
}

/// The current half's top customers per category, summed over its months.
fn bridge_customers(
    matrix: &Matrix,
    first: i64,
    last: i64,
    top_n: usize,
    names: &[String],
    segment_at: &dyn Fn(usize, usize) -> Option<String>,
    inferred_at: &dyn Fn(usize, usize) -> bool,
) -> Result<BridgeCustomers, ReportError> {
    // (amount, last month, inferred in any month it was measured on).
    let mut totals: BTreeMap<(Category, usize), (Dec, i64, bool)> = BTreeMap::new();
    for index in first..=last {
        for (row, category, amount) in matrix.bridge(index)?.1 {
            // A move is measured on q's month, a churn on p's.
            let measured = if category == Category::Churn {
                index - 1
            } else {
                index
            };
            let inferred =
                usize::try_from(measured - matrix.lo).is_ok_and(|offset| inferred_at(row, offset));
            let entry = totals
                .entry((category, row))
                .or_insert((Dec::zero(), index, false));
            entry.0 = entry.0.add(amount)?;
            entry.1 = index;
            entry.2 |= inferred;
        }
    }
    let list = |wanted: Category| -> Vec<CustomerMove> {
        let mut moves: Vec<CustomerMove> = totals
            .iter()
            .filter(|((category, _), _)| *category == wanted)
            .map(|((category, row), (amount, index, inferred))| {
                // A churn's segment is the one the revenue left from.
                let at = if *category == Category::Churn {
                    index - 1
                } else {
                    *index
                };
                CustomerMove {
                    customer: matrix.keys[*row].clone(),
                    name: names[*row].clone(),
                    segment: usize::try_from(at - matrix.lo)
                        .ok()
                        .and_then(|offset| segment_at(*row, offset)),
                    amount: *amount,
                    inferred: *inferred,
                }
            })
            .collect();
        moves.sort_by(|x, y| y.amount.cmp(&x.amount).then_with(|| x.name.cmp(&y.name)));
        moves.truncate(top_n);
        moves
    };
    Ok(BridgeCustomers {
        new: list(Category::New),
        expansion: list(Category::Expansion),
        contraction: list(Category::Contraction),
        churn: list(Category::Churn),
        reactivation: list(Category::Reactivation),
    })
}

/// The largest customers at month `m`, by positive MRR.
fn top_customers(
    matrix: &Matrix,
    m: i64,
    names: &[String],
) -> Result<Vec<CustomerShare>, ReportError> {
    let total = matrix.total.get(m);
    let mut rows: Vec<CustomerShare> = (0..matrix.rows.len())
        .map(|row| (row, matrix.at(row, m)))
        .filter(|(_, value)| *value > Dec::zero())
        .map(|(row, value)| CustomerShare {
            customer: matrix.keys[row].clone(),
            name: names[row].clone(),
            mrr: value,
            pct: pct(value, total).unwrap_or(0.0),
        })
        .collect();
    rows.sort_by(|x, y| y.mrr.cmp(&x.mrr).then_with(|| x.name.cmp(&y.name)));
    rows.truncate(TOP_CUSTOMERS);
    Ok(rows)
}
