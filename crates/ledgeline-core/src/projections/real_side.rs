//! The REAL side of a projection, computed once and reused by every run over
//! the same journal (plan 29).
//!
//! An edit in the what-if table changes only the forecast — a hundred or so
//! generated transactions. Everything else a run reads comes from the real
//! journal, and the full-scan path ([`super::project`],
//! [`reference_statements`]) re-reads it on every run. Here it is read once
//! into plain values:
//!
//! - [`JournalFacts`]: what depends on the journal alone;
//! - [`RealSide`]: what depends on the journal and a [`RealKey`] — the cutoff,
//!   the interval, the lookback and the valuation commodity.
//!
//! A run is then the forecast plus a seed: [`project_cached`],
//! [`projected_statements`] and [`projected_accounts`]. They return EXACTLY
//! what the full scan over `real ++ forecast` returns, down to each decimal's
//! scale, because the seeds are the same accumulators the full-scan builders
//! use, stopped at the cutoff:
//!
//! - The cutoff and the first lookback day are period starts, so every
//!   lookback column ends before the cutoff and every forecast column starts
//!   on or after it. A real posting dated before the cutoff ("early") lands
//!   only in a lookback column or before them; the forecast lands only in
//!   forecast columns. So the lookback columns are cached whole and the
//!   forecast columns are seeded with the running state at the cutoff.
//! - A real transaction dated before the cutoff can carry a POSTING dated on
//!   or after it. Those postings ("late") are kept, beside their
//!   transaction's closing-entry verdict, and replayed with the forecast.
//! - Every sum is exact (`Dec::add` keeps `max(places)`) into ordered maps,
//!   with no pruning until a snapshot is taken, so regrouping the addends by
//!   seed and tail cannot move a value or a scale.
//!
//! Nothing here reads a clock or holds a lock. The values are owned, `Send`
//! and `Sync`, so a host (the server, the FFI crate) keeps them however it
//! likes and invalidates them when its journal changes.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Commodity, Journal, PriceDirective, Transaction};
use crate::reports::ReportError;
use crate::reports::account_periods::{AccountPeriodsOpts, BalanceMode, account_periods_from};
use crate::reports::account_types::{AccountType, AccountTypes};
use crate::reports::aggregate::add_posting;
use crate::reports::balance_sheet::{
    BalanceSheetReport, BsOpts, RunningTotals, SheetState, Valuation, balance_sheet_columns_seeded,
};
use crate::reports::cash_flow::bucket_totals;
use crate::reports::cash_flow_statement::{
    CashFlowStatement, CfAccumulator, CfClassifier, CfOpts, CfTags,
};
use crate::reports::income_statement::{
    Columns, DateRange, IncomeStatementReport, IsOpts, Totals, income_statement_columns_from,
    window_direct,
};
use crate::reports::periods::{BucketSpan, Interval, add_days, range_buckets};
use crate::reports::prices::PriceDb;
use crate::reports::types::PeriodReport;
use crate::reports::{
    StatementShape, account_periods, balance_sheet_columns, cash_flow_statement,
    income_statement_columns,
};

use super::forecast::{self, RealInputs};
use super::{
    AccountBalances, CounterDefaults, Forecast, ForecastWindow, Openings, Projection, RealRead,
    Scenario, account_balances_at, forecast_cutoff, latest_real_date, merged_prices,
    merged_transactions, roll_forward,
};

// ===========================================================================
// Journal facts and the key
// ===========================================================================

/// What every projection over one journal reads that depends on the journal
/// alone. Build it once per journal (a server: once per snapshot).
#[derive(Debug, Clone)]
pub struct JournalFacts {
    latest_real: Option<String>,
    first_index: u32,
    /// `infer_market_prices` over every real transaction, undated-filtered:
    /// each [`RealSide`] keeps those before its cutoff.
    inferred: Vec<PriceDirective>,
    /// Every declared account and every account a real transaction posts to.
    known: BTreeSet<String>,
    shape: StatementShape,
    cf_tags: CfTags,
}

