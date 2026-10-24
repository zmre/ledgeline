//! A scenario, expanded into dated, BALANCED transactions (plan 28, Phase 1).
//!
//! This is what `hledger --forecast` does with a `~` rule, plus the two things
//! only Ledgeline does: stepwise `growth:` and asset appreciation. The output is
//! ordinary [`Transaction`]s, so every report that is a pure function over
//! `&[Transaction]` — the balance sheet, the income statement, the cash flow
//! statement — reads `real ++ forecast` without knowing a scenario exists.
//!
//! # The rules, each stated once
//!
//! - **The cutoff (Decision 5).** The forecast starts at the first day of the
//!   period containing the EARLIER of `today` and the day after the latest real
//!   transaction ([`forecast_cutoff`]). Every real figure is read strictly
//!   before it; nothing real on or after it counts. There is no mixed period.
//! - **Every line has a counter-account.** A line that names one
//!   ([`ScenarioLine::counter`]) is balanced against it, posting for posting.
//! - **Legacy input keeps its numbers.** Lines that name no counter are summed
//!   per group, per occurrence date; a non-zero sum (the old engine's
//!   "residual") is posted to the group's DEFAULT counter-account
//!   ([`CounterDefaults`], Decision 7). A group that already balances — a rule
//!   that states its own cash leg — gets nothing.
//! - **Appreciation on an UNPRICED asset is a posting** from the asset to its
//!   gains account ([`ScenarioLine::gains`], default
//!   [`DEFAULT_GAINS_ACCOUNT`]), at each growth step (Decision 3).
//! - **Appreciation on a PRICED holding is a price.** An asset row whose
//!   account holds commodities with a market price grows those PRICES, in
//!   memory, from each one's latest price before the cutoff
//!   ([`Forecast::prices`]). They are never written anywhere. A price belongs to
//!   the commodity, so a second row growing the same commodity at a different
//!   rate is warned about and the first row's rate is kept.
//!
//! Every generated transaction carries hledger's own marker,
//! `generated-transaction: ~ <period>` (and the hidden
//! `_generated-transaction`), so a reader filters them exactly as it would
//! filter hledger's: [`is_generated`].
//!
//! # What this module does not decide
//!
//! It never classifies an account by name. "Cash" is [`AccountTypes::is_cash`],
//! "asset" is [`AccountTypes::is_type`], exactly as everywhere else.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::decimal::Dec;
use crate::model::{
    AccountName, Amount, AmountStyle, Commodity, CommoditySide, PeriodKind, Posting, PostingType,
    PriceDirective, SourcePos, Status, Tindex, Transaction,
};
use crate::reports::ReportError;
use crate::reports::account_types::{AccountType, AccountTypes};
use crate::reports::budget::occurrences;
use crate::reports::mixed_amount::MixedAmount;
use crate::reports::net_worth::valued;
use crate::reports::periods::{
    Interval, add_days, add_months, bucket_end, bucket_key, bucket_start, compare_iso,
    last_n_buckets, next_n_buckets,
};
use crate::reports::prices::{PriceDb, infer_market_prices};

use super::LineRole;
use super::{
    AccountBalances, AssetRow, MAX_GROWTH_STEPS, Scenario, ScenarioEvent, ScenarioLine,
    account_balances_at, describe_type, grow_once, growth_boundaries, growth_steps, in_subtree,
    line_label, show, subtree_balance,
};

/// How many full periods before the cutoff a projection shows by default
/// (Decision 6). `0` is allowed.
pub const DEFAULT_LOOKBACK: usize = 2;

/// The gains account an asset row's appreciation is booked against when the
/// row names none (Decision 4). EQUITY, not income: paper gains do not touch
/// net income unless the user deliberately points them at an income account.
pub const DEFAULT_GAINS_ACCOUNT: &str = "equity:unrealized gains";

/// The tag hledger puts on every transaction `--forecast` generates.
pub const GENERATED_TAG: &str = "generated-transaction";

/// hledger's hidden twin of [`GENERATED_TAG`].
const HIDDEN_GENERATED_TAG: &str = "_generated-transaction";

/// The trailing window the default counter-account is inferred over: the
/// seed's own twelve months (`plans/22-projections.md` amendment 11), ending
/// the day before the cutoff.
const COUNTER_WINDOW_MONTHS: i64 = 12;

/// The counter-account of last resort, for a journal with no cash account at
/// all. hledger's own name heuristic reads it as Cash, so the runway still sees
/// the money even when nothing is declared.
const FALLBACK_CASH_ACCOUNT: &str = "assets:cash";

/// The fewest decimal places a grown PRICE is rounded to at each step. A
/// price's own precision is often whole units (`P … AAPL $100`), and rounding a
/// 1%/yr step on a $5 price to whole dollars would never let it grow.
const PRICE_FLOOR_PLACES: u32 = 4;

// ===========================================================================
// Public types
// ===========================================================================

/// The real journal a forecast is read against.
#[derive(Debug, Clone, Copy)]
pub struct History<'a> {
    /// Every real transaction, in any order. Only those before the cutoff are
    /// read.
    pub txns: &'a [Transaction],
    /// The journal's explicit `P` directives. Only those dated before the
    /// cutoff are used; the inferred cost prices are derived inside.
    pub prices: &'a [PriceDirective],
}

/// Inputs to [`forecast_transactions`].
#[derive(Debug, Clone)]
pub struct ForecastOpts<'a> {
    /// The caller's today. Passed in, never read from a clock.
    pub today: &'a str,
    /// The projection interval: the cutoff is a period start on it.
    pub interval: Interval,
    /// How many forecast periods, starting at the cutoff.
    pub count: usize,
    /// How many FULL periods before the cutoff the window also covers
    /// (Decision 6). Actuals only; nothing is generated there.
    pub lookback: usize,
    /// Override the valuation commodity; else the price database's base.
    pub value_in: Option<Commodity>,
}

/// The span a forecast covers, and where actuals end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForecastWindow {
    /// The first projected day (Decision 5): actuals are everything strictly
    /// before it. The wire's `actualsEnd` in Phase 4.
    pub cutoff: String,
    /// The first day of the first lookback period; `cutoff` when `lookback` is
    /// zero.
    pub from: String,
    /// The last day of the last forecast period; `cutoff` when `count` is zero.
    pub end: String,
    /// The interval every key below is on.
    pub interval: Interval,
    /// The full actual periods before the cutoff, oldest → newest.
    pub lookback: Vec<String>,
    /// The forecast periods, oldest → newest. The first one starts at `cutoff`.
    pub buckets: Vec<String>,
}

impl ForecastWindow {
    /// Resolve the window from `today` and the latest real transaction.
    ///
    /// # Errors
    /// [`ReportError::InvalidBucketKey`] from the bucket math (unreachable for
    /// valid ISO dates).
    pub fn resolve(
        today: &str,
        latest_real: Option<&str>,
        interval: Interval,
        lookback: usize,
        count: usize,
    ) -> Result<Self, ReportError> {
        let cutoff = forecast_cutoff(today, latest_real, interval)?;
        let lookback_keys = if lookback == 0 {
            Vec::new()
        } else {
            last_n_buckets(&add_days(&cutoff, -1), interval, lookback)?
        };
        let buckets = next_n_buckets(&cutoff, interval, count)?;
        let from = match lookback_keys.first() {
            Some(key) => bucket_start(key)?,
            None => cutoff.clone(),
        };
        let end = match buckets.last() {
            Some(key) => bucket_end(key)?,
            None => cutoff.clone(),
        };
        Ok(Self {
            cutoff,
            from,
            end,
            interval,
            lookback: lookback_keys,
            buckets,
        })
    }

    /// The last day of actuals: the day before the cutoff. Every opening
    /// balance is read as of this date.
    #[must_use]
    pub fn actuals_as_of(&self) -> String {
        add_days(&self.cutoff, -1)
    }
}

