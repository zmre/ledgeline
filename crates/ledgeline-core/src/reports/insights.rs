//! Insights dashboard — period-over-period summary metrics.
//!
//! A single [`insights`] entry point composes the golden-validated report
//! engine (income statement, net worth, holdings, cash aggregation) into a
//! Monarch-style dashboard comparing a **current** period against the
//! **previous** one. The caller passes an inclusive `[start, end]` span; the
//! engine splits it down the middle (`mid`), so the previous period is
//! `[start, mid]` and the current period is `[mid + 1 day, end]`. For a span of
//! two whole years anchored on month boundaries this yields two clean 12-month
//! halves (the "Year-over-year" preset).
//!
//! No new money-math primitive is introduced: every metric reuses exact-decimal
//! [`MixedAmount`]/[`Dec`] building blocks. Percent changes are the only `f64`s,
//! computed at the display boundary on the report's base commodity (so the types
//! are `PartialEq` but not `Eq`).

use super::ReportError;
use super::account_types::{
    AccountType, account_decls, cash_predicate, declared_types, resolve_account_type,
};
use super::accounts::account_matches;
use super::aggregate::{PostingFilter, account_totals};
use super::income_statement::income_statement;
use super::mixed_amount::MixedAmount;
use super::net_worth::net_worth;
use super::periods::{Interval, add_days, bucket_end, bucket_key, days_between};
use super::prices::{PriceDb, infer_market_prices};
use crate::decimal::Dec;
use crate::holdings::{HoldingsReport, HoldingsScope, PriceSource, ScopeMode, compute_holdings};
use crate::model::{Commodity, Journal, Transaction};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// Number of rows shown in each "biggest / top" list box.
const TOP_N: usize = 5;

/// Inputs to [`insights`]. `start`/`end` are inclusive ISO dates.
#[derive(Debug, Clone)]
pub struct InsightsOpts<'a> {
    /// Inclusive start of the whole comparison span.
    pub start: &'a str,
    /// Inclusive end of the whole comparison span.
    pub end: &'a str,
    /// Account-name prefixes to exclude from the cost-of-living metric (matched
    /// as subtrees). Empty = all expenses count.
    pub cost_exclude: &'a [String],
    /// Minimum base-commodity magnitude for a leaf account to qualify as a
    /// "biggest change" (filters near-zero swings out of Boxes 7 & 9). Zero = no
    /// filter.
    pub change_min: Dec,
}

/// The resolved comparison window: the whole span plus its midpoint split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightsPeriod {
    /// Inclusive span start (= `previous` start).
    pub start: String,
    /// Split boundary: the inclusive last day of the previous period.
    pub mid: String,
    /// Inclusive span end (= `current` end).
    pub end: String,
    /// Previous period start (`= start`).
    pub prev_start: String,
    /// Previous period end (`= mid`).
    pub prev_end: String,
    /// Current period start (`= mid + 1 day`).
    pub curr_start: String,
    /// Current period end (`= end`).
    pub curr_end: String,
}

/// A metric's current + previous value with the exact change and a base-
/// commodity percent change.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricDelta {
    /// Value over the current period (or as of its end).
    pub current: MixedAmount,
    /// Value over the previous period (or as of its end).
    pub previous: MixedAmount,
    /// Exact `current − previous`.
    pub delta: MixedAmount,
    /// Percent change of the base commodity (`(cur − prev) / |prev| × 100`);
    /// `None` when the previous base value is absent or zero.
    pub pct: Option<f64>,
}

/// Average monthly cost of living: expenses (minus the exclusion list) over each
/// period, with the month counts needed to average. Averaging is a display-time
/// division, so it is deliberately left to the presentation layer to keep the
/// engine exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostOfLiving {
    /// Total non-excluded expenses over the current period.
    pub current_total: MixedAmount,
    /// Total non-excluded expenses over the previous period.
    pub previous_total: MixedAmount,
    /// Number of calendar months spanned by the current period.
    pub months_current: u32,
    /// Number of calendar months spanned by the previous period.
    pub months_previous: u32,
}

/// One period's portfolio performance (base commodity): the market-value change
/// over the period and its percent. Both are `None` for an unpriced/empty
/// portfolio.
#[derive(Debug, Clone, PartialEq)]
pub struct PerfPoint {
    /// `marketValue(end) − marketValue(start)` in the base commodity.
    pub gain: Option<Dec>,
    /// `gain / marketValue(start) × 100`.
    pub gain_pct: Option<f64>,
}

/// Investment performance for both periods (Box 5). Unlike the other boxes the
/// "small" figure is the previous period's own performance, not a difference.
#[derive(Debug, Clone, PartialEq)]
pub struct InvestmentPerf {
    /// Performance over the current period.
    pub current: PerfPoint,
    /// Performance over the previous period.
    pub previous: PerfPoint,
}

/// How a leaf account changed between the two periods.
///
/// Accounts with NO activity in the previous period are deliberately not
/// reported at all: with nothing to compare against, their percent change is
/// undefined, and ranking them alongside real comparisons (they used to sort
/// first) crowded genuine changes out of the list entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Both periods had activity.
    Changed,
    /// No activity in the current period (a −100% change).
    Ended,
}