impl JournalFacts {
    /// Read the facts off `journal`.
    ///
    /// # Errors
    /// [`ReportError`] on decimal overflow while inferring cost prices.
    pub fn new(journal: &Journal) -> Result<Self, ReportError> {
        Ok(Self {
            latest_real: latest_real_date(&journal.transactions).map(str::to_string),
            first_index: forecast::next_index(&journal.transactions),
            inferred: crate::reports::infer_market_prices(&journal.transactions)?,
            known: journal
                .accounts
                .iter()
                .map(|decl| decl.name.0.clone())
                .chain(
                    journal.transactions.iter().flat_map(|txn| {
                        txn.postings.iter().map(|posting| posting.account.0.clone())
                    }),
                )
                .collect(),
            shape: StatementShape::of(journal),
            cf_tags: CfTags::from_journal(journal),
        })
    }

    /// The journal's statement declarations ([`StatementShape::of`]).
    #[must_use]
    pub fn shape(&self) -> &StatementShape {
        &self.shape
    }

    /// The journal's cash flow tags ([`CfTags::from_journal`]).
    #[must_use]
    pub fn cf_tags(&self) -> &CfTags {
        &self.cf_tags
    }

    /// The key a run for `today` reads its real side under: the cutoff is
    /// computed exactly as [`ForecastWindow::resolve`] computes it, so a new
    /// day that stays inside the same period keeps the same key, and one that
    /// crosses into the next does not.
    ///
    /// # Errors
    /// [`ReportError::InvalidBucketKey`] from the bucket math.
    pub fn key(
        &self,
        today: &str,
        interval: Interval,
        lookback: usize,
        value_in: Option<Commodity>,
    ) -> Result<RealKey, ReportError> {
        Ok(RealKey {
            cutoff: forecast_cutoff(today, self.latest_real.as_deref(), interval)?,
            interval,
            lookback,
            value_in,
        })
    }
}

/// Everything a [`RealSide`] depends on besides the journal. `today` is not
/// here: it reaches the real side only through the cutoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealKey {
    /// The first projected day ([`forecast_cutoff`]).
    pub cutoff: String,
    /// The projection interval.
    pub interval: Interval,
    /// How many full actual periods precede the cutoff.
    pub lookback: usize,
    /// The requested valuation commodity, if any.
    pub value_in: Option<Commodity>,
}

// ===========================================================================
// The real side
// ===========================================================================

/// A real transaction dated before the cutoff, holding only its postings
/// dated on or after it — replayed beside the forecast.
#[derive(Debug, Clone)]
struct LateTxn {
    txn: Transaction,
    /// [`crate::reports::is_closing_entry`] of the WHOLE transaction.
    closing: bool,
}

/// Everything a projection run reads from the real journal, for one
/// [`RealKey`]. See the module docs.
#[derive(Debug, Clone)]
pub struct RealSide {
    key: RealKey,
    declared: BTreeMap<String, AccountType>,
    first_index: u32,

    // --- The expansion and the series (`project`). ---
    /// Every real price before the cutoff: inferred, then explicit.
    prices: Vec<PriceDirective>,
    db: PriceDb,
    target: Option<Commodity>,
    balances: AccountBalances,
    /// Built with no `; cash:` header; each run applies its scenario's.
    defaults: CounterDefaults,
    openings: Openings,

    // --- The statements (`merged_transactions`, `merged_prices`). ---
    /// The real EXPLICIT directives before the cutoff, and their table.
    explicit: Vec<PriceDirective>,
    explicit_db: PriceDb,
    /// The commodities `explicit` prices: a forecast price for any other is
    /// left out of the statements.
    priced: BTreeSet<Commodity>,
    /// The lookback columns, oldest first; empty with no lookback.
    lookback: Vec<BucketSpan>,
    late: Vec<LateTxn>,
    /// The balance sheet at each lookback end, and the running totals after
    /// every early posting.
    sheets: Vec<(String, SheetState)>,
    sheet_running: RunningTotals,
    /// Each lookback window's direct totals, as written and at cost.
    written: Vec<Totals>,
    at_cost: Vec<Totals>,
    /// The cash flow pass over every early posting, on the lookback columns.
    cash_flow: CfAccumulator,
    /// Every account's balance before the first lookback day (Balance mode).
    before_from: Totals,
}

