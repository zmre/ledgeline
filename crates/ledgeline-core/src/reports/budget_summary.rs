//! Budget summary — the period-total view the Budget tab draws (plan 26 B6).
//!
//! [`budget_report`](super::budget_report) answers per bucket. The tab reads ONE
//! `{actual, goal}` pair per account over the whole span — a $200/mo goal across
//! seven months reads as $1,400 budgeted to date — and judges each bar against
//! where it should be BY NOW (its pace), not against the whole span's goal.
//! This module is that reading, ported from the SPA's
//! `reports/budgetSummary.ts` and the logic that had leaked into
//! `BudgetSummary.svelte` / `BudgetGaps.svelte`. It is locked to the
//! TypeScript it replaced by `tests/budget_summary_golden.rs`.
//!
//! # Exact, including pace
//!
//! Every figure a reader sees stays a [`MixedAmount`]. The one ratio — the
//! fraction of the span that has elapsed — is kept as the exact pair of day
//! counts ([`Elapsed`]) rather than a float:
//!
//! - the health verdict compares `actual × span` with `goal × done`, exactly;
//! - the pace AMOUNT is `goal × done ÷ span`, rounded half away from zero at
//!   six places past the goal's own scale ([`PACE_EXTRA_PLACES`]).
//!
//! The TypeScript snapped the fraction to six places FIRST and multiplied after,
//! so its pace could differ from this one by up to `|goal| × 5e-7` — well under
//! a displayed cent for any goal below $20,000 a span. The golden test compares
//! pace within exactly that bound and everything else exactly.
//!
//! # Signs
//!
//! hledger writes revenue goals negative. Every bar here is in MAGNITUDES
//! ([`magnitude`]), and `income` records which way round the goal was written,
//! so a client never negates anything itself.

use super::ReportError;
use super::account_types::{AccountType, is_account_type};
use super::budget::{BudgetGaps, BudgetReport};
use super::mixed_amount::MixedAmount;
use super::periods::{bucket_start, days_between};
use crate::decimal::Dec;
use crate::model::PeriodExpr;
use std::collections::{BTreeMap, BTreeSet};

/// How many fractional digits past the goal's own scale a pace amount is
/// carried at. Six is far past what a day-count ratio can resolve (a 366-day
/// span steps by 0.0027) and keeps the mantissa small; it is also the scale
/// the TypeScript snapped its fraction to, so the two agree to the same digit.
pub const PACE_EXTRA_PLACES: u32 = 6;

/// One account's span totals: actual summed across buckets, and goal summed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetLine {
    /// Full account name, as the report row carries it.
    pub account: String,
    /// Number of `:`-separated segments in `account`.
    pub depth: usize,
    /// Subaccount-inclusive actual over the span.
    pub actual: MixedAmount,
    /// Summed goal, or `None` when no bucket had one (`<unbudgeted>`, or an
    /// account budgeted only by a rule `budgetDesc` filtered out).
    pub goal: Option<MixedAmount>,
}

/// Sum each row's per-bucket cells into one span total. A row is budgeted
/// (`goal` is `Some`) iff any of its cells had a goal.
///
/// # Errors
/// [`ReportError::Decimal`] on overflow.
pub fn summarize_budget(report: &BudgetReport) -> Result<Vec<BudgetLine>, ReportError> {
    report
        .rows
        .iter()
        .map(|row| {
            let mut actual = MixedAmount::new();
            let mut goal: Option<MixedAmount> = None;
            for cell in &row.cells {
                actual.ma_add_assign(&cell.actual)?;
                if let Some(cell_goal) = &cell.goal {
                    goal.get_or_insert_with(MixedAmount::new)
                        .ma_add_assign(cell_goal)?;
                }
            }
            Ok(BudgetLine {
                account: row.account.clone(),
                depth: row.depth,
                actual,
                goal,
            })
        })
        .collect()
}

