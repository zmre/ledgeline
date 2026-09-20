//! Projections — a what-if scenario read forward in time.
//!
//! A [`Scenario`] is the model of a projection file (`plans/22-projections.md`,
//! §"The file format"): a set of recurring [`ScenarioLine`]s and dated
//! [`ScenarioEvent`]s, both expressed as unbalanced virtual postings exactly as
//! budget goals are. [`project`] reads one forward over a run of buckets and
//! answers the three questions the tab asks — what net income does this imply,
//! what does cash do, and what does net worth do — plus the one that actually
//! matters: when does the cash run out.
//!
//! # What this module does NOT decide
//!
//! It does not decide whether a line is income or spending from its name, or
//! from a tag. Every classification here goes through
//! [`super::reports::account_types`]' effective type, which is the rule
//! everywhere else in the engine: a chart of accounts written in Spanish, or one
//! that books costs under `cogs:`, projects correctly without saying anything
//! extra. A `$2M` raise posting to `assets:cash` and `equity:preferred` moves
//! cash and net worth and leaves net income alone, because that is what those
//! accounts ARE — not because the file said so.
//!
//! # The two derived legs
//!
//! A projection line has no funding leg, so the balance-sheet effect is implied.
//! Stated once, and tested:
//!
//! > For each GROUP (one `~` rule, or one dated event), the implied leg is the
//! > negation of the sum of that group's revenue and expense postings — i.e. its
//! > net income. It is contributed to the CASH series unless the group already
//! > posts to a cash-like account, and to the NET WORTH series unless the group
//! > already posts to an asset or liability account.
//!
//! The two guards are what stop a user who *does* write `(assets:checking)
//! $-4200` beside their rent line from having the rent counted twice. Cash-
//! likeness is the predicate the cash-flow report uses
//! ([`AccountTypes::is_cash`]), so the two reports cannot disagree about what
//! counts as cash.
//!
//! # Growth is stepwise
//!
//! A line marked `+3%/yr` holds flat for twelve months and then bumps 3%, from
//! the new base. That is how a rent increase or a salary review actually lands,
//! and it means the amount is a real number a user could have typed — so it is
//! rounded back to the amount's own display precision AT EVERY STEP, not once at
//! the end.

use std::collections::{BTreeMap, BTreeSet};

use crate::decimal::{Dec, DecError};
use crate::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, PeriodExpr, PeriodKind, PeriodSpec,
    PeriodicTransaction, Posting, PostingType, PriceDirective, Status, Transaction,
};
use crate::reports::ReportError;
use crate::reports::account_types::{AccountType, AccountTypes};
use crate::reports::aggregate::{at_depth, roll_up};
use crate::reports::budget::{BudgetOpts, budget_gaps, occurrences};
use crate::reports::mixed_amount::MixedAmount;
use crate::reports::net_worth::{NetWorthOpts, net_worth};
use crate::reports::periods::{
    Interval, bucket_end, bucket_key, bucket_start, clamped_date, compare_iso, days_between,
    next_bucket, next_n_buckets, parts,
};
use crate::reports::prices::{PriceDb, infer_market_prices, value_at};
use crate::reports::types::{PeriodReport, PeriodRow};

// ===========================================================================
// The scenario model
// ===========================================================================

/// The unit a growth rate is quoted per.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowthUnit {
    /// `3%/wk`.
    Week,
    /// `3%/mo`.
    Month,
    /// `3%/yr`.
    Year,
}

impl GrowthUnit {
    /// The tag spelling, and the wire spelling — one table, so the file format
    /// and the JSON cannot drift.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }

    /// Read a unit from a `growth:` tag or a wire field, case-insensitively.
    /// Accepts the long and short spellings a user might write.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "week" | "weekly" | "wk" | "w" => Some(Self::Week),
            "month" | "monthly" | "mo" | "m" => Some(Self::Month),
            "year" | "yearly" | "yr" | "y" | "annual" | "annually" => Some(Self::Year),
            _ => None,
        }
    }
}

/// A stepwise growth rate: `rate` per completed `unit`.
///
/// `rate` is a FRACTION, not a percentage — `3%/yr` is `Dec::new(3, 2)`. The
/// percent sign belongs to the file format and the UI; keeping the model in
/// fractions means the engine never has to remember to divide by a hundred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Growth {
    /// The fractional rate applied per completed unit (`0.03` for `3%`).
    pub rate: Dec,
    /// The unit the rate is quoted per.
    pub unit: GrowthUnit,
}

/// Where a scenario line came from, so the UI can say so.
///
/// A seeded scenario mixes two kinds of row and they are not equally
/// trustworthy: one is what the user's own `~` rules say, the other is an
/// estimate this engine derived from twelve months of history. A table that
/// showed them identically would be claiming the second is as authored as the
/// first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSource {
    /// Read from a `~` rule in a journal (or typed by the user).
    Journal,
    /// Derived from [`budget_gaps`]: a category with activity that no goal
    /// measures, at its trailing average.
    Unbudgeted,
}

impl LineSource {
    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Journal => "journal",
            Self::Unbudgeted => "unbudgeted",
        }
    }
}

/// One recurring row of the what-if table.
///
/// # `id` versus `group`
///
/// They answer different questions and the engine needs both.
///
/// - `id` is the LOGICAL ROW. A step change — payroll jumps in April and stays
///   up — is two bounded segments of one row, sharing an `id`, which is what
///   lets the editor show them as a single line with a "from" on it.
/// - `group` is the SOURCE RULE: every posting of one `~` block shares it. It is
///   what the two derived-leg guards are stated over (see the module docs), and
///   a model with only `id` has nothing to group on — a rule that states its own
///   cash leg would have its spending counted twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLine {
    /// The logical row this segment belongs to.
    pub id: String,
    /// The source rule every posting of one `~` block shares.
    pub group: String,
    /// The account posted to, without any `(…)` wrapper.
    pub account: AccountName,
    /// The amount, signed as hledger signs it (revenue negative).
    pub amount: Amount,
    /// The recurrence, read by the same grammar the journal parser uses.
    pub period: PeriodSpec,
    /// Stepwise growth, or `None` for a flat line.
    pub growth: Option<Growth>,
    /// Free text for the row — the rule's description, or where a seeded row
    /// came from.
    pub note: String,
    /// Authored, or estimated from history.
    pub source: LineSource,
}

/// A dated one-off: its postings say whether it touches the P&L, the balance
/// sheet, or both.
///
/// An event is its own group, so its postings are read together by the two
/// derived-leg guards: `(assets:cash) $2M` beside `(equity:preferred) $-2M`
/// states its own asset leg and implies nothing, while a lone
/// `(expenses:legal) $45k` implies both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioEvent {
    /// A stable handle for the row. Not in `plans/22-projections.md`'s sketch;
    /// added because the editor needs a key for a row it can delete, and the
    /// engine needs a group key that cannot collide with a line's.
    pub id: String,
    /// The date it lands on, ISO `YYYY-MM-DD`.
    pub date: String,
    /// What it is, for the row and for a `~ DATE  DESCRIPTION` header.
    pub description: String,
    /// Its postings.
    pub postings: Vec<Posting>,
}

/// A whole what-if.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scenario {
    /// The display name — `; projection: <name>` in the file. May be empty; a
    /// file without the marker takes its name from its filename instead.
    pub name: String,
    /// `; created:` from the file, if it has one.
    pub created: Option<String>,
    /// `; updated:` from the file, if it has one.
    pub updated: Option<String>,
    /// The recurring rows.
    pub lines: Vec<ScenarioLine>,
    /// The dated one-offs.
    pub events: Vec<ScenarioEvent>,
}

// ===========================================================================
// Running a projection
// ===========================================================================

/// Inputs to [`project`].
#[derive(Debug, Clone)]
pub struct ProjectionOpts<'a> {
    /// Opening balances are taken as of this date; the first bucket is the next
    /// WHOLE one after it (decision 9 — a partial first period would make the
    /// first bar shorter than the rest for reasons that have nothing to do with
    /// the scenario).
    pub as_of: &'a str,
    /// Bucketing interval.
    pub interval: Interval,
    /// How many buckets to project.
    pub count: usize,
    /// Account-depth clamp for the net-income rows. The totals are summed over
    /// the unclamped accounts, so none of them move with it.
    pub depth: usize,
    /// Declared account types, so every classification here is by effective
    /// type rather than by root name.
    pub declared: &'a BTreeMap<String, AccountType>,
    /// Override the commodity opening balances are valued into.
    pub value_in: Option<Commodity>,
}

/// An opening balance and the closing balance at the end of each bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceSeries {
    /// The balance as of [`ProjectionOpts::as_of`], before any projected flow.
    pub opening: MixedAmount,
    /// The closing balance at the end of each bucket, oldest → newest.
    pub values: Vec<MixedAmount>,
}