/// A scenario, expanded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forecast {
    /// The span, and where actuals end.
    pub window: ForecastWindow,
    /// The generated transactions, ascending by date, every one balanced and
    /// every one tagged [`GENERATED_TAG`]. Dated within `[cutoff, end]`.
    pub txns: Vec<Transaction>,
    /// In-memory future prices for priced holdings an asset row grows. Never
    /// serialized (Decision 3). Dated within `(cutoff, end]`.
    pub prices: Vec<PriceDirective>,
    /// One entry per asset row actually modelled, in scenario order.
    pub assets: Vec<AssetRow>,
    /// Everything the expansion could not do.
    pub warnings: Vec<String>,
    /// The commodity balances were valued into, if any.
    pub target: Option<Commodity>,
}

/// Whether `txn` is one a forecast generated, by hledger's own marker.
#[must_use]
pub fn is_generated(txn: &Transaction) -> bool {
    txn.tags
        .iter()
        .any(|(name, _)| name == GENERATED_TAG || name == HIDDEN_GENERATED_TAG)
}

/// The latest real transaction date, if the journal has any.
#[must_use]
pub fn latest_real_date(txns: &[Transaction]) -> Option<&str> {
    txns.iter()
        .filter(|txn| !is_generated(txn))
        .map(|txn| txn.date.as_str())
        .max_by(|a, b| compare_iso(a, b))
}

/// The cutoff (Decision 5): the start of the `interval` period containing the
/// earlier of `today` and the day after `latest_real`.
///
/// - today 2026-10-23, latest 2026-10-20, monthly → 2026-10-01;
/// - today 2026-11-10, latest 2026-10-15 → 2026-10-01 (October is incomplete);
/// - today 2026-11-10, latest 2026-10-31 → 2026-11-01 (October is complete).
///
/// # Errors
/// [`ReportError::InvalidBucketKey`] from the bucket math.
pub fn forecast_cutoff(
    today: &str,
    latest_real: Option<&str>,
    interval: Interval,
) -> Result<String, ReportError> {
    let anchor = match latest_real.map(|latest| add_days(latest, 1)) {
        Some(day_after) if compare_iso(&day_after, today) == std::cmp::Ordering::Less => day_after,
        _ => today.to_string(),
    };
    bucket_start(&bucket_key(&anchor, interval))
}

/// Real transactions before the cutoff, then the forecast: the merged list
/// every statement is computed over.
#[must_use]
pub fn merged_transactions(history: &[Transaction], forecast: &Forecast) -> Vec<Transaction> {
    history
        .iter()
        .filter(|txn| compare_iso(&txn.date, &forecast.window.cutoff) == std::cmp::Ordering::Less)
        .chain(&forecast.txns)
        .cloned()
        .collect()
}

/// Real `P` directives before the cutoff, then the forecast's in-memory ones.
#[must_use]
pub fn merged_prices(history: &[PriceDirective], forecast: &Forecast) -> Vec<PriceDirective> {
    history
        .iter()
        .filter(|price| before(&price.date, &forecast.window.cutoff))
        .chain(&forecast.prices)
        .cloned()
        .collect()
}

// ===========================================================================
// The default counter-account (Decision 7)
// ===========================================================================

/// Which account a line with no counter-account is balanced against.
///
/// In order:
///
/// 1. the CASH-typed account the line's account (or anything under it) was most
///    often posted against in a real transaction over the trailing twelve
///    months before the cutoff — salary that always landed in
///    `assets:checking` projects into `assets:checking`;
/// 2. else the scenario header's `; cash: <account>`;
/// 3. else the cash account with the largest balance at the cutoff;
/// 4. else the first DECLARED cash account, else [`FALLBACK_CASH_ACCOUNT`].
///
/// Ties go to the lexically first account, so the answer is deterministic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterDefaults {
    /// account → cash account → how many real transactions paired them.
    pairings: BTreeMap<String, BTreeMap<String, usize>>,
    /// The scenario header's `; cash:`.
    header: Option<AccountName>,
    /// Steps 3 and 4, resolved once.
    fallback: String,
}

impl CounterDefaults {
    /// Build the defaults from the real journal, as of `cutoff`.
    ///
    /// # Errors
    /// [`ReportError`] on decimal overflow.
    pub fn new(
        history: &History<'_>,
        types: &AccountTypes,
        cutoff: &str,
        header: Option<&AccountName>,
        value_in: Option<&Commodity>,
    ) -> Result<Self, ReportError> {
        let as_of = add_days(cutoff, -1);
        let prices = real_prices(history, cutoff)?;
        let db = PriceDb::build(&prices);
        let target = value_in.cloned().or_else(|| db.base_commodity().cloned());
        let balances = account_balances_at(history.txns, &as_of)?;
        Self::from_parts(
            history.txns,
            types,
            cutoff,
            header,
            &balances,
            target.as_ref(),
            &db,
        )
    }

    fn from_parts(
        txns: &[Transaction],
        types: &AccountTypes,
        cutoff: &str,
        header: Option<&AccountName>,
        balances: &AccountBalances<'_>,
        target: Option<&Commodity>,
        db: &PriceDb,
    ) -> Result<Self, ReportError> {
        let window_start = add_months(cutoff, -COUNTER_WINDOW_MONTHS);
        let mut pairings: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        for txn in txns {
            if !before(&txn.date, cutoff) || before(&txn.date, &window_start) {
                continue;
            }
            let cash: BTreeSet<&str> = txn
                .postings
                .iter()
                .map(|posting| posting.account.0.as_str())
                .filter(|account| types.is_cash(account))
                .collect();
            if cash.is_empty() {
                continue;
            }
            let accounts: BTreeSet<&str> = txn
                .postings
                .iter()
                .map(|posting| posting.account.0.as_str())
                .collect();
            for account in accounts {
                for cash_account in &cash {
                    if *cash_account == account {
                        continue;
                    }
                    *pairings
                        .entry(account.to_string())
                        .or_default()
                        .entry((*cash_account).to_string())
                        .or_insert(0) += 1;
                }
            }
        }

        // Step 3: the largest cash balance at the cutoff, valued the way the
        // opening cash is. Ties go to the lexically first, which a `BTreeMap`
        // walk and a strict `>` give for free.
        let mut largest: Option<(&str, Dec)> = None;
        for (account, balance) in balances {
            if !types.is_cash(account) {
                continue;
            }
            let value = valued(balance, target, db, &add_days(cutoff, -1), None)?
                .iter()
                .map(|(_, qty)| *qty)
                .try_fold(Dec::zero(), Dec::add)?;
            if largest.is_none_or(|(_, best)| value > best) {
                largest = Some((account, value));
            }
        }
        let fallback = largest.map_or_else(
            || {
                types
                    .declared()
                    .iter()
                    .find(|(_, ty)| **ty == AccountType::Cash)
                    .map_or_else(
                        || FALLBACK_CASH_ACCOUNT.to_string(),
                        |(name, _)| name.clone(),
                    )
            },
            |(account, _)| account.to_string(),
        );
        Ok(Self {
            pairings,
            header: header.cloned(),
            fallback,
        })
    }

    /// The default counter-account for one line's account.
    #[must_use]
    pub fn for_account(&self, account: &str) -> AccountName {
        self.for_accounts([account])
    }

    /// The default counter-account for a GROUP of lines: the pairing counts of
    /// every account named are pooled, so a one-line group answers exactly as
    /// [`CounterDefaults::for_account`] does.
    #[must_use]
    pub fn for_accounts<'x>(&self, accounts: impl IntoIterator<Item = &'x str>) -> AccountName {
        let roots: Vec<&str> = accounts.into_iter().collect();
        let mut pooled: BTreeMap<&str, usize> = BTreeMap::new();
        for (account, partners) in &self.pairings {
            let Some(root) = roots.iter().find(|root| in_subtree(account, root)) else {
                continue;
            };
            for (cash, count) in partners {
                // A transfer INSIDE the line's own subtree is not a pairing.
                if in_subtree(cash, root) {
                    continue;
                }
                *pooled.entry(cash.as_str()).or_insert(0) += count;
            }
        }
        // Highest count; a strict `>` over a lexical walk keeps the first tie.
        let best = pooled
            .iter()
            .fold(None::<(&str, usize)>, |best, (cash, count)| match best {
                Some((_, top)) if *count <= top => best,
                _ => Some((cash, *count)),
            });
        if let Some((cash, _)) = best {
            return AccountName(cash.to_string());
        }
        self.header
            .clone()
            .unwrap_or_else(|| AccountName(self.fallback.clone()))
    }
}

