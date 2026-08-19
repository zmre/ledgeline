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
//! - [`classify`] — the `holdings:` account tag that splits the page's two tabs.
//! - [`other`] — the ACCOUNT-keyed report behind the Other tab: houses, cars,
//!   partnership interests. A separate engine, not a filter over [`engine`];
//!   see its header for why a currency-denominated asset is invisible here.
//!
//! Money stays exact-decimal (`Dec`); every fallible op surfaces through
//! [`crate::reports::ReportError`] rather than unwrapping.

pub mod classify;
pub mod commodities;
pub mod engine;
pub mod other;
pub mod series;
pub mod types;

#[cfg(test)]
mod test_helpers;

pub use classify::{
    HOLDINGS_TAG, HoldingsClass, declared_holdings_classes, parse_holdings_tag,
    resolve_holdings_class,
};
pub use commodities::is_currency;
pub use engine::{compute_holdings, prices_any_held, valuation_base};
pub use other::{
    OtherHolding, OtherHoldingsReport, OtherHoldingsTotals, OtherHoldingsWarning, OtherWarningKind,
    other_holdings, other_holdings_series,
};
pub use series::{HoldingsPoint, HoldingsSeries, holdings_series};
pub use types::{
    Holding, HoldingPrice, HoldingsReport, HoldingsScope, HoldingsTotals, HoldingsWarning,
    PriceSource, ScopeMode, WarningKind,
};