/// Where the cash crosses zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runway {
    /// 0-based index into [`Projection::buckets`].
    pub bucket: usize,
    /// The last day of that bucket — the date the closing balance is negative
    /// as of, which is the date the series actually asserts something about.
    pub date: String,
    /// How many whole periods that is from the start of the projection
    /// (`bucket + 1`), so a caller can say "18 months" without knowing the
    /// interval's name.
    pub periods: usize,
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    /// Bucket keys, oldest → newest.
    pub buckets: Vec<String>,
    /// The first day of the first bucket. Echoed because decision 9 makes the
    /// start a derived fact the tab has to state, and a UI that re-derived it
    /// would be re-deriving "the next whole bucket".
    pub start: String,
    /// Projected revenue and expense accounts per bucket, in CASH-FLOW
    /// ORIENTATION — revenue positive (an inflow), expenses negative (an
    /// outflow) — so that `totals[i]` is the sum of `rows[i]` and IS net income.
    ///
    /// This is a deliberate sign flip away from the natural posting signs every
    /// other report here carries, and it is what makes the pair a
    /// [`PeriodReport`] rather than a report whose total contradicts its rows.
    /// It is also the orientation the chart wants: inflows above the axis,
    /// outflows below, net as the sum.
    pub net_income: PeriodReport,
    /// Cash, opening balance plus the projected cash deltas.
    pub cash: BalanceSeries,
    /// Net worth, opening balance plus the projected net-worth deltas.
    pub net_worth: BalanceSeries,
    /// The first bucket whose closing cash is negative, if any.
    pub runway: Option<Runway>,
    /// Everything the projection could not do. A projection that quietly drops a
    /// line is worse than one that says so.
    pub warnings: Vec<String>,
}

/// The most growth steps one line may be walked through.
///
/// Growth is applied one step at a time because each step rounds (see the module
/// docs), so the cost is linear in the number of completed units across the
/// span. The span already bounds that — `periods::MAX_BUCKETS` is 1200, and
/// 1200 yearly buckets is twelve centuries, over which a WEEKLY growth rate
/// completes some 62,000 units. This puts a number on it, in the same spirit as
/// the budget report's `MAX_OCCURRENCES`.
const MAX_GROWTH_STEPS: i64 = 100_000;

/// A group's postings for one bucket: account name → amount.
type GroupLegs = BTreeMap<String, MixedAmount>;

/// The scenario, laid out over the projected buckets.
///
/// A struct rather than a pile of `&mut` parameters: placing a line needs the
/// span, the bucket index, the per-bucket accumulator, the commodity set AND the
/// warning list, and threading six of those through two free functions is how
/// one of them silently stops being updated.
struct Layout<'a> {
    start: String,
    end: String,
    interval: Interval,
    bucket_index: BTreeMap<&'a str, usize>,
    /// `[bucket][group][account] → amount`.
    per_bucket: Vec<BTreeMap<String, GroupLegs>>,
    /// Every commodity the scenario contributes, for the mismatch warning.
    commodities: BTreeSet<Commodity>,
    warnings: Vec<String>,
}

impl<'a> Layout<'a> {
    fn new(buckets: &'a [String], start: String, end: String, interval: Interval) -> Self {
        Self {
            start,
            end,
            interval,
            bucket_index: buckets
                .iter()
                .enumerate()
                .map(|(index, key)| (key.as_str(), index))
                .collect(),
            per_bucket: (0..buckets.len()).map(|_| BTreeMap::new()).collect(),
            commodities: BTreeSet::new(),
            warnings: Vec::new(),
        }
    }

    /// Which bucket `date` falls in, or `None` when it is outside the span.
    fn bucket_of(&self, date: &str) -> Option<usize> {
        self.bucket_index
            .get(bucket_key(date, self.interval).as_str())
            .copied()
    }

    /// Place one recurring line's occurrences, stepping its growth as the dates
    /// advance.
    fn place_line(&mut self, line: &ScenarioLine) -> Result<(), ReportError> {
        let label = line_label(line);
        if line.period.kind == PeriodKind::Unsupported {
            self.warnings.push(format!(
                "{label}: the period '{}' is not a recurrence this engine can enumerate, \
                 so the line contributes nothing",
                line.period.raw
            ));
            return Ok(());
        }
        if line.growth.is_some() && line.period.kind == PeriodKind::Once {
            self.warnings.push(format!(
                "{label}: growth is set on a line that fires once, so it never applies"
            ));
        }

        // The growth anchor. With a `from`, growth is measured from the date the
        // segment starts — which is what makes the second half of a step change
        // grow from its NEW base rather than from the projection's start.
        let anchor = line
            .period
            .start
            .clone()
            .unwrap_or_else(|| self.start.clone());
        let mut current = MixedAmount::single(line.amount.commodity.clone(), line.amount.quantity);
        let mut applied: i64 = 0;
        let mut overflowed = false;

        // `occurrences` returns ASCENDING dates, so the step count is
        // non-decreasing and the growth walk is done once rather than
        // recomputed from the anchor per occurrence.
        for date in occurrences(&self.start, &self.end, &line.period)? {
            let Some(index) = self.bucket_of(&date) else {
                continue;
            };
            if let Some(growth) = &line.growth
                && !overflowed
            {
                let wanted = growth_steps(&anchor, &date, growth.unit).min(MAX_GROWTH_STEPS);
                let factor = Dec::new(1, 0).add(growth.rate)?;
                while applied < wanted {
                    match grow_once(&current, factor, line.amount.style.precision) {
                        Ok(next) => current = next,
                        // A compounding amount can outgrow `i128` long before the
                        // span does (10%/week over 1200 weeks is 10^49). That is
                        // a scenario this engine cannot state, not a server
                        // fault, so the line holds its last representable amount
                        // and SAYS so rather than failing the whole projection
                        // with a 500.
                        Err(_) => {
                            overflowed = true;
                            self.warnings.push(format!(
                                "{label}: growth overflowed the exact-decimal range after \
                                 {applied} steps; the line is held flat from there on"
                            ));
                            break;
                        }
                    }
                    applied += 1;
                }
            }
            self.commodities.insert(line.amount.commodity.clone());
            self.per_bucket[index]
                .entry(format!("line:{}", line.group))
                .or_default()
                .entry(line.account.0.clone())
                .or_default()
                .ma_add_assign(&current)?;
        }
        Ok(())
    }

    /// Place one dated event. An event is its own group, so its postings are
    /// read together by the two derived-leg guards.
    fn place_event(&mut self, event: &ScenarioEvent) -> Result<(), ReportError> {
        let Some(index) = self.bucket_of(&event.date) else {
            return Ok(());
        };
        for posting in &event.postings {
            let legs = self.per_bucket[index]
                .entry(format!("event:{}", event.id))
                .or_default()
                .entry(posting.account.0.clone())
                .or_default();
            for amount in &posting.amounts {
                self.commodities.insert(amount.commodity.clone());
                legs.accumulate(&amount.commodity, amount.quantity)?;
            }
        }
        Ok(())
    }
}