/// The leaf-most budgeted lines: an aggregate PARENT is hidden when a deeper
/// budgeted line is also present.
///
/// The report reproduces hledger's tree budget, which emits both `expenses`
/// (the depth-clamped parent aggregate) and its budgeted children — redundant
/// in a flat bar list, where the parent's bar just re-sums its children.
/// Standalone budgets stay at whatever depth. Unbudgeted lines are excluded.
#[must_use]
pub fn budget_leaves(lines: &[BudgetLine]) -> Vec<BudgetLine> {
    let budgeted: Vec<&BudgetLine> = lines.iter().filter(|line| line.goal.is_some()).collect();
    budgeted
        .iter()
        .filter(|leaf| {
            let prefix = format!("{}:", leaf.account);
            !budgeted
                .iter()
                .any(|other| other.account.starts_with(&prefix))
        })
        .map(|leaf| (*leaf).clone())
        .collect()
}

/// Whether any strict ancestor of `account` is itself budgeted.
fn has_budgeted_ancestor(account: &str, budgeted: &BTreeSet<&str>) -> bool {
    let mut current = account;
    while let Some(cut) = current.rfind(':') {
        current = &current[..cut];
        if budgeted.contains(current) {
            return true;
        }
    }
    false
}

/// "Spent of budgeted" over `lines`: the sum of the TOP-LEVEL budgeted lines
/// only, as `(actual, goal)`.
///
/// Nested budgets are inclusive, so counting a budgeted child under a budgeted
/// parent would double count; the outermost is kept. Unbudgeted lines are
/// excluded.
///
/// # Errors
/// [`ReportError::Decimal`] on overflow.
pub fn budget_totals(lines: &[BudgetLine]) -> Result<(MixedAmount, MixedAmount), ReportError> {
    let budgeted: BTreeSet<&str> = lines
        .iter()
        .filter(|line| line.goal.is_some())
        .map(|line| line.account.as_str())
        .collect();
    let mut actual = MixedAmount::new();
    let mut goal = MixedAmount::new();
    for line in lines {
        let Some(line_goal) = &line.goal else {
            continue;
        };
        if has_budgeted_ancestor(&line.account, &budgeted) {
            continue;
        }
        actual.ma_add_assign(&line.actual)?;
        goal.ma_add_assign(line_goal)?;
    }
    Ok((actual, goal))
}

/// The single number a bar is sized and judged by: the quantity of the only
/// commodity, zero for the empty amount, `None` for a multi-commodity one
/// ($100 and €100 have no single magnitude without a price).
///
/// Deliberately commodity-blind, as the TypeScript was: a $100 goal against a
/// 50 EUR actual compares 50 with 100. That only arises for a goal written in a
/// different commodity from the spending it measures.
#[must_use]
pub fn primary_value(amount: &MixedAmount) -> Option<Dec> {
    let mut quantities = amount.iter().map(|(_, qty)| *qty);
    match (quantities.next(), quantities.next()) {
        (None, _) => Some(Dec::zero()),
        (Some(qty), None) => Some(qty),
        (Some(_), Some(_)) => None,
    }
}

/// `|amount|` by the primary commodity's sign: a credit-normal (negative)
/// amount is flipped so income reads "earned $X of $Y". A no-op for positive
/// and for multi-commodity amounts.
///
/// # Errors
/// [`ReportError::Decimal`] when a mantissa cannot be negated.
pub fn magnitude(amount: &MixedAmount) -> Result<MixedAmount, ReportError> {
    match primary_value(amount) {
        Some(qty) if qty < Dec::zero() => Ok(amount.ma_neg()?),
        _ => Ok(amount.clone()),
    }
}

/// How much of the inclusive span `[from, to]` has elapsed as of a date, as an
/// exact ratio of day counts, already clamped to `[0, 1]`.
///
/// `done == span` is a finished span (also a malformed, inverted one: there is
/// nothing to pace against); `done == 0` is one that has not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed {
    /// Days elapsed, `0..=span`.
    pub done: u32,
    /// Days in the span, at least 1.
    pub span: u32,
}

impl Elapsed {
    /// The finished span.
    const WHOLE: Self = Self { done: 1, span: 1 };

