//! Compare vs actuals: backtest a projection over its own actual periods
//! (plan 34).
//!
//! A backtest is an ordinary projection whose cutoff is the first day of the
//! first actual period (`F`) instead of today's period start (`C`), run
//! forward over exactly the `N` actual periods (D1):
//!
//! ```text
//! main      RealKey {cutoff: C, interval, lookback: N, value_in}
//! backtest  RealKey {cutoff: F, interval, lookback: 0, value_in}
//! ```
//!
//! [`super::RealSide::build`] under the backtest key already reads only what
//! was known on `F`: the balances the day before it, the prices dated before
//! it, and no real transaction dated on or after it. So nothing real inside
//! the window can leak into the as-projected side.
//!
//! Two deviations from "the run at `asOf = F`", both settled with Patrick:
//!
//! - a counter-less line takes the FORWARD run's inferred default, the one
//!   the editor shows and a save would write (D1);
//! - an asset row's `opening:` override is ignored: it states today's
//!   balance, and the backtest starts every account from its real balance on
//!   `F` (D2). The scenario is never edited; the expansion reads the override
//!   as absent.
//!
//! The joined statements put the actual block (the forward run's lookback
//! columns, from the main side's seeds) and the as-projected block (the
//! backtest's columns, keys prefixed [`PROJECTED_KEY_PREFIX`]) side by side
//! as ONE report of each type, so every reading renders them with the code it
//! already has.

use crate::model::{Commodity, Journal, PriceDirective, Transaction};
use crate::reports::ReportError;
use crate::reports::account_periods::before;
use crate::reports::account_report::{AccountReport, AccountReportOpts, account_report_restarting};
use crate::reports::account_types::AccountTypes;
use crate::reports::balance_sheet::{BalanceSheetReport, BsOpts, Valuation};
use crate::reports::cash_flow::bucket_totals;
use crate::reports::cash_flow_statement::{CashFlowStatement, CfAccumulator, CfClassifier, CfOpts};
use crate::reports::income_statement::{
    Columns, DateRange, IncomeStatementReport, IsOpts, Totals, income_statement_windows,
    window_direct,
};
use crate::reports::periods::{BucketSpan, add_days, range_buckets};
use crate::reports::prices::PriceDb;
use crate::reports::{account_decls, balance_sheet_columns, declared_types};

use super::real_side::{
    AccountsOpts, JournalFacts, RealKey, RealSide, StatementOpts, cash_flow_pass, known_with,
    run_with, seeded_direct, seeded_sheets,
};
use super::{Forecast, ForecastWindow, Projection, Scenario, merged_prices, merged_transactions};

/// The prefix on every as-projected column key (`projected:2026-08`), so a
/// joined report never repeats a key. Labels and bounds see through it.
pub use crate::reports::periods::PROJECTED_KEY_PREFIX;

impl RealKey {
    /// D1: the key of the projection made on the first actual day: cutoff
    /// `F`, the first lookback day, with no lookback of its own. `None` at
    /// lookback 0, where there is nothing to compare. Pure, no clock.
    ///
    /// # Errors
    /// [`ReportError::InvalidBucketKey`] from the bucket math.
    pub fn backtest(&self) -> Result<Option<Self>, ReportError> {
        if self.lookback == 0 {
            return Ok(None);
        }
        let from =
            ForecastWindow::from_cutoff(self.cutoff.clone(), self.interval, self.lookback, 0)?.from;
        Ok(Some(Self {
            cutoff: from,
            interval: self.interval,
            lookback: 0,
            value_in: self.value_in.clone(),
        }))
    }
}

