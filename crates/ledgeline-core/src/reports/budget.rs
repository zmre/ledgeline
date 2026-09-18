//! Budget report — hledger `bal -M --budget`.
//!
//! This is NET-NEW logic (there is no TypeScript engine to port): budgets are
//! impossible via the hledger-web JSON API, so this is the native engine's
//! headline capability. The output is verified byte-for-byte against committed
//! hledger goldens (`fixtures/budget/*.budget.json`) in `tests/budget_golden.rs`.
//!
//! # hledger semantics reproduced here (inferred from the CLI + JSON oracle)
//!
//! Goals come from the journal's `~` periodic rules (fed in as
//! [`PeriodicTransaction`]s). The report is computed as two multi-period balance
//! reports that are then combined:
//!
//! - **Budgeted account set** — the union, over *all* periodic rules (regardless
//!   of `--budget=DESCPAT`), of every rule-posting account and its ancestors. It
//!   controls how actual postings are re-homed and is *not* narrowed by
//!   `DESCPAT`.
//! - **Actuals** — a normal per-bucket balance of the real transactions, but with
//!   every posting's account remapped: an account that is itself budgeted stays;
//!   otherwise it moves to its nearest budgeted ancestor; failing that, to a
//!   single synthetic `<unbudgeted>` account. Displayed amounts are
//!   subaccount-INCLUSIVE (even in flat mode).
//! - **Goals** — generated only from the rules selected by `DESCPAT`
//!   (case-insensitive substring of the rule description; all rules when absent).
//!   Each selected rule contributes its posting amounts once per occurrence of
//!   *its own* interval within the report span, bucketed by the *report*
//!   interval; parent goals are the aggregate of their children's.
//! - **Rows shown** — every account with a non-zero OWN amount (remapped actual
//!   or goal) in some bucket. Boring pass-through parents (no own amount) are
//!   elided; a shown row still displays its inclusive total. A cell's goal is
//!   `Some` iff the account is part of the selected goal tree (else `None`, as
//!   for `<unbudgeted>`); a budgeted account with no goal this bucket shows an
//!   empty (`Some`) goal.
//! - **Totals** — the per-bucket sum of every remapped actual / every goal.
//!
//! All money math is exact (`Dec` via [`MixedAmount`]); every fallible step is
//! surfaced through [`ReportError`].

use super::ReportError;
use super::aggregate::roll_up;
use super::mixed_amount::MixedAmount;
use super::periods::{
    Interval, add_days, add_months, bucket_key, bucket_span, bucket_start, clamped_date,
    compare_iso, days_between, last_n_buckets, parts, weekday_of,
};
use crate::model::{Anchor, PeriodExpr, PeriodKind, PeriodSpec, PeriodicTransaction, Transaction};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The synthetic account collecting actuals with no budgeted ancestor. Matches
/// hledger's literal `<unbudgeted>` pseudo-account name.
pub const UNBUDGETED: &str = "<unbudgeted>";

/// One account × bucket budget cell: the actual balance and, when the account is
/// part of the selected goal tree, its goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetCell {
    /// Subaccount-inclusive actual for the bucket.
    pub actual: MixedAmount,
    /// Subaccount-inclusive goal, or `None` when the account has no goal
    /// (e.g. `<unbudgeted>`, or an account budgeted only by a non-selected rule).
    pub goal: Option<MixedAmount>,
}

/// One budget report row: an account and its per-bucket cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetRow {
    /// Full, colon-delimited account name (clamped to the report depth; the
    /// synthetic [`UNBUDGETED`] name for the catch-all row).
    pub account: String,
    /// Number of `:`-separated segments in `account`.
    pub depth: usize,
    /// One cell per bucket, oldest → newest.
    pub cells: Vec<BudgetCell>,
}

/// A budget report: bucket keys, rows (union of shown accounts, sorted), and a
/// grand-total row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetReport {
    /// Bucket keys, oldest → newest.
    pub buckets: Vec<String>,
    /// Rows, sorted by account name (`<unbudgeted>` sorts first).
    pub rows: Vec<BudgetRow>,
    /// One grand-total cell per bucket.
    pub totals: Vec<BudgetCell>,
}

/// Budget report parameters.
#[derive(Debug, Clone)]
pub struct BudgetOpts<'a> {
    /// Inclusive report end (hledger's `-e DATE` minus one day).
    pub end: &'a str,
    /// Report bucketing interval.
    pub interval: Interval,
    /// Number of buckets ending with the one containing `end`.
    pub count: usize,
    /// Account depth limit (deeper accounts aggregate into their depth-`depth`
    /// ancestor). Use a large value for "no limit".
    pub depth: usize,
    /// `--budget=DESCPAT`: keep only rules whose description contains this
    /// (case-insensitive) substring. `None` selects all rules.
    pub budget_desc: Option<&'a str>,
}

/// Map a rule's fixed-interval UNIT to the report [`Interval`] its occurrences
/// are phased against.
const fn period_interval(unit: PeriodExpr) -> Interval {
    match unit {
        PeriodExpr::Daily => Interval::Daily,
        PeriodExpr::Weekly => Interval::Weekly,
        PeriodExpr::Monthly => Interval::Monthly,
        PeriodExpr::Quarterly => Interval::Quarterly,
        PeriodExpr::Yearly => Interval::Yearly,
    }
}