    /// The fraction as a float, for display only — nothing here decides on it.
    #[must_use]
    pub fn fraction(self) -> f64 {
        f64::from(self.done) / f64::from(self.span)
    }
}

/// [`Elapsed`] for `[from, to]` as of `as_of`.
///
/// 1 for a span that has already ended — looking at last year, the pace mark
/// sits on the goal marker and every bar means what it means today — and 0
/// for one that has not started. A single-day span is whole on its own day.
/// Prorated by DAYS, not by completed buckets, so a bar does not change colour
/// on the 1st of the month for reasons that have nothing to do with money.
#[must_use]
pub fn elapsed(from: &str, to: &str, as_of: &str) -> Elapsed {
    let span = days_between(from, to) + 1;
    if span <= 0 {
        return Elapsed::WHOLE;
    }
    let done = days_between(from, as_of) + 1;
    if done <= 0 {
        return Elapsed { done: 0, span: 1 };
    }
    if done >= span {
        return Elapsed::WHOLE;
    }
    // `0 < done < span`, and a span of dates fits u32 by a wide margin; the
    // fallback is unreachable but honest (a finished span) rather than a panic.
    match (u32::try_from(done), u32::try_from(span)) {
        (Ok(done), Ok(span)) => Elapsed { done, span },
        _ => Elapsed::WHOLE,
    }
}

/// The goal prorated to `elapsed` — where a line should be by now — exact to
/// [`PACE_EXTRA_PLACES`] past each commodity's scale, half away from zero.
///
/// # Errors
/// [`ReportError::Decimal`] on overflow.
pub fn pace_amount(goal: &MixedAmount, elapsed: Elapsed) -> Result<MixedAmount, ReportError> {
    if elapsed.done == 0 {
        return Ok(MixedAmount::new());
    }
    if elapsed.done == elapsed.span {
        return Ok(goal.clone());
    }
    let mut out = MixedAmount::new();
    for (commodity, qty) in goal.iter() {
        let widened = qty.rounded(qty.places + PACE_EXTRA_PLACES)?;
        let scaled = Dec::new(widened.mantissa, widened.places)
            .mul(Dec::new(i128::from(elapsed.done), 0))?;
        let prorated = scaled
            .rounded(widened.places)?
            .div_int(elapsed.span)?
            .normalized();
        out.accumulate(commodity, prorated)?;
    }
    out.drop_zeros();
    Ok(out)
}

/// How a bar is doing. `Behind` is the unhealthy side of pace in either
/// column; `Over` is the expense-only third state — past the GOAL, not merely
/// past pace — that an envelope user must not miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BudgetHealth {
    /// On the right side of pace.
    Healthy,
    /// On the wrong side of pace.
    Behind,
    /// An expense past its whole goal.
    Over,
}

/// Healthy iff the actual is on the right side of pace — at or above it for
/// revenue, at or below it for expense. All three figures are magnitudes; pace
/// is the exact `goal × done ÷ span`, compared by cross-multiplying.
///
/// # Errors
/// [`ReportError::Decimal`] on overflow.
pub fn budget_health(
    actual: Dec,
    goal: Dec,
    elapsed: Elapsed,
    income: bool,
) -> Result<BudgetHealth, ReportError> {
    let span = Dec::new(i128::from(elapsed.span), 0);
    let done = Dec::new(i128::from(elapsed.done), 0);
    // actual ⋚ goal·done/span  ⇔  actual·span ⋚ goal·done  (span > 0)
    let scaled_actual = actual.mul(span)?;
    let scaled_pace = goal.mul(done)?;
    if income {
        return Ok(if scaled_actual >= scaled_pace {
            BudgetHealth::Healthy
        } else {
            BudgetHealth::Behind
        });
    }
    if actual > goal {
        return Ok(BudgetHealth::Over);
    }
    Ok(if scaled_actual <= scaled_pace {
        BudgetHealth::Healthy
    } else {
        BudgetHealth::Behind
    })
}