/// Project `scenario` forward over `opts.count` buckets.
///
/// `txns` and `prices` are the REAL journal's — they supply the opening
/// balances and nothing else. The scenario supplies every flow.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or unrecognized bucket math. A
/// scenario this engine cannot fully model is NOT an error: it produces
/// [`Projection::warnings`] and the part of the answer it can stand behind.
pub fn project(
    scenario: &Scenario,
    txns: &[Transaction],
    prices: &[PriceDirective],
    opts: &ProjectionOpts,
) -> Result<Projection, ReportError> {
    let mut warnings: Vec<String> = Vec::new();

    // --- The span: the next WHOLE bucket after `as_of`, and `count` of them. ---
    let first_key = next_bucket(&bucket_key(opts.as_of, opts.interval), opts.interval)?;
    let start = bucket_start(&first_key)?;
    let buckets = next_n_buckets(&start, opts.interval, opts.count)?;
    let end = match buckets.last() {
        Some(key) => bucket_end(key)?,
        // No buckets is no span. The openings are still real and still reported;
        // everything per-bucket is simply empty. (`count == 0` reaches here the
        // way it reaches `budget_report`'s own empty-report guard.)
        None => start.clone(),
    };
    // --- Every projected posting, per bucket, per group. ---
    let mut layout = Layout::new(&buckets, start.clone(), end, opts.interval);
    for line in &scenario.lines {
        layout.place_line(line)?;
    }
    for event in &scenario.events {
        layout.place_event(event)?;
    }
    warnings.append(&mut layout.warnings);

    // --- Opening balances, from the REAL journal. ---
    let mut all_prices = infer_market_prices(txns)?;
    all_prices.extend_from_slice(prices);
    let db = PriceDb::build(&all_prices);
    let target: Option<Commodity> = opts
        .value_in
        .clone()
        .or_else(|| db.base_commodity().cloned());
    let types = AccountTypes::from_declared(opts.declared.clone());

    let opening_cash = valued(
        &cash_balance_at(txns, opts.as_of, &types)?,
        target.as_ref(),
        &db,
        opts.as_of,
    )?;
    let opening_net_worth = net_worth(
        txns,
        prices,
        &NetWorthOpts {
            end: opts.as_of,
            interval: opts.interval,
            count: 1,
            // The total is summed over every asset/liability ACCOUNT, so it is
            // depth-independent; 1 is simply the cheapest rows to build.
            depth: 1,
            value_in: target.clone(),
            declared: opts.declared,
        },
    )?
    .totals
    .first()
    .cloned()
    .unwrap_or_default();

    warn_on_commodity_mismatch(&layout.commodities, target.as_ref(), &mut warnings);

    // --- Roll forward. ---
    let mut income_rows: Vec<BTreeMap<String, MixedAmount>> = Vec::with_capacity(buckets.len());
    let mut income_totals: Vec<MixedAmount> = Vec::with_capacity(buckets.len());
    let mut cash_values: Vec<MixedAmount> = Vec::with_capacity(buckets.len());
    let mut net_worth_values: Vec<MixedAmount> = Vec::with_capacity(buckets.len());
    let mut cash_running = opening_cash.clone();
    let mut net_worth_running = opening_net_worth.clone();

    for groups in &layout.per_bucket {
        let mut income_own: BTreeMap<String, MixedAmount> = BTreeMap::new();
        let mut cash_delta = MixedAmount::new();
        let mut net_worth_delta = MixedAmount::new();

        for legs in groups.values() {
            let mut profit_and_loss = MixedAmount::new();
            let mut cash = MixedAmount::new();
            let mut balance_sheet = MixedAmount::new();
            let (mut has_cash, mut has_balance_sheet) = (false, false);

            for (account, ma) in legs {
                if types.is_cash(account) {
                    has_cash = true;
                    cash.ma_add_assign(ma)?;
                }
                if types.is_type(account, AccountType::Asset)
                    || types.is_type(account, AccountType::Liability)
                {
                    has_balance_sheet = true;
                    balance_sheet.ma_add_assign(ma)?;
                }
                if types.is_type(account, AccountType::Revenue)
                    || types.is_type(account, AccountType::Expense)
                {
                    profit_and_loss.ma_add_assign(ma)?;
                    // Cash-flow orientation: revenue up, expenses down.
                    income_own
                        .entry(account.clone())
                        .or_default()
                        .ma_add_assign(&ma.ma_neg()?)?;
                }
            }

            // The implied leg IS this group's net income; see the module docs
            // for why each series takes it only when the group did not state
            // one of its own.
            let implied = profit_and_loss.ma_neg()?;
            cash_delta.ma_add_assign(&cash)?;
            if !has_cash {
                cash_delta.ma_add_assign(&implied)?;
            }
            net_worth_delta.ma_add_assign(&balance_sheet)?;
            if !has_balance_sheet {
                net_worth_delta.ma_add_assign(&implied)?;
            }
        }

        // The total is the sum of the OWN amounts, exactly as `cash_flow`'s is,
        // so it does not move with `depth`.
        let mut total = MixedAmount::new();
        for ma in income_own.values() {
            total.ma_add_assign(ma)?;
        }
        income_totals.push(total);
        income_rows.push(at_depth(&roll_up(&income_own)?, opts.depth));

        cash_running.ma_add_assign(&cash_delta)?;
        net_worth_running.ma_add_assign(&net_worth_delta)?;
        cash_values.push(cash_running.clone());
        net_worth_values.push(net_worth_running.clone());
    }

    let accounts: BTreeSet<String> = income_rows
        .iter()
        .flat_map(|bucket| bucket.keys().cloned())
        .collect();
    let rows: Vec<PeriodRow> = accounts
        .into_iter()
        .map(|account| PeriodRow {
            depth: account.split(':').count(),
            values: income_rows
                .iter()
                .map(|bucket| bucket.get(&account).cloned().unwrap_or_default())
                .collect(),
            account,
        })
        .collect();

    let runway = cash_values
        .iter()
        .position(|ma| ma.iter().any(|(_, qty)| qty.mantissa < 0))
        .map(|bucket| {
            Ok::<_, ReportError>(Runway {
                bucket,
                date: bucket_end(&buckets[bucket])?,
                periods: bucket + 1,
            })
        })
        .transpose()?;

    Ok(Projection {
        net_income: PeriodReport {
            buckets: buckets.clone(),
            rows,
            totals: income_totals,
            meta: None,
        },
        cash: BalanceSeries {
            opening: opening_cash,
            values: cash_values,
        },
        net_worth: BalanceSeries {
            opening: opening_net_worth,
            values: net_worth_values,
        },
        runway,
        warnings,
        buckets,
        start,
    })
}

/// How a warning names a line: its note when it has one, else its account.
fn line_label(line: &ScenarioLine) -> String {
    if line.note.trim().is_empty() {
        line.account.0.clone()
    } else {
        format!("{} ({})", line.account.0, line.note.trim())
    }
}

/// One growth step: scale by `factor`, then round every commodity back to the
/// amount's own display precision.
///
/// Rounding HERE rather than at the end is decision 4 of the plan: stepwise
/// growth means each step's figure is a number the user could have typed, so
/// rounding to it loses nothing — and a caller that compounded the raw products
/// would drift away from the amount the file would state if it were expanded.
fn grow_once(ma: &MixedAmount, factor: Dec, precision: u32) -> Result<MixedAmount, DecError> {
    let scaled = ma.ma_scale(factor)?;
    let mut out = MixedAmount::new();
    for (commodity, qty) in scaled.iter() {
        out.accumulate(commodity, qty.rounded(precision)?)?;
    }
    out.drop_zeros();
    Ok(out)
}

/// Whole growth units completed between `anchor` and `date` (never negative).
///
/// Months count ANNIVERSARIES, not calendar-month differences: from 2026-01-15,
/// 2026-02-14 is zero months and 2026-02-15 is one. The anniversary is clamped
/// into its target month exactly as a `~ monthly from 2026-01-31` rule's
/// occurrences are (`periods::clamped_date`), so a line anchored on the 31st
/// bumps on Feb 28 rather than waiting for March.
///
/// Years are whole months / 12, which lands on the same anniversary for the same
/// reason.
fn growth_steps(anchor: &str, date: &str, unit: GrowthUnit) -> i64 {
    match unit {
        GrowthUnit::Week => (days_between(anchor, date) / 7).max(0),
        GrowthUnit::Month => whole_months(anchor, date),
        GrowthUnit::Year => whole_months(anchor, date) / 12,
    }
}

/// Whole calendar months from `anchor` to `date`, counted by anniversary.
fn whole_months(anchor: &str, date: &str) -> i64 {
    let (anchor_year, anchor_month, anchor_day) = parts(anchor);
    let (year, month, _) = parts(date);
    let months = (year * 12 + month) - (anchor_year * 12 + anchor_month);
    if months <= 0 {
        return 0;
    }
    // The anniversary inside `date`'s own month, clamped to that month's length.
    let anniversary = clamped_date(year, month, anchor_day);
    if compare_iso(date, &anniversary) == std::cmp::Ordering::Less {
        months - 1
    } else {
        months
    }
}

/// Cumulative balance of every cash-like account at `as_of`.
///
/// One filtered pass rather than a bucketed walk: the projection needs a single
/// snapshot, and `cash_flow`'s per-bucket deltas would have to be re-summed to
/// produce it. The membership question is `AccountTypes::is_cash`, which is the
/// same predicate `cash_flow` is handed, so the opening balance and the report
/// above it cannot disagree about what counts as cash.
fn cash_balance_at(
    txns: &[Transaction],
    as_of: &str,
    types: &AccountTypes,
) -> Result<MixedAmount, ReportError> {
    let mut total = MixedAmount::new();
    for txn in txns {
        for posting in &txn.postings {
            let date = posting.date.as_deref().unwrap_or(&txn.date);
            if compare_iso(date, as_of) == std::cmp::Ordering::Greater {
                continue;
            }
            if !types.is_cash(&posting.account.0) {
                continue;
            }
            for amount in &posting.amounts {
                total.accumulate(&amount.commodity, amount.quantity)?;
            }
        }
    }
    total.drop_zeros();
    Ok(total)
}

/// Value `ma` into `target`, collapsing to a single-commodity amount — the same
/// shape `net_worth` gives its own figures, so the two opening balances are
/// denominated alike.
fn valued(
    ma: &MixedAmount,
    target: Option<&Commodity>,
    db: &PriceDb,
    as_of: &str,
) -> Result<MixedAmount, ReportError> {
    match target {
        None => Ok(ma.clone()),
        Some(commodity) => {
            let value = value_at(ma, commodity, db, as_of, None)?;
            Ok(MixedAmount::single(commodity.clone(), value))
        }
    }
}

/// Warn when a scenario's money is not the money the opening balances are in.
///
/// The projected flows are NOT valued — there are no market prices for a future
/// date, and hledger never looks ahead to one — so a EUR scenario added onto a
/// `$` opening balance produces a two-commodity series that is arithmetically
/// correct and financially meaningless. Saying so is the whole of what this
/// engine can honestly do about it.
fn warn_on_commodity_mismatch(
    commodities: &BTreeSet<Commodity>,
    target: Option<&Commodity>,
    warnings: &mut Vec<String>,
) {
    let Some(target) = target else {
        return;
    };
    let strangers: Vec<&str> = commodities
        .iter()
        .filter(|commodity| *commodity != target)
        .map(|commodity| commodity.0.as_str())
        .collect();
    if !strangers.is_empty() {
        warnings.push(format!(
            "the scenario is partly in {}, but opening balances are valued in {}; \
             the projected flows are added without conversion",
            strangers.join(", "),
            target.0
        ));
    }
}

// ===========================================================================
// Seeding a scenario from the journal
// ===========================================================================

/// Inputs to [`seed_scenario`].
#[derive(Debug, Clone)]
pub struct SeedOpts<'a> {
    /// The window the unbudgeted averages are measured over — the same struct
    /// `/api/budget/gaps` resolves, so the seeded figures cover the span the
    /// Budget tab would show for the same params.
    pub window: &'a BudgetOpts<'a>,
    /// Declared account types, for [`budget_gaps`]' revenue/expense split.
    pub declared: &'a BTreeMap<String, AccountType>,
    /// The journal's canonical commodity styles, so a seeded `EUR` amount keeps
    /// its right-hand symbol and comma decimal rather than being reinvented as
    /// `$`-shaped.
    pub styles: &'a [(Commodity, AmountStyle)],
}

