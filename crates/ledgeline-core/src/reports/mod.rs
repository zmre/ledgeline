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

pub mod account_groups;
pub mod account_types;
pub mod accounts;
pub mod aggregate;
pub mod balance_sheet;
pub mod budget;
pub mod cash_flow;
pub mod flows;
pub mod income_statement;
pub mod insights;
pub mod mixed_amount;
pub mod net_worth;
pub mod periods;
pub mod prices;
pub mod reference;
mod sections;
pub mod subscriptions;
pub mod types;

#[cfg(test)]
mod test_support;

use crate::decimal::DecError;
use thiserror::Error;

pub use account_groups::{
    AccountGroups, BS_GROUP_TAG, BS_TERM_TAG, BsTerm, CASH_GROUP, GroupSource, INVESTMENTS_GROUP,
    IS_GROUP_TAG, RETAINED_EARNINGS_GROUP, VALUATION_ADJUSTMENT_GROUP, account_groups,
    account_groups_from, bs_terms, declared_bs_terms, declared_groups, declared_groups_from,
    group_rank, parse_bs_term_tag, resolve_bs_term,
};
pub use account_types::{
    ACCOUNT_TYPE_TAG, AccountDecl, AccountType, AccountTypes, account_decls, account_decls_from,
    cash_predicate, declared_types, is_account_type, parse_account_type_tag, resolve_account_type,
};
pub use accounts::{RootCategory, account_matches, categorize};
pub use aggregate::{PostingFilter, account_totals, at_depth, roll_up};
pub use balance_sheet::{
    BalanceSheetReport, BsGroup, BsOpts, BsSection, BsSectionKind, BsSubsection, Valuation,
    balance_sheet, balance_sheet_grouped, prices_any_on_sheet,
};
pub use budget::{BudgetCell, BudgetOpts, BudgetReport, BudgetRow, UNBUDGETED, budget_report};
pub use cash_flow::{cash_flow, is_cash_like};
pub use flows::{
    FlowGraph, FlowLink, FlowNode, FlowOpts, FlowReport, FlowSide, income_statement_flows,
};
pub use income_statement::{
    Amounts, DateRange, IS_SECTION_TAG, IncomeStatementReport, IsGroup, IsOpts, IsRow, IsSection,
    IsSectionKind, IsSubtotal, IsSubtotalKind, account_sections, account_sections_from,
    income_statement, income_statement_grouped, parse_is_section_tag, prices_any_on_statement,
};
pub use insights::{
    ChangeKind, ChangeRow, CostOfLiving, InsightsOpts, InsightsPeriod, InsightsReport,
    InvestmentPerf, MetricDelta, MoverRow, PerfPoint, TopTxn, insights,
};
pub use mixed_amount::MixedAmount;
pub use net_worth::{NetWorthOpts, net_worth};
pub use periods::{
    Interval, add_days, add_months, bucket_as_of, bucket_end, bucket_key, bucket_label,
    bucket_span, bucket_start, compare_iso, days_between, iso_from_days, last_n_buckets,
    next_bucket,
};
pub use prices::{PriceDb, ValuationMeta, infer_market_prices, value_at};
pub use reference::{AccountHistory, ReferenceOpts, ReferencePeriod, account_reference};
pub use subscriptions::{
    Cadence, DEFAULT_EXCLUDE_DESC, Subscription, SubscriptionOpts, SubscriptionsReport,
    detect_subscriptions,
};
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