/// One leaf-account change between the two periods (Boxes 7 & 9). Values are the
/// base commodity, sign-adjusted so an increase reads positive for both expense
/// and revenue lists.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeRow {
    /// The leaf account name.
    pub account: String,
    /// Current-period amount (base commodity, display-signed).
    pub current: Dec,
    /// Previous-period amount (base commodity, display-signed).
    pub previous: Dec,
    /// Exact `current − previous`.
    pub delta: Dec,
    /// Percent change against the previous period (always present — rows without
    /// a previous value are not reported).
    pub pct: Option<f64>,
    /// Changed vs. ended classification.
    pub kind: ChangeKind,
}

/// One stock's percent move over the current period (Box 8).
#[derive(Debug, Clone, PartialEq)]
pub struct MoverRow {
    /// The commodity symbol.
    pub symbol: String,
    /// The security's display name (`name:` tag), else the symbol.
    pub name: String,
    /// Windowed dollar gain over the current period (base commodity).
    pub gain: Option<Dec>,
    /// Windowed percent move over the current period.
    pub gain_pct: Option<f64>,
    /// True when the position's value at the START of the window had no market
    /// price directive and fell back to its purchase-cost annotation.
    ///
    /// The move is then measured from what the shares COST rather than what they
    /// were worth at the window start, so it degenerates toward the all-time gain
    /// since purchase. Journals without `P` price history before the window hit
    /// this for every holding; the UI flags it rather than silently presenting a
    /// lifetime gain as a period return.
    pub start_estimated: bool,
}

/// One of the largest transactions in the current period (Box 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopTxn {
    /// The transaction's 1-based journal index.
    pub index: u32,
    /// Transaction date (`YYYY-MM-DD`).
    pub date: String,
    /// The full description string.
    pub description: String,
    /// The base-commodity magnitude of money moved (one side's sum).
    pub amount: Dec,
}

/// The Insights dashboard's period-over-period core metrics (Boxes 1–6) plus the
/// list boxes (7–10).
#[derive(Debug, Clone, PartialEq)]
pub struct InsightsReport {
    /// The resolved comparison window.
    pub period: InsightsPeriod,
    /// Base commodity symbol used for percent changes and the headline figure.
    pub base: String,
    /// Date of the journal's earliest transaction, if any.
    ///
    /// Lets the UI tell an honest comparison from a misleading one: when the
    /// journal only starts partway through the previous period, that period's
    /// totals cover less time than the current one, so every delta overstates
    /// growth. A caller that ignores this shows a doubling that is really just
    /// six months of history compared against twelve.
    pub journal_start: Option<String>,
    /// Box 1: total revenue.
    pub revenue: MetricDelta,
    /// Box 2: total expenses.
    pub expenses: MetricDelta,
    /// Box 3: net worth at each period end.
    pub net_worth: MetricDelta,
    /// Box 4: average monthly cost of living.
    pub cost_of_living: CostOfLiving,
    /// Box 5: investment performance.
    pub investment: InvestmentPerf,
    /// Box 6: cash balance at each period end.
    pub cash_balance: MetricDelta,
    /// Box 7: biggest leaf-account expense changes (current vs previous).
    pub expense_changes: Vec<ChangeRow>,
    /// Box 9: biggest leaf-account revenue changes (current vs previous).
    pub revenue_changes: Vec<ChangeRow>,
    /// Box 8: biggest stock movers over the current period.
    pub movers: Vec<MoverRow>,
    /// Box 10: largest transactions in the current period.
    pub top_txns: Vec<TopTxn>,
}

/// The base valuation commodity: the combined explicit + inferred price set's
/// base, or `$` when the journal declares no prices.
pub(super) fn base_commodity(journal: &Journal) -> Result<Commodity, ReportError> {
    let mut all = infer_market_prices(&journal.transactions)?;
    all.extend_from_slice(&journal.prices);
    Ok(PriceDb::build(&all)
        .base_commodity()
        .cloned()
        .unwrap_or_else(|| Commodity("$".to_string())))
}

/// Base-commodity percent change, `None` when the previous base value is absent
/// or zero (a change from nothing has no defined percent).
fn pct_change(current: &MixedAmount, previous: &MixedAmount, base: &Commodity) -> Option<f64> {
    let prev = previous.get(base)?;
    if prev.is_zero() {
        return None;
    }
    let cur = current.get(base).unwrap_or_else(Dec::zero);
    let diff = cur.floating_point() - prev.floating_point();
    Some(diff / prev.floating_point().abs() * 100.0)
}

/// Assemble a [`MetricDelta`] from a current/previous pair.
fn metric_delta(
    current: MixedAmount,
    previous: MixedAmount,
    base: &Commodity,
) -> Result<MetricDelta, ReportError> {
    let delta = current.ma_add(&previous.ma_neg()?)?;
    let pct = pct_change(&current, &previous, base);
    Ok(MetricDelta {
        current,
        previous,
        delta,
        pct,
    })
}

/// Net worth (market-valued in `base`) as of exactly `as_of`. A single daily
/// bucket ending on `as_of` reuses the [`net_worth`] engine so valuation and
/// price inference match the Net Worth report.
fn net_worth_at(
    journal: &Journal,
    as_of: &str,
    base: &Commodity,
) -> Result<MixedAmount, ReportError> {
    let report = net_worth(
        &journal.transactions,
        &journal.prices,
        as_of,
        Interval::Daily,
        1,
        1,
        Some(base.clone()),
    )?;
    Ok(report.totals.into_iter().next().unwrap_or_default())
}