impl RealSide {
    /// Read the real side for `key` off `journal`. `facts` must be
    /// [`JournalFacts::new`] of the same journal.
    ///
    /// # Errors
    /// [`ReportError`] on decimal overflow or bucket math.
    pub fn build(
        journal: &Journal,
        facts: &JournalFacts,
        key: &RealKey,
    ) -> Result<Self, ReportError> {
        let txns = &journal.transactions;
        let declared = facts.shape.declared.clone();
        let types = AccountTypes::from_declared(declared.clone());
        let cutoff = key.cutoff.as_str();
        let window =
            ForecastWindow::from_cutoff(key.cutoff.clone(), key.interval, key.lookback, 0)?;
        let as_of = window.actuals_as_of();

        // Exactly `real_prices`: inferred then explicit, before the cutoff.
        let mut prices = facts.inferred.clone();
        prices.extend_from_slice(&journal.prices);
        prices.retain(|price| price.date.as_str() < cutoff);
        let db = PriceDb::build(&prices);
        let target = key
            .value_in
            .clone()
            .or_else(|| db.base_commodity().cloned());
        let balances = account_balances_at(txns, &as_of)?;
        let defaults = CounterDefaults::from_parts(
            txns,
            &types,
            cutoff,
            None,
            &balances,
            target.as_ref(),
            &db,
        )?;
        let openings = Openings::scan(
            txns,
            &prices,
            &db,
            &balances,
            &window,
            target.as_ref(),
            &types,
            &declared,
        )?;

        let explicit: Vec<PriceDirective> = journal
            .prices
            .iter()
            .filter(|price| price.date.as_str() < cutoff)
            .cloned()
            .collect();
        let explicit_db = PriceDb::build(&explicit);
        let priced = explicit
            .iter()
            .map(|price| price.commodity.clone())
            .collect();
        let lookback = if key.lookback == 0 {
            Vec::new()
        } else {
            range_buckets(&window.from, &as_of, Some(key.interval))?
        };
        let seeds = Seeds::scan(txns, cutoff, &window.from, &as_of, &lookback, facts)?;

        Ok(Self {
            key: key.clone(),
            declared,
            first_index: facts.first_index,
            prices,
            db,
            target,
            balances,
            defaults,
            openings,
            explicit,
            explicit_db,
            priced,
            lookback,
            late: seeds.late,
            sheets: seeds.sheets,
            sheet_running: seeds.sheet_running,
            written: seeds.written,
            at_cost: seeds.at_cost,
            cash_flow: seeds.cash_flow,
            before_from: seeds.before_from,
        })
    }

    /// The key this real side was built for.
    #[must_use]
    pub fn key(&self) -> &RealKey {
        &self.key
    }

    /// `forecast`'s statement columns, after checking it was expanded against
    /// this real side: the same cutoff, interval and lookback columns.
    fn columns(&self, forecast: &Forecast) -> Result<Vec<BucketSpan>, ReportError> {
        let window = &forecast.window;
        let spans = range_buckets(&window.from, &window.end, Some(window.interval))?;
        let matches = window.cutoff == self.key.cutoff
            && window.interval == self.key.interval
            && spans.len() >= self.lookback.len()
            && spans[..self.lookback.len()] == self.lookback[..];
        if !matches {
            return Err(ReportError::InvalidBucketKey(format!(
                "the forecast window from {} is not the real side's (cutoff {})",
                window.from, self.key.cutoff
            )));
        }
        Ok(spans)
    }

    /// The late real postings, then the forecast: everything after the seed.
    fn tail<'f>(&self, forecast: &'f Forecast) -> Cow<'f, [Transaction]> {
        if self.late.is_empty() {
            Cow::Borrowed(&forecast.txns)
        } else {
            Cow::Owned(
                self.late
                    .iter()
                    .map(|late| late.txn.clone())
                    .chain(forecast.txns.iter().cloned())
                    .collect(),
            )
        }
    }

    /// `merged_prices`' table: the real explicit directives, plus the
    /// forecast's for commodities they price. Without any of the latter it is
    /// the real table as it stands, which is what building it again would give.
    fn statement_prices(&self, forecast: &Forecast) -> Cow<'_, PriceDb> {
        let extra: Vec<&PriceDirective> = forecast
            .prices
            .iter()
            .filter(|price| self.priced.contains(&price.commodity))
            .collect();
        if extra.is_empty() {
            Cow::Borrowed(&self.explicit_db)
        } else {
            let all: Vec<PriceDirective> = self.explicit.iter().chain(extra).cloned().collect();
            Cow::Owned(PriceDb::build(&all))
        }
    }
}