/// Hard cap on how many times one rule may fire inside one report span.
///
/// The span itself already bounds the walk — every step advances at least a day,
/// and the loop stops past the report end — so this is a backstop, not the
/// control. It bounds the one combination the span does not: [`MAX_BUCKETS`] is
/// 1200, and 1200 *yearly* buckets is twelve centuries, over which a daily rule
/// fires some 438,000 times. The pre-existing bucket-stepping walk had the same
/// exposure; this puts a number on it.
///
/// [`MAX_BUCKETS`]: super::periods::MAX_BUCKETS
const MAX_OCCURRENCES: usize = 100_000;

/// Whole calendar months from `a`'s month to `b`'s month (`b − a`), ignoring the
/// day. Negative when `b` precedes `a`.
fn months_between(a: &str, b: &str) -> i64 {
    let (ay, am, _) = parts(a);
    let (by, bm, _) = parts(b);
    (by * 12 + bm) - (ay * 12 + am)
}

/// The last day number of `year`-`month`.
///
/// Read back out of [`clamped_date`] rather than from a second month-length
/// table: 31 is at least as long as any month, so the clamp *is* the answer.
fn last_day_of(year: i64, month: i64) -> i64 {
    parts(&clamped_date(year, month, 31)).2
}

/// `steps` units after `anchor`, counted from the anchor rather than walked.
///
/// Counting matters for the month family. `~ monthly from 2026-01-31` fires
/// 01-31, 02-28, **03-31** (verified against hledger 1.52): each occurrence is
/// `anchor + n months` with the day clamped for its own target month. Walking
/// month by month would clamp once and stick at the 28th forever.
fn step_from(anchor: &str, unit: PeriodExpr, steps: i64) -> String {
    match unit {
        PeriodExpr::Daily => add_days(anchor, steps),
        PeriodExpr::Weekly => add_days(anchor, steps * 7),
        PeriodExpr::Monthly => add_months(anchor, steps),
        PeriodExpr::Quarterly => add_months(anchor, steps * 3),
        PeriodExpr::Yearly => add_months(anchor, steps * 12),
    }
}

/// The date an anchored rule fires on in `year`-`month`, or `None` when that
/// month has no such day (a 5th Tuesday in a four-Tuesday month).
fn anchored_in_month(year: i64, month: i64, anchor: Anchor) -> Option<String> {
    match anchor {
        Anchor::DayOfMonth(day) => Some(clamped_date(year, month, i64::from(day))),
        Anchor::NthWeekday { nth, weekday } => {
            let first = clamped_date(year, month, 1);
            // Days from the 1st to the first `weekday` of the month.
            let offset = (i64::from(weekday) - weekday_of(&first)).rem_euclid(7);
            let day = 1 + offset + 7 * (i64::from(nth) - 1);
            let (year, month, _) = parts(&first);
            (day <= last_day_of(year, month)).then(|| clamped_date(year, month, day))
        }
        // A weekday anchor belongs to the weekly family, which never asks.
        Anchor::Weekday(_) => None,
    }
}

/// The dates `spec` fires on within the inclusive report span `[start, end]`.
///
/// # What each bound means
///
/// Both were established by driving `hledger 1.52 bal -M --budget` over scratch
/// journals, because neither is guessable from the manual:
///
/// - **`from` is a phase anchor, not a filter.** `~ monthly from 2026-01-15`
///   fires on the 15th of every month. With no `from`, the phase comes from the
///   report span instead — the start of the rule-unit period containing the
///   report start — which is what makes a bare `~ weekly` report on Mondays.
/// - **`to` is exclusive.** `~ monthly from 2026 to 2028` fires exactly 24
///   times, 2026-01 through 2027-12.
///
/// A partial leading period is NOT counted: for a weekly rule reported from a
/// non-Monday start, the ISO week that merely *contains* the start began before
/// it and does not fire. That is hledger's behaviour and the pre-existing one,
/// pinned by the `weekly.journal` golden.
///
/// [`PeriodKind::Unsupported`] fires never. It is the one place this engine
/// knowingly under-reports rather than guessing at a recurrence it cannot state,
/// and the rule is locked in the editor so the user is told.
fn occurrences(start: &str, end: &str, spec: &PeriodSpec) -> Result<Vec<String>, ReportError> {
    let dates = match spec.kind {
        PeriodKind::Every { unit, multiplier } => every_dates(start, end, spec, unit, multiplier)?,
        PeriodKind::Anchored { anchor, .. } => anchored_dates(start, end, spec, anchor),
        PeriodKind::Annual { month, day } => annual_dates(start, end, spec, month, day),
        // `~ 2027-03-01`, or `~ from D to D2` spanning one period: the `from`
        // date is the occurrence. With neither, the span's own start is.
        PeriodKind::Once => vec![spec.start.clone().unwrap_or_else(|| start.to_string())],
        PeriodKind::Unsupported => Vec::new(),
    };
    Ok(dates
        .into_iter()
        .filter(|date| {
            compare_iso(date, start) != Ordering::Less
                && compare_iso(date, end) != Ordering::Greater
                && spec
                    .end
                    .as_deref()
                    .is_none_or(|to| compare_iso(date, to) == Ordering::Less)
        })
        .collect())
}