/// Summed balance of all cash-like accounts as of `as_of` (postings dated ≤
/// `as_of`). Direct per-account totals never overlap, so summing them cannot
/// double-count. Natural signs; multi-commodity cash stays mixed.
fn cash_balance(
    txns: &[Transaction],
    as_of: &str,
    is_cash: &dyn Fn(&str) -> bool,
) -> Result<MixedAmount, ReportError> {
    let direct = account_totals(
        txns,
        &PostingFilter {
            to: Some(as_of),
            ..PostingFilter::default()
        },
    )?;
    let mut sum = MixedAmount::new();
    for (account, ma) in &direct {
        if is_cash(account) {
            sum = sum.ma_add(ma)?;
        }
    }
    Ok(sum)
}

/// Total expenses over `[from, to]` excluding any account under a `cost_exclude`
/// prefix. Expense postings carry their natural (positive-spent) sign.
fn cost_of_living_total(
    txns: &[Transaction],
    from: &str,
    to: &str,
    cost_exclude: &[String],
    declared: &BTreeMap<String, AccountType>,
) -> Result<MixedAmount, ReportError> {
    let direct = account_totals(
        txns,
        &PostingFilter {
            from: Some(from),
            to: Some(to),
            ..PostingFilter::default()
        },
    )?;
    let mut sum = MixedAmount::new();
    for (account, ma) in &direct {
        if resolve_account_type(account, declared) != Some(AccountType::Expense) {
            continue;
        }
        if cost_exclude
            .iter()
            .any(|prefix| account_matches(prefix, account))
        {
            continue;
        }
        sum = sum.ma_add(ma)?;
    }
    Ok(sum)
}

/// `(year, month)` parsed from an ISO date (0 for a malformed field).
fn year_month(date: &str) -> (i64, i64) {
    let year = date.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let month = date.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(0);
    (year, month)
}

/// True when `date` is the first day of its month.
fn is_month_start(date: &str) -> bool {
    date.get(8..10) == Some("01")
}

/// True when `date` is the last day of its month (leap-aware, via bucket math).
fn is_month_end(date: &str) -> bool {
    bucket_end(&bucket_key(date, Interval::Monthly)).is_ok_and(|end| end == date)
}

/// The inclusive last day of the previous (first) period.
///
/// For a month-aligned span of an even number of months the split lands exactly
/// on the calendar-month boundary — two equal halves of `months / 2` months
/// each — so a leap day never shifts the divide (the "Year-over-year" preset
/// gets two clean 12-month halves). Any other span falls back to the pure
/// day-count midpoint.
fn split_mid(start: &str, end: &str) -> String {
    if is_month_start(start) && is_month_end(end) {
        let months = months_between(start, end);
        if months >= 2 && months.is_multiple_of(2) {
            let (year, month) = year_month(start);
            let index = year * 12 + (month - 1) + (i64::from(months / 2) - 1);
            let mid_year = index.div_euclid(12);
            let mid_month = index.rem_euclid(12) + 1;
            if let Ok(mid) = bucket_end(&format!("{mid_year:04}-{mid_month:02}")) {
                return mid;
            }
        }
    }
    add_days(start, days_between(start, end) / 2)
}

/// Whole calendar months spanned by an inclusive `[from, to]` range (e.g.
/// `2025-07-01 … 2026-06-30` → 12). Never zero for a valid range.
fn months_between(from: &str, to: &str) -> u32 {
    let month_index = |date: &str| -> i64 {
        let year: i64 = date.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(0);
        let month: i64 = date.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(0);
        year * 12 + month
    };
    let diff = month_index(to) - month_index(from) + 1;
    u32::try_from(diff.max(0)).unwrap_or(0)
}

/// Portfolio performance over `(since, as_of]`: the windowed holdings gain,
/// `marketValue(as_of) − marketValue(since)`, in the base commodity. The
/// whole (unscoped) portfolio is used.
fn perf(journal: &Journal, as_of: &str, since: &str) -> Result<PerfPoint, ReportError> {
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: as_of.to_string(),
        gain_since: Some(since.to_string()),
    };
    let report = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
    )?;
    Ok(PerfPoint {
        gain: report.totals.gain,
        gain_pct: report.totals.gain_pct,
    })
}

/// The base-commodity quantity of an optional account total (zero when absent).
fn base_of(total: Option<&MixedAmount>, base: &Commodity) -> Dec {
    total.and_then(|ma| ma.get(base)).unwrap_or_else(Dec::zero)
}

/// Biggest leaf-account changes within `category` between the two periods
/// (Boxes 7 & 9). Only leaves (accounts that are not an ancestor of another
/// posted account) are compared, so a parent rollup never double-counts. Values
/// are the base commodity; `flip` negates them so revenue increases read
/// positive (income is stored negative). Accounts with no previous-period
/// activity are skipped (nothing to compare); the rest are ranked by the size of
/// the move in real money, then top `TOP_N`.
struct ChangeOpts<'a> {
    /// Inclusive current-period range.
    current: (&'a str, &'a str),
    /// Inclusive previous-period range.
    previous: (&'a str, &'a str),
    /// Which accounts to compare, by effective type.
    category: AccountType,
    base: &'a Commodity,
    change_min: Dec,
    /// Negate values so an increase reads positive (revenue is stored negative).
    flip: bool,
    /// Declared account types, so classification never rests on account names.
    declared: &'a BTreeMap<String, AccountType>,
}