/// The statement seeds, gathered in one pass over the real transactions
/// `merged_transactions` keeps.
struct Seeds {
    late: Vec<LateTxn>,
    sheets: Vec<(String, SheetState)>,
    sheet_running: RunningTotals,
    written: Vec<Totals>,
    at_cost: Vec<Totals>,
    cash_flow: CfAccumulator,
    before_from: Totals,
}

impl Seeds {
    /// Every real transaction dated before `cutoff`, split by posting date
    /// into the seeds (early) and [`LateTxn`]s (late).
    ///
    /// Each seed sums exactly as its builder's own pass does: the balance
    /// sheet's `RunningTotals::add`, the income statement's and accounts'
    /// `account_totals`/`bucket_totals` (one `add_posting` per posting, pruned
    /// once at the end), the cash flow statement's `CfAccumulator::add`
    /// (whose `to` of the day before the cutoff leaves every late posting for
    /// the tail), and Balance mode's `account_totals(to: from − 1)`.
    fn scan(
        txns: &[Transaction],
        cutoff: &str,
        from: &str,
        as_of: &str,
        lookback: &[BucketSpan],
        facts: &JournalFacts,
    ) -> Result<Self, ReportError> {
        let classify = CfClassifier::new(&facts.cf_tags);
        let mut cash_flow =
            CfAccumulator::new(lookback.to_vec(), as_of, facts.cf_tags.base.clone());
        let ends: Vec<&str> = lookback.iter().map(|span| span.to.as_str()).collect();
        // One slot per lookback end, and one past the last for postings after
        // it (only when there is no lookback: every early posting is on or
        // before the day before the cutoff, the last lookback end).
        let mut slots: Vec<RunningTotals> = vec![RunningTotals::default(); ends.len() + 1];
        let mut written: Vec<Totals> = vec![Totals::new(); lookback.len()];
        let mut at_cost: Vec<Totals> = vec![Totals::new(); lookback.len()];
        let mut before_from = Totals::new();
        let before_day = add_days(from, -1);
        let mut late = Vec::new();

        for txn in txns.iter().filter(|txn| txn.date.as_str() < cutoff) {
            let closing = classify.is_closing(txn);
            let mut tail = Vec::new();
            for posting in &txn.postings {
                let date = posting.date.as_deref().unwrap_or(&txn.date);
                if date >= cutoff {
                    tail.push(posting.clone());
                    continue;
                }
                slots[ends.partition_point(|end| *end < date)].add(posting)?;
                let window = lookback.partition_point(|span| span.to.as_str() < date);
                if lookback
                    .get(window)
                    .is_some_and(|span| date >= span.from.as_str())
                {
                    add_posting(&mut written[window], posting, false)?;
                    add_posting(&mut at_cost[window], posting, true)?;
                }
                if date <= before_day.as_str() {
                    add_posting(&mut before_from, posting, false)?;
                }
            }
            cash_flow.add(txn, closing, &classify)?;
            if !tail.is_empty() {
                late.push(LateTxn {
                    txn: Transaction {
                        postings: tail,
                        ..txn.clone()
                    },
                    closing,
                });
            }
        }

        let mut sheet_running = RunningTotals::default();
        let mut sheets = Vec::with_capacity(lookback.len());
        for (end, slot) in ends.iter().zip(&slots) {
            sheet_running.absorb(slot)?;
            sheets.push(((*end).to_string(), sheet_running.state()));
        }
        if let Some(rest) = slots.last() {
            sheet_running.absorb(rest)?;
        }
        Ok(Self {
            late,
            sheets,
            sheet_running,
            written: written.into_iter().map(pruned).collect(),
            at_cost: at_cost.into_iter().map(pruned).collect(),
            cash_flow,
            before_from: pruned(before_from),
        })
    }
}

/// `account_totals`' final sweep: zero commodities dropped.
fn pruned(mut totals: Totals) -> Totals {
    for ma in totals.values_mut() {
        ma.drop_zeros();
    }
    totals
}

// ===========================================================================
// A run
// ===========================================================================