/// What the words under a bar say, in plans/20's vocabulary. The client
/// formats `amount` and supplies the wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelKind {
    /// Exactly on the goal, or a line with no single comparison to make.
    OnPlan,
    /// Revenue short of pace by `amount`.
    BehindPace,
    /// Revenue past its whole goal by `amount`.
    AboveTarget,
    /// Revenue on pace with `amount` still to earn.
    ToGo,
    /// An expense past its whole goal by `amount`.
    Over,
    /// An expense running ahead of pace by `amount`.
    AheadOfPace,
    /// An expense on pace with `amount` left.
    Left,
}

/// A bar's label: what kind of sentence, and the amount it quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarLabel {
    /// Which sentence.
    pub kind: LabelKind,
    /// The amount the sentence quotes (a magnitude); `None` for
    /// [`LabelKind::OnPlan`].
    pub amount: Option<MixedAmount>,
}

/// One bar: an account's span totals in magnitudes, its pace, its health and
/// its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetBar {
    /// The account, or `""` for a section's overall bar.
    pub account: String,
    /// The goal was written credit-normal (negative): read it as income.
    pub income: bool,
    /// `|actual|`.
    pub actual: MixedAmount,
    /// `|goal|`, the span total.
    pub goal: MixedAmount,
    /// Where `actual` should be by now.
    pub pace: MixedAmount,
    /// `|goal| − |actual|`.
    pub remaining: MixedAmount,
    /// `None` when any of actual, goal or pace has no single magnitude (a
    /// multi-commodity line): there is no bar to draw and no verdict to give.
    pub health: Option<BudgetHealth>,
    /// The words under the bar.
    pub label: BarLabel,
}

/// Build one bar from a span line (or a section's totals).
///
/// # Errors
/// [`ReportError::Decimal`] on overflow.
pub fn budget_bar(
    account: &str,
    actual: &MixedAmount,
    goal: &MixedAmount,
    elapsed: Elapsed,
) -> Result<BudgetBar, ReportError> {
    let income = primary_value(goal).is_some_and(|qty| qty < Dec::zero());
    let goal = magnitude(goal)?;
    let actual = magnitude(actual)?;
    let pace = pace_amount(&goal, elapsed)?;
    let remaining = goal.ma_add(&actual.ma_neg()?)?;
    let health = match (
        primary_value(&actual),
        primary_value(&goal),
        primary_value(&pace),
    ) {
        (Some(spent), Some(budget), Some(_)) => {
            Some(budget_health(spent, budget, elapsed, income)?)
        }
        _ => None,
    };
    let label = bar_label(health, income, &actual, &pace, &remaining)?;
    Ok(BudgetBar {
        account: account.to_string(),
        income,
        actual,
        goal,
        pace,
        remaining,
        health,
        label,
    })
}

/// The six sentences, three per column, plus "on plan" for an exact-zero
/// remainder in either (and for a line with no health at all).
fn bar_label(
    health: Option<BudgetHealth>,
    income: bool,
    actual: &MixedAmount,
    pace: &MixedAmount,
    remaining: &MixedAmount,
) -> Result<BarLabel, ReportError> {
    let label = |kind, amount| Ok(BarLabel { kind, amount });
    let Some(health) = health else {
        return label(LabelKind::OnPlan, None);
    };
    let left = primary_value(remaining).unwrap_or_else(Dec::zero);
    let zero = Dec::zero();
    if income {
        if health == BudgetHealth::Behind {
            return label(LabelKind::BehindPace, Some(pace.ma_add(&actual.ma_neg()?)?));
        }
        if left < zero {
            return label(LabelKind::AboveTarget, Some(remaining.ma_neg()?));
        }
        if left == zero {
            return label(LabelKind::OnPlan, None);
        }
        return label(LabelKind::ToGo, Some(remaining.clone()));
    }
    match health {
        BudgetHealth::Over => label(LabelKind::Over, Some(remaining.ma_neg()?)),
        BudgetHealth::Behind => label(
            LabelKind::AheadOfPace,
            Some(actual.ma_add(&pace.ma_neg()?)?),
        ),
        BudgetHealth::Healthy if left == zero => label(LabelKind::OnPlan, None),
        BudgetHealth::Healthy => label(LabelKind::Left, Some(remaining.clone())),
    }
}