fn leaf_changes(txns: &[Transaction], opts: &ChangeOpts) -> Result<Vec<ChangeRow>, ReportError> {
    let &ChangeOpts {
        current,
        previous,
        category,
        base,
        change_min,
        flip,
        declared,
    } = opts;
    let curr_totals = account_totals(
        txns,
        &PostingFilter {
            from: Some(current.0),
            to: Some(current.1),
            ..PostingFilter::default()
        },
    )?;
    let prev_totals = account_totals(
        txns,
        &PostingFilter {
            from: Some(previous.0),
            to: Some(previous.1),
            ..PostingFilter::default()
        },
    )?;

    // Union of in-category account names seen in either period.
    let mut names: BTreeSet<String> = BTreeSet::new();
    for account in curr_totals.keys().chain(prev_totals.keys()) {
        if resolve_account_type(account, declared) == Some(category) {
            names.insert(account.clone());
        }
    }
    // Keep only leaves — an account that is not an ancestor of another posted one.
    let leaves: Vec<&String> = names
        .iter()
        .filter(|account| {
            let prefix = format!("{account}:");
            !names.iter().any(|other| other.starts_with(prefix.as_str()))
        })
        .collect();

    let threshold = change_min.abs()?;
    let mut rows: Vec<ChangeRow> = Vec::new();
    for leaf in leaves {
        let mut cur = base_of(curr_totals.get(leaf), base);
        let mut prev = base_of(prev_totals.get(leaf), base);
        if flip {
            cur = cur.neg()?;
            prev = prev.neg()?;
        }
        // Nothing to compare against: a category with no previous activity has no
        // defined percent change, so it is not a "change" at all (see [`ChangeKind`]).
        if prev.is_zero() {
            continue;
        }
        // At least one side must clear the noise threshold.
        if cur.abs()? < threshold && prev.abs()? < threshold {
            continue;
        }
        let delta = cur.sub(prev)?;
        let (pct, kind) = if cur.is_zero() {
            (Some(-100.0), ChangeKind::Ended)
        } else {
            let change = (cur.floating_point() - prev.floating_point())
                / prev.floating_point().abs()
                * 100.0;
            (Some(change), ChangeKind::Changed)
        };
        rows.push(ChangeRow {
            account: leaf.clone(),
            current: cur,
            previous: prev,
            delta,
            pct,
            kind,
        });
    }
    // Rank by the SIZE OF THE MOVE in real money (desc), then name. Ranking by
    // percent instead would let a $10 → $30 category outrank a $2,000 → $3,000 one.
    rows.sort_by(|a, b| {
        b.delta
            .floating_point()
            .abs()
            .partial_cmp(&a.delta.floating_point().abs())
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.account.cmp(&b.account))
    });
    rows.truncate(TOP_N);
    Ok(rows)
}

/// Snapshot the whole portfolio as of `as_of` (no gain window).
fn portfolio_at(journal: &Journal, as_of: &str) -> Result<HoldingsReport, ReportError> {
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: as_of.to_string(),
        gain_since: None,
    };
    compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
    )
}