/// D1: expand `scenario` against `backtest`, a real side built for
/// `main.key().backtest()`, over the N lookback periods of `main`, with
/// `main`'s counter defaults (the scenario's `; cash:` header applied inside
/// the expansion, as always). `opening:` overrides read as absent (D2).
///
/// # Errors
/// [`ReportError::InvalidBucketKey`] when `backtest` is not `main`'s backtest
/// side; [`ReportError`] on decimal overflow or bucket math.
pub fn project_backtest(
    scenario: &Scenario,
    main: &RealSide,
    backtest: &RealSide,
    depth: usize,
) -> Result<Projection, ReportError> {
    check_pair(main, backtest)?;
    run_with(
        scenario,
        backtest,
        &main.defaults,
        main.key.lookback,
        depth,
        true,
    )
}

/// `backtest` must be the side of `main.key().backtest()`.
fn check_pair(main: &RealSide, backtest: &RealSide) -> Result<(), ReportError> {
    if main.key.backtest()?.as_ref() == Some(&backtest.key) {
        Ok(())
    } else {
        Err(ReportError::InvalidBucketKey(format!(
            "the real side at {} is not the backtest of the one at {} (lookback {})",
            backtest.key.cutoff, main.key.cutoff, main.key.lookback
        )))
    }
}

/// The three statements over `actual ++ projected`: 2N columns, the actual
/// block first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparedStatements {
    /// The N actual periods (the forward run's lookback columns).
    pub actual: Vec<BucketSpan>,
    /// The same N periods, keys prefixed [`PROJECTED_KEY_PREFIX`].
    pub projected: Vec<BucketSpan>,
    /// One balance sheet per column end, 2N of them.
    pub balance_sheet: Vec<BalanceSheetReport>,
    /// Interval mode, 2N buckets. Sections, `multi_step` and the subtotal
    /// ladder are resolved over both blocks at once.
    pub income_statement: IncomeStatementReport<Columns>,
    /// 2N buckets; the cash balances start again at the as-projected block.
    pub cash_flow: CashFlowStatement,
}

impl ComparedStatements {
    /// Every column, actual then as projected.
    #[must_use]
    pub fn spans(&self) -> Vec<BucketSpan> {
        self.actual.iter().chain(&self.projected).cloned().collect()
    }
}

/// `spans` with every key prefixed [`PROJECTED_KEY_PREFIX`].
fn prefixed(spans: &[BucketSpan]) -> Vec<BucketSpan> {
    spans
        .iter()
        .map(|span| BucketSpan {
            key: format!("{PROJECTED_KEY_PREFIX}{}", span.key),
            ..span.clone()
        })
        .collect()
}

/// The joined report's range: the first actual day to the day before `C`.
fn range(main: &RealSide) -> Result<(String, String), ReportError> {
    let first = main
        .lookback
        .first()
        .map(|span| span.from.clone())
        .ok_or_else(|| {
            ReportError::InvalidBucketKey("a comparison needs at least one actual period".into())
        })?;
    Ok((first, add_days(&main.key.cutoff, -1)))
}