/// Build a starting scenario from the journal: its `~` rules, plus one monthly
/// line per unbudgeted category at its trailing average.
///
/// # Why [`budget_gaps`] and not a second pass
///
/// "What does my budget not mention" is a question the budget tab already
/// answers, byte-for-byte, with a committed golden. Re-deriving it here would be
/// a second definition of "unbudgeted" — and the subtle half of that definition
/// (measure against the rule accounts THEMSELVES, not their ancestors) is
/// exactly the part a re-derivation gets wrong, producing an empty list for
/// every journal that has a budget.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or unrecognized bucket math.
pub fn seed_scenario(
    txns: &[Transaction],
    rules: &[PeriodicTransaction],
    opts: &SeedOpts,
) -> Result<Scenario, ReportError> {
    let mut lines: Vec<ScenarioLine> = Vec::new();

    // --- What the journal already says. ---
    for (rule_index, rule) in rules.iter().enumerate() {
        let group = format!("rule:{rule_index}");
        for (posting_index, posting) in rule.postings.iter().enumerate() {
            for amount in &posting.amounts {
                lines.push(ScenarioLine {
                    id: tag(&posting.tags, "line")
                        .map_or_else(|| format!("{group}:{posting_index}"), str::to_string),
                    group: group.clone(),
                    account: posting.account.clone(),
                    amount: amount.clone(),
                    period: rule.period.clone(),
                    // A posting's own `growth:` wins over the rule header's, the
                    // way a posting tag wins over a transaction tag everywhere
                    // else.
                    growth: parse_growth(&posting.tags).or_else(|| parse_growth(&rule.tags)),
                    note: rule.description.clone(),
                    source: LineSource::Journal,
                });
            }
        }
    }

    // --- What it does not. ---
    let gaps = budget_gaps(txns, rules, opts.declared, opts.window)?;
    // The divisor is the number of BUCKETS the window covers, which is what
    // "the 12-month average" means when the window is twelve monthly buckets.
    // The last bucket is truncated at the report end, so a part-month at the
    // right-hand edge makes the average slightly conservative — deliberately, a
    // projection that over-states income is the worse failure.
    let divisor = u32::try_from(opts.window.count).unwrap_or(u32::MAX).max(1);
    for row in gaps.revenue.iter().chain(&gaps.expense) {
        for (commodity, total) in row.total.iter() {
            let quantity = total.div_int(divisor)?;
            if quantity.is_zero() {
                continue;
            }
            let id = format!("gap:{}:{}", row.account, commodity.0);
            lines.push(ScenarioLine {
                group: id.clone(),
                id,
                account: AccountName(row.account.clone()),
                amount: Amount {
                    commodity: commodity.clone(),
                    quantity,
                    style: style_for(opts.styles, commodity, quantity.places),
                    cost: None,
                },
                period: monthly_spec(),
                growth: None,
                note: format!("unbudgeted — average over {} to {}", gaps.from, gaps.to),
                source: LineSource::Unbudgeted,
            });
        }
    }

    Ok(Scenario {
        // A seed has no name: naming it is what the Save As dialog is for, and a
        // placeholder here would be a name the user never chose showing up in a
        // filename.
        name: String::new(),
        created: None,
        updated: None,
        lines,
        events: Vec::new(),
    })
}

/// A bare `~ monthly` spec, spelled the way the parser spells one.
fn monthly_spec() -> PeriodSpec {
    PeriodSpec {
        raw: crate::periodic::period_word(PeriodExpr::Monthly).to_string(),
        kind: PeriodKind::Every {
            unit: PeriodExpr::Monthly,
            multiplier: 1,
        },
        start: None,
        end: None,
    }
}

/// The first value of `key` among `tags`.
fn tag<'a>(tags: &'a [(String, String)], key: &str) -> Option<&'a str> {
    tags.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// Read a `growth: 3%/yr` tag.
///
/// The percent sign is optional and the unit spelling is generous
/// ([`GrowthUnit::parse`]), because this is a tag a human types into a journal
/// by hand. An unreadable value is `None` — a flat line — rather than an error:
/// the file still has to open.
#[must_use]
pub fn parse_growth(tags: &[(String, String)]) -> Option<Growth> {
    let raw = tag(tags, "growth")?;
    let (rate_text, unit_text) = raw.split_once('/')?;
    let unit = GrowthUnit::parse(unit_text)?;
    let rate_text = rate_text.trim();
    let percent = rate_text.ends_with('%');
    let number = rate_text.trim_end_matches('%').trim();
    let value = Dec::parse(number, '.').ok()?;
    // `5%` is five hundredths. Scaling the mantissa by two decimal places rather
    // than dividing keeps it exact.
    let rate = if percent {
        Dec::new(value.mantissa, value.places.checked_add(2)?)
    } else {
        value
    };
    Some(Growth { rate, unit })
}

/// The journal's declared style for `commodity`, else a plain one at `places`.
fn style_for(
    styles: &[(Commodity, AmountStyle)],
    commodity: &Commodity,
    places: u32,
) -> AmountStyle {
    styles
        .iter()
        .find(|(declared, _)| declared == commodity)
        .map_or_else(
            || AmountStyle {
                side: CommoditySide::Left,
                spaced: false,
                decimal_mark: Some('.'),
                digit_groups: None,
                precision: places,
            },
            |(_, style)| style.clone(),
        )
}