/// Give every group that does not balance an explicit counter-account — the
/// save-time upgrade of a pre-plan-28 file (Decision 1: "Ledgeline picks a
/// default and writes it on save").
///
/// Only a group where NO line names a counter and whose base amounts do not sum
/// to zero is touched; every line in it gets the group's default. That posts the
/// same money to the same account the engine's legacy rule would, so saving
/// changes no number. A group that already balances (a rule that states its own
/// cash leg) is left alone, and so is an event that balances. Never used on
/// `budget.journal`, which is never written (plan 22 Decision 5).
#[must_use]
pub fn upgrade_legacy_counters(scenario: &Scenario, defaults: &CounterDefaults) -> Scenario {
    let mut out = scenario.clone();
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, line) in scenario.lines.iter().enumerate() {
        groups.entry(line.group.as_str()).or_default().push(index);
    }
    for members in groups.values() {
        let lines: Vec<&ScenarioLine> = members.iter().map(|&i| &scenario.lines[i]).collect();
        if lines.iter().any(|line| line.counter.is_some()) {
            continue;
        }
        let mut sum = MixedAmount::new();
        for line in &lines {
            if sum
                .accumulate(&line.amount.commodity, line.amount.quantity)
                .is_err()
            {
                // An overflowing sum is not zero; let the default stand.
                sum = MixedAmount::single(line.amount.commodity.clone(), Dec::new(1, 0));
                break;
            }
        }
        sum.drop_zeros();
        if sum.is_empty() {
            continue;
        }
        let counter = defaults.for_accounts(lines.iter().map(|line| line.account.0.as_str()));
        for &index in members {
            out.lines[index].counter = Some(counter.clone());
        }
    }
    for event in &mut out.events {
        if event.counter.is_some() {
            continue;
        }
        let mut sum = MixedAmount::new();
        let overflowed = event
            .postings
            .iter()
            .flat_map(|posting| &posting.amounts)
            .any(|amount| sum.accumulate(&amount.commodity, amount.quantity).is_err());
        sum.drop_zeros();
        if overflowed || !sum.is_empty() {
            event.counter = Some(
                defaults.for_accounts(
                    event
                        .postings
                        .iter()
                        .map(|posting| posting.account.0.as_str()),
                ),
            );
        }
    }
    out
}

// ===========================================================================
// The expansion
// ===========================================================================

/// Expand `scenario` into dated, balanced transactions over the forecast span.
///
/// Pure: `opts.today` is the only date it knows, and `history` the only
/// journal. See the module docs for the rules.
///
/// # Errors
/// [`ReportError`] on decimal overflow or unrecognized bucket math. A scenario
/// this engine cannot fully model is NOT an error: it produces
/// [`Forecast::warnings`] and the part it can stand behind.
pub fn forecast_transactions(
    scenario: &Scenario,
    opts: &ForecastOpts<'_>,
    types: &AccountTypes,
    history: &History<'_>,
) -> Result<Forecast, ReportError> {
    expand(scenario, opts, types, history).map(|(forecast, _)| forecast)
}

/// [`forecast_transactions`], plus the real price directives it valued with,
/// so [`super::project`] does not re-derive them.
pub(super) fn expand(
    scenario: &Scenario,
    opts: &ForecastOpts<'_>,
    types: &AccountTypes,
    history: &History<'_>,
) -> Result<(Forecast, Vec<PriceDirective>), ReportError> {
    let window = ForecastWindow::resolve(
        opts.today,
        latest_real_date(history.txns),
        opts.interval,
        opts.lookback,
        opts.count,
    )?;
    let as_of = window.actuals_as_of();
    let prices = real_prices(history, &window.cutoff)?;
    let db = PriceDb::build(&prices);
    let target = opts
        .value_in
        .clone()
        .or_else(|| db.base_commodity().cloned());
    let balances = account_balances_at(history.txns, &as_of)?;
    let defaults = CounterDefaults::from_parts(
        history.txns,
        types,
        &window.cutoff,
        scenario.cash.as_ref(),
        &balances,
        target.as_ref(),
        &db,
    )?;

    let mut expansion = Expansion {
        start: window.cutoff.clone(),
        end: window.end.clone(),
        as_of: &as_of,
        types,
        balances: &balances,
        db: &db,
        real_prices: &prices,
        target: target.as_ref(),
        defaults: &defaults,
        groups: BTreeMap::new(),
        group_order: Vec::new(),
        txns: Vec::new(),
        prices: Vec::new(),
        price_owners: BTreeMap::new(),
        assets: Vec::new(),
        priced_parts: Vec::new(),
        commodities: BTreeSet::new(),
        warnings: Vec::new(),
    };
    if !window.buckets.is_empty() {
        for line in &scenario.lines {
            expansion.place_line(line)?;
        }
        expansion.close_groups()?;
        for event in &scenario.events {
            expansion.place_event(event)?;
        }
        expansion.revalue_priced_rows()?;
    }
    let Expansion {
        mut txns,
        prices: forecast_prices,
        assets,
        commodities,
        mut warnings,
        ..
    } = expansion;
    super::warn_on_commodity_mismatch(&commodities, target.as_ref(), &mut warnings);

    // Ascending by date, stable, so a reader that walks them once is right.
    txns.sort_by(|a, b| compare_iso(&a.date, &b.date));
    let first_index = history
        .txns
        .iter()
        .map(|txn| txn.index.0)
        .max()
        .map_or(0, |max| max.saturating_add(1));
    for (offset, txn) in txns.iter_mut().enumerate() {
        txn.index = Tindex(first_index.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX)));
    }

    Ok((
        Forecast {
            window,
            txns,
            prices: forecast_prices,
            assets,
            warnings,
            target,
        },
        prices,
    ))
}

/// Every real price in effect before `cutoff`: the inferred cost prices, then
/// the explicit directives, the order `net_worth` builds them in.
fn real_prices(history: &History<'_>, cutoff: &str) -> Result<Vec<PriceDirective>, ReportError> {
    let mut prices = infer_market_prices(history.txns)?;
    prices.extend_from_slice(history.prices);
    prices.retain(|price| before(&price.date, cutoff));
    Ok(prices)
}

/// `a < b` for ISO dates.
fn before(a: &str, b: &str) -> bool {
    compare_iso(a, b) == std::cmp::Ordering::Less
}

/// One group's postings for one date, before it is closed into a transaction.
#[derive(Default)]
struct DatedLegs {
    /// The description and period the transaction is labelled with.
    description: String,
    period: String,
    /// Postings in the order they were placed: the lines', then the explicit
    /// counters'.
    postings: Vec<(String, MixedAmount, AmountStyle)>,
    /// Explicit counter-account → the amount it takes.
    counters: BTreeMap<String, MixedAmount>,
    /// The sum of the counter-less postings: the legacy residual.
    unbalanced: MixedAmount,
    /// The accounts of the counter-less postings, for the group's default.
    unbalanced_accounts: BTreeSet<String>,
}

/// The expansion in progress. A struct for the reason `Layout` was one before
/// it: placing a line needs the span, the journal's balances, the price
/// database, the defaults AND the outputs, and threading them through free
/// functions is how one of them stops being updated.
struct Expansion<'a> {
    start: String,
    end: String,
    as_of: &'a str,
    types: &'a AccountTypes,
    balances: &'a AccountBalances<'a>,
    db: &'a PriceDb,
    /// The real directives `db` was built from, for the grown database.
    real_prices: &'a [PriceDirective],
    target: Option<&'a Commodity>,
    defaults: &'a CounterDefaults,
    /// group → date → legs.
    groups: BTreeMap<String, BTreeMap<String, DatedLegs>>,
    /// Groups in first-appearance order, so transactions are built in the
    /// order the scenario states them.
    group_order: Vec<String>,
    txns: Vec<Transaction>,
    prices: Vec<PriceDirective>,
    /// commodity → (the rate that owns its trend, the owning row's label).
    price_owners: BTreeMap<Commodity, (Dec, String)>,
    assets: Vec<AssetRow>,
    /// Per placed priced asset row: its index in `assets` and the holdings
    /// whose prices grow, for the appreciation total.
    priced_parts: Vec<(usize, MixedAmount)>,
    commodities: BTreeSet<Commodity>,
    warnings: Vec<String>,
}