/// `daily` … `yearly`, with or without a multiplier.
fn every_dates(
    start: &str,
    end: &str,
    spec: &PeriodSpec,
    unit: PeriodExpr,
    multiplier: u32,
) -> Result<Vec<String>, ReportError> {
    let anchor = match spec.start.clone() {
        Some(from) => from,
        None => bucket_start(&bucket_key(start, period_interval(unit)))?,
    };
    // `.max(1)`: the parser refuses `every 0 weeks`, so a zero can only arrive
    // from a hand-built spec — but it would divide by zero below, and a report
    // is not a place to learn that. A zero step reads as one.
    let step = i64::from(multiplier).max(1);
    // Skip forward to the span rather than walking to it: a `from 1900-01-01`
    // daily rule would otherwise take 45,000 steps to reach a report it may not
    // even touch. Floor division (and `max(0)`), so the index this lands on is
    // never PAST the first occurrence inside the span — the occurrence one
    // before it necessarily falls in an earlier day/month than `start`.
    let span = match unit {
        PeriodExpr::Daily => days_between(&anchor, start) / step,
        PeriodExpr::Weekly => days_between(&anchor, start) / (step * 7),
        PeriodExpr::Monthly => months_between(&anchor, start) / step,
        PeriodExpr::Quarterly => months_between(&anchor, start) / (step * 3),
        PeriodExpr::Yearly => months_between(&anchor, start) / (step * 12),
    };
    let mut out = Vec::new();
    for n in span.max(0).. {
        let date = step_from(&anchor, unit, n * step);
        if compare_iso(&date, end) == Ordering::Greater || out.len() >= MAX_OCCURRENCES {
            break;
        }
        out.push(date);
    }
    Ok(out)
}

/// `every 15th day of month`, `every 3rd tuesday of month`, `every tuesday`.
///
/// The two families treat `from` differently, and hledger is the reason rather
/// than this module:
///
/// - **Monthly-anchored** periods are calendar months whose occurrence sits on
///   the anchored day, so `from` is an ordinary lower bound.
///   `every 15th day of month from 2026-03-16` first fires 04-15, not 03-15.
/// - **Weekly-anchored** periods run from the anchored weekday to the next one,
///   so `from` snaps DOWN onto one and the first occurrence may precede it.
///   `every tuesday from 2026-03-02` (a Monday) first fires 2026-02-24.
///
/// Both verified. They are two code paths in hledger and so are two here.
fn anchored_dates(start: &str, end: &str, spec: &PeriodSpec, anchor: Anchor) -> Vec<String> {
    let base = spec.start.as_deref().unwrap_or(start);
    if let Anchor::Weekday(weekday) = anchor {
        // Back up onto the anchored weekday on-or-before `base`.
        let back = (weekday_of(base) - i64::from(weekday)).rem_euclid(7);
        let first = add_days(base, -back);
        let skip = (days_between(&first, start) / 7).max(0);
        let mut out = Vec::new();
        for n in skip.. {
            let date = add_days(&first, n * 7);
            if compare_iso(&date, end) == Ordering::Greater || out.len() >= MAX_OCCURRENCES {
                break;
            }
            out.push(date);
        }
        return out;
    }
    // Monthly-anchored: one candidate month per month of the span. Bounded by
    // the span itself, so a month with no such day can simply be skipped without
    // risking a loop that never reaches its stop condition.
    let (year, month, _) = parts(base);
    let first = months_between(base, start).max(0);
    (first..=months_between(base, end).max(-1))
        .filter_map(|n| anchored_in_month(year, month + n, anchor))
        // `from` is a plain lower bound for this family.
        .filter(|date| {
            spec.start
                .as_deref()
                .is_none_or(|from| compare_iso(date, from) != Ordering::Less)
        })
        .collect()
}

/// `every 12/25` — once a year on a fixed month/day.
fn annual_dates(start: &str, end: &str, spec: &PeriodSpec, month: u32, day: u32) -> Vec<String> {
    let base = spec.start.as_deref().unwrap_or(start);
    let (base_year, _, _) = parts(base);
    let (last_year, _, _) = parts(end);
    (base_year..=last_year)
        .map(|year| clamped_date(year, i64::from(month), i64::from(day)))
        .filter(|date| {
            spec.start
                .as_deref()
                .is_none_or(|from| compare_iso(date, from) != Ordering::Less)
        })
        .collect()
}

/// Clamp a full account name to at most `depth` segments (`min` 1). Deeper
/// accounts collapse onto their depth-`depth` ancestor.
///
/// Returns a borrowed prefix: the clamped name is always a byte-prefix of the
/// input, so no allocation is needed to find it.
fn clip(account: &str, depth: usize) -> &str {
    match account.match_indices(':').nth(depth.max(1) - 1) {
        Some((cut, _)) => &account[..cut],
        None => account,
    }
}

/// Re-home an actual account under the budget tree: keep it if budgeted, else
/// move it to its nearest budgeted ancestor, else to [`UNBUDGETED`].
///
/// Walks the ancestry by slicing at each `:` from the right — nearest ancestor
/// first — rather than materializing every ancestor up front. The old version
/// `join(":")`ed all of them on every call, which is `O(S²)` bytes allocated per
/// account for an `S`-segment name (PERF-5), and it was called once per account
/// PER BUCKET.
///
/// The result always borrows from `account` (or is the `'static` [`UNBUDGETED`]),
/// which is what lets the caller memoize it without cloning.
fn remap_account<'a>(account: &'a str, budgeted: &BTreeSet<String>) -> &'a str {
    if budgeted.contains(account) {
        return account;
    }
    let mut name = account;
    while let Some(cut) = name.rfind(':') {
        name = &name[..cut];
        if budgeted.contains(name) {
            return name;
        }
    }
    UNBUDGETED
}

/// Add every commodity of `src` into `dst` (in place), preserving zeros for a
/// later single prune.
fn accumulate_into(dst: &mut MixedAmount, src: &MixedAmount) -> Result<(), ReportError> {
    for (commodity, qty) in src.iter() {
        dst.accumulate(commodity, *qty)?;
    }
    Ok(())
}

