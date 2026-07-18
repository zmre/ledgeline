//! Native report engine — a faithful port of the golden-validated TypeScript
//! reports under `web/src/lib/{domain,reports}`.
//!
//! This module consumes a parsed [`crate::model::Journal`] (its
//! `transactions`/`prices`/`accounts`) and produces balance-sheet, income-
//! statement, cash-flow and net-worth reports whose numbers reproduce hledger's
//! CLI output exactly (verified against `fixtures/golden/` in
//! `tests/reports_golden.rs`).
//!
//! Design mirrors the TS layering:
//! - [`mixed_amount`] — `MixedAmount = BTreeMap<Commodity, Dec>` with the
//!   `maAdd`/`maNeg` semantics from `domain/money.ts` (zero commodities dropped).
//! - [`accounts`] / [`account_types`] — root categorization and hledger's
//!   declared/inferred account-type resolution (incl. the Cash-name heuristic).
//! - [`aggregate`] — `accountTotals`/`rollUp`/`atDepth` over postings.
//! - [`periods`] — pure string/integer bucket date math (never `Date`).
//! - [`prices`] — the `PriceDb` and market-price valuation.
//! - [`balance_sheet`], [`income_statement`], [`cash_flow`], [`net_worth`].
//!
//! Money arithmetic stays exact-decimal (`Dec`); every fallible `Dec` op is
//! surfaced through [`ReportError`] rather than unwrapped.

pub mod account_types;
pub mod accounts;
pub mod aggregate;
pub mod balance_sheet;
pub mod budget;
pub mod cash_flow;
pub mod income_statement;
pub mod mixed_amount;
pub mod net_worth;
pub mod periods;
pub mod prices;
mod sections;
pub mod types;

#[cfg(test)]
mod test_support;

use crate::decimal::DecError;
use thiserror::Error;

pub use account_types::{AccountDecl, AccountType, account_decls, cash_predicate};
pub use accounts::{RootCategory, account_matches, categorize};
pub use aggregate::{PostingFilter, account_totals, at_depth, roll_up};
pub use balance_sheet::balance_sheet;
pub use budget::{BudgetCell, BudgetOpts, BudgetReport, BudgetRow, UNBUDGETED, budget_report};
pub use cash_flow::{cash_flow, is_cash_like};
pub use income_statement::income_statement;
pub use mixed_amount::MixedAmount;
pub use net_worth::net_worth;
pub use periods::{
    Interval, bucket_end, bucket_key, bucket_label, bucket_start, compare_iso, last_n_buckets,
    next_bucket,
};
pub use prices::{PriceDb, ValuationMeta, infer_market_prices, value_at};
pub use types::{PeriodReport, PeriodRow, ReportMeta, ReportRow, Section, SectionedReport};

/// Errors surfaced by the report engine.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ReportError {
    /// Exact-decimal arithmetic failed (overflow); mirrors the parser's use of
    /// [`DecError`]. Unreachable for any realistic journal, but never unwrapped.
    #[error(transparent)]
    Decimal(#[from] DecError),
    /// A bucket key that no interval recognizes (mirrors the TS `RangeError`
    /// from `bucketStart`/`bucketEnd`).
    #[error("unrecognized bucket key: '{0}'")]
    InvalidBucketKey(String),
}