/// The three statements over the actual block (from `main`'s seeds, no
/// forecast) and the as-projected block (`backtest` plus `forecast`, the
/// backtest's own expansion: [`project_backtest`]'s `forecast`).
///
/// Both blocks are valued into `main`'s target (D3); the actual block at the
/// real explicit prices to each column's end, the as-projected block at the
/// real explicit prices before `F` plus the forecast's grown prices.
///
/// # Errors
/// [`ReportError::InvalidBucketKey`] for a mismatched pair or forecast;
/// [`ReportError`] on decimal overflow.
pub fn compared_statements(
    main: &RealSide,
    backtest: &RealSide,
    facts: &JournalFacts,
    forecast: &Forecast,
    opts: &StatementOpts,
) -> Result<ComparedStatements, ReportError> {
    check_pair(main, backtest)?;
    let (from, to) = range(main)?;
    let actual = main.lookback.clone();
    let spans = backtest.columns(forecast)?;
    let projected = prefixed(&spans);
    let tail = backtest.tail(forecast);
    let projected_db = backtest.statement_prices(forecast);
    let target = main.target.clone();
    let shape = facts.shape();

    // Balance sheet: one report per column end, each block from its own side.
    let mut balance_sheet = seeded_sheets(
        main,
        facts,
        &[],
        &main.explicit_db,
        &actual,
        target.clone(),
        opts,
    )?;
    balance_sheet.extend(seeded_sheets(
        backtest,
        facts,
        &tail,
        &projected_db,
        &spans,
        target.clone(),
        opts,
    )?);

    // Income statement: ONE build over 2N windows, each valued through its
    // block's table, so the ladder sees both blocks at once.
    let mut direct = seeded_direct(main, &actual, &[], opts.valuation)?;
    direct.extend(seeded_direct(backtest, &spans, &tail, opts.valuation)?);
    let tables: Vec<&PriceDb> = std::iter::repeat_n(&main.explicit_db, actual.len())
        .chain(std::iter::repeat_n(&*projected_db, spans.len()))
        .collect();
    let income_statement = income_statement_windows(
        direct,
        actual.iter().chain(&projected).cloned().collect(),
        &tables,
        &IsOpts {
            from: &from,
            to: &to,
            value: opts.valuation,
            value_in: target,
            compare: false,
            depth: opts.is_depth,
        },
        &shape.declared,
        &shape.is_sections,
        &shape.is_groups,
    )?;

    // Cash flow: the two passes side by side, finished once, so both blocks
    // share sections, lines and the investments set.
    let cash_flow = main
        .cash_flow
        .clone()
        .beside(cash_flow_pass(
            backtest,
            facts,
            projected.clone(),
            forecast,
        )?)
        .finish(
            facts.cf_tags(),
            &CfOpts {
                from: &from,
                to: &to,
                interval: Some(main.key.interval),
                depth: opts.cf_depth,
            },
        )?;

    Ok(ComparedStatements {
        actual,
        projected,
        balance_sheet,
        income_statement,
        cash_flow,
    })
}

/// The Accounts reading over the actual block and the as-projected block,
/// unvalued (D3): each block's balances open from its own side's balances
/// before `F`.
///
/// # Errors
/// As [`compared_statements`].
pub fn compared_accounts(
    main: &RealSide,
    backtest: &RealSide,
    facts: &JournalFacts,
    forecast: &Forecast,
    opts: &AccountsOpts,
) -> Result<AccountReport, ReportError> {
    check_pair(main, backtest)?;
    let (from, to) = range(main)?;
    let spans = backtest.columns(forecast)?;
    let tail = backtest.tail(forecast);
    let ranges: Vec<(String, String)> = spans
        .iter()
        .map(|span| (span.from.clone(), span.to.clone()))
        .collect();
    let projected = bucket_totals(&tail, &ranges, &|_| true)?;
    let direct: Vec<Totals> = main.written.iter().cloned().chain(projected).collect();
    let buckets = main
        .lookback
        .iter()
        .chain(&prefixed(&spans))
        .map(|span| span.key.clone())
        .collect();
    let known = opts
        .empty
        .then(|| known_with(facts.known_accounts(), forecast));
    let types = AccountTypes::from_declared(facts.shape().declared.clone());
    account_report_restarting(
        direct,
        || Ok(main.before_from.clone()),
        Some((main.lookback.len(), backtest.before_from.clone())),
        buckets,
        &AccountReportOpts {
            from: &from,
            to: &to,
            interval: Some(main.key.interval),
            depth: opts.depth,
            known: known.as_deref(),
            types: &types,
        },
    )
}

/// What a comparison adds to the forward run's warnings: the backtest's own
/// warnings the forward run does not already hold, and D3's target mismatch.
#[must_use]
pub fn backtest_warnings(forward: &Projection, backtest: &Projection) -> Vec<String> {
    let mut warnings: Vec<String> = backtest
        .warnings
        .iter()
        .filter(|warning| !forward.warnings.contains(warning))
        .cloned()
        .collect();
    let (actual, projected) = (&forward.forecast.target, &backtest.forecast.target);
    if actual != projected {
        let name = |target: &Option<crate::model::Commodity>| {
            target
                .as_ref()
                .map_or_else(|| "no commodity".to_string(), |c| c.0.clone())
        };
        warnings.push(format!(
            "the actual periods are valued in {} and the projection made on {} would have \
             valued in {}, so the as-projected columns are shown in {} too",
            name(actual),
            backtest.start,
            name(projected),
            name(actual),
        ));
    }
    warnings
}