/// Compute the budget report for `txns`/`rules` under `opts`.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or unrecognized bucket math.
pub fn budget_report(
    txns: &[Transaction],
    rules: &[PeriodicTransaction],
    opts: &BudgetOpts,
) -> Result<BudgetReport, ReportError> {
    let buckets = last_n_buckets(opts.end, opts.interval, opts.count)?;

    // `last_n_buckets` documents an empty vec for `count == 0`. With no buckets
    // there is no report span to anchor the goal walk on, and `buckets[0]`
    // below would panic. Every per-bucket vec would be empty anyway, so the
    // empty report IS the correct answer for a zero-bucket span — return it
    // rather than indexing. All three fields stay present as empty arrays,
    // which is what the SPA's `decodeBudgetReport` requires.
    if buckets.is_empty() {
        return Ok(BudgetReport {
            buckets,
            rows: Vec::new(),
            totals: Vec::new(),
        });
    }

    // Budgeted account set: every rule-posting account + ancestors, ALL rules.
    let budgeted: BTreeSet<String> = rules
        .iter()
        .flat_map(|rule| &rule.postings)
        .flat_map(|posting| posting.account.self_and_ancestors())
        .collect();

    // Rules selected for goals by `--budget=DESCPAT` (all when absent/empty).
    let pattern = opts.budget_desc.map(str::to_lowercase);
    let selected: Vec<&PeriodicTransaction> = rules
        .iter()
        .filter(|rule| match &pattern {
            Some(pat) => rule.description.to_lowercase().contains(pat.as_str()),
            None => true,
        })
        .collect();

    // Accounts that make up the selected goal tree — controls goal `Some`/`None`.
    let goal_accts: BTreeSet<String> = selected
        .iter()
        .flat_map(|rule| &rule.postings)
        .flat_map(|posting| posting.account.self_and_ancestors())
        .collect();

    // --- Actuals: per bucket, remap + clip own totals, then roll up. ---
    //
    // Each bucket's inclusive `[start, to]` range, with the last one truncated
    // at `opts.end`. `last_n_buckets` yields CONTIGUOUS, non-overlapping buckets
    // oldest → newest, so the `to` bounds ascend strictly and every posting
    // falls in at most one of them — which is what lets the binary search below
    // replace one `account_totals` re-scan per bucket (PERF-5).
    let ranges: Vec<(String, String)> = buckets
        .iter()
        .map(|key| bucket_span(key, opts.end))
        .collect::<Result<_, ReportError>>()?;

    // ONE pass over every posting, summing per FULL account name into its own
    // bucket — i.e. exactly what `account_totals(from, to)` produced per bucket,
    // in the same transaction order. The remap+clip is deliberately NOT folded
    // into this pass: it merges several accounts onto one name, and merging
    // pruned per-account totals (as below) is not the same as merging their raw
    // postings, because a commodity that nets to zero within one account is
    // dropped before the merge and so cannot widen the merged scale.
    let mut per_bucket_direct: Vec<BTreeMap<&str, MixedAmount>> =
        vec![BTreeMap::new(); ranges.len()];
    for txn in txns {
        for posting in &txn.postings {
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            let index = ranges.partition_point(|(_, to)| to.as_str() < date);
            let Some((start, _)) = ranges.get(index) else {
                continue; // after the last bucket
            };
            if date < start.as_str() {
                continue; // before the report span
            }
            let entry = per_bucket_direct[index]
                .entry(posting.account.0.as_str())
                .or_default();
            for amount in &posting.amounts {
                entry.accumulate(&amount.commodity, amount.quantity)?;
            }
        }
    }

    // The remap+clip depends only on the account name and the (bucket-
    // independent) budgeted set, so it is resolved once per DISTINCT name
    // instead of once per account per bucket (PERF-5).
    let mut remapped: HashMap<&str, &str> = HashMap::new();
    let mut actual_own: Vec<BTreeMap<String, MixedAmount>> = Vec::with_capacity(buckets.len());
    let mut actual_incl: Vec<BTreeMap<String, MixedAmount>> = Vec::with_capacity(buckets.len());
    for direct in &per_bucket_direct {
        let mut own: BTreeMap<String, MixedAmount> = BTreeMap::new();
        for (account, ma) in direct {
            // `account_totals` prunes zero commodities in one final sweep,
            // BEFORE the remap merges accounts together.
            let mut pruned = ma.clone();
            pruned.drop_zeros();
            let name = *remapped
                .entry(account)
                .or_insert_with(|| clip(remap_account(account, &budgeted), opts.depth));
            accumulate_into(own.entry(name.to_string()).or_default(), &pruned)?;
        }
        for ma in own.values_mut() {
            ma.drop_zeros();
        }
        actual_incl.push(roll_up(&own)?);
        actual_own.push(own);
    }

    // --- Goals: per bucket, sum selected-rule occurrences, then roll up. ---
    let report_start = bucket_start(&buckets[0])?;
    let bucket_index: BTreeMap<&str, usize> = buckets
        .iter()
        .enumerate()
        .map(|(index, key)| (key.as_str(), index))
        .collect();
    let mut goal_own: Vec<BTreeMap<String, MixedAmount>> =
        (0..buckets.len()).map(|_| BTreeMap::new()).collect();
    for rule in &selected {
        for date in occurrences(&report_start, opts.end, &rule.period)? {
            let Some(&index) = bucket_index.get(bucket_key(&date, opts.interval).as_str()) else {
                continue;
            };
            for posting in &rule.postings {
                let name = clip(&posting.account.0, opts.depth).to_string();
                let entry = goal_own[index].entry(name).or_default();
                for amount in &posting.amounts {
                    entry.accumulate(&amount.commodity, amount.quantity)?;
                }
            }
        }
    }
    let goal_incl: Vec<BTreeMap<String, MixedAmount>> =
        goal_own.iter().map(roll_up).collect::<Result<_, _>>()?;

    // --- Rows: accounts with any non-zero OWN amount (actual or goal). ---
    let row_accounts: BTreeSet<String> = actual_own
        .iter()
        .chain(&goal_own)
        .flat_map(|own| own.iter())
        .filter(|(_, ma)| !ma.is_zero())
        .map(|(account, _)| account.clone())
        .collect();

    let rows = row_accounts
        .into_iter()
        .map(|account| {
            let in_goal_tree = goal_accts.contains(&account);
            let cells = (0..buckets.len())
                .map(|bucket| BudgetCell {
                    actual: actual_incl[bucket]
                        .get(&account)
                        .cloned()
                        .unwrap_or_default(),
                    goal: in_goal_tree
                        .then(|| goal_incl[bucket].get(&account).cloned().unwrap_or_default()),
                })
                .collect();
            BudgetRow {
                depth: account.split(':').count(),
                account,
                cells,
            }
        })
        .collect();

    // --- Totals: per-bucket sum of every remapped actual / every goal. ---
    let has_goals = !goal_accts.is_empty();
    let totals = (0..buckets.len())
        .map(|bucket| {
            let mut actual = MixedAmount::new();
            for ma in actual_own[bucket].values() {
                actual = actual.ma_add(ma)?;
            }
            let goal = if has_goals {
                let mut sum = MixedAmount::new();
                for ma in goal_own[bucket].values() {
                    sum = sum.ma_add(ma)?;
                }
                Some(sum)
            } else {
                None
            };
            Ok(BudgetCell { actual, goal })
        })
        .collect::<Result<_, ReportError>>()?;

    Ok(BudgetReport {
        buckets,
        rows,
        totals,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{txn, usd};
    use super::*;
    use crate::decimal::Dec;
    use crate::model::{
        AccountName, Amount, AmountStyle, Commodity, CommoditySide, PeriodExpr, Posting,
        PostingType, SourcePos, Status,
    };
    use std::path::PathBuf;

    /// A `MixedAmount` of `cents` USD (2 places).
    fn usd_ma(cents: i128) -> MixedAmount {
        MixedAmount::single(Commodity("$".into()), Dec::new(cents, 2))
    }

    /// A whole-dollar unbalanced-virtual (`(account)`) posting, mirroring how
    /// budget goals are written.
    fn goal_posting(account: &str, dollars: i128) -> Posting {
        Posting {
            status: Status::Unmarked,
            ptype: PostingType::Virtual,
            account: AccountName(account.to_string()),
            amounts: vec![Amount {
                commodity: Commodity("$".into()),
                quantity: Dec::new(dollars, 0),
                style: AmountStyle {
                    side: CommoditySide::Left,
                    spaced: false,
                    decimal_mark: Some('.'),
                    digit_groups: None,
                    precision: 0,
                },
                cost: None,
            }],
            balance_assertion: None,
            date: None,
            date2: None,
            comment: String::new(),
            tags: Vec::new(),
        }
    }

    /// A bare fixed-interval rule — `~ monthly`, `~ weekly`. The shape every
    /// fixture in this repo used before period expressions grew a grammar, kept
    /// as the default so those tests go on asserting what they always did.
    fn rule(period: PeriodExpr, description: &str, postings: Vec<Posting>) -> PeriodicTransaction {
        spec_rule(
            PeriodSpec {
                raw: crate::periodic::period_word(period).to_string(),
                kind: PeriodKind::Every { unit: period, multiplier: 1 },
                start: None,
                end: None,
            },
            description,
            postings,
        )
    }

    /// A rule with an arbitrary period spec.
    fn spec_rule(
        period: PeriodSpec,
        description: &str,
        postings: Vec<Posting>,
    ) -> PeriodicTransaction {
        PeriodicTransaction {
            period,
            description: description.to_string(),
            comment: String::new(),
            tags: Vec::new(),
            postings,
            // The report never reads a rule's position; only the editor does.
            source_span: (
                SourcePos { line: 1, column: 1 },
                SourcePos { line: 2, column: 1 },
            ),
            source_file: PathBuf::from("budget.journal"),
        }
    }

    fn row<'a>(report: &'a BudgetReport, account: &str) -> &'a BudgetRow {
        report
            .rows
            .iter()
            .find(|r| r.account == account)
            .unwrap_or_else(|| panic!("row {account} exists"))
    }

    fn opts<'a>(end: &'a str, count: usize, budget_desc: Option<&'a str>) -> BudgetOpts<'a> {
        BudgetOpts {
            end,
            interval: Interval::Monthly,
            count,
            depth: 99,
            budget_desc,
        }
    }

    /// A monthly food+bus goal with an unbudgeted cash leg: goals, an inclusive
    /// (subaccount-rolled) actual, and the `<unbudgeted>` catch-all.
    #[test]
    fn basic_goal_actual_and_unbudgeted() {
        let rules = vec![rule(
            PeriodExpr::Monthly,
            "household budget",
            vec![
                goal_posting("expenses:food", 400),
                goal_posting("expenses:bus", 20),
            ],
        )];
        let txns = vec![
            txn(
                1,
                "2026-01-05",
                vec![
                    ("expenses:food", vec![usd(35_200)]),
                    ("assets:checking", vec![usd(-35_200)]),
                ],
            ),
            txn(
                2,
                "2026-01-12",
                vec![
                    ("expenses:bus", vec![usd(2300)]),
                    ("assets:checking", vec![usd(-2300)]),
                ],
            ),
            // A subaccount without its own goal rolls into expenses:food.
            txn(
                3,
                "2026-01-20",
                vec![
                    ("expenses:food:dining", vec![usd(8000)]),
                    ("assets:checking", vec![usd(-8000)]),
                ],
            ),
        ];

        let report = budget_report(&txns, &rules, &opts("2026-01-31", 1, None)).unwrap();
        assert_eq!(report.buckets, ["2026-01"]);
        assert_eq!(
            report
                .rows
                .iter()
                .map(|r| r.account.as_str())
                .collect::<Vec<_>>(),
            ["<unbudgeted>", "expenses:bus", "expenses:food"]
        );

        // food actual is subaccount-inclusive ($352 + $80 dining) with a $400 goal.
        let food = &row(&report, "expenses:food").cells[0];
        assert_eq!(food.actual, usd_ma(43_200));
        assert_eq!(food.goal, Some(usd_ma(40_000)));

        let bus = &row(&report, "expenses:bus").cells[0];
        assert_eq!(bus.actual, usd_ma(2300));
        assert_eq!(bus.goal, Some(usd_ma(2000)));

        // <unbudgeted> = all cash, no goal.
        let unbudgeted = &row(&report, "<unbudgeted>").cells[0];
        assert_eq!(unbudgeted.actual, usd_ma(-45_500));
        assert_eq!(unbudgeted.goal, None);

        // Totals net to zero; goal is the sum of the goals.
        assert_eq!(report.totals[0].actual, MixedAmount::new());
        assert_eq!(report.totals[0].goal, Some(usd_ma(42_000)));
    }

    /// Goals on leaf accounts aggregate onto the parent row, whose inclusive
    /// actual includes an unbudgeted sibling remapped onto it.
    #[test]
    fn parent_goal_is_aggregated_from_children() {
        let rules = vec![rule(
            PeriodExpr::Monthly,
            "category goals",
            vec![
                goal_posting("expenses:food:groceries", 300),
                goal_posting("expenses:food:dining", 150),
            ],
        )];
        let txns = vec![
            txn(
                1,
                "2026-01-03",
                vec![
                    ("expenses:food:groceries", vec![usd(28_000)]),
                    ("assets:checking", vec![usd(-28_000)]),
                ],
            ),
            txn(
                2,
                "2026-01-09",
                vec![
                    ("expenses:food:dining", vec![usd(12_000)]),
                    ("assets:checking", vec![usd(-12_000)]),
                ],
            ),
            // Unbudgeted sibling: remaps onto expenses:food, keeping it visible.
            txn(
                3,
                "2026-01-14",
                vec![
                    ("expenses:food:snacks", vec![usd(2500)]),
                    ("assets:checking", vec![usd(-2500)]),
                ],
            ),
        ];

        let report = budget_report(&txns, &rules, &opts("2026-01-31", 1, None)).unwrap();
        // The parent row aggregates the children's goals ($300 + $150) and shows
        // the inclusive actual ($280 + $120 + $25 snacks).
        let food = &row(&report, "expenses:food").cells[0];
        assert_eq!(food.goal, Some(usd_ma(45_000)));
        assert_eq!(food.actual, usd_ma(42_500));
        // The bare `expenses` parent has no own amount → elided.
        assert!(report.rows.iter().all(|r| r.account != "expenses"));
    }

    /// `--budget=DESCPAT` narrows which rules supply goals; accounts budgeted
    /// only by a non-selected rule still show, but with a `None` goal.
    #[test]
    fn budget_descpat_selects_goals_only() {
        let rules = vec![
            rule(
                PeriodExpr::Monthly,
                "groceries and transit",
                vec![
                    goal_posting("expenses:food", 400),
                    goal_posting("expenses:bus", 30),
                ],
            ),
            rule(
                PeriodExpr::Monthly,
                "housing costs",
                vec![goal_posting("expenses:rent", 1500)],
            ),
        ];
        let txns = vec![
            txn(
                1,
                "2026-01-06",
                vec![
                    ("expenses:food", vec![usd(35_200)]),
                    ("assets:checking", vec![usd(-35_200)]),
                ],
            ),
            txn(
                2,
                "2026-01-15",
                vec![
                    ("expenses:rent", vec![usd(150_000)]),
                    ("assets:checking", vec![usd(-150_000)]),
                ],
            ),
        ];

        let report = budget_report(&txns, &rules, &opts("2026-01-31", 1, Some("housing"))).unwrap();
        // rent has a goal from the selected rule…
        assert_eq!(
            row(&report, "expenses:rent").cells[0].goal,
            Some(usd_ma(150_000))
        );
        // …food is still a row (budgeted by the other rule) but has no goal.
        let food = &row(&report, "expenses:food").cells[0];
        assert_eq!(food.actual, usd_ma(35_200));
        assert_eq!(food.goal, None);
        // Case-insensitive substring: "GROCER" matches "groceries and transit".
        let grocer = budget_report(&txns, &rules, &opts("2026-01-31", 1, Some("GROCER"))).unwrap();
        assert_eq!(
            row(&grocer, "expenses:food").cells[0].goal,
            Some(usd_ma(40_000))
        );
        assert_eq!(row(&grocer, "expenses:rent").cells[0].goal, None);
    }

    /// SEC-2: `count == 0` gives `last_n_buckets` an empty vec, and
    /// `bucket_start(&buckets[0])` panicked with `index out of bounds`. It must
    /// return the well-formed empty report instead — all three fields present
    /// and empty, which is what the SPA's `decodeBudgetReport` requires.
    #[test]
    fn zero_count_yields_an_empty_report_not_a_panic() {
        let rules = vec![rule(
            PeriodExpr::Monthly,
            "household budget",
            vec![goal_posting("expenses:food", 400)],
        )];
        let txns = vec![txn(
            1,
            "2026-01-05",
            vec![
                ("expenses:food", vec![usd(35_200)]),
                ("assets:checking", vec![usd(-35_200)]),
            ],
        )];

        let report = budget_report(&txns, &rules, &opts("2026-01-31", 0, None)).unwrap();
        assert!(report.buckets.is_empty());
        assert!(report.rows.is_empty());
        assert!(report.totals.is_empty());
    }

    /// The empty-bucket guard must not disturb the ordinary path: one bucket
    /// still reports normally with the very same inputs.
    #[test]
    fn one_count_still_reports_normally() {
        let rules = vec![rule(
            PeriodExpr::Monthly,
            "household budget",
            vec![goal_posting("expenses:food", 400)],
        )];
        let txns = vec![txn(
            1,
            "2026-01-05",
            vec![
                ("expenses:food", vec![usd(35_200)]),
                ("assets:checking", vec![usd(-35_200)]),
            ],
        )];

        let report = budget_report(&txns, &rules, &opts("2026-01-31", 1, None)).unwrap();
        assert_eq!(report.buckets, ["2026-01"]);
        assert_eq!(report.totals.len(), 1);
        assert_eq!(
            row(&report, "expenses:food").cells[0].actual,
            usd_ma(35_200)
        );
    }

    /// A weekly rule contributes once per occurrence within a monthly bucket.
    #[test]
    fn weekly_rule_sums_occurrences_into_monthly_bucket() {
        let rules = vec![rule(
            PeriodExpr::Weekly,
            "",
            vec![goal_posting("expenses:food", 100)],
        )];
        let report = budget_report(&[], &rules, &opts("2026-02-28", 2, None)).unwrap();
        // Jan and Feb 2026 each have 4 Monday occurrences (5/12/19/26 and
        // 2/9/16/23); hledger does NOT clip a partial first week, so both months
        // are 4 × $100 = $400 (verified against the weekly.journal golden).
        assert_eq!(report.buckets, ["2026-01", "2026-02"]);
        let food = row(&report, "expenses:food");
        assert_eq!(food.cells[0].goal, Some(usd_ma(40_000)));
        assert_eq!(food.cells[1].goal, Some(usd_ma(40_000)));
    }

    // -----------------------------------------------------------------------
    // Period grammar: every expectation below was read off `hledger 1.52
    // bal -M --budget` before it was written down.
    // -----------------------------------------------------------------------

    /// A spec built the way the parser builds one, for a test that cares about
    /// the shape rather than the spelling.
    fn spec(kind: PeriodKind, start: Option<&str>, end: Option<&str>) -> PeriodSpec {
        PeriodSpec {
            raw: "(test)".to_string(),
            kind,
            start: start.map(str::to_string),
            end: end.map(str::to_string),
        }
    }

    /// The goal in each bucket of a one-rule report over Jan–Jun 2026, in whole
    /// dollars, so a test reads as the row hledger prints.
    fn monthly_goals(period: PeriodSpec) -> Vec<i128> {
        let rules = vec![spec_rule(period, "", vec![goal_posting("expenses:x", 10)])];
        let report = budget_report(&[], &rules, &opts("2026-06-30", 6, None)).unwrap();
        match report.rows.iter().find(|r| r.account == "expenses:x") {
            Some(row) => row
                .cells
                .iter()
                .map(|cell| {
                    // Whole dollars, whatever scale the accumulation settled on.
                    cell.goal.as_ref().map_or(0, |goal| {
                        goal.iter().next().map_or(0, |(_, dec)| {
                            dec.mantissa / 10_i128.pow(dec.places)
                        })
                    })
                })
                .collect(),
            // No row at all is the honest answer for a rule that never fires.
            None => vec![0; 6],
        }
    }

    /// `from` is a phase ANCHOR, not a lower bound: a mid-month `from` moves
    /// every occurrence to that day of the month. `to` is EXCLUSIVE.
    #[test]
    fn from_anchors_the_phase_and_to_is_exclusive() {
        let monthly = PeriodKind::Every { unit: PeriodExpr::Monthly, multiplier: 1 };
        // Unbounded: every month.
        assert_eq!(monthly_goals(spec(monthly, None, None)), [10; 6]);
        // `from` in the middle of the span: March onward.
        assert_eq!(
            monthly_goals(spec(monthly, Some("2026-03-01"), None)),
            [0, 0, 10, 10, 10, 10]
        );
        // `to 2026-05-01` is exclusive, so May does NOT fire.
        assert_eq!(
            monthly_goals(spec(monthly, Some("2026-03-01"), Some("2026-05-01"))),
            [0, 0, 10, 10, 0, 0]
        );
        // `to 2026-05-15` DOES admit the May 1 occurrence — the bound is
        // compared against the occurrence date, with no snapping.
        assert_eq!(
            monthly_goals(spec(monthly, Some("2026-03-01"), Some("2026-05-15"))),
            [0, 0, 10, 10, 10, 0]
        );
        // A mid-month anchor fires on the 15th, so `to 2026-05-15` now excludes
        // May: the occurrence falls exactly on the exclusive bound.
        assert_eq!(
            monthly_goals(spec(monthly, Some("2026-03-15"), Some("2026-05-15"))),
            [0, 0, 10, 10, 0, 0]
        );
    }

    /// A multiplier steps N units at a time, phased from `from` when there is
    /// one and from the report span otherwise.
    #[test]
    fn multiplier_steps_and_takes_its_phase_from_the_bound() {
        let every_2_months = PeriodKind::Every { unit: PeriodExpr::Monthly, multiplier: 2 };
        // No bound: phase comes from the span start (January).
        assert_eq!(
            monthly_goals(spec(every_2_months, None, None)),
            [10, 0, 10, 0, 10, 0]
        );
        // `from` February shifts the whole sequence.
        assert_eq!(
            monthly_goals(spec(every_2_months, Some("2026-02-01"), None)),
            [0, 10, 0, 10, 0, 10]
        );
        // A `from` BEFORE the span still sets the phase.
        assert_eq!(
            monthly_goals(spec(every_2_months, Some("2025-12-01"), None)),
            [0, 10, 0, 10, 0, 10]
        );
    }

    /// `~ 2026-04-15` contributes to exactly one bucket, and nothing outside the
    /// span contributes at all.
    #[test]
    fn once_fires_in_a_single_bucket() {
        assert_eq!(
            monthly_goals(spec(PeriodKind::Once, Some("2026-04-15"), None)),
            [0, 0, 0, 10, 0, 0]
        );
        assert_eq!(
            monthly_goals(spec(PeriodKind::Once, Some("2027-04-15"), None)),
            [0; 6]
        );
    }

    /// `every 12/25` fires once a year, so a Jan–Jun report sees it never.
    #[test]
    fn annual_fires_on_its_month_and_day() {
        let christmas = PeriodKind::Annual { month: 12, day: 25 };
        assert_eq!(monthly_goals(spec(christmas, None, None)), [0; 6]);
        let june = PeriodKind::Annual { month: 6, day: 30 };
        assert_eq!(monthly_goals(spec(june, None, None)), [0, 0, 0, 0, 0, 10]);
    }

    /// An anchored rule fires once per month regardless of which day it names,
    /// so a monthly report cannot tell it from `~ monthly` — which is exactly
    /// what hledger shows.
    #[test]
    fn monthly_anchored_fires_once_per_month() {
        for anchor in [
            Anchor::DayOfMonth(15),
            Anchor::NthWeekday { nth: 3, weekday: 2 },
        ] {
            let kind = PeriodKind::Anchored { unit: PeriodExpr::Monthly, anchor };
            assert_eq!(monthly_goals(spec(kind, None, None)), [10; 6], "{anchor:?}");
        }
        // A `from` past the anchored day skips that month: hledger's
        // `every 15th day of month from 2026-03-16` first fires 04-15.
        let fifteenth = PeriodKind::Anchored {
            unit: PeriodExpr::Monthly,
            anchor: Anchor::DayOfMonth(15),
        };
        assert_eq!(
            monthly_goals(spec(fifteenth, Some("2026-03-16"), None)),
            [0, 0, 0, 10, 10, 10]
        );
    }

    /// A weekday-anchored rule fires 4 or 5 times a month. 2026 Tuesdays:
    /// Jan 4, Feb 4, Mar 5, Apr 4, May 4, Jun 5 — verified against hledger.
    #[test]
    fn weekly_anchored_counts_its_weekday_per_month() {
        let tuesdays = PeriodKind::Anchored {
            unit: PeriodExpr::Weekly,
            anchor: Anchor::Weekday(2),
        };
        assert_eq!(
            monthly_goals(spec(tuesdays, None, None)),
            [40, 40, 50, 40, 40, 50]
        );
    }

    /// A rule whose period this engine cannot state contributes no goals. It is
    /// deliberate under-reporting rather than a guess — and the editor locks the
    /// rule, so the user is told rather than left to wonder.
    #[test]
    fn unsupported_period_contributes_nothing() {
        assert_eq!(
            monthly_goals(spec(PeriodKind::Unsupported, None, None)),
            [0; 6]
        );
    }

    /// The regression that matters most: a bare fixed interval must produce the
    /// dates the pre-grammar bucket walk produced, or every committed golden
    /// moves. Asserted directly on `occurrences` for all five units.
    #[test]
    fn bare_intervals_still_walk_bucket_starts() {
        let bare = |unit| spec(PeriodKind::Every { unit, multiplier: 1 }, None, None);
        // Monthly/quarterly/yearly from a bucket-aligned start.
        assert_eq!(
            occurrences("2026-01-01", "2026-06-30", &bare(PeriodExpr::Monthly)).unwrap(),
            [
                "2026-01-01",
                "2026-02-01",
                "2026-03-01",
                "2026-04-01",
                "2026-05-01",
                "2026-06-01"
            ]
        );
        assert_eq!(
            occurrences("2026-01-01", "2026-12-31", &bare(PeriodExpr::Quarterly)).unwrap(),
            ["2026-01-01", "2026-04-01", "2026-07-01", "2026-10-01"]
        );
        assert_eq!(
            occurrences("2026-01-01", "2027-12-31", &bare(PeriodExpr::Yearly)).unwrap(),
            ["2026-01-01", "2027-01-01"]
        );
        // Weekly phases on ISO Mondays and drops the partial leading week —
        // 2026-01-01 is a Thursday, and its week began 2025-12-29.
        assert_eq!(
            occurrences("2026-01-01", "2026-01-31", &bare(PeriodExpr::Weekly)).unwrap(),
            ["2026-01-05", "2026-01-12", "2026-01-19", "2026-01-26"]
        );
        assert_eq!(
            occurrences("2026-01-01", "2026-01-04", &bare(PeriodExpr::Daily))
                .unwrap()
                .len(),
            4
        );
    }
}