/// [`super::project`] against a cached real side: the same [`Projection`] it
/// returns for the same scenario, `count` and `depth`, with `today`,
/// `interval`, `lookback`, `value_in` and the declared types those `real` was
/// built for.
///
/// # Errors
/// [`ReportError`] on decimal overflow or bucket math.
pub fn project_cached(
    scenario: &Scenario,
    real: &RealSide,
    count: usize,
    depth: usize,
) -> Result<Projection, ReportError> {
    let types = AccountTypes::from_declared(real.declared.clone());
    let window = ForecastWindow::from_cutoff(
        real.key.cutoff.clone(),
        real.key.interval,
        real.key.lookback,
        count,
    )?;
    let forecast = forecast::expand_from(
        scenario,
        &types,
        RealInputs {
            window,
            prices: &real.prices,
            db: &real.db,
            target: real.target.clone(),
            balances: &real.balances,
            defaults: &real.defaults,
            first_index: real.first_index,
        },
    )?;
    roll_forward(
        forecast,
        &RealRead {
            openings: &real.openings,
            balances: &real.balances,
            prices: &real.prices,
            db: &real.db,
        },
        &types,
        depth,
    )
}

/// How the projected statements are presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementOpts {
    /// The basis of the balance sheet and the income statement. The run
    /// endpoint uses [`Valuation::Market`].
    pub valuation: Valuation,
    /// The balance sheet's row depth (`None`: unclamped).
    pub bs_depth: Option<usize>,
    /// The income statement's row depth, as `hledger is --depth` (`None`:
    /// every account).
    pub is_depth: Option<usize>,
    /// The cash flow statement's line depth.
    pub cf_depth: usize,
}

/// The three statements over `lookback ++ buckets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedStatements {
    /// The statement columns ([`range_buckets`] of the window).
    pub spans: Vec<BucketSpan>,
    /// One balance sheet per column end.
    pub balance_sheet: Vec<BalanceSheetReport>,
    pub income_statement: IncomeStatementReport<Columns>,
    pub cash_flow: CashFlowStatement,
}

/// The Accounts reading's options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountsOpts {
    pub mode: BalanceMode,
    pub depth: usize,
    /// List every known account, zero or not ([`super::known_accounts`]).
    pub empty: bool,
}

/// The three statements over `real ++ forecast`, from a cached real side:
/// exactly [`reference_statements`] over the same journal and forecast.
/// `forecast` must have been expanded against `real` ([`project_cached`]).
///
/// # Errors
/// [`ReportError::InvalidBucketKey`] for a forecast of another window;
/// [`ReportError`] on decimal overflow.
pub fn projected_statements(
    real: &RealSide,
    facts: &JournalFacts,
    forecast: &Forecast,
    opts: &StatementOpts,
) -> Result<ProjectedStatements, ReportError> {
    let window = &forecast.window;
    let spans = real.columns(forecast)?;
    let ends: Vec<String> = spans.iter().map(|span| span.to.clone()).collect();
    let tail = real.tail(forecast);
    let db = real.statement_prices(forecast);
    let shape = &facts.shape;

    let balance_sheet = balance_sheet_columns_seeded(
        &real.sheets,
        &real.sheet_running,
        &tail,
        &db,
        &BsOpts {
            as_of: &window.end,
            depth: opts.bs_depth,
            value: opts.valuation,
            value_in: forecast.target.clone(),
        },
        &ends,
        &shape.declared,
        &shape.bs_groups,
        &shape.bs_terms,
    )?;

    let cost = opts.valuation == Valuation::Cost;
    let seeded = if cost { &real.at_cost } else { &real.written };
    let direct = spans
        .iter()
        .enumerate()
        .map(|(index, span)| match seeded.get(index) {
            Some(totals) => Ok(totals.clone()),
            None => window_direct(&tail, &DateRange::from(span), cost),
        })
        .collect::<Result<Vec<_>, ReportError>>()?;
    let income_statement = income_statement_columns_from(
        direct,
        &db,
        &IsOpts {
            from: &window.from,
            to: &window.end,
            value: opts.valuation,
            value_in: forecast.target.clone(),
            compare: false,
            depth: opts.is_depth,
        },
        window.interval,
        &shape.declared,
        &shape.is_sections,
        &shape.is_groups,
    )?;

    let classify = CfClassifier::new(&facts.cf_tags);
    let mut cash_flow = real.cash_flow.clone();
    cash_flow.extend(spans.clone(), &window.end);
    for late in &real.late {
        cash_flow.add(&late.txn, late.closing, &classify)?;
    }
    for txn in &forecast.txns {
        cash_flow.add(txn, classify.is_closing(txn), &classify)?;
    }
    let cash_flow = cash_flow.finish(
        &facts.cf_tags,
        &CfOpts {
            from: &window.from,
            to: &window.end,
            interval: Some(window.interval),
            depth: opts.cf_depth,
        },
    )?;

    Ok(ProjectedStatements {
        spans,
        balance_sheet,
        income_statement,
        cash_flow,
    })
}