// ===========================================================================
// The reference: the full scan over the merged lists
// ===========================================================================

/// [`compared_statements`] by the full scan, the definition it must equal:
/// the actual block from every real transaction dated before `C` and every
/// explicit `P` before it; the as-projected block from
/// [`merged_transactions`] and [`merged_prices`] over the backtest
/// `forecast`. Each through the Reports builders' own posting passes, then
/// joined by the same code. For tests and benches only.
///
/// # Errors
/// As [`compared_statements`].
pub fn reference_compared_statements(
    journal: &Journal,
    facts: &JournalFacts,
    main_key: &RealKey,
    forecast: &Forecast,
    opts: &StatementOpts,
) -> Result<ComparedStatements, ReportError> {
    let full = FullScan::new(journal, main_key, forecast)?;
    let shape = facts.shape();
    let bs = |txns: &[Transaction], prices: &[PriceDirective], spans: &[BucketSpan]| {
        let ends: Vec<String> = spans.iter().map(|span| span.to.clone()).collect();
        balance_sheet_columns(
            txns,
            prices,
            &BsOpts {
                as_of: &full.to,
                depth: opts.bs_depth,
                value: opts.valuation,
                value_in: full.target.clone(),
            },
            &ends,
            &shape.declared,
            &shape.bs_groups,
            &shape.bs_terms,
        )
    };
    let mut balance_sheet = bs(&full.actual_txns, &full.actual_prices, &full.actual)?;
    balance_sheet.extend(bs(
        &full.projected_txns,
        &full.projected_prices,
        &full.spans,
    )?);

    let cost = opts.valuation == Valuation::Cost;
    let direct = full
        .actual
        .iter()
        .map(|span| window_direct(&full.actual_txns, &DateRange::from(span), cost))
        .chain(
            full.spans
                .iter()
                .map(|span| window_direct(&full.projected_txns, &DateRange::from(span), cost)),
        )
        .collect::<Result<Vec<_>, ReportError>>()?;
    let actual_db = PriceDb::build(&full.actual_prices);
    let projected_db = PriceDb::build(&full.projected_prices);
    let tables: Vec<&PriceDb> = std::iter::repeat_n(&actual_db, full.actual.len())
        .chain(std::iter::repeat_n(&projected_db, full.spans.len()))
        .collect();
    let income_statement = income_statement_windows(
        direct,
        full.actual.iter().chain(&full.projected).cloned().collect(),
        &tables,
        &IsOpts {
            from: &full.from,
            to: &full.to,
            value: opts.valuation,
            value_in: full.target.clone(),
            compare: false,
            depth: opts.is_depth,
        },
        &shape.declared,
        &shape.is_sections,
        &shape.is_groups,
    )?;

    let tags = facts.cf_tags();
    let classify = CfClassifier::new(tags);
    let pass = |spans: Vec<BucketSpan>, txns: &[Transaction]| {
        let mut sums = CfAccumulator::new(spans, &full.to, tags.base.clone());
        for txn in txns {
            sums.add(txn, classify.is_closing(txn), &classify)?;
        }
        Ok::<_, ReportError>(sums)
    };
    let cash_flow = pass(full.actual.clone(), &full.actual_txns)?
        .beside(pass(full.projected.clone(), &full.projected_txns)?)
        .finish(
            tags,
            &CfOpts {
                from: &full.from,
                to: &full.to,
                interval: Some(main_key.interval),
                depth: opts.cf_depth,
            },
        )?;

    Ok(ComparedStatements {
        actual: full.actual,
        projected: full.projected,
        balance_sheet,
        income_statement,
        cash_flow,
    })
}