impl<'a> Expansion<'a> {
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
        match line.role {
            LineRole::Flow => self.place_flow(line, &label),
            LineRole::Asset => self.place_asset(line, &label),
        }
    }

    /// The date a row's growth is measured from: its own `from`, else the
    /// cutoff — so a step change's second segment grows from its NEW base.
    fn anchor_of(&self, line: &ScenarioLine) -> String {
        line.period
            .start
            .clone()
            .unwrap_or_else(|| self.start.clone())
    }

    /// One flow line: one posting per occurrence, growth stepped as the dates
    /// advance.
    fn place_flow(&mut self, line: &ScenarioLine, label: &str) -> Result<(), ReportError> {
        if line.growth.is_some() && line.period.kind == PeriodKind::Once {
            self.warnings.push(format!(
                "{label}: growth is set on a line that fires once, so it never applies"
            ));
        }
        let anchor = self.anchor_of(line);
        let factor = match &line.growth {
            Some(growth) => Dec::new(1, 0).add(growth.rate)?,
            None => Dec::new(1, 0),
        };
        let mut current = MixedAmount::single(line.amount.commodity.clone(), line.amount.quantity);
        let mut applied: i64 = 0;
        let mut overflowed = false;
        let mut placed = false;
        // `occurrences` is ASCENDING, so the step count never decreases and the
        // growth walk is done once.
        for date in occurrences(&self.start, &self.end, &line.period)? {
            if let Some(growth) = &line.growth
                && !overflowed
            {
                let wanted = growth_steps(&anchor, &date, growth.unit).min(MAX_GROWTH_STEPS);
                while applied < wanted {
                    match grow_once(&current, factor, line.amount.style.precision) {
                        Ok(next) => current = next,
                        // A scenario this engine cannot state, not a fault: the
                        // line holds its last representable amount and says so.
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
            placed = true;
            self.leg(line, &date, &current)?;
        }
        if placed {
            self.commodities.insert(line.amount.commodity.clone());
        }
        Ok(())
    }

    /// Place one line's amount on `date` in its group, balanced against its
    /// counter-account, or held for the group's legacy residual.
    fn leg(
        &mut self,
        line: &ScenarioLine,
        date: &str,
        amount: &MixedAmount,
    ) -> Result<(), ReportError> {
        if amount.is_zero() {
            return Ok(());
        }
        if !self.groups.contains_key(&line.group) {
            self.group_order.push(line.group.clone());
        }
        let legs = self
            .groups
            .entry(line.group.clone())
            .or_default()
            .entry(date.to_string())
            .or_default();
        if legs.postings.is_empty() {
            legs.description = description_of(&line.note);
            legs.period = line.period.raw.clone();
        }
        legs.postings.push((
            line.account.0.clone(),
            amount.clone(),
            line.amount.style.clone(),
        ));
        match &line.counter {
            Some(counter) => legs
                .counters
                .entry(counter.0.clone())
                .or_default()
                .ma_add_assign(&amount.ma_neg()?)?,
            None => {
                legs.unbalanced.ma_add_assign(amount)?;
                legs.unbalanced_accounts.insert(line.account.0.clone());
            }
        }
        Ok(())
    }

    /// Turn every group's dated legs into balanced transactions.
    fn close_groups(&mut self) -> Result<(), ReportError> {
        for group in std::mem::take(&mut self.group_order) {
            let Some(dates) = self.groups.remove(&group) else {
                continue;
            };
            for (date, legs) in dates {
                let mut postings: Vec<Posting> = legs
                    .postings
                    .iter()
                    .flat_map(|(account, amount, style)| posting(account, amount, style))
                    .collect();
                for (counter, amount) in &legs.counters {
                    postings.extend(posting(counter, amount, &plain_style()));
                }
                let mut residual = legs.unbalanced.clone();
                residual.drop_zeros();
                if !residual.is_empty() {
                    let counter = self
                        .defaults
                        .for_accounts(legs.unbalanced_accounts.iter().map(String::as_str));
                    postings.extend(posting(&counter.0, &residual.ma_neg()?, &plain_style()));
                }
                self.txns
                    .push(generated(&date, &legs.description, &legs.period, postings));
            }
        }
        Ok(())
    }

    /// One dated event: its postings plus its counter (explicit, else the
    /// default when the event does not balance on its own).
    fn place_event(&mut self, event: &ScenarioEvent) -> Result<(), ReportError> {
        if before(&event.date, &self.start) || before(&self.end, &event.date) {
            return Ok(());
        }
        let mut sum = MixedAmount::new();
        let mut postings = Vec::new();
        for item in &event.postings {
            for amount in &item.amounts {
                self.commodities.insert(amount.commodity.clone());
                sum.accumulate(&amount.commodity, amount.quantity)?;
            }
            if !item.amounts.is_empty() {
                postings.push(Posting {
                    ptype: PostingType::Regular,
                    ..item.clone()
                });
            }
        }
        sum.drop_zeros();
        if !sum.is_empty() {
            let counter = event.counter.clone().unwrap_or_else(|| {
                self.defaults
                    .for_accounts(event.postings.iter().map(|item| item.account.0.as_str()))
            });
            postings.extend(posting(&counter.0, &sum.ma_neg()?, &plain_style()));
        }
        if postings.is_empty() {
            return Ok(());
        }
        self.txns.push(generated(
            &event.date,
            &description_of(&event.description),
            &event.date,
            postings,
        ));
        Ok(())
    }

    /// One ASSET row: its contributions are ordinary legs; its growth is gains
    /// postings on the unpriced part and price trends on the priced part.
    fn place_asset(&mut self, line: &ScenarioLine, label: &str) -> Result<(), ReportError> {
        // Liabilities are out of scope (plan 23 decision 8).
        if !self.types.is_type(&line.account.0, AccountType::Asset) {
            self.warnings.push(format!(
                "{label}: an asset row needs an account of type asset, and '{}' is {}, \
                 so the row contributes nothing (liabilities are out of scope)",
                line.account.0,
                describe_type(self.types.resolve(&line.account.0)),
            ));
            return Ok(());
        }
        // ONE asset row per account, first in the file owns it (plan 23
        // amendment 18): two rows on overlapping subtrees would each compound
        // the same money.
        let clash = self
            .assets
            .iter()
            .find(|placed| {
                in_subtree(&line.account.0, &placed.account)
                    || in_subtree(&placed.account, &line.account.0)
            })
            .map(|placed| placed.account.clone());
        if let Some(placed) = clash {
            let detail = if placed == line.account.0 {
                format!(
                    "an earlier asset row already models '{}', so the row contributes \
                     nothing (one asset row per account; both would compound the same \
                     balance)",
                    line.account.0
                )
            } else {
                format!(
                    "'{}' shares its balance with '{placed}', which an earlier asset row \
                     already models, so the row contributes nothing (an asset row's \
                     balance is the account and everything under it; both would compound \
                     the shared money)",
                    line.account.0
                )
            };
            self.warnings.push(format!("{label}: {detail}"));
            return Ok(());
        }

        let gains = line
            .gains
            .as_ref()
            .map_or(DEFAULT_GAINS_ACCOUNT, |gains| gains.0.as_str())
            .to_string();
        let raw = subtree_balance(self.balances, &line.account.0)?;
        let journal_opening = valued(&raw, self.target, self.db, self.as_of, None)?;

        // PRICED holdings grow through their prices (Decision 3). Only without
        // an `opening:` override: an override restates the balance in the row's
        // own commodity, and how much of it is which holding is not a question
        // the journal answers — so an overridden row compounds as one balance.
        let priced: Vec<(Commodity, Amount)> = match (self.target, &line.opening) {
            (Some(target), None) => raw
                .iter()
                .filter(|(commodity, _)| *commodity != target)
                .filter_map(|(commodity, _)| {
                    self.db
                        .lookup(commodity, self.as_of)
                        .map(|price| (commodity.clone(), price.clone()))
                })
                .collect(),
            _ => Vec::new(),
        };

        let mut balance = if !priced.is_empty() {
            // The part of the account that is NOT a priced holding — its cash
            // in the target commodity — compounds as a balance, as an unpriced
            // row's whole balance does.
            self.target.map_or_else(MixedAmount::new, |target| {
                MixedAmount::single(target.clone(), raw.get(target).unwrap_or_else(Dec::zero))
            })
        } else if let Some(opening) = &line.opening {
            let stated = MixedAmount::single(opening.commodity.clone(), opening.quantity);
            let adjustment = stated.ma_add(&journal_opening.ma_neg()?)?;
            self.commodities.insert(opening.commodity.clone());
            self.warnings.push(format!(
                "{label}: the opening balance is stated as {}, but the journal says {}; \
                 net worth is adjusted by {} in the first bucket",
                show(&stated),
                show(&journal_opening),
                show(&adjustment),
            ));
            // The override's difference is booked once, at the cutoff, against
            // the gains account — never to cash.
            let start = self.start.clone();
            self.gains_txn(
                &start,
                &line.account.0,
                &gains,
                &adjustment,
                &line.amount.style,
                &format!("{} opening adjustment", line.account.0),
            );
            stated
        } else {
            journal_opening.clone()
        };
        let opening = if priced.is_empty() {
            balance.clone()
        } else {
            journal_opening.clone()
        };

        let anchor = self.anchor_of(line);
        // A zero rate is no growth at all: `grow_once` ROUNDS, so walking a 0%
        // row would call the rounding difference appreciation.
        let (steps, rate) = match &line.growth {
            Some(growth) if !growth.rate.is_zero() => (
                growth_boundaries(&anchor, growth.unit, &self.start, &self.end),
                Some(growth.rate),
            ),
            _ => (Vec::new(), None),
        };
        let factor = Dec::new(1, 0).add(rate.unwrap_or_else(Dec::zero))?;
        let contributions = if line.amount.quantity.is_zero() {
            Vec::new()
        } else {
            occurrences(&self.start, &self.end, &line.period)?
        };
        let contribution = MixedAmount::single(line.amount.commodity.clone(), line.amount.quantity);

        // The two date series, merged; GROWTH WINS A TIE (plan 23 decision 7).
        let mut appreciation = MixedAmount::new();
        let (mut next_step, mut next_contribution) = (0usize, 0usize);
        let mut applied = 0usize;
        let mut overflowed = false;
        let mut contributed = false;
        let description = format!("{} growth", line.account.0);
        while next_step < steps.len() || next_contribution < contributions.len() {
            let grow_now = match (steps.get(next_step), contributions.get(next_contribution)) {
                (Some(step), Some(date)) => compare_iso(step, date) != std::cmp::Ordering::Greater,
                (Some(_), None) => true,
                _ => false,
            };
            if grow_now {
                let date = steps[next_step].clone();
                next_step += 1;
                if overflowed {
                    continue;
                }
                match grow_once(&balance, factor, line.amount.style.precision) {
                    Ok(grown) => {
                        let step = grown.ma_add(&balance.ma_neg()?)?;
                        self.gains_txn(
                            &date,
                            &line.account.0,
                            &gains,
                            &step,
                            &line.amount.style,
                            &description,
                        );
                        appreciation.ma_add_assign(&step)?;
                        balance = grown;
                        applied += 1;
                    }
                    Err(_) => {
                        overflowed = true;
                        self.warnings.push(format!(
                            "{label}: growth overflowed the exact-decimal range after \
                             {applied} steps; the balance is held flat from there on"
                        ));
                    }
                }
                continue;
            }
            let date = contributions[next_contribution].clone();
            next_contribution += 1;
            contributed = true;
            self.leg(line, &date, &contribution)?;
            balance.accumulate(&line.amount.commodity, line.amount.quantity)?;
        }
        if contributed {
            self.commodities.insert(line.amount.commodity.clone());
        }

        if let Some(rate) = rate {
            self.trend_prices(&priced, rate, &steps, label)?;
        }
        if !priced.is_empty() {
            let mut holdings = MixedAmount::new();
            for (commodity, _) in &priced {
                holdings.accumulate(commodity, raw.get(commodity).unwrap_or_else(Dec::zero))?;
            }
            self.priced_parts.push((self.assets.len(), holdings));
        }
        self.assets.push(AssetRow {
            group: line.group.clone(),
            account: line.account.0.clone(),
            journal_opening,
            opening,
            growth: appreciation,
        });
        Ok(())
    }

    /// A balanced appreciation posting: the asset up by `step`, the gains
    /// account down by it. Zero is no transaction.
    fn gains_txn(
        &mut self,
        date: &str,
        account: &str,
        gains: &str,
        step: &MixedAmount,
        style: &AmountStyle,
        description: &str,
    ) {
        let mut step = step.clone();
        step.drop_zeros();
        if step.is_empty() {
            return;
        }
        let Ok(negated) = step.ma_neg() else {
            return;
        };
        let postings: Vec<Posting> = posting(account, &step, style)
            .into_iter()
            .chain(posting(gains, &negated, style))
            .collect();
        self.txns
            .push(generated(date, description, "growth", postings));
    }

    /// Stepwise in-memory prices for each priced holding, at the row's growth
    /// boundaries. A commodity another row already trends keeps that row's
    /// rate, and a different rate here is warned about.
    fn trend_prices(
        &mut self,
        priced: &[(Commodity, Amount)],
        rate: Dec,
        steps: &[String],
        label: &str,
    ) -> Result<(), ReportError> {
        let factor = Dec::new(1, 0).add(rate)?;
        for (commodity, base) in priced {
            if let Some((owner_rate, owner)) = self.price_owners.get(commodity) {
                if owner_rate.cmp(&rate) != std::cmp::Ordering::Equal {
                    self.warnings.push(format!(
                        "{label}: '{}' is already grown by {owner} at a different rate; a \
                         price belongs to the commodity, so the first row's rate is used",
                        commodity.0
                    ));
                }
                continue;
            }
            self.price_owners
                .insert(commodity.clone(), (rate, label.to_string()));
            let places = base
                .quantity
                .places
                .max(base.style.precision)
                .max(PRICE_FLOOR_PLACES);
            let mut price = base.quantity;
            for date in steps {
                let Ok(next) = price.mul(factor).and_then(|grown| grown.rounded(places)) else {
                    self.warnings.push(format!(
                        "{label}: the price of '{}' overflowed the exact-decimal range; it is \
                         held flat from there on",
                        commodity.0
                    ));
                    break;
                };
                price = next;
                self.prices.push(PriceDirective {
                    date: date.clone(),
                    commodity: commodity.clone(),
                    price: Amount {
                        quantity: price,
                        style: AmountStyle {
                            precision: places,
                            ..base.style.clone()
                        },
                        ..base.clone()
                    },
                    source_file: PathBuf::new(),
                });
            }
        }
        Ok(())
    }

    /// Add each priced row's revaluation over the span to its `growth`, once
    /// every trend is known: the holdings valued at the end of the span under
    /// the grown prices, less the same holdings valued at the cutoff.
    fn revalue_priced_rows(&mut self) -> Result<(), ReportError> {
        if self.priced_parts.is_empty() || self.prices.is_empty() {
            return Ok(());
        }
        let mut all = Vec::new();
        all.extend(self.real_prices.iter().cloned());
        all.extend(self.prices.iter().cloned());
        let grown = PriceDb::build(&all);
        for (index, holdings) in std::mem::take(&mut self.priced_parts) {
            let at_end = valued(&holdings, self.target, &grown, &self.end, None)?;
            let at_start = valued(&holdings, self.target, &grown, self.as_of, None)?;
            let delta = at_end.ma_add(&at_start.ma_neg()?)?;
            self.assets[index].growth.ma_add_assign(&delta)?;
        }
        Ok(())
    }
}

/// A transaction's description: the note, else hledger's own default.
fn description_of(note: &str) -> String {
    let note = note.trim();
    if note.is_empty() {
        "projection".to_string()
    } else {
        note.to_string()
    }
}

/// A generated transaction, marked as hledger marks its own.
fn generated(date: &str, description: &str, period: &str, postings: Vec<Posting>) -> Transaction {
    let marker = format!("~ {period}");
    let pos = SourcePos { line: 0, column: 0 };
    Transaction {
        index: Tindex(0),
        date: date.to_string(),
        date2: None,
        status: Status::Unmarked,
        code: String::new(),
        description: description.to_string(),
        comment: format!("{GENERATED_TAG}: {marker}"),
        preceding_comment: String::new(),
        tags: vec![
            (GENERATED_TAG.to_string(), marker.clone()),
            (HIDDEN_GENERATED_TAG.to_string(), marker),
        ],
        postings,
        source_span: (pos, pos),
        source_file: PathBuf::new(),
    }
}

/// A plain display style, for a posting with no line amount to borrow one from.
fn plain_style() -> AmountStyle {
    AmountStyle {
        side: CommoditySide::Left,
        spaced: false,
        decimal_mark: Some('.'),
        digit_groups: None,
        precision: 0,
    }
}

/// One regular posting of `amount` to `account`, or none when it is zero.
fn posting(account: &str, amount: &MixedAmount, style: &AmountStyle) -> Option<Posting> {
    let amounts: Vec<Amount> = amount
        .iter()
        .filter(|(_, qty)| !qty.is_zero())
        .map(|(commodity, qty)| Amount {
            commodity: commodity.clone(),
            quantity: *qty,
            style: AmountStyle {
                precision: style.precision.max(qty.places),
                ..style.clone()
            },
            cost: None,
        })
        .collect();
    (!amounts.is_empty()).then(|| Posting {
        status: Status::Unmarked,
        ptype: PostingType::Regular,
        account: AccountName(account.to_string()),
        amounts,
        balance_assertion: None,
        date: None,
        date2: None,
        comment: String::new(),
        tags: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PeriodSpec;
    use crate::parse::parse_period_spec;
    use crate::projections::{
        Growth, GrowthUnit, LineSource, ProjectionOpts, Scenario, project, virtual_posting,
    };
    use crate::reports::test_support::{amount, price, txn, usd};

    fn line(group: &str, account: &str, money: Amount, period: &str) -> ScenarioLine {
        ScenarioLine {
            id: group.to_string(),
            group: group.to_string(),
            role: LineRole::Flow,
            account: AccountName(account.to_string()),
            amount: money,
            period: parse_period_spec(period),
            growth: None,
            opening: None,
            counter: None,
            gains: None,
            note: String::new(),
            source: LineSource::Journal,
        }
    }

    fn countered(mut line: ScenarioLine, counter: &str) -> ScenarioLine {
        line.counter = Some(AccountName(counter.to_string()));
        line
    }

    fn asset(group: &str, account: &str, money: Amount, rate: Dec) -> ScenarioLine {
        ScenarioLine {
            role: LineRole::Asset,
            growth: Some(Growth {
                rate,
                unit: GrowthUnit::Year,
            }),
            ..line(group, account, money, "monthly")
        }
    }

    fn event(
        id: &str,
        date: &str,
        postings: &[(&str, Amount)],
        counter: Option<&str>,
    ) -> ScenarioEvent {
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
            counter: counter.map(|counter| AccountName(counter.to_string())),
        }
    }

    fn scenario(lines: Vec<ScenarioLine>, events: Vec<ScenarioEvent>) -> Scenario {
        Scenario {
            lines,
            events,
            ..Scenario::default()
        }
    }

    /// Checking and savings, with salary always landing in checking and rent
    /// always paid from savings. Latest transaction 2026-09-30.
    fn journal() -> Vec<Transaction> {
        let mut txns = vec![txn(
            1,
            "2026-01-05",
            vec![
                ("assets:checking", vec![usd(500_000)]),
                ("assets:savings", vec![usd(2_000_000)]),
                ("equity:opening", vec![usd(-2_500_000)]),
            ],
        )];
        for (index, month) in ["07", "08", "09"].iter().enumerate() {
            let base = u32::try_from(index).expect("small") * 2 + 2;
            txns.push(txn(
                base,
                &format!("2026-{month}-15"),
                vec![
                    ("assets:checking", vec![usd(300_000)]),
                    ("revenues:salary:base", vec![usd(-300_000)]),
                ],
            ));
            txns.push(txn(
                base + 1,
                &format!("2026-{month}-30"),
                vec![
                    ("expenses:rent", vec![usd(100_000)]),
                    ("assets:savings", vec![usd(-100_000)]),
                ],
            ));
        }
        txns
    }

    fn opts(count: usize) -> ForecastOpts<'static> {
        ForecastOpts {
            today: "2026-10-15",
            interval: Interval::Monthly,
            count,
            lookback: 0,
            value_in: None,
        }
    }

    fn expand_over(
        scenario: &Scenario,
        txns: &[Transaction],
        prices: &[PriceDirective],
        count: usize,
    ) -> Forecast {
        let types = AccountTypes::from_declared(BTreeMap::new());
        forecast_transactions(scenario, &opts(count), &types, &History { txns, prices })
            .expect("the forecast expands")
    }

    fn defaults_over(txns: &[Transaction], header: Option<&str>) -> CounterDefaults {
        let types = AccountTypes::from_declared(BTreeMap::new());
        let header = header.map(|account| AccountName(account.to_string()));
        CounterDefaults::new(
            &History { txns, prices: &[] },
            &types,
            "2026-10-01",
            header.as_ref(),
            None,
        )
        .expect("the defaults build")
    }

    /// Each generated transaction's postings, as `(account, cents)`.
    fn legs(txn: &Transaction) -> Vec<(String, i128)> {
        txn.postings
            .iter()
            .flat_map(|posting| {
                posting.amounts.iter().map(|amount| {
                    (
                        posting.account.0.clone(),
                        amount.quantity.rounded(2).expect("cents").mantissa,
                    )
                })
            })
            .collect()
    }

    fn assert_balanced(forecast: &Forecast) {
        for txn in &forecast.txns {
            let mut sum = MixedAmount::new();
            for posting in &txn.postings {
                for amount in &posting.amounts {
                    sum.accumulate(&amount.commodity, amount.quantity)
                        .expect("sums");
                }
            }
            sum.drop_zeros();
            assert!(
                sum.is_empty(),
                "{} does not balance: {:?}",
                txn.date,
                legs(txn)
            );
            assert!(is_generated(txn), "{} is not marked generated", txn.date);
        }
    }

    // --- The cutoff (Decision 5) -------------------------------------------

    #[test]
    fn the_cutoff_follows_the_three_worked_examples() {
        let monthly = |today, latest| forecast_cutoff(today, latest, Interval::Monthly).unwrap();
        assert_eq!(monthly("2026-10-23", Some("2026-10-20")), "2026-10-01");
        assert_eq!(monthly("2026-11-10", Some("2026-10-15")), "2026-10-01");
        assert_eq!(monthly("2026-11-10", Some("2026-10-31")), "2026-11-01");
        // No journal: today alone.
        assert_eq!(monthly("2026-10-23", None), "2026-10-01");
        // On another interval, the period containing the same day.
        assert_eq!(
            forecast_cutoff("2026-11-10", Some("2026-10-31"), Interval::Quarterly).unwrap(),
            "2026-10-01"
        );
    }

    #[test]
    fn the_window_covers_the_lookback_and_the_forecast() {
        let window =
            ForecastWindow::resolve("2026-10-23", Some("2026-10-20"), Interval::Monthly, 2, 3)
                .unwrap();
        assert_eq!(window.cutoff, "2026-10-01");
        assert_eq!(window.lookback, ["2026-08", "2026-09"]);
        assert_eq!(window.buckets, ["2026-10", "2026-11", "2026-12"]);
        assert_eq!(window.from, "2026-08-01");
        assert_eq!(window.end, "2026-12-31");
        assert_eq!(window.actuals_as_of(), "2026-09-30");
    }

    // --- The default counter-account (Decision 7) ----------------------------

    #[test]
    fn the_default_counter_is_the_most_common_cash_pairing() {
        let defaults = defaults_over(&journal(), Some("assets:header"));
        // Salary always landed in checking — and a parent line pools its children.
        assert_eq!(defaults.for_account("revenues:salary").0, "assets:checking");
        assert_eq!(
            defaults.for_account("revenues:salary:base").0,
            "assets:checking"
        );
        // Rent was always paid from savings.
        assert_eq!(defaults.for_account("expenses:rent").0, "assets:savings");
        // No history: the scenario header's `; cash:`.
        assert_eq!(defaults.for_account("expenses:travel").0, "assets:header");
        // A group pools its lines: two salary pairings beat one rent pairing? No —
        // three each; the tie goes to the lexically first.
        assert_eq!(
            defaults
                .for_accounts(["revenues:salary", "expenses:rent"])
                .0,
            "assets:checking"
        );
    }

    #[test]
    fn without_history_or_header_the_largest_cash_balance_is_the_default() {
        let defaults = defaults_over(&journal(), None);
        // Savings ends September at $17,000, checking at $14,000.
        assert_eq!(defaults.for_account("expenses:travel").0, "assets:savings");
        // A journal with no cash account at all falls back to `assets:cash`,
        // which hledger's own heuristic reads as Cash.
        let none = defaults_over(&[], None);
        assert_eq!(none.for_account("expenses:travel").0, FALLBACK_CASH_ACCOUNT);
        // A declared cash account wins over the fallback even with no balance.
        let types = AccountTypes::from_declared(
            [("activo:banco".to_string(), AccountType::Cash)]
                .into_iter()
                .collect(),
        );
        let declared = CounterDefaults::new(
            &History {
                txns: &[],
                prices: &[],
            },
            &types,
            "2026-10-01",
            None,
            None,
        )
        .unwrap();
        assert_eq!(declared.for_account("gastos:viaje").0, "activo:banco");
    }

    #[test]
    fn pairings_outside_the_trailing_window_do_not_count() {
        // A year-old pairing with checking, and a recent one with savings.
        let txns = vec![
            txn(
                1,
                "2025-08-01",
                vec![
                    ("expenses:gym", vec![usd(100)]),
                    ("assets:checking", vec![usd(-100)]),
                ],
            ),
            txn(
                2,
                "2025-08-02",
                vec![
                    ("expenses:gym", vec![usd(100)]),
                    ("assets:checking", vec![usd(-100)]),
                ],
            ),
            txn(
                3,
                "2026-09-01",
                vec![
                    ("expenses:gym", vec![usd(100)]),
                    ("assets:savings", vec![usd(-100)]),
                ],
            ),
        ];
        assert_eq!(
            defaults_over(&txns, None).for_account("expenses:gym").0,
            "assets:savings"
        );
    }

    // --- The expansion -----------------------------------------------------

    /// Legacy input keeps its numbers: an unbalanced group's sum is posted to
    /// its default counter, per occurrence, and every transaction balances.
    #[test]
    fn a_legacy_group_s_residual_goes_to_its_default_counter() {
        let forecast = expand_over(
            &scenario(
                vec![
                    line("pay", "revenues:salary", usd(-300_000), "monthly"),
                    line("pay", "expenses:rent", usd(100_000), "monthly"),
                ],
                Vec::new(),
            ),
            &journal(),
            &[],
            2,
        );
        assert_balanced(&forecast);
        assert_eq!(forecast.window.cutoff, "2026-10-01");
        assert_eq!(forecast.txns.len(), 2);
        // One transaction per occurrence: both lines plus the residual, pooled
        // over the group (salary and rent tie three-all; checking is first).
        assert_eq!(
            legs(&forecast.txns[0]),
            [
                ("revenues:salary".to_string(), -300_000),
                ("expenses:rent".to_string(), 100_000),
                ("assets:checking".to_string(), 200_000),
            ]
        );
        assert_eq!(
            forecast.txns[0].tags[0],
            (GENERATED_TAG.to_string(), "~ monthly".to_string())
        );
    }

    /// A group that states its own cash leg balances already, and gets nothing.
    #[test]
    fn a_balanced_legacy_group_gets_no_counter_posting() {
        let forecast = expand_over(
            &scenario(
                vec![
                    line("rent", "expenses:rent", usd(100_000), "monthly"),
                    line("rent", "assets:savings", usd(-100_000), "monthly"),
                ],
                Vec::new(),
            ),
            &journal(),
            &[],
            1,
        );
        assert_balanced(&forecast);
        assert_eq!(forecast.txns[0].postings.len(), 2);
    }

    /// An explicit counter is used posting for posting, whatever the history.
    #[test]
    fn an_explicit_counter_balances_its_own_line() {
        let forecast = expand_over(
            &scenario(
                vec![countered(
                    line("card", "expenses:food", usd(50_000), "monthly"),
                    "liabilities:card",
                )],
                Vec::new(),
            ),
            &journal(),
            &[],
            1,
        );
        assert_balanced(&forecast);
        assert_eq!(
            legs(&forecast.txns[0]),
            [
                ("expenses:food".to_string(), 50_000),
                ("liabilities:card".to_string(), -50_000),
            ]
        );
    }

    /// Plan 22's residual table, restated with EXPLICIT counter-accounts: the
    /// accrual, the partial payment and the equity raise are now ordinary
    /// balanced transactions, and the figures are the table's.
    #[test]
    fn plan_22_s_residual_table_as_explicit_counters() {
        let declared = BTreeMap::new();
        let run = |scenario: &Scenario| {
            project(
                scenario,
                &journal(),
                &[],
                &ProjectionOpts {
                    today: "2026-10-15",
                    interval: Interval::Monthly,
                    count: 1,
                    lookback: 0,
                    depth: 99,
                    declared: &declared,
                    value_in: None,
                },
            )
            .expect("projects")
        };
        let cash_delta = |projection: &crate::projections::Projection| {
            projection.cash.values[0]
                .get(&crate::model::Commodity("$".into()))
                .unwrap()
                .mantissa
                - projection
                    .cash
                    .opening
                    .get(&crate::model::Commodity("$".into()))
                    .unwrap()
                    .mantissa
        };
        let worth_delta = |projection: &crate::projections::Projection| {
            projection.net_worth.values[0]
                .get(&crate::model::Commodity("$".into()))
                .unwrap()
                .mantissa
                - projection
                    .net_worth
                    .opening
                    .get(&crate::model::Commodity("$".into()))
                    .unwrap()
                    .mantissa
        };

        // Accrual: legal fees against payables — no cash, net worth down.
        let accrual = run(&scenario(
            Vec::new(),
            vec![event(
                "legal",
                "2026-10-15",
                &[("expenses:legal", usd(4_500_000))],
                Some("liabilities:payable"),
            )],
        ));
        assert_eq!(cash_delta(&accrual), 0);
        assert_eq!(worth_delta(&accrual), -4_500_000);

        // Partial payment: $2,000 from cash, the rest accrued.
        let partial = run(&scenario(
            Vec::new(),
            vec![event(
                "rent",
                "2026-10-15",
                &[
                    ("expenses:rent", usd(420_000)),
                    ("assets:checking", usd(-200_000)),
                ],
                Some("liabilities:payable"),
            )],
        ));
        assert_eq!(cash_delta(&partial), -200_000);
        assert_eq!(worth_delta(&partial), -420_000);

        // Equity raise: cash and net worth up, net income untouched.
        let raise = run(&scenario(
            Vec::new(),
            vec![event(
                "raise",
                "2026-10-15",
                &[("assets:checking", usd(200_000_000))],
                Some("equity:preferred"),
            )],
        ));
        assert_eq!(cash_delta(&raise), 200_000_000);
        assert_eq!(worth_delta(&raise), 200_000_000);
        assert!(raise.net_income.rows.is_empty());

        // And a recurring line accrued to a card moves no cash either.
        let carded = run(&scenario(
            vec![countered(
                line("food", "expenses:food", usd(50_000), "monthly"),
                "liabilities:card",
            )],
            Vec::new(),
        ));
        assert_eq!(cash_delta(&carded), 0);
        assert_eq!(worth_delta(&carded), -50_000);
    }

    // --- Asset rows: gains postings and price trends (Decision 3) -----------

    /// Appreciation on an UNPRICED asset is a balanced posting from the asset to
    /// its gains account, `equity:unrealized gains` by default.
    #[test]
    fn unpriced_growth_is_a_gains_posting() {
        let mut txns = journal();
        txns.push(txn(
            20,
            "2026-09-30",
            vec![
                ("assets:house", vec![usd(50_000_000)]),
                ("equity:opening", vec![usd(-50_000_000)]),
            ],
        ));
        let mut house = asset("house", "assets:house", usd(0), Dec::new(10, 2));
        let forecast = expand_over(&scenario(vec![house.clone()], Vec::new()), &txns, &[], 13);
        assert_balanced(&forecast);
        assert_eq!(forecast.txns.len(), 1, "one step in thirteen months");
        assert_eq!(forecast.txns[0].date, "2027-10-01");
        assert_eq!(
            legs(&forecast.txns[0]),
            [
                ("assets:house".to_string(), 5_000_000),
                (DEFAULT_GAINS_ACCOUNT.to_string(), -5_000_000),
            ]
        );
        assert!(forecast.prices.is_empty());

        // A named gains account is used instead.
        house.gains = Some(AccountName("revenues:appreciation".to_string()));
        let named = expand_over(&scenario(vec![house], Vec::new()), &txns, &[], 13);
        assert_eq!(named.txns[0].postings[1].account.0, "revenues:appreciation");
    }

    /// A row on an account holding PRICED shares grows their prices, in memory,
    /// stepwise from the latest price before the cutoff — no posting at all.
    #[test]
    fn priced_growth_is_an_in_memory_price_trend() {
        let mut txns = journal();
        txns.push(txn(
            20,
            "2026-09-30",
            vec![
                ("assets:broker:aapl", vec![amount("AAPL", 10, 0)]),
                ("equity:opening", vec![usd(-100_000)]),
            ],
        ));
        let prices = [
            price("2026-06-01", "AAPL", usd(9_000)),
            price("2026-09-01", "AAPL", usd(10_000)),
            // Dated after the cutoff: an actual from the excluded period.
            price("2026-10-10", "AAPL", usd(99_900)),
        ];
        let broker = asset("broker", "assets:broker", usd(0), Dec::new(10, 2));
        let forecast = expand_over(&scenario(vec![broker], Vec::new()), &txns, &prices, 25);
        assert!(
            forecast.txns.is_empty(),
            "a priced holding grows by price, not by posting"
        );
        let trend: Vec<(&str, i128)> = forecast
            .prices
            .iter()
            .map(|p| {
                (
                    p.date.as_str(),
                    p.price.quantity.rounded(2).unwrap().mantissa,
                )
            })
            .collect();
        assert_eq!(trend, [("2027-10-01", 11_000), ("2028-10-01", 12_100)]);
        // The row's appreciation is the holding revalued: 10 × ($121 − $100).
        assert_eq!(
            forecast.assets[0]
                .growth
                .get(&crate::model::Commodity("$".into()))
                .unwrap()
                .rounded(2)
                .unwrap()
                .mantissa,
            21_000
        );
        assert!(forecast.warnings.is_empty(), "{:?}", forecast.warnings);

        // Two rows growing the same commodity at different rates: warned, and
        // the first rate wins (a price belongs to the commodity).
        txns.push(txn(
            21,
            "2026-09-30",
            vec![
                ("assets:ira:aapl", vec![amount("AAPL", 5, 0)]),
                ("equity:opening", vec![usd(-50_000)]),
            ],
        ));
        let both = expand_over(
            &scenario(
                vec![
                    asset("broker", "assets:broker", usd(0), Dec::new(10, 2)),
                    asset("ira", "assets:ira", usd(0), Dec::new(20, 2)),
                ],
                Vec::new(),
            ),
            &txns,
            &prices,
            13,
        );
        assert_eq!(both.prices.len(), 1);
        assert_eq!(
            both.prices[0].price.quantity.rounded(2).unwrap().mantissa,
            11_000
        );
        assert_eq!(both.warnings.len(), 1, "{:?}", both.warnings);
        assert!(both.warnings[0].contains("AAPL") && both.warnings[0].contains("first row"));

        // …and the projection's net worth revalues EVERY real AAPL holding with
        // the trend — the IRA's too, because a price belongs to the commodity —
        // while cash never moves.
        let declared = BTreeMap::new();
        let projection = project(
            &scenario(
                vec![asset("broker", "assets:broker", usd(0), Dec::new(10, 2))],
                Vec::new(),
            ),
            &txns,
            &prices,
            &ProjectionOpts {
                today: "2026-10-15",
                interval: Interval::Monthly,
                count: 13,
                lookback: 0,
                depth: 99,
                declared: &declared,
                value_in: None,
            },
        )
        .unwrap();
        let dollars = |ma: &MixedAmount| {
            ma.get(&crate::model::Commodity("$".into()))
                .unwrap()
                .rounded(2)
                .unwrap()
                .mantissa
        };
        let worth: Vec<i128> = projection.net_worth.values.iter().map(dollars).collect();
        assert_eq!(worth[11], worth[0], "flat until the anniversary");
        assert_eq!(worth[12] - worth[11], 15 * 1_000, "15 shares × $10");
        assert_eq!(
            projection
                .cash
                .values
                .iter()
                .map(dollars)
                .collect::<Vec<_>>(),
            vec![dollars(&projection.cash.opening); 13]
        );
    }

    // --- The save-time upgrade -----------------------------------------------

    #[test]
    fn the_upgrade_writes_defaults_only_where_a_group_does_not_balance() {
        let defaults = defaults_over(&journal(), None);
        let upgraded = upgrade_legacy_counters(
            &scenario(
                vec![
                    line("pay", "revenues:salary", usd(-300_000), "monthly"),
                    line("rent", "expenses:rent", usd(100_000), "monthly"),
                    line("rent", "assets:savings", usd(-100_000), "monthly"),
                    countered(
                        line("card", "expenses:food", usd(1), "monthly"),
                        "liabilities:card",
                    ),
                ],
                vec![
                    event("legal", "2027-01-01", &[("expenses:legal", usd(100))], None),
                    event(
                        "move",
                        "2027-01-01",
                        &[("assets:checking", usd(100)), ("assets:savings", usd(-100))],
                        None,
                    ),
                ],
            ),
            &defaults,
        );
        let counters: Vec<Option<&str>> = upgraded
            .lines
            .iter()
            .map(|line| line.counter.as_ref().map(|c| c.0.as_str()))
            .collect();
        assert_eq!(
            counters,
            [
                Some("assets:checking"),
                None,
                None,
                Some("liabilities:card")
            ]
        );
        // No history pairs `expenses:legal` and there is no header, so it is
        // the largest cash balance.
        assert_eq!(
            upgraded.events[0].counter.as_ref().map(|c| c.0.as_str()),
            Some("assets:savings")
        );
        assert_eq!(upgraded.events[1].counter, None);
    }

    /// The upgrade changes no number: the same scenario projects identically
    /// before and after it.
    #[test]
    fn the_upgrade_moves_no_number() {
        let legacy = scenario(
            vec![
                line("pay", "revenues:salary", usd(-300_000), "monthly"),
                line("pay", "expenses:rent", usd(100_000), "monthly"),
            ],
            vec![event(
                "legal",
                "2026-11-03",
                &[("expenses:legal", usd(250_000))],
                None,
            )],
        );
        let defaults = defaults_over(&journal(), None);
        let upgraded = upgrade_legacy_counters(&legacy, &defaults);
        let a = expand_over(&legacy, &journal(), &[], 3);
        let b = expand_over(&upgraded, &journal(), &[], 3);
        let totals = |forecast: &Forecast| {
            crate::reports::account_totals(
                &forecast.txns,
                &crate::reports::PostingFilter::default(),
            )
            .unwrap()
        };
        assert_eq!(totals(&a), totals(&b));
    }

    #[test]
    fn an_unsupported_period_spec_is_still_a_warning() {
        let spec = PeriodSpec {
            kind: PeriodKind::Unsupported,
            ..parse_period_spec("monthly")
        };
        let odd = ScenarioLine {
            period: spec,
            ..line("odd", "expenses:odd", usd(100), "monthly")
        };
        let forecast = expand_over(&scenario(vec![odd], Vec::new()), &journal(), &[], 2);
        assert!(forecast.txns.is_empty());
        assert_eq!(forecast.warnings.len(), 1);
    }
}