/// The Accounts reading over `real ++ forecast`, from a cached real side:
/// exactly [`reference_accounts`].
///
/// # Errors
/// As [`projected_statements`].
pub fn projected_accounts(
    real: &RealSide,
    facts: &JournalFacts,
    forecast: &Forecast,
    opts: &AccountsOpts,
) -> Result<PeriodReport, ReportError> {
    let window = &forecast.window;
    let spans = real.columns(forecast)?;
    let tail = real.tail(forecast);
    let ranges: Vec<(String, String)> = spans
        .iter()
        .map(|span| (span.from.clone(), span.to.clone()))
        .collect();
    let direct = bucket_totals(&tail, &ranges, &|_| true)?
        .into_iter()
        .enumerate()
        .map(|(index, tail)| real.written.get(index).cloned().unwrap_or(tail))
        .collect();
    let known = opts.empty.then(|| known_with(&facts.known, forecast));
    account_periods_from(
        direct,
        || Ok(real.before_from.clone()),
        &AccountPeriodsOpts {
            from: &window.from,
            to: &window.end,
            interval: window.interval,
            depth: opts.depth,
            mode: opts.mode,
            known: known.as_deref(),
        },
    )
}

/// [`super::known_accounts`] from the journal's half, already gathered.
fn known_with(journal: &BTreeSet<String>, forecast: &Forecast) -> Vec<String> {
    let generated: BTreeSet<&str> = forecast
        .txns
        .iter()
        .flat_map(|txn| {
            txn.postings
                .iter()
                .map(|posting| posting.account.0.as_str())
        })
        .collect();
    journal
        .iter()
        .map(String::as_str)
        .chain(generated)
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .map(str::to_string)
        .collect()
}

// ===========================================================================
// The reference: the full scan over the merged lists
// ===========================================================================

/// The three statements as the run endpoint built them before plan 29: every
/// builder over [`merged_transactions`] and [`merged_prices`]. The definition
/// of what [`projected_statements`] must return, kept for tests and benches.
///
/// # Errors
/// [`ReportError`] on decimal overflow or bucket math.
pub fn reference_statements(
    journal: &Journal,
    facts: &JournalFacts,
    forecast: &Forecast,
    opts: &StatementOpts,
) -> Result<ProjectedStatements, ReportError> {
    let window = &forecast.window;
    let txns = merged_transactions(&journal.transactions, forecast);
    let prices = merged_prices(&journal.prices, forecast);
    let spans = range_buckets(&window.from, &window.end, Some(window.interval))?;
    let ends: Vec<String> = spans.iter().map(|span| span.to.clone()).collect();
    let shape = &facts.shape;
    let balance_sheet = balance_sheet_columns(
        &txns,
        &prices,
        &BsOpts {
            as_of: &window.end,
            depth: opts.bs_depth,
            value: opts.valuation,
            value_in: forecast.target.clone(),
        },
        &ends,
        &shape.declared,
        &shape.bs_groups,
        &shape.bs_terms,
    )?;
    let income_statement = income_statement_columns(
        &txns,
        &prices,
        &IsOpts {
            from: &window.from,
            to: &window.end,
            value: opts.valuation,
            value_in: forecast.target.clone(),
            compare: false,
            depth: opts.is_depth,
        },
        window.interval,
        &shape.declared,
        &shape.is_sections,
        &shape.is_groups,
    )?;
    let cash_flow = cash_flow_statement(
        &txns,
        &CfOpts {
            from: &window.from,
            to: &window.end,
            interval: Some(window.interval),
            depth: opts.cf_depth,
        },
        &CfTags::from_journal(journal),
    )?;
    Ok(ProjectedStatements {
        spans,
        balance_sheet,
        income_statement,
        cash_flow,
    })
}