/// [`compared_accounts`] by the full scan; see
/// [`reference_compared_statements`].
///
/// # Errors
/// As [`compared_accounts`].
pub fn reference_compared_accounts(
    journal: &Journal,
    main_key: &RealKey,
    forecast: &Forecast,
    opts: &AccountsOpts,
) -> Result<AccountReport, ReportError> {
    let full = FullScan::new(journal, main_key, forecast)?;
    let ranges = |spans: &[BucketSpan]| -> Vec<(String, String)> {
        spans
            .iter()
            .map(|span| (span.from.clone(), span.to.clone()))
            .collect()
    };
    let mut direct = bucket_totals(&full.actual_txns, &ranges(&full.actual), &|_| true)?;
    direct.extend(bucket_totals(
        &full.projected_txns,
        &ranges(&full.spans),
        &|_| true,
    )?);
    let known = opts.empty.then(|| super::known_accounts(journal, forecast));
    let types = AccountTypes::from_declared(declared_types(&account_decls(journal)));
    account_report_restarting(
        direct,
        || before(&full.actual_txns, &full.from),
        Some((full.actual.len(), before(&full.projected_txns, &full.from)?)),
        full.actual
            .iter()
            .chain(&full.projected)
            .map(|span| span.key.clone())
            .collect(),
        &AccountReportOpts {
            from: &full.from,
            to: &full.to,
            interval: Some(main_key.interval),
            depth: opts.depth,
            known: known.as_deref(),
            types: &types,
        },
    )
}

/// The two blocks' inputs, read straight off the journal.
struct FullScan {
    from: String,
    to: String,
    target: Option<Commodity>,
    actual: Vec<BucketSpan>,
    /// The as-projected columns, unprefixed and prefixed.
    spans: Vec<BucketSpan>,
    projected: Vec<BucketSpan>,
    actual_txns: Vec<Transaction>,
    actual_prices: Vec<PriceDirective>,
    projected_txns: Vec<Transaction>,
    projected_prices: Vec<PriceDirective>,
}

impl FullScan {
    fn new(
        journal: &Journal,
        main_key: &RealKey,
        forecast: &Forecast,
    ) -> Result<Self, ReportError> {
        let backtest = main_key.backtest()?.ok_or_else(|| {
            ReportError::InvalidBucketKey("a comparison needs at least one actual period".into())
        })?;
        if forecast.window.cutoff != backtest.cutoff {
            return Err(ReportError::InvalidBucketKey(format!(
                "the forecast from {} is not the backtest from {}",
                forecast.window.cutoff, backtest.cutoff
            )));
        }
        let cutoff = main_key.cutoff.as_str();
        let from = backtest.cutoff;
        let to = add_days(cutoff, -1);
        let interval = Some(main_key.interval);
        let real = super::forecast::real_prices(
            &super::History {
                txns: &journal.transactions,
                prices: &journal.prices,
            },
            cutoff,
        )?;
        let target = main_key
            .value_in
            .clone()
            .or_else(|| PriceDb::build(&real).base_commodity().cloned());
        let spans = range_buckets(&forecast.window.from, &forecast.window.end, interval)?;
        Ok(Self {
            actual: range_buckets(&from, &to, interval)?,
            projected: prefixed(&spans),
            spans,
            actual_txns: journal
                .transactions
                .iter()
                .filter(|txn| txn.date.as_str() < cutoff)
                .cloned()
                .collect(),
            actual_prices: journal
                .prices
                .iter()
                .filter(|price| price.date.as_str() < cutoff)
                .cloned()
                .collect(),
            projected_txns: merged_transactions(&journal.transactions, forecast),
            projected_prices: merged_prices(&journal.prices, forecast),
            from,
            to,
            target,
        })
    }
}
