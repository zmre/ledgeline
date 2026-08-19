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
pub mod income_statement;
pub mod insights;
pub mod mixed_amount;
pub mod net_worth;
pub mod periods;
pub mod prices;
mod sections;
pub mod subscriptions;
pub mod types;

#[cfg(test)]
mod test_support;

use crate::decimal::DecError;
use thiserror::Error;

pub use account_groups::{
    AccountGroups, BS_GROUP_TAG, CASH_GROUP, GroupSource, INVESTMENTS_GROUP, IS_GROUP_TAG,
    RETAINED_EARNINGS_GROUP, VALUATION_ADJUSTMENT_GROUP, account_groups, account_groups_from,
    declared_groups, declared_groups_from, group_rank,
};
pub use account_types::{
    AccountDecl, AccountType, AccountTypes, account_decls, account_decls_from, cash_predicate,
    declared_types, is_account_type, resolve_account_type,
};
pub use accounts::{RootCategory, account_matches, categorize};
pub use aggregate::{PostingFilter, account_totals, at_depth, roll_up};
pub use balance_sheet::{
    BalanceSheetReport, BsGroup, BsOpts, BsSection, BsSectionKind, Valuation, balance_sheet,
    balance_sheet_grouped,
};
pub use budget::{BudgetCell, BudgetOpts, BudgetReport, BudgetRow, UNBUDGETED, budget_report};
pub use cash_flow::{cash_flow, is_cash_like};
pub use income_statement::{
    Amounts, DateRange, IS_SECTION_TAG, IncomeStatementReport, IsGroup, IsOpts, IsRow, IsSection,
    IsSectionKind, IsSubtotal, IsSubtotalKind, account_sections, account_sections_from,
    income_statement, income_statement_grouped, parse_is_section_tag,
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
    /// An `account` directive declared an `issection:` value outside the closed
    /// vocabulary — see [`income_statement::parse_is_section_tag`].
    ///
    /// This and [`Self::UnknownHoldingsClass`] are the only pieces of journal
    /// content the report engine refuses outright, and it is deliberate.
    /// Everything else it reads from a tag has a
    /// total fallback, because a fallback there is harmless; here it is not.
    /// `issection:` decides section MEMBERSHIP, so a silent `None` would drop
    /// the account back to its type-inferred section and the box the user
    /// spelled would read zero with nothing on screen to say why — the exact
    /// `account-type-not-name` failure `parse_account_type_tag` had to be
    /// corrected for (`account_types.rs:91-113`). A misspelt code is a typo in a
    /// closed seven-word vocabulary; naming it and its alternatives is the only
    /// answer that leads anywhere.
    #[error(
        "account '{account}' declares `issection: {value}`, which is not one of \
         revenue, cogs, opex, depreciation, interest, tax, other"
    )]
    UnknownIsSection {
        /// The declaring account.
        account: String,
        /// The value as written, trimmed.
        value: String,
    },
    /// An `account` directive declared a `holdings:` value outside the closed
    /// vocabulary — see [`crate::holdings::parse_holdings_tag`].
    ///
    /// Refused for [`Self::UnknownIsSection`]'s reason, one step milder in its
    /// consequence and identical in its shape: `holdings:` decides which
    /// Holdings TAB an account appears on, so a silent `None` returns it to the
    /// mechanical default — and the user who tagged a commodity-booked house to
    /// move it off Stocks finds it still sitting on Stocks, with nothing on
    /// screen to say why. Three-word vocabulary, so a miss is a typo.
    #[error(
        "account '{account}' declares `holdings: {value}`, which is not one of \
         stocks, other, none"
    )]
    UnknownHoldingsClass {
        /// The declaring account.
        account: String,
        /// The value as written, trimmed.
        value: String,
    },
    /// An `account` directive declared a `valuation:` value outside the closed
    /// vocabulary — see [`crate::holdings::parse_valuation_tag`].
    ///
    /// Refused for the same reason as its two siblings above, with the sharpest
    /// consequence of the three: `valuation:` decides whether an account is
    /// money-in or a mark-to-market adjustment, so a silent fallback to `cost`
    /// folds a holding's unrealized gain into its own basis and reports the gain
    /// as exactly zero — a real number replaced by a plausible wrong one.
    #[error(
        "account '{account}' declares `valuation: {value}`, which is not one of \
         cost, unrealized"
    )]
    UnknownValuationRole {
        /// The declaring account.
        account: String,
        /// The value as written, trimmed.
        value: String,
    },
}