/// Which half of the Budget tab a section is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BudgetSectionKind {
    /// Income: revenue-typed accounts (gain included).
    Revenue,
    /// Expenses: expense-typed accounts.
    Expense,
}

/// One section: its overall bar and one bar per leaf, in account order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetSection {
    /// Income or Expenses.
    pub kind: BudgetSectionKind,
    /// The section's "spent of budgeted" bar ([`budget_totals`] over its
    /// leaves); `account` is empty.
    pub overall: BudgetBar,
    /// One bar per leaf budget, in the report's account order.
    pub bars: Vec<BudgetBar>,
}

/// The whole span summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetSummary {
    /// Inclusive span start: the first day of the oldest bucket.
    pub from: String,
    /// Inclusive span end.
    pub to: String,
    /// The date pace is measured at.
    pub as_of: String,
    /// How much of `[from, to]` had elapsed by `as_of`.
    pub elapsed: Elapsed,
    /// Income then Expenses, each present only when it has a leaf budget.
    pub sections: Vec<BudgetSection>,
}

/// Summarize `report` for the Budget tab as of `as_of`.
///
/// `end` is the report's inclusive end; the span starts on the first day of
/// its oldest bucket. Leaves are split into Income and Expenses by the
/// account's EFFECTIVE type ([`is_account_type`], subtypes folded — a declared
/// `type: G` goal is income), never by name. Those two are the whole
/// partition, as `budget_gaps` partitions: a goal on an account of neither
/// type (an asset transfer) has no bar.
///
/// # Errors
/// [`ReportError`] on overflow or a malformed bucket key.
pub fn budget_summary(
    report: &BudgetReport,
    declared: &BTreeMap<String, AccountType>,
    end: &str,
    as_of: &str,
) -> Result<BudgetSummary, ReportError> {
    let from = match report.buckets.first() {
        Some(first) => bucket_start(first)?,
        None => end.to_string(),
    };
    let elapsed = elapsed(&from, end, as_of);
    let leaves = budget_leaves(&summarize_budget(report)?);
    let sections = [
        (BudgetSectionKind::Revenue, AccountType::Revenue),
        (BudgetSectionKind::Expense, AccountType::Expense),
    ]
    .into_iter()
    .filter_map(|(kind, ty)| {
        let lines: Vec<BudgetLine> = leaves
            .iter()
            .filter(|line| is_account_type(&line.account, declared, ty))
            .cloned()
            .collect();
        (!lines.is_empty()).then(|| section(kind, &lines, elapsed))
    })
    .collect::<Result<Vec<_>, _>>()?;
    Ok(BudgetSummary {
        from,
        to: end.to_string(),
        as_of: as_of.to_string(),
        elapsed,
        sections,
    })
}

fn section(
    kind: BudgetSectionKind,
    lines: &[BudgetLine],
    elapsed: Elapsed,
) -> Result<BudgetSection, ReportError> {
    let (actual, goal) = budget_totals(lines)?;
    let bars = lines
        .iter()
        .map(|line| {
            let goal = line.goal.clone().unwrap_or_default();
            budget_bar(&line.account, &line.actual, &goal, elapsed)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BudgetSection {
        kind,
        overall: budget_bar("", &actual, &goal, elapsed)?,
        bars,
    })
}

/// The "Not budgeted" section's figures, all in magnitudes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapTotals {
    /// `|Σ revenue rows|`.
    pub revenue: MixedAmount,
    /// `|Σ expense rows|`.
    pub expense: MixedAmount,
    /// `|Σ revenue| + |Σ expense|` — the two sections are summed SEPARATELY and
    /// their magnitudes added. A straight sum would net unbudgeted income
    /// against unbudgeted spending and report the difference as "not
    /// budgeted": a smaller, friendlier number that answers nothing.
    pub headline: MixedAmount,
}

