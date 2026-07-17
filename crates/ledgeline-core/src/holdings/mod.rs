//! Native average-cost stock-holdings engine — a faithful port of the
//! golden-validated TypeScript under `web/src/lib/holdings`.
//!
//! Given a parsed journal's `transactions` + `prices` and a [`HoldingsScope`],
//! [`compute_holdings`] produces a per-symbol average-cost position report
//! (basis, market value, gains, warnings); [`holdings_series`] maps it over a
//! date series for the holdings-over-time trend.
//!
//! Design mirrors the TS layering and reuses the report engine's substrate:
//! - [`commodities`] — currency-vs-stock classification (`is_currency`).
//! - [`types`] — the serde-free report contracts.
//! - [`engine`] — the average-cost pool math (`compute_holdings`), reusing
//!   `reports::{PriceDb, account_matches}` and the non-normalizing `mul_raw`.
//! - [`series`] — `holdings_series`, reusing `reports::periods` bucket math.
//!
//! Money stays exact-decimal (`Dec`); every fallible op surfaces through
//! [`crate::reports::ReportError`] rather than unwrapping.

pub mod commodities;
pub mod engine;
pub mod series;
pub mod types;

#[cfg(test)]
mod test_helpers;

pub use commodities::is_currency;
pub use engine::compute_holdings;
pub use series::{HoldingsPoint, HoldingsSeries, holdings_series};
pub use types::{
    Holding, HoldingPrice, HoldingsReport, HoldingsScope, HoldingsTotals, HoldingsWarning,
    PriceSource, ScopeMode, WarningKind,
};