/// The Accounts reading as the run endpoint built it before plan 29:
/// `account_periods` over [`merged_transactions`], with
/// [`super::known_accounts`]. The definition of what [`projected_accounts`]
/// must return.
///
/// # Errors
/// [`ReportError`] on decimal overflow or bucket math.
pub fn reference_accounts(
    journal: &Journal,
    forecast: &Forecast,
    opts: &AccountsOpts,
) -> Result<PeriodReport, ReportError> {
    let window = &forecast.window;
    let txns = merged_transactions(&journal.transactions, forecast);
    let known = opts.empty.then(|| super::known_accounts(journal, forecast));
    account_periods(
        &txns,
        &AccountPeriodsOpts {
            from: &window.from,
            to: &window.end,
            interval: window.interval,
            depth: opts.depth,
            mode: opts.mode,
            known: known.as_deref(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `Debug` of a value: prints every `Dec`'s mantissa AND scale, so two
    /// values that are equal by value but written at different scales differ.
    fn exact<T: std::fmt::Debug>(value: &T) -> String {
        format!("{value:?}")
    }

    fn journal(text: &str) -> Journal {
        crate::parse_journal(text, "test.journal").expect("journal parses")
    }

    const MAIN: &str = "\
account assets:checking  ; type: C
account assets:broker    ; type: A
account liabilities:card ; type: L
account equity:opening   ; type: E
account income:salary    ; type: R
account expenses:food    ; type: X
account expenses:rent    ; type: X

P 2026-01-01 AAPL $100
P 2026-08-01 AAPL $120

2026-01-01 opening
    assets:checking   $5,000.00
    equity:opening

2026-02-01 buy
    assets:broker     10 AAPL @ $100
    assets:checking

2026-07-15 salary
    income:salary     $-3,000.00
    assets:checking

2026-08-20 dinner, paid in september
    expenses:food     $40.125
    liabilities:card  ; date: 2026-10-02

2026-09-10 rent
    expenses:rent     $1,200
    assets:checking

2026-10-05 after the cutoff
    expenses:food     $7
    assets:checking
";

    #[test]
    fn real_prices_are_the_expansions_own() {
        let journal = journal(MAIN);
        let facts = JournalFacts::new(&journal).unwrap();
        let key = facts.key("2026-10-20", Interval::Monthly, 2, None).unwrap();
        let real = RealSide::build(&journal, &facts, &key).unwrap();
        assert_eq!(
            exact(&real.prices),
            exact(
                &forecast::real_prices(
                    &super::super::History {
                        txns: &journal.transactions,
                        prices: &journal.prices,
                    },
                    &key.cutoff
                )
                .unwrap()
            )
        );
    }

    #[test]
    fn a_posting_dated_after_the_cutoff_is_kept_for_the_tail() {
        let journal = journal(MAIN);
        let facts = JournalFacts::new(&journal).unwrap();
        let key = facts.key("2026-10-20", Interval::Monthly, 2, None).unwrap();
        assert_eq!(key.cutoff, "2026-10-01");
        let real = RealSide::build(&journal, &facts, &key).unwrap();
        assert_eq!(real.late.len(), 1);
        assert_eq!(real.late[0].txn.postings.len(), 1);
        assert_eq!(real.late[0].txn.postings[0].account.0, "liabilities:card");
    }

    #[test]
    fn the_key_moves_with_the_period_not_the_day() {
        let journal = journal(MAIN);
        let facts = JournalFacts::new(&journal).unwrap();
        let day = |today| facts.key(today, Interval::Monthly, 2, None).unwrap();
        // The latest real transaction is 2026-10-05, so October is incomplete
        // until November is reached; the cutoff stays October 1st.
        assert_eq!(day("2026-10-20"), day("2026-10-21"));
        assert_eq!(day("2026-10-20"), day("2026-11-03"));
        // A journal whose last transaction is the 5th answers by today.
        let earlier = facts.key("2026-09-30", Interval::Monthly, 2, None).unwrap();
        assert_eq!(earlier.cutoff, "2026-09-01");
        assert_ne!(earlier, day("2026-10-20"));
    }
}