/// Totals for [`BudgetGaps`]. Each section's rows are disjoint subtrees at one
/// depth, so summing them does not double count.
///
/// # Errors
/// [`ReportError::Decimal`] on overflow.
pub fn gap_totals(gaps: &BudgetGaps) -> Result<GapTotals, ReportError> {
    let sum = |rows: &[super::budget::GapRow]| -> Result<MixedAmount, ReportError> {
        let mut total = MixedAmount::new();
        for row in rows {
            total.ma_add_assign(&row.total)?;
        }
        magnitude(&total)
    };
    let revenue = sum(&gaps.revenue)?;
    let expense = sum(&gaps.expense)?;
    let headline = revenue.ma_add(&expense)?;
    Ok(GapTotals {
        revenue,
        expense,
        headline,
    })
}

/// How many times a goal of `period` recurs in a year — the factor that lets a
/// $100 weekly goal and a $300 monthly one be compared.
#[must_use]
pub const fn annual_factor(period: PeriodExpr) -> u32 {
    match period {
        PeriodExpr::Daily => 365,
        PeriodExpr::Weekly => 52,
        PeriodExpr::Monthly => 12,
        PeriodExpr::Quarterly => 4,
        PeriodExpr::Yearly => 1,
    }
}

/// `amount` normalised to a year, exactly; `None` for a rule that is not a
/// bare fixed interval (`~ every 2 weeks`, `~ monthly from 2027`), which is NOT
/// guessed at — inventing a sort position for a rule Ledgeline will not model
/// would be the same claim the editor refuses to make.
///
/// # Errors
/// [`ReportError::Decimal`] on overflow.
pub fn annualised(
    amount: &MixedAmount,
    period: Option<PeriodExpr>,
) -> Result<Option<MixedAmount>, ReportError> {
    period
        .map(|period| {
            amount
                .ma_scale(Dec::new(i128::from(annual_factor(period)), 0))
                .map_err(ReportError::from)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Commodity;

    fn usd(mantissa: i128, places: u32) -> MixedAmount {
        MixedAmount::single(Commodity("$".into()), Dec::new(mantissa, places))
    }

    fn line(account: &str, actual: MixedAmount, goal: Option<MixedAmount>) -> BudgetLine {
        BudgetLine {
            depth: account.split(':').count(),
            account: account.into(),
            actual,
            goal,
        }
    }

    #[test]
    fn elapsed_clamps_and_counts_inclusive_days() {
        assert_eq!(
            elapsed("2026-01-01", "2026-01-31", "2025-12-31"),
            Elapsed { done: 0, span: 1 }
        );
        assert_eq!(
            elapsed("2026-01-01", "2026-01-31", "2026-01-01"),
            Elapsed { done: 1, span: 31 }
        );
        assert_eq!(
            elapsed("2026-01-01", "2026-01-31", "2026-01-31"),
            Elapsed::WHOLE
        );
        assert_eq!(
            elapsed("2026-01-01", "2026-01-31", "2027-01-01"),
            Elapsed::WHOLE
        );
        // A single-day span is whole on its own day; an inverted one is whole.
        assert_eq!(
            elapsed("2026-01-05", "2026-01-05", "2026-01-05"),
            Elapsed::WHOLE
        );
        assert_eq!(
            elapsed("2026-02-01", "2026-01-01", "2026-01-15"),
            Elapsed::WHOLE
        );
    }

    #[test]
    fn pace_is_the_exact_ratio_rounded_past_the_goal_scale() {
        // $1400 × 10/31 = 451.6129032258… → 451.612903 at 0 + 6 places.
        let pace = pace_amount(&usd(1400, 0), Elapsed { done: 10, span: 31 }).unwrap();
        assert_eq!(pace, usd(451_612_903, 6));
        // A whole span is the goal itself; an unstarted one is nothing.
        assert_eq!(
            pace_amount(&usd(1400, 0), Elapsed::WHOLE).unwrap(),
            usd(1400, 0)
        );
        assert!(
            pace_amount(&usd(1400, 0), Elapsed { done: 0, span: 1 })
                .unwrap()
                .is_empty()
        );
        // Negative goals round away from zero too.
        assert_eq!(
            pace_amount(&usd(-1, 0), Elapsed { done: 2, span: 3 }).unwrap(),
            usd(-666_667, 6)
        );
    }

    #[test]
    fn health_is_decided_exactly_at_the_pace_boundary() {
        let half = Elapsed { done: 1, span: 2 };
        // Expense: exactly on pace is healthy, a hair over is behind.
        assert_eq!(
            budget_health(Dec::new(50, 0), Dec::new(100, 0), half, false).unwrap(),
            BudgetHealth::Healthy
        );
        assert_eq!(
            budget_health(Dec::new(5001, 2), Dec::new(100, 0), half, false).unwrap(),
            BudgetHealth::Behind
        );
        assert_eq!(
            budget_health(Dec::new(101, 0), Dec::new(100, 0), half, false).unwrap(),
            BudgetHealth::Over
        );
        // Income: exactly on pace is healthy, earning past the goal is healthy.
        assert_eq!(
            budget_health(Dec::new(50, 0), Dec::new(100, 0), half, true).unwrap(),
            BudgetHealth::Healthy
        );
        assert_eq!(
            budget_health(Dec::new(4999, 2), Dec::new(100, 0), half, true).unwrap(),
            BudgetHealth::Behind
        );
        assert_eq!(
            budget_health(Dec::new(500, 0), Dec::new(100, 0), half, true).unwrap(),
            BudgetHealth::Healthy
        );
    }

    #[test]
    fn leaves_hide_parents_and_totals_keep_the_outermost_budget() {
        let lines = vec![
            line("expenses", usd(500, 0), Some(usd(450, 0))),
            line("expenses:food", usd(300, 0), Some(usd(300, 0))),
            line("expenses:food:dining", usd(120, 0), Some(usd(150, 0))),
            line("expenses:bus", usd(55, 0), Some(usd(60, 0))),
            line("<unbudgeted>", usd(-500, 0), None),
        ];
        let leaves: Vec<String> = budget_leaves(&lines)
            .into_iter()
            .map(|l| l.account)
            .collect();
        assert_eq!(leaves, vec!["expenses:food:dining", "expenses:bus"]);
        let (actual, goal) = budget_totals(&lines).unwrap();
        assert_eq!((actual, goal), (usd(500, 0), usd(450, 0)));
    }

    #[test]
    fn an_income_bar_reads_in_magnitudes() {
        let bar = budget_bar(
            "income:salary",
            &usd(-2000, 0),
            &usd(-6000, 0),
            Elapsed { done: 1, span: 2 },
        )
        .unwrap();
        assert!(bar.income);
        assert_eq!(bar.goal, usd(6000, 0));
        assert_eq!(bar.actual, usd(2000, 0));
        assert_eq!(bar.health, Some(BudgetHealth::Behind));
        assert_eq!(
            bar.label,
            BarLabel {
                kind: LabelKind::BehindPace,
                amount: Some(usd(1000, 0))
            }
        );
    }

    #[test]
    fn a_multi_commodity_bar_has_no_verdict() {
        let mut goal = usd(10, 0);
        goal.accumulate(&Commodity("EUR".into()), Dec::new(5, 0))
            .unwrap();
        let bar = budget_bar("expenses:mixed", &usd(3, 0), &goal, Elapsed::WHOLE).unwrap();
        assert_eq!(bar.health, None);
        assert_eq!(bar.label.kind, LabelKind::OnPlan);
    }

    #[test]
    fn annualised_scales_modelled_periods_only() {
        assert_eq!(
            annualised(&usd(100, 0), Some(PeriodExpr::Weekly)).unwrap(),
            Some(usd(5200, 0))
        );
        assert_eq!(annualised(&usd(100, 0), None).unwrap(), None);
        assert_eq!(
            annualised(&MixedAmount::new(), Some(PeriodExpr::Monthly)).unwrap(),
            Some(MixedAmount::new())
        );
    }
}