/// An unbalanced-virtual posting, the form every projection line takes.
///
/// Exposed because the HTTP layer builds [`ScenarioEvent`] postings from a
/// request body and a second spelling of "a `(account) amount` posting with no
/// assertion, date or comment" is a second set of defaults to keep in step.
#[must_use]
pub fn virtual_posting(account: AccountName, amount: Amount) -> Posting {
    Posting {
        status: Status::Unmarked,
        ptype: PostingType::Virtual,
        account,
        amounts: vec![amount],
        balance_assertion: None,
        date: None,
        date2: None,
        comment: String::new(),
        tags: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SourcePos, Tindex};
    use crate::parse::parse_period_spec;
    use crate::reports::test_support::{amount, txn, usd};

    // -----------------------------------------------------------------------
    // Builders
    // -----------------------------------------------------------------------

    /// A recurring line. `period` goes through the REAL grammar, so a test that
    /// writes `"monthly to 2027-04-01"` asserts about the same reading a journal
    /// would get.
    fn line(group: &str, account: &str, money: Amount, period: &str) -> ScenarioLine {
        ScenarioLine {
            id: group.to_string(),
            group: group.to_string(),
            account: AccountName(account.to_string()),
            amount: money,
            period: parse_period_spec(period),
            growth: None,
            note: String::new(),
            source: LineSource::Journal,
        }
    }

    fn growing(mut line: ScenarioLine, rate: Dec, unit: GrowthUnit) -> ScenarioLine {
        line.growth = Some(Growth { rate, unit });
        line
    }

    fn event(id: &str, date: &str, postings: &[(&str, Amount)]) -> ScenarioEvent {
        ScenarioEvent {
            id: id.to_string(),
            date: date.to_string(),
            description: id.to_string(),
            postings: postings
                .iter()
                .map(|(account, money)| {
                    virtual_posting(AccountName((*account).to_string()), money.clone())
                })
                .collect(),
        }
    }

    fn scenario(lines: Vec<ScenarioLine>, events: Vec<ScenarioEvent>) -> Scenario {
        Scenario {
            name: "test".to_string(),
            created: None,
            updated: None,
            lines,
            events,
        }
    }

    /// Run a projection over an empty journal (so there are no openings and the
    /// figures are the scenario's own).
    fn run(scenario: &Scenario, as_of: &str, interval: Interval, count: usize) -> Projection {
        run_over(scenario, &[], as_of, interval, count)
    }

    fn run_over(
        scenario: &Scenario,
        txns: &[Transaction],
        as_of: &str,
        interval: Interval,
        count: usize,
    ) -> Projection {
        let declared = BTreeMap::new();
        run_typed(scenario, txns, as_of, interval, count, 99, &declared, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_typed(
        scenario: &Scenario,
        txns: &[Transaction],
        as_of: &str,
        interval: Interval,
        count: usize,
        depth: usize,
        declared: &BTreeMap<String, AccountType>,
        value_in: Option<Commodity>,
    ) -> Projection {
        project(
            scenario,
            txns,
            &[],
            &ProjectionOpts {
                as_of,
                interval,
                count,
                depth,
                declared,
                value_in,
            },
        )
        .expect("the projection computes")
    }

    fn usd_ma(cents: i128) -> MixedAmount {
        MixedAmount::single(Commodity("$".into()), Dec::new(cents, 2))
    }

    /// One account's row, per bucket, in whole units of its own commodity — so
    /// an expectation reads as the figures a user would see.
    fn row_units(projection: &Projection, account: &str) -> Vec<i128> {
        projection
            .net_income
            .rows
            .iter()
            .find(|row| row.account == account)
            .map_or_else(Vec::new, |row| row.values.iter().map(ma_units).collect())
    }

    fn totals_units(projection: &Projection) -> Vec<i128> {
        projection.net_income.totals.iter().map(ma_units).collect()
    }

    fn series_units(series: &BalanceSeries) -> Vec<i128> {
        series.values.iter().map(ma_units).collect()
    }

    /// The primary commodity's quantity, truncated to whole units.
    fn ma_units(ma: &MixedAmount) -> i128 {
        ma.iter()
            .next()
            .map_or(0, |(_, dec)| dec.mantissa / 10_i128.pow(dec.places))
    }

    // -----------------------------------------------------------------------
    // The span
    // -----------------------------------------------------------------------

    /// Decision 9: opening balances are as of today, and the first projected
    /// period is the next COMPLETE one. A partial first bucket would make the
    /// first bar shorter than the rest for reasons that have nothing to do with
    /// the scenario.
    #[test]
    fn the_projection_starts_at_the_next_whole_bucket() {
        let flat = scenario(
            vec![line("rent", "expenses:rent", usd(420_000), "monthly")],
            Vec::new(),
        );

        let monthly = run(&flat, "2026-09-20", Interval::Monthly, 3);
        assert_eq!(monthly.buckets, ["2026-10", "2026-11", "2026-12"]);
        assert_eq!(monthly.start, "2026-10-01");

        // The last day of a bucket is still INSIDE it, so the next one is next.
        let boundary = run(&flat, "2026-09-30", Interval::Monthly, 1);
        assert_eq!(boundary.buckets, ["2026-10"]);

        let quarterly = run(&flat, "2026-09-20", Interval::Quarterly, 2);
        assert_eq!(quarterly.buckets, ["2026-Q4", "2027-Q1"]);
        assert_eq!(quarterly.start, "2026-10-01");

        let yearly = run(&flat, "2026-09-20", Interval::Yearly, 2);
        assert_eq!(yearly.buckets, ["2027", "2028"]);
        assert_eq!(yearly.start, "2027-01-01");
    }

    /// `count == 0` has no span and must answer with the empty projection —
    /// openings intact — rather than indexing off the end of an empty bucket
    /// list (the SEC-2 guard `budget_report` carries).
    #[test]
    fn zero_count_yields_an_empty_projection_not_a_panic() {
        let flat = scenario(
            vec![line("rent", "expenses:rent", usd(420_000), "monthly")],
            Vec::new(),
        );
        let projection = run(&flat, "2026-09-20", Interval::Monthly, 0);
        assert!(projection.buckets.is_empty());
        assert!(projection.net_income.rows.is_empty());
        assert!(projection.net_income.totals.is_empty());
        assert!(projection.cash.values.is_empty());
        assert!(projection.net_worth.values.is_empty());
        assert_eq!(projection.runway, None);
        assert_eq!(projection.start, "2026-10-01");
    }

    // -----------------------------------------------------------------------
    // Flat lines
    // -----------------------------------------------------------------------

    /// The base case: one flat monthly expense over twelve buckets. Net income
    /// is the outflow, cash draws down by it, and net worth follows.
    #[test]
    fn a_flat_monthly_line_repeats_across_every_bucket() {
        let flat = scenario(
            vec![line("rent", "expenses:rent", usd(420_000), "monthly")],
            Vec::new(),
        );
        let projection = run(&flat, "2026-09-20", Interval::Monthly, 12);

        assert_eq!(projection.buckets.len(), 12);
        assert_eq!(projection.buckets.first().unwrap(), "2026-10");
        assert_eq!(projection.buckets.last().unwrap(), "2027-09");

        // Cash-flow orientation: an expense is an outflow, so the row is
        // NEGATIVE even though the posting is written positive.
        assert_eq!(row_units(&projection, "expenses:rent"), [-4200; 12]);
        assert_eq!(totals_units(&projection), [-4200; 12]);

        // …and the total IS the sum of the depth-1 rows, which is the
        // `PeriodReport` contract every other report here keeps.
        for (index, total) in projection.net_income.totals.iter().enumerate() {
            let summed = projection
                .net_income
                .rows
                .iter()
                .filter(|row| row.depth == 1)
                .try_fold(MixedAmount::new(), |acc, row| {
                    acc.ma_add(&row.values[index])
                })
                .unwrap();
            assert_eq!(*total, summed, "bucket {index}");
        }

        // No cash posting of its own, so the whole outflow is implied.
        let drawdown: Vec<i128> = (1..=12).map(|k| -4200 * k).collect();
        assert_eq!(series_units(&projection.cash), drawdown);
        assert_eq!(series_units(&projection.net_worth), drawdown);
    }

    /// A revenue line is an INFLOW: positive in the rows, positive in net
    /// income, and it builds cash.
    #[test]
    fn a_revenue_line_is_an_inflow() {
        let earning = scenario(
            vec![line("pay", "income:salary", usd(-1_200_000), "monthly")],
            Vec::new(),
        );
        let projection = run(&earning, "2026-09-20", Interval::Monthly, 3);
        assert_eq!(row_units(&projection, "income:salary"), [12_000; 3]);
        assert_eq!(totals_units(&projection), [12_000; 3]);
        assert_eq!(series_units(&projection.cash), [12_000, 24_000, 36_000]);
    }

    // -----------------------------------------------------------------------
    // Growth
    // -----------------------------------------------------------------------

    /// Decision 4: growth is STEPWISE. A line marked `+10%/yr` holds flat for
    /// twelve months and then bumps — on the anniversary, and not one month
    /// before it.
    #[test]
    fn growth_bumps_on_the_anniversary_and_not_before() {
        let rent = growing(
            line("rent", "expenses:rent", usd(100_000), "monthly"),
            Dec::new(10, 2),
            GrowthUnit::Year,
        );
        // 14 buckets from 2026-10: the twelfth anniversary month is 2027-10.
        let projection = run(
            &scenario(vec![rent], Vec::new()),
            "2026-09-20",
            Interval::Monthly,
            14,
        );
        let mut expected = vec![-1000; 12];
        expected.push(-1100); // 2027-10: the first bump
        expected.push(-1100); // 2027-11: still the new base
        assert_eq!(row_units(&projection, "expenses:rent"), expected);
    }

    /// The rounding is applied at EVERY STEP, not once at the end — which is
    /// the whole difference between a stepped figure a user could have typed
    /// and a compounded one they could not.
    ///
    /// $1000 at 5%/yr, whole dollars: 1050, 1103, 1158, 1216, **1277**.
    /// Compounding first and rounding once gives 1000 × 1.05^5 = 1276.28 →
    /// 1276. The fifth year is where the two answers part.
    #[test]
    fn growth_rounds_at_each_step_not_once_at_the_end() {
        let whole_dollars = amount("$", 1000, 0);
        assert_eq!(whole_dollars.style.precision, 0);
        let salary = growing(
            line("pay", "expenses:payroll", whole_dollars, "yearly"),
            Dec::new(5, 2),
            GrowthUnit::Year,
        );
        let projection = run(
            &scenario(vec![salary], Vec::new()),
            "2026-06-30",
            Interval::Yearly,
            6,
        );
        assert_eq!(
            projection.buckets,
            ["2027", "2028", "2029", "2030", "2031", "2032"]
        );
        assert_eq!(
            row_units(&projection, "expenses:payroll"),
            [-1000, -1050, -1103, -1158, -1216, -1277]
        );
    }

    /// A monthly rate bumps every month, from the line's OWN anchor rather than
    /// from the first of the month.
    #[test]
    fn a_monthly_rate_bumps_every_month_from_its_own_anchor() {
        let subscription = growing(
            line(
                "sub",
                "expenses:software",
                amount("$", 100, 0),
                "monthly from 2026-10-15",
            ),
            Dec::new(10, 2),
            GrowthUnit::Month,
        );
        let projection = run(
            &scenario(vec![subscription], Vec::new()),
            "2026-09-20",
            Interval::Monthly,
            4,
        );
        // Fires on the 15th of each month; the anchor is the first occurrence,
        // so the bump count is 0, 1, 2, 3.
        assert_eq!(
            row_units(&projection, "expenses:software"),
            [-100, -110, -121, -133]
        );
    }

    /// The anniversary is CLAMPED into its target month, exactly as a
    /// `~ monthly from 2026-01-31` rule's occurrences are: a line anchored on
    /// the 31st bumps on Feb 28 rather than waiting for March.
    #[test]
    fn growth_steps_count_anniversaries_not_calendar_differences() {
        assert_eq!(whole_months("2026-01-31", "2026-02-27"), 0);
        assert_eq!(whole_months("2026-01-31", "2026-02-28"), 1);
        assert_eq!(whole_months("2026-01-15", "2026-02-14"), 0);
        assert_eq!(whole_months("2026-01-15", "2026-02-15"), 1);
        // Never negative, and never confused by a date before the anchor.
        assert_eq!(whole_months("2027-04-01", "2026-12-01"), 0);
        assert_eq!(
            growth_steps("2027-04-01", "2026-12-01", GrowthUnit::Year),
            0
        );
        // Weeks are whole seven-day steps.
        assert_eq!(
            growth_steps("2026-10-01", "2026-10-07", GrowthUnit::Week),
            0
        );
        assert_eq!(
            growth_steps("2026-10-01", "2026-10-08", GrowthUnit::Week),
            1
        );
        // Years land on the twelfth month, not the eleventh.
        assert_eq!(
            growth_steps("2026-10-01", "2027-09-01", GrowthUnit::Year),
            0
        );
        assert_eq!(
            growth_steps("2026-10-01", "2027-10-01", GrowthUnit::Year),
            1
        );
    }

    /// Growth on a line that fires once never applies, and the projection says
    /// so rather than leaving the user to wonder.
    #[test]
    fn growth_on_a_one_shot_line_warns() {
        let once = growing(
            line("bonus", "income:bonus", usd(-500_000), "2027-01-15"),
            Dec::new(10, 2),
            GrowthUnit::Year,
        );
        let projection = run(
            &scenario(vec![once], Vec::new()),
            "2026-09-20",
            Interval::Monthly,
            6,
        );
        assert_eq!(
            row_units(&projection, "income:bonus"),
            [0, 0, 0, 5000, 0, 0]
        );
        assert!(
            projection
                .warnings
                .iter()
                .any(|warning| warning.contains("fires once")),
            "{:?}",
            projection.warnings
        );
    }

    // -----------------------------------------------------------------------
    // Step changes
    // -----------------------------------------------------------------------

    /// The shape the ask is really about: payroll jumps in April and STAYS up,
    /// growing from the new base. Two bounded rules for one account, and `to`
    /// is exclusive — verified against `hledger 1.52 bal --budget -M`, which
    /// bills Jan/Feb/Mar at $50k and Apr/May/Jun at $120k over the same file.
    #[test]
    fn a_two_segment_step_line_tiles_at_the_step_date() {
        let mut before = line(
            "rule:0",
            "expenses:payroll",
            usd(5_000_000),
            "monthly to 2027-04-01",
        );
        let mut after = growing(
            line(
                "rule:1",
                "expenses:payroll",
                usd(12_000_000),
                "monthly from 2027-04-01",
            ),
            Dec::new(5, 2),
            GrowthUnit::Year,
        );
        // One logical row, two segments.
        before.id = "payroll".to_string();
        after.id = "payroll".to_string();

        let projection = run(
            &scenario(vec![before, after], Vec::new()),
            "2026-12-31",
            Interval::Monthly,
            6,
        );
        assert_eq!(
            projection.buckets,
            [
                "2027-01", "2027-02", "2027-03", "2027-04", "2027-05", "2027-06"
            ]
        );
        // Exactly one segment fires per bucket: no month is billed twice, and
        // none is skipped at the seam.
        assert_eq!(
            row_units(&projection, "expenses:payroll"),
            [-50_000, -50_000, -50_000, -120_000, -120_000, -120_000]
        );
    }

    /// A step's growth is measured from ITS OWN start, so the second segment
    /// grows from the new base rather than from the projection's start.
    #[test]
    fn a_steps_growth_is_anchored_on_the_step_date() {
        let after = growing(
            line(
                "rule:1",
                "expenses:payroll",
                amount("$", 120_000, 0),
                "yearly from 2027-04-01",
            ),
            Dec::new(5, 2),
            GrowthUnit::Year,
        );
        let projection = run(
            &scenario(vec![after], Vec::new()),
            "2026-12-31",
            Interval::Yearly,
            3,
        );
        assert_eq!(projection.buckets, ["2027", "2028", "2029"]);
        // Fires 2027-04-01, 2028-04-01, 2029-04-01 — measuring growth from the
        // projection's own start would give a different sequence entirely.
        assert_eq!(
            row_units(&projection, "expenses:payroll"),
            [-120_000, -126_000, -132_300]
        );
    }

    // -----------------------------------------------------------------------
    // One-offs: the two kinds move different things
    // -----------------------------------------------------------------------

    /// A P&L one-off hits all three: net income, cash and net worth. Both
    /// balance-sheet legs are IMPLIED — the event writes neither, and the engine
    /// must not invent an equity posting to balance it.
    #[test]
    fn a_profit_and_loss_one_off_moves_net_income_cash_and_net_worth() {
        let legal = event("legal", "2027-06-15", &[("expenses:legal", usd(4_500_000))]);
        let projection = run(
            &scenario(Vec::new(), vec![legal]),
            "2027-03-31",
            Interval::Monthly,
            4,
        );
        assert_eq!(
            projection.buckets,
            ["2027-04", "2027-05", "2027-06", "2027-07"]
        );
        assert_eq!(row_units(&projection, "expenses:legal"), [0, 0, -45_000, 0]);
        assert_eq!(totals_units(&projection), [0, 0, -45_000, 0]);
        assert_eq!(series_units(&projection.cash), [0, 0, -45_000, -45_000]);
        assert_eq!(
            series_units(&projection.net_worth),
            [0, 0, -45_000, -45_000]
        );
        // Nothing invented an equity row to balance it.
        assert!(
            projection
                .net_income
                .rows
                .iter()
                .all(|row| !row.account.starts_with("equity")),
            "the P&L must not grow an equity row"
        );
    }

    /// A balance-sheet one-off — a $2M raise posting to cash and equity — moves
    /// cash and net worth and leaves net income ALONE. Decision 2: what an
    /// event hits is decided by the accounts it posts to, never by a tag.
    #[test]
    fn a_balance_sheet_one_off_moves_cash_and_net_worth_but_not_net_income() {
        let raise = event(
            "series-a",
            "2027-03-01",
            &[
                ("assets:cash", usd(200_000_000)),
                ("equity:preferred", usd(-200_000_000)),
            ],
        );
        let projection = run(
            &scenario(Vec::new(), vec![raise]),
            "2026-12-31",
            Interval::Monthly,
            4,
        );
        assert_eq!(
            projection.buckets,
            ["2027-01", "2027-02", "2027-03", "2027-04"]
        );
        // No revenue or expense posting, so there is no net income and no row.
        assert!(projection.net_income.rows.is_empty());
        assert_eq!(totals_units(&projection), [0, 0, 0, 0]);
        // …but the cash and the net worth are real.
        assert_eq!(series_units(&projection.cash), [0, 0, 2_000_000, 2_000_000]);
        assert_eq!(
            series_units(&projection.net_worth),
            [0, 0, 2_000_000, 2_000_000]
        );
    }

    /// Equity is not an asset or a liability, so an equity-only event moves
    /// nothing — and the asset leg beside it in the raise above is what stopped
    /// an implied leg being added there.
    #[test]
    fn an_equity_only_event_moves_nothing() {
        let reclass = event(
            "reclass",
            "2027-01-10",
            &[
                ("equity:preferred", usd(-100_000)),
                ("equity:common", usd(100_000)),
            ],
        );
        let projection = run(
            &scenario(Vec::new(), vec![reclass]),
            "2026-12-31",
            Interval::Monthly,
            2,
        );
        assert_eq!(series_units(&projection.cash), [0, 0]);
        assert_eq!(series_units(&projection.net_worth), [0, 0]);
        assert_eq!(totals_units(&projection), [0, 0]);
    }

    // -----------------------------------------------------------------------
    // The double-count guard
    // -----------------------------------------------------------------------

    /// A rule that states its OWN cash leg contributes no implied one. Without
    /// the guard the rent would be counted twice — once as the posting the user
    /// wrote, once as the leg the engine derives — and the cash line would fall
    /// at twice the rate the scenario says.
    #[test]
    fn a_rule_that_states_its_own_cash_leg_implies_nothing() {
        // One GROUP, two postings: exactly what one `~` block parses to.
        let rent = line("rule:0", "expenses:rent", usd(420_000), "monthly");
        let funding = line("rule:0", "assets:checking", usd(-420_000), "monthly");
        let with_leg = run(
            &scenario(vec![rent.clone(), funding], Vec::new()),
            "2026-09-20",
            Interval::Monthly,
            3,
        );
        assert_eq!(series_units(&with_leg.cash), [-4200, -8400, -12_600]);
        assert_eq!(series_units(&with_leg.net_worth), [-4200, -8400, -12_600]);
        // Net income is unaffected by the funding leg: `assets:checking` is
        // neither revenue nor expense.
        assert_eq!(totals_units(&with_leg), [-4200; 3]);

        // The same scenario WITHOUT the stated leg draws down identically,
        // because the implied leg is exactly the one the user wrote.
        let implied = run(
            &scenario(vec![rent], Vec::new()),
            "2026-09-20",
            Interval::Monthly,
            3,
        );
        assert_eq!(
            series_units(&implied.cash),
            series_units(&with_leg.cash),
            "the stated leg and the implied leg must agree"
        );
    }

    /// The guard is per GROUP, not per scenario: a second rule with no cash leg
    /// of its own still gets one, even when another rule stated theirs.
    #[test]
    fn the_guard_is_scoped_to_the_rule_that_states_the_leg() {
        let projection = run(
            &scenario(
                vec![
                    line("rule:0", "expenses:rent", usd(420_000), "monthly"),
                    line("rule:0", "assets:checking", usd(-420_000), "monthly"),
                    line("rule:1", "expenses:software", usd(90_000), "monthly"),
                ],
                Vec::new(),
            ),
            "2026-09-20",
            Interval::Monthly,
            2,
        );
        // −$4200 stated + −$900 implied, per bucket.
        assert_eq!(series_units(&projection.cash), [-5100, -10_200]);
        assert_eq!(totals_units(&projection), [-5100, -5100]);
    }

    // -----------------------------------------------------------------------
    // Runway
    // -----------------------------------------------------------------------

    /// The question the tab exists to answer. Opening cash comes from the REAL
    /// journal; the burn comes from the scenario.
    #[test]
    fn runway_is_the_first_bucket_whose_closing_cash_is_negative() {
        let burn = scenario(
            vec![line("burn", "expenses:burn", usd(400_000), "monthly")],
            Vec::new(),
        );
        let projection = run_over(&burn, &funded_journal(), "2026-09-20", Interval::Monthly, 6);

        assert_eq!(projection.cash.opening, usd_ma(1_000_000));
        assert_eq!(
            series_units(&projection.cash),
            [6000, 2000, -2000, -6000, -10_000, -14_000]
        );
        assert_eq!(
            projection.runway,
            Some(Runway {
                bucket: 2,
                date: "2026-12-31".to_string(),
                periods: 3,
            })
        );
    }

    /// A scenario that builds cash has no runway, and the engine says `None`
    /// rather than pointing at the last bucket.
    #[test]
    fn a_cash_positive_scenario_has_no_runway() {
        let earning = scenario(
            vec![line("pay", "income:salary", usd(-400_000), "monthly")],
            Vec::new(),
        );
        let projection = run_over(
            &earning,
            &funded_journal(),
            "2026-09-20",
            Interval::Monthly,
            6,
        );
        assert_eq!(projection.runway, None);
        assert_eq!(
            series_units(&projection.cash),
            [14_000, 18_000, 22_000, 26_000, 30_000, 34_000]
        );
        // Net worth opens at the real journal's own figure, not at zero.
        assert_eq!(projection.net_worth.opening, usd_ma(1_000_000));
    }

    /// $10,000 of cash, funded from equity — the opening balance every runway
    /// test starts from.
    fn funded_journal() -> Vec<Transaction> {
        vec![txn(
            1,
            "2026-01-10",
            vec![
                ("assets:bank:checking", vec![usd(1_000_000)]),
                ("equity:opening", vec![usd(-1_000_000)]),
            ],
        )]
    }

    // -----------------------------------------------------------------------
    // Degrading rather than failing
    // -----------------------------------------------------------------------

    /// A scenario this engine cannot enumerate produces WARNINGS and a zero
    /// projection — not an error, and not a silent zero.
    #[test]
    fn an_unsupported_period_warns_and_contributes_nothing() {
        let unsupported = line("odd", "expenses:odd", usd(100_000), "every weekday");
        assert_eq!(unsupported.period.kind, PeriodKind::Unsupported);

        let projection = run(
            &scenario(vec![unsupported], Vec::new()),
            "2026-09-20",
            Interval::Monthly,
            3,
        );
        assert_eq!(projection.buckets.len(), 3);
        assert!(projection.net_income.rows.is_empty());
        assert_eq!(totals_units(&projection), [0, 0, 0]);
        assert_eq!(series_units(&projection.cash), [0, 0, 0]);
        assert_eq!(projection.warnings.len(), 1);
        assert!(
            projection.warnings[0].contains("every weekday"),
            "the warning must quote what the user wrote: {:?}",
            projection.warnings
        );
    }

    /// A scenario denominated in money the opening balances are not in is
    /// arithmetically fine and financially meaningless, so it is flagged.
    #[test]
    fn a_commodity_the_openings_are_not_in_is_flagged() {
        let euros = scenario(
            vec![line(
                "rent",
                "expenses:rent",
                amount("EUR", 90_000, 2),
                "monthly",
            )],
            Vec::new(),
        );
        let declared = BTreeMap::new();
        let projection = run_typed(
            &euros,
            &funded_journal(),
            "2026-09-20",
            Interval::Monthly,
            2,
            99,
            &declared,
            // An explicit target, so there IS something to disagree with.
            Some(Commodity("$".into())),
        );
        assert!(
            projection
                .warnings
                .iter()
                .any(|warning| warning.contains("EUR") && warning.contains("without conversion")),
            "{:?}",
            projection.warnings
        );
    }

    // -----------------------------------------------------------------------
    // Classification is by TYPE, never by name
    // -----------------------------------------------------------------------

    /// A Spanish chart of accounts projects correctly, because every
    /// classification here goes through the declared `type:` — including the
    /// subtypes, which `is_account_type` folds into their parents.
    #[test]
    fn accounts_are_classified_by_declared_type_not_by_name() {
        let declared: BTreeMap<String, AccountType> = [
            ("ingresos:consultoria", AccountType::Revenue),
            ("cogs:infraestructura", AccountType::Expense),
            ("activo:banco", AccountType::Cash),
            ("plusvalia:acciones", AccountType::Gain),
        ]
        .into_iter()
        .map(|(name, ty)| (name.to_string(), ty))
        .collect();

        let scenario = scenario(
            vec![
                line("r0", "ingresos:consultoria", usd(-1_000_000), "monthly"),
                line("r1", "cogs:infraestructura", usd(600_000), "monthly"),
                // A `type: G` gain counts as revenue, as hledger's `type:R`
                // query says it does.
                line("r2", "plusvalia:acciones", usd(-50_000), "monthly"),
            ],
            Vec::new(),
        );
        let projection = run_typed(
            &scenario,
            &[],
            "2026-09-20",
            Interval::Monthly,
            1,
            99,
            &declared,
            None,
        );

        assert_eq!(row_units(&projection, "ingresos:consultoria"), [10_000]);
        assert_eq!(row_units(&projection, "cogs:infraestructura"), [-6000]);
        assert_eq!(row_units(&projection, "plusvalia:acciones"), [500]);
        assert_eq!(totals_units(&projection), [4500]);
    }

    /// A declared `type: C` cash account states its own cash leg, so the guard
    /// fires on a name no English heuristic would have recognised.
    #[test]
    fn the_cash_guard_honours_a_declared_cash_type() {
        let declared: BTreeMap<String, AccountType> = [
            ("cogs:infraestructura".to_string(), AccountType::Expense),
            ("activo:banco".to_string(), AccountType::Cash),
        ]
        .into_iter()
        .collect();
        let scenario = scenario(
            vec![
                line("rule:0", "cogs:infraestructura", usd(600_000), "monthly"),
                line("rule:0", "activo:banco", usd(-600_000), "monthly"),
            ],
            Vec::new(),
        );
        let projection = run_typed(
            &scenario,
            &[],
            "2026-09-20",
            Interval::Monthly,
            2,
            99,
            &declared,
            None,
        );
        assert_eq!(series_units(&projection.cash), [-6000, -12_000]);
    }

    /// `depth` clamps the ROWS and never the total, exactly as it does on every
    /// other period report here.
    #[test]
    fn depth_clamps_the_rows_and_never_the_total() {
        let scenario = scenario(
            vec![
                line("r0", "expenses:food:groceries", usd(30_000), "monthly"),
                line("r1", "expenses:food:dining", usd(12_000), "monthly"),
            ],
            Vec::new(),
        );
        let declared = BTreeMap::new();
        let shallow = run_typed(
            &scenario,
            &[],
            "2026-09-20",
            Interval::Monthly,
            1,
            1,
            &declared,
            None,
        );
        assert_eq!(
            shallow
                .net_income
                .rows
                .iter()
                .map(|row| row.account.as_str())
                .collect::<Vec<_>>(),
            ["expenses"]
        );
        assert_eq!(totals_units(&shallow), [-420]);
    }

    // -----------------------------------------------------------------------
    // Growth tags
    // -----------------------------------------------------------------------

    #[test]
    fn growth_tags_are_read_as_fractions() {
        let tagged = |value: &str| parse_growth(&[("growth".to_string(), value.to_string())]);

        let three_percent = tagged("3%/yr").expect("3%/yr reads");
        assert_eq!(three_percent.rate, Dec::new(3, 2));
        assert_eq!(three_percent.unit, GrowthUnit::Year);

        // The percent sign is optional, and a bare fraction means itself.
        assert_eq!(tagged("0.03/year").unwrap().rate, Dec::new(3, 2));
        // Fractional percentages survive: 2.5% is 0.025.
        assert_eq!(tagged("2.5%/mo").unwrap().rate, Dec::new(25, 3));
        assert_eq!(tagged("2.5%/mo").unwrap().unit, GrowthUnit::Month);
        assert_eq!(tagged("-1%/wk").unwrap().rate, Dec::new(-1, 2));
        assert_eq!(tagged("-1%/wk").unwrap().unit, GrowthUnit::Week);

        // An unreadable tag is a FLAT line, not a failed file.
        assert!(tagged("3%").is_none());
        assert!(tagged("3%/fortnight").is_none());
        assert!(tagged("lots/yr").is_none());
        assert!(parse_growth(&[]).is_none());
    }

    // -----------------------------------------------------------------------
    // Seeding
    // -----------------------------------------------------------------------

    fn goal_rule(description: &str, postings: Vec<Posting>) -> PeriodicTransaction {
        PeriodicTransaction {
            period: parse_period_spec("monthly"),
            description: description.to_string(),
            comment: String::new(),
            tags: Vec::new(),
            postings,
            source_span: (
                SourcePos { line: 1, column: 1 },
                SourcePos { line: 2, column: 1 },
            ),
            source_file: std::path::PathBuf::from("budget.journal"),
        }
    }

    fn goal(account: &str, money: Amount, tags: &[(&str, &str)]) -> Posting {
        let mut posting = virtual_posting(AccountName(account.to_string()), money);
        posting.tags = tags
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        posting
    }

    fn seed_txns() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2026-01-05",
                vec![
                    ("expenses:food:groceries", vec![usd(35_200)]),
                    ("assets:checking", vec![usd(-35_200)]),
                ],
            ),
            txn(
                2,
                "2026-01-12",
                vec![
                    ("expenses:insurance:auto", vec![usd(120_000)]),
                    ("assets:checking", vec![usd(-120_000)]),
                ],
            ),
            txn(
                3,
                "2026-01-25",
                vec![
                    ("income:consulting", vec![usd(-240_000)]),
                    ("assets:checking", vec![usd(240_000)]),
                ],
            ),
        ]
    }

    /// The seed window every test below shares: twelve monthly buckets to the
    /// end of 2026, at the depth the Budget tab opens on.
    fn seed_window() -> BudgetOpts<'static> {
        BudgetOpts {
            end: "2026-12-31",
            interval: Interval::Monthly,
            count: 12,
            depth: 2,
            budget_desc: None,
        }
    }

    fn seed(txns: &[Transaction], rules: &[PeriodicTransaction]) -> Scenario {
        seed_styled(txns, rules, &[])
    }

    fn seed_styled(
        txns: &[Transaction],
        rules: &[PeriodicTransaction],
        styles: &[(Commodity, AmountStyle)],
    ) -> Scenario {
        let declared = BTreeMap::new();
        seed_scenario(
            txns,
            rules,
            &SeedOpts {
                window: &seed_window(),
                declared: &declared,
                styles,
            },
        )
        .expect("the seed computes")
    }

    fn by_source(scenario: &Scenario, source: LineSource) -> Vec<&ScenarioLine> {
        scenario
            .lines
            .iter()
            .filter(|line| line.source == source)
            .collect()
    }

    /// The seed is the journal's own `~` rules plus what they do not cover, and
    /// the two are FLAGGED differently: one is what the user wrote, the other is
    /// an estimate from history.
    #[test]
    fn the_seed_carries_the_rules_and_the_gaps_and_marks_which_is_which() {
        let rules = vec![goal_rule(
            "household budget",
            vec![goal("expenses:food", usd(40_000), &[("growth", "3%/yr")])],
        )];
        let scenario = seed(&seed_txns(), &rules);

        let authored = by_source(&scenario, LineSource::Journal);
        assert_eq!(authored.len(), 1);
        assert_eq!(authored[0].account.0, "expenses:food");
        assert_eq!(authored[0].amount.quantity, Dec::new(40_000, 2));
        assert_eq!(authored[0].period.raw, "monthly");
        assert_eq!(authored[0].note, "household budget");
        // The posting's own `growth:` tag comes through.
        assert_eq!(
            authored[0].growth,
            Some(Growth {
                rate: Dec::new(3, 2),
                unit: GrowthUnit::Year,
            })
        );

        let estimated: Vec<(&str, i128)> = by_source(&scenario, LineSource::Unbudgeted)
            .iter()
            .map(|line| (line.account.0.as_str(), line.amount.quantity.mantissa))
            .collect();
        // `expenses:food` is budgeted, so it is not a gap; the insurance and the
        // income are, at a twelfth of their twelve-month totals.
        assert_eq!(
            estimated,
            [
                ("income:consulting", -20_000),
                ("expenses:insurance", 10_000)
            ]
        );
        // Every seeded gap line is monthly, flat, and says where it came from.
        for line in by_source(&scenario, LineSource::Unbudgeted) {
            assert_eq!(line.period.raw, "monthly");
            assert_eq!(line.growth, None);
            assert!(line.note.starts_with("unbudgeted"), "{}", line.note);
        }
        // A seed has no name: naming it is the Save As dialog's job.
        assert_eq!(scenario.name, "");
        assert_eq!(scenario.created, None);
    }

    /// Every posting of one `~` block shares a GROUP, so the double-count guard
    /// reads them together — while each keeps its own row id.
    #[test]
    fn a_rules_postings_share_a_group_but_not_an_id() {
        let rules = vec![goal_rule(
            "projection",
            vec![
                goal("expenses:rent", usd(420_000), &[]),
                goal("assets:checking", usd(-420_000), &[]),
            ],
        )];
        let seeded = seed(&[], &rules);
        let authored = by_source(&seeded, LineSource::Journal);
        assert_eq!(authored.len(), 2);
        assert_eq!(authored[0].group, authored[1].group);
        assert_ne!(authored[0].id, authored[1].id);

        // …and the guard fires on the seeded scenario, unchanged.
        let projection = run(&seeded, "2026-09-20", Interval::Monthly, 2);
        assert_eq!(series_units(&projection.cash), [-4200, -8400]);
    }

    /// A `line:` tag names the logical row, which is how two bounded segments of
    /// one step change arrive as one line in the editor.
    #[test]
    fn a_line_tag_names_the_logical_row() {
        let rules = vec![
            goal_rule(
                "before",
                vec![goal(
                    "expenses:payroll",
                    usd(5_000_000),
                    &[("line", "payroll")],
                )],
            ),
            goal_rule(
                "after",
                vec![goal(
                    "expenses:payroll",
                    usd(12_000_000),
                    &[("line", "payroll")],
                )],
            ),
        ];
        let seeded = seed(&[], &rules);
        let authored = by_source(&seeded, LineSource::Journal);
        assert_eq!(
            authored
                .iter()
                .map(|line| line.id.as_str())
                .collect::<Vec<_>>(),
            ["payroll", "payroll"]
        );
        assert_ne!(
            authored[0].group, authored[1].group,
            "two rules are two groups"
        );
    }

    /// A gap in two commodities becomes two lines. One [`ScenarioLine`] holds
    /// one [`Amount`], and collapsing the two would mean picking a commodity —
    /// which is a valuation the seed has no business performing.
    #[test]
    fn a_two_commodity_gap_seeds_one_line_per_commodity() {
        let txns = vec![txn(
            1,
            "2026-03-05",
            vec![
                (
                    "expenses:travel",
                    vec![usd(120_000), amount("EUR", 60_000, 2)],
                ),
                ("assets:checking", vec![usd(-120_000)]),
            ],
        )];
        let seeded = seed(&txns, &[]);
        let travel: Vec<(&str, i128)> = seeded
            .lines
            .iter()
            .filter(|line| line.account.0 == "expenses:travel")
            .map(|line| {
                (
                    line.amount.commodity.0.as_str(),
                    line.amount.quantity.mantissa,
                )
            })
            .collect();
        assert_eq!(travel, [("$", 10_000), ("EUR", 5000)]);
        // The ids are distinct, or the editor would show one row for two.
        let ids: BTreeSet<&str> = seeded.lines.iter().map(|line| line.id.as_str()).collect();
        assert_eq!(ids.len(), seeded.lines.len());
    }

    /// A seeded amount keeps the journal's own style for its commodity, so a
    /// `EUR` line is not reinvented as a `$`-shaped one.
    #[test]
    fn a_seeded_amount_keeps_the_journals_style() {
        let declared_style = AmountStyle {
            side: CommoditySide::Right,
            spaced: true,
            decimal_mark: Some(','),
            digit_groups: None,
            precision: 2,
        };
        let txns = vec![txn(
            1,
            "2026-03-05",
            vec![
                ("expenses:travel", vec![amount("EUR", 60_000, 2)]),
                ("assets:checking", vec![amount("EUR", -60_000, 2)]),
            ],
        )];
        let seeded = seed_styled(
            &txns,
            &[],
            &[(Commodity("EUR".into()), declared_style.clone())],
        );
        let travel = seeded
            .lines
            .iter()
            .find(|line| line.account.0 == "expenses:travel")
            .expect("travel is a gap");
        assert_eq!(travel.amount.style, declared_style);
    }

    /// The seed is the empty scenario for an empty journal rather than a
    /// failure, so a brand-new ledger opens the tab on an empty table.
    #[test]
    fn an_empty_journal_seeds_an_empty_scenario() {
        let seeded = seed(&[], &[]);
        assert!(seeded.lines.is_empty());
        assert!(seeded.events.is_empty());
    }

    /// The opening balances are a filtered SUM, so the order the journal states
    /// its transactions in cannot move them.
    #[test]
    fn opening_balances_do_not_depend_on_transaction_order() {
        let ascending = vec![
            txn(
                1,
                "2026-01-10",
                vec![
                    ("assets:bank:checking", vec![usd(1_000_000)]),
                    ("equity:opening", vec![usd(-1_000_000)]),
                ],
            ),
            txn(
                2,
                "2026-02-10",
                vec![
                    ("assets:bank:checking", vec![usd(-250_000)]),
                    ("expenses:food", vec![usd(250_000)]),
                ],
            ),
        ];
        let mut descending = ascending.clone();
        descending.reverse();
        descending[0].index = Tindex(1);
        descending[1].index = Tindex(2);

        let empty = scenario(Vec::new(), Vec::new());
        let a = run_over(&empty, &ascending, "2026-09-20", Interval::Monthly, 1);
        let b = run_over(&empty, &descending, "2026-09-20", Interval::Monthly, 1);
        assert_eq!(a.cash.opening, usd_ma(750_000));
        assert_eq!(a.cash.opening, b.cash.opening);
        assert_eq!(a.net_worth.opening, b.net_worth.opening);
    }

    /// A posting dated after the as-of is not an opening balance.
    #[test]
    fn opening_balances_stop_at_the_as_of_date() {
        let later = vec![
            txn(
                1,
                "2026-01-10",
                vec![
                    ("assets:bank:checking", vec![usd(1_000_000)]),
                    ("equity:opening", vec![usd(-1_000_000)]),
                ],
            ),
            txn(
                2,
                "2026-10-10",
                vec![
                    ("assets:bank:checking", vec![usd(500_000)]),
                    ("income:salary", vec![usd(-500_000)]),
                ],
            ),
        ];
        let empty = scenario(Vec::new(), Vec::new());
        let projection = run_over(&empty, &later, "2026-09-20", Interval::Monthly, 1);
        assert_eq!(projection.cash.opening, usd_ma(1_000_000));
    }
}