/// Biggest stock movers over `(since, as_of]` (Box 8): the whole portfolio's
/// per-symbol windowed percent moves, ranked by magnitude, top `TOP_N`.
///
/// Each row records whether its value at the window START came from a real `P`
/// price directive or fell back to the purchase-cost annotation — see
/// [`MoverRow::start_estimated`], which the UI surfaces so a lifetime gain is
/// never mistaken for a period return.
fn movers(journal: &Journal, as_of: &str, since: &str) -> Result<Vec<MoverRow>, ReportError> {
    let scope = HoldingsScope {
        accounts: BTreeSet::new(),
        mode: ScopeMode::Include,
        as_of: as_of.to_string(),
        gain_since: Some(since.to_string()),
    };
    let report = compute_holdings(
        &journal.transactions,
        &journal.prices,
        &journal.accounts,
        &journal.commodity_tags,
        &scope,
    )?;

    // Re-run the snapshot at the window start to inspect HOW each position was
    // priced there: a `Cost`-sourced (or absent) price means the baseline is the
    // purchase cost, not a market value.
    let estimated: BTreeSet<String> = portfolio_at(journal, since)?
        .holdings
        .iter()
        .filter(|holding| {
            holding
                .price
                .as_ref()
                .is_none_or(|price| price.source == PriceSource::Cost)
        })
        .map(|holding| holding.symbol.clone())
        .collect();

    let mut rows: Vec<MoverRow> = report
        .holdings
        .iter()
        .filter(|holding| holding.gain_pct.is_some())
        .map(|holding| MoverRow {
            symbol: holding.symbol.clone(),
            name: holding.name.clone(),
            gain: holding.gain,
            gain_pct: holding.gain_pct,
            start_estimated: estimated.contains(&holding.symbol),
        })
        .collect();
    rows.sort_by(|a, b| {
        let (ma, mb) = (a.gain_pct.map(f64::abs), b.gain_pct.map(f64::abs));
        mb.partial_cmp(&ma)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    rows.truncate(TOP_N);
    Ok(rows)
}

/// The largest transactions over `[from, to]` by base-commodity magnitude moved
/// (Box 10): per transaction, the greater of its summed positive and summed
/// negative base legs.
fn top_transactions(
    txns: &[Transaction],
    from: &str,
    to: &str,
    base: &Commodity,
) -> Result<Vec<TopTxn>, ReportError> {
    let mut scored: Vec<TopTxn> = Vec::new();
    for txn in txns {
        if txn.date.as_str() < from || txn.date.as_str() > to {
            continue;
        }
        let mut positive = Dec::zero();
        let mut negative = Dec::zero();
        for posting in &txn.postings {
            for amount in &posting.amounts {
                if amount.commodity != *base {
                    continue;
                }
                if amount.quantity.mantissa > 0 {
                    positive = positive.add(amount.quantity)?;
                } else if amount.quantity.mantissa < 0 {
                    negative = negative.add(amount.quantity)?;
                }
            }
        }
        let amount = positive.max(negative.abs()?);
        if amount.is_zero() {
            continue;
        }
        scored.push(TopTxn {
            index: txn.index.0,
            date: txn.date.clone(),
            description: txn.description.clone(),
            amount,
        });
    }
    scored.sort_by(|a, b| b.amount.cmp(&a.amount).then_with(|| a.index.cmp(&b.index)));
    scored.truncate(TOP_N);
    Ok(scored)
}

/// Compute the Insights dashboard's core metrics over `opts`'s span, split at
/// its midpoint into a previous and current period.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or bad bucket math (both
/// unreachable for realistic journals, but never unwrapped).
pub fn insights(journal: &Journal, opts: &InsightsOpts) -> Result<InsightsReport, ReportError> {
    let txns = &journal.transactions;
    let (start, end) = (opts.start, opts.end);
    let mid = split_mid(start, end);
    let curr_start = add_days(&mid, 1);

    let base = base_commodity(journal)?;

    // Boxes 1 & 2 — revenue and expenses per period (one income statement each).
    let is_curr = income_statement(txns, &curr_start, end, 1)?;
    let is_prev = income_statement(txns, start, &mid, 1)?;
    let revenue = metric_delta(
        is_curr.sections[0].total.clone(),
        is_prev.sections[0].total.clone(),
        &base,
    )?;
    let expenses = metric_delta(
        is_curr.sections[1].total.clone(),
        is_prev.sections[1].total.clone(),
        &base,
    )?;

    // Box 3 — net worth at each period end (current end vs previous end = mid).
    let net_worth_delta = metric_delta(
        net_worth_at(journal, end, &base)?,
        net_worth_at(journal, &mid, &base)?,
        &base,
    )?;

    // Box 6 — cash balance at each period end. `declared` also drives the
    // expense/revenue filters below, so costs booked outside an `expenses:`
    // root (or under non-English names) are classified by their declared type
    // rather than by what they happen to be called.
    let decls = account_decls(journal);
    let declared = declared_types(&decls);
    let is_cash = cash_predicate(&decls);
    let cash_balance_delta = metric_delta(
        cash_balance(txns, end, &is_cash)?,
        cash_balance(txns, &mid, &is_cash)?,
        &base,
    )?;

    // Box 4 — average monthly cost of living (totals + month counts; averaged
    // at the display boundary).
    let cost_of_living = CostOfLiving {
        current_total: cost_of_living_total(txns, &curr_start, end, opts.cost_exclude, &declared)?,
        previous_total: cost_of_living_total(txns, start, &mid, opts.cost_exclude, &declared)?,
        months_current: months_between(&curr_start, end),
        months_previous: months_between(start, &mid),
    };

    // Box 5 — investment performance (current: since mid; previous: since start).
    let investment = InvestmentPerf {
        current: perf(journal, end, &mid)?,
        previous: perf(journal, &mid, start)?,
    };

    // Boxes 7 & 9 — biggest leaf-account expense / revenue changes (current period
    // vs previous). Revenue is sign-flipped so an increase reads positive.
    let current_range = (curr_start.as_str(), end);
    let previous_range = (start, mid.as_str());
    let change_opts = ChangeOpts {
        current: current_range,
        previous: previous_range,
        category: AccountType::Expense,
        base: &base,
        change_min: opts.change_min,
        flip: false,
        declared: &declared,
    };
    let expense_changes = leaf_changes(txns, &change_opts)?;
    let revenue_changes = leaf_changes(
        txns,
        &ChangeOpts {
            category: AccountType::Revenue,
            flip: true,
            ..change_opts
        },
    )?;

    // Box 8 — biggest stock movers over the current period.
    let movers_list = movers(journal, end, &mid)?;

    // Box 10 — largest transactions in the current period.
    let top_txns = top_transactions(txns, &curr_start, end, &base)?;

    Ok(InsightsReport {
        period: InsightsPeriod {
            start: start.to_string(),
            mid: mid.clone(),
            end: end.to_string(),
            prev_start: start.to_string(),
            prev_end: mid.clone(),
            curr_start,
            curr_end: end.to_string(),
        },
        base: base.0,
        journal_start: txns.iter().map(|txn| txn.date.clone()).min(),
        revenue,
        expenses,
        net_worth: net_worth_delta,
        cost_of_living,
        investment,
        cash_balance: cash_balance_delta,
        expense_changes,
        revenue_changes,
        movers: movers_list,
        top_txns,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{amount, price, txn, usd};
    use super::*;
    use crate::model::{Amount, Cost, CostKind, Journal, PriceDirective};

    /// A minimal journal wrapping hand-built transactions (no prices/decls).
    fn journal(transactions: Vec<Transaction>) -> Journal {
        journal_with_prices(transactions, Vec::new())
    }

    /// A minimal journal carrying explicit `P` price directives.
    fn journal_with_prices(transactions: Vec<Transaction>, prices: Vec<PriceDirective>) -> Journal {
        Journal {
            source_name: String::new(),
            source_files: Vec::new(),
            transactions,
            periodic_transactions: Vec::new(),
            accounts: Vec::new(),
            commodity_styles: Vec::new(),
            commodity_tags: Vec::new(),
            prices,
        }
    }

    /// Attach a per-unit (`@`) cost annotation to an amount.
    fn with_unit_cost(mut held: Amount, unit_cost: Amount) -> Amount {
        held.cost = Some(Box::new(Cost {
            kind: CostKind::Unit,
            amount: unit_cost,
        }));
        held
    }

    fn usd_ma(cents: i128) -> MixedAmount {
        MixedAmount::single(Commodity("$".into()), Dec::new(cents, 2))
    }

    /// Two 5- and 5-day halves of a 10-day span:
    /// previous = [01-01, 01-06], current = [01-07, 01-11].
    fn sample() -> Journal {
        journal(vec![
            // previous half: revenue $10, food $2
            txn(
                1,
                "2025-01-02",
                vec![
                    ("income:salary", vec![usd(-100_000)]),
                    ("assets:bank:checking", vec![usd(100_000)]),
                ],
            ),
            txn(
                2,
                "2025-01-03",
                vec![
                    ("expenses:food", vec![usd(20_000)]),
                    ("assets:bank:checking", vec![usd(-20_000)]),
                ],
            ),
            // current half: revenue $15, food $5, taxes $3
            txn(
                3,
                "2025-01-08",
                vec![
                    ("income:salary", vec![usd(-150_000)]),
                    ("assets:bank:checking", vec![usd(150_000)]),
                ],
            ),
            txn(
                4,
                "2025-01-09",
                vec![
                    ("expenses:food", vec![usd(50_000)]),
                    ("assets:bank:checking", vec![usd(-50_000)]),
                ],
            ),
            txn(
                5,
                "2025-01-10",
                vec![
                    ("expenses:taxes", vec![usd(30_000)]),
                    ("assets:bank:checking", vec![usd(-30_000)]),
                ],
            ),
        ])
    }

    fn run(j: &Journal) -> InsightsReport {
        insights(
            j,
            &InsightsOpts {
                start: "2025-01-01",
                end: "2025-01-11",
                cost_exclude: &["expenses:taxes".to_string()],
                change_min: Dec::zero(),
            },
        )
        .expect("insights succeeds")
    }

    #[test]
    fn splits_span_at_the_midpoint() {
        let report = run(&sample());
        assert_eq!(report.period.start, "2025-01-01");
        assert_eq!(report.period.mid, "2025-01-06");
        assert_eq!(report.period.curr_start, "2025-01-07");
        assert_eq!(report.period.end, "2025-01-11");
        assert_eq!(report.base, "$");
    }

    #[test]
    fn month_aligned_span_splits_on_calendar_boundary_despite_leap_day() {
        // 24 months with the leap day (Feb 29 2024) in the SECOND half: a naive
        // day-midpoint would land one day into the second half; the calendar
        // split gives two clean 12-month halves.
        let report = insights(
            &journal(Vec::new()),
            &InsightsOpts {
                start: "2023-01-01",
                end: "2024-12-31",
                cost_exclude: &[],
                change_min: Dec::zero(),
            },
        )
        .expect("insights succeeds");
        assert_eq!(report.period.mid, "2023-12-31");
        assert_eq!(report.period.curr_start, "2024-01-01");
        assert_eq!(report.cost_of_living.months_current, 12);
        assert_eq!(report.cost_of_living.months_previous, 12);
    }

    #[test]
    fn revenue_and_expense_deltas() {
        let report = run(&sample());
        assert_eq!(report.revenue.current, usd_ma(150_000));
        assert_eq!(report.revenue.previous, usd_ma(100_000));
        assert_eq!(report.revenue.delta, usd_ma(50_000));
        assert_eq!(report.revenue.pct, Some(50.0));

        // Expenses: current = food $5 + taxes $3 = $8; previous = food $2.
        assert_eq!(report.expenses.current, usd_ma(80_000));
        assert_eq!(report.expenses.previous, usd_ma(20_000));
        assert_eq!(report.expenses.delta, usd_ma(60_000));
        assert_eq!(report.expenses.pct, Some(300.0));
    }

    #[test]
    fn cost_of_living_excludes_the_exclusion_list() {
        let report = run(&sample());
        // Taxes excluded → current food-only $5, previous $2.
        assert_eq!(report.cost_of_living.current_total, usd_ma(50_000));
        assert_eq!(report.cost_of_living.previous_total, usd_ma(20_000));
        assert_eq!(report.cost_of_living.months_current, 1);
        assert_eq!(report.cost_of_living.months_previous, 1);
    }

    #[test]
    fn cash_balance_and_net_worth_at_period_ends() {
        let report = run(&sample());
        // Cash at mid (01-06): +100 − 20 = $80; at end (01-11): 80 + 150 − 50 − 30 = $150.
        assert_eq!(report.cash_balance.current, usd_ma(150_000));
        assert_eq!(report.cash_balance.previous, usd_ma(80_000));
        assert_eq!(report.cash_balance.delta, usd_ma(70_000));
        assert_eq!(report.cash_balance.pct, Some(87.5));

        // No liabilities/other assets, so net worth == cash here.
        assert_eq!(report.net_worth.current, usd_ma(150_000));
        assert_eq!(report.net_worth.previous, usd_ma(80_000));
        assert_eq!(report.net_worth.delta, usd_ma(70_000));
    }

    #[test]
    fn investment_is_empty_without_stocks() {
        let report = run(&sample());
        // No securities → windowed portfolio gain is a real zero, percent undefined.
        assert_eq!(report.investment.current.gain, Some(Dec::zero()));
        assert_eq!(report.investment.current.gain_pct, None);
        assert_eq!(report.investment.previous.gain_pct, None);
    }

    #[test]
    fn leaf_changes_movers_and_top_txns() {
        let report = run(&sample());

        // Expense changes: only food is comparable — the taxes category has no
        // previous-period activity, so it is not reported at all.
        let expenses: Vec<(&str, ChangeKind)> = report
            .expense_changes
            .iter()
            .map(|row| (row.account.as_str(), row.kind))
            .collect();
        assert_eq!(expenses, [("expenses:food", ChangeKind::Changed)]);
        let food = report
            .expense_changes
            .iter()
            .find(|row| row.account == "expenses:food")
            .unwrap();
        assert_eq!(food.current, Dec::new(50_000, 2));
        assert_eq!(food.previous, Dec::new(20_000, 2));
        assert_eq!(food.pct, Some(150.0));

        // Revenue changes are sign-flipped so a bigger paycheck reads positive.
        assert_eq!(report.revenue_changes.len(), 1);
        let salary = &report.revenue_changes[0];
        assert_eq!(salary.account, "income:salary");
        assert_eq!(salary.current, Dec::new(150_000, 2));
        assert_eq!(salary.previous, Dec::new(100_000, 2));
        assert_eq!(salary.pct, Some(50.0));

        // No securities → no movers.
        assert!(report.movers.is_empty());

        // Top transactions in the current half, by money moved, desc.
        let top: Vec<(u32, Dec)> = report
            .top_txns
            .iter()
            .map(|txn| (txn.index, txn.amount))
            .collect();
        assert_eq!(
            top,
            [
                (3, Dec::new(150_000, 2)),
                (4, Dec::new(50_000, 2)),
                (5, Dec::new(30_000, 2)),
            ]
        );
    }

    // ---- diagnostics: how movers behave with and without price history ----

    /// One 10-share buy at $100/sh on 2025-03-01 (before the split), priced by
    /// `prices`. Span 2025-01-01…2026-12-31 splits at 2025-12-31.
    fn stock_journal(prices: Vec<PriceDirective>) -> Journal {
        journal_with_prices(
            vec![txn(
                1,
                "2025-03-01",
                vec![
                    (
                        "assets:broker:stk",
                        vec![with_unit_cost(
                            amount("STK", 10, 0),
                            amount("$", 10_000, 2), // @ $100.00
                        )],
                    ),
                    ("assets:bank:checking", vec![usd(-100_000)]),
                ],
            )],
            prices,
        )
    }

    fn stock_insights(prices: Vec<PriceDirective>) -> InsightsReport {
        insights(
            &stock_journal(prices),
            &InsightsOpts {
                start: "2025-01-01",
                end: "2026-12-31",
                cost_exclude: &[],
                change_min: Dec::zero(),
            },
        )
        .expect("insights succeeds")
    }

    #[test]
    fn movers_report_a_true_period_move_when_priced_at_the_window_start() {
        // P at the split ($120) and later ($150): the period move is measured from
        // the market value at the split, NOT from the purchase cost.
        let report = stock_insights(vec![
            price("2025-12-31", "STK", amount("$", 12_000, 2)),
            price("2026-06-30", "STK", amount("$", 15_000, 2)),
        ]);
        assert_eq!(report.movers.len(), 1);
        let stk = &report.movers[0];
        // 10 × $150 − 10 × $120 = $300 (+25%), distinct from the all-time
        // $1,500 − $1,000 = $500 (+50%).
        assert_eq!(stk.gain, Some(Dec::new(300, 0)));
        assert!((stk.gain_pct.unwrap() - 25.0).abs() < 1e-9);
        assert!(
            !stk.start_estimated,
            "a real price directive at the window start is not an estimate"
        );
    }

    #[test]
    fn movers_degenerate_to_all_time_gain_without_a_price_before_the_window() {
        // The ONLY price is dated after the split, so valuing the position at the
        // split falls back to the purchase cost annotation ($100). The "period"
        // move then equals the all-time gain — the reported behavior.
        let report = stock_insights(vec![price("2026-06-30", "STK", amount("$", 15_000, 2))]);
        assert_eq!(report.movers.len(), 1);
        let stk = &report.movers[0];
        // 10 × $150 − 10 × $100 (cost fallback) = $500 (+50%) = the all-time gain.
        assert_eq!(stk.gain, Some(Dec::new(500, 0)));
        assert!((stk.gain_pct.unwrap() - 50.0).abs() < 1e-9);
        // ...and the row says so, so the UI can flag it rather than pass a
        // lifetime gain off as a period return.
        assert!(stk.start_estimated);
    }

    /// Five brand-new leaves plus one genuine change: the `New` rows currently
    /// sort first (infinite key) and crowd the real comparison out of the top 5.
    fn crowded_journal() -> Journal {
        let mut txns = vec![
            // A real comparison present in BOTH halves: $100 → $300.
            txn(
                1,
                "2025-03-01",
                vec![
                    ("expenses:food:groceries", vec![usd(10_000)]),
                    ("assets:bank:checking", vec![usd(-10_000)]),
                ],
            ),
            txn(
                2,
                "2026-03-01",
                vec![
                    ("expenses:food:groceries", vec![usd(30_000)]),
                    ("assets:bank:checking", vec![usd(-30_000)]),
                ],
            ),
        ];
        // Five categories that exist ONLY in the current half.
        for (i, name) in ["alpha", "bravo", "charlie", "delta", "echo"]
            .iter()
            .enumerate()
        {
            txns.push(txn(
                (i as u32) + 3,
                "2026-04-01",
                vec![
                    (
                        Box::leak(format!("expenses:new:{name}").into_boxed_str()) as &str,
                        vec![usd(5_000)],
                    ),
                    ("assets:bank:checking", vec![usd(-5_000)]),
                ],
            ));
        }
        journal(txns)
    }

    #[test]
    fn expense_changes_surface_real_comparisons_not_only_new_categories() {
        let report = insights(
            &crowded_journal(),
            &InsightsOpts {
                start: "2025-01-01",
                end: "2026-12-31",
                cost_exclude: &[],
                change_min: Dec::zero(),
            },
        )
        .expect("insights succeeds");

        let accounts: Vec<&str> = report
            .expense_changes
            .iter()
            .map(|row| row.account.as_str())
            .collect();
        // Only the genuine comparison survives; the five previous-period-less
        // categories are not changes and never enter the ranking.
        assert_eq!(accounts, ["expenses:food:groceries"]);
        assert_eq!(report.expense_changes[0].pct, Some(200.0));
    }

    #[test]
    fn journal_start_exposes_partial_previous_period_coverage() {
        // The sample journal's earliest transaction is 2025-01-02, i.e. INSIDE the
        // previous half — so the UI can tell that half is only partly covered and
        // the deltas overstate growth.
        let report = run(&sample());
        assert_eq!(report.journal_start.as_deref(), Some("2025-01-02"));
        assert!(report.journal_start.as_deref() > Some(report.period.prev_start.as_str()));

        // An empty journal has no start date at all.
        let empty = run(&journal(Vec::new()));
        assert_eq!(empty.journal_start, None);
    }

    #[test]
    fn changes_rank_by_money_moved_not_percent() {
        // A small category doubling (+$20) must NOT outrank a large one that moved
        // ten times more money (+$200), even though its percent change is bigger.
        let j = journal(vec![
            txn(
                1,
                "2025-03-01",
                vec![
                    ("expenses:small", vec![usd(2_000)]),
                    ("expenses:big", vec![usd(100_000)]),
                    ("assets:bank:checking", vec![usd(-102_000)]),
                ],
            ),
            txn(
                2,
                "2026-03-01",
                vec![
                    ("expenses:small", vec![usd(4_000)]), // +$20  (+100%)
                    ("expenses:big", vec![usd(120_000)]), // +$200 (+20%)
                    ("assets:bank:checking", vec![usd(-124_000)]),
                ],
            ),
        ]);
        let report = insights(
            &j,
            &InsightsOpts {
                start: "2025-01-01",
                end: "2026-12-31",
                cost_exclude: &[],
                change_min: Dec::zero(),
            },
        )
        .expect("insights succeeds");
        let accounts: Vec<&str> = report
            .expense_changes
            .iter()
            .map(|row| row.account.as_str())
            .collect();
        assert_eq!(accounts, ["expenses:big", "expenses:small"]);
    }

    #[test]
    fn change_min_filters_out_noise() {
        // A $1,000 threshold is above every expense leaf here (food $500, taxes
        // $300), so the biggest-changes list comes back empty.
        let report = insights(
            &sample(),
            &InsightsOpts {
                start: "2025-01-01",
                end: "2025-01-11",
                cost_exclude: &[],
                change_min: Dec::new(100_000, 2), // $1,000 — above every leaf here
            },
        )
        .expect("insights succeeds");
        assert!(report.expense_changes.is_empty());
    }

    #[test]
    fn pct_is_none_when_previous_is_zero() {
        // Revenue only in the current half → previous base is zero → no percent.
        let j = journal(vec![txn(
            1,
            "2025-01-09",
            vec![
                ("income:salary", vec![usd(-100_000)]),
                ("assets:bank:checking", vec![usd(100_000)]),
            ],
        )]);
        let report = run(&j);
        assert_eq!(report.revenue.current, usd_ma(100_000));
        assert!(report.revenue.previous.is_zero());
        assert_eq!(report.revenue.pct, None);
    }
}
