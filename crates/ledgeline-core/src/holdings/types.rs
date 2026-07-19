//! Holdings report contracts — port of `web/src/lib/holdings/types.ts`.
//!
//! Plain, serde-free domain types (like [`crate::reports`]); the wire layer in
//! `ledgeline-server` maps them to camelCase JSON. Money is exact [`Dec`];
//! `gain_pct` is a display-boundary `f64` (so the report types are `PartialEq`
//! but not `Eq`).

use crate::decimal::Dec;
use std::collections::BTreeSet;

/// Include vs. exclude semantics for a [`HoldingsScope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeMode {
    /// Keep accounts matching the set (empty set = everything).
    Include,
    /// Drop accounts matching the set (empty set = everything).
    Exclude,
}

/// The account subtree selection + valuation date a report is computed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingsScope {
    /// Subtree roots (same subtree invariant as the journal filter).
    pub accounts: BTreeSet<String>,
    /// Include or exclude; `Include` + empty set = everything.
    pub mode: ScopeMode,
    /// Snapshot date (`YYYY-MM-DD`), inclusive.
    pub as_of: String,
    /// Optional gain-measurement window start (`YYYY-MM-DD`).
    ///
    /// `None` (the default) = all-time average-cost gain: per-holding
    /// `gain` = `market_value − basis`, exactly as before. `Some(start)`
    /// switches `gain`/`gain_pct` (and the portfolio totals + gainers/losers) to
    /// a windowed gain `market_value(as_of) − value_at_start`, where
    /// `value_at_start` is the position's market value at `start` (the shares
    /// held at `start`, priced as of `start`; `0` when not held then, and
    /// null-propagating when held-but-unpriced at `start`). `basis` is
    /// unaffected — it always stays the all-time average-cost basis.
    pub gain_since: Option<String>,
}

/// Where a holding's price came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceSource {
    /// A `P` price directive dated ≤ `as_of`.
    Directive,
    /// A usable cost annotation (the fallback when no directive prices it).
    Cost,
}

/// A holding's resolved price: a per-unit quantity in the base commodity, its
/// date, and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingPrice {
    /// Per-unit price in the base commodity.
    pub qty: Dec,
    /// Date of the pricing directive / cost annotation.
    pub date: String,
    /// Directive vs. cost-annotation provenance.
    pub source: PriceSource,
}

/// One stock position in the scoped journal.
#[derive(Debug, Clone, PartialEq)]
pub struct Holding {
    /// The commodity symbol.
    pub symbol: String,
    /// The `name:` tag if seen, else the symbol.
    pub name: String,
    /// In-scope accounts currently holding shares (net > 0), sorted.
    pub accounts: Vec<String>,
    /// Net shares held (`> 0` by construction — negative/zero rows are dropped).
    pub shares: Dec,
    /// Average-cost basis in the base commodity; `None` = tainted (some lot
    /// lacked a usable cost).
    pub basis: Option<Dec>,
    /// Date the current position was opened (reset on each full sell-out);
    /// `None` only if never bought in scope.
    pub first_basis_date: Option<String>,
    /// The resolved price, or `None` when unpriced.
    pub price: Option<HoldingPrice>,
    /// `shares × price`; `None` when unpriced.
    pub market_value: Option<Dec>,
    /// `market_value − basis`; `None` when either is missing.
    pub gain: Option<Dec>,
    /// `gain / basis × 100` (display-boundary float); `None` when basis
    /// missing/zero.
    pub gain_pct: Option<f64>,
}

/// The kind of a scope-local holdings warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// A lot was acquired without a usable cost annotation (basis unknown).
    MissingBasis,
    /// Net shares went negative (an opening position was likely never entered).
    NegativeShares,
    /// No market price or usable cost annotation (excluded from totals).
    Unpriced,
}

/// A scope-local warning surfaced alongside the holdings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingsWarning {
    /// The affected symbol.
    pub symbol: String,
    /// What went wrong.
    pub kind: WarningKind,
    /// Human-readable detail (matches the TS message strings).
    pub message: String,
}

/// Portfolio-level totals. `basis`/`gain`/`gain_pct` are PARTIAL: they sum over
/// only the holdings that carry the needed inputs (a known basis / a reference,
/// plus a market value), so a single cost-less or unpriced row no longer blanks
/// the whole portfolio. Each is `None` only when its set is empty — every shown
/// holding excluded (an empty portfolio still reports a real zero).
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingsTotals {
    /// Sum of priced market values (unpriced holdings excluded).
    pub market_value: Dec,
    /// Sum of basis over priced holdings with a known basis; `None` only when
    /// none qualify (all shown holdings tainted/unpriced).
    pub basis: Option<Dec>,
    /// Sum of `market_value − reference` over the qualifying rows, or `None`.
    pub gain: Option<Dec>,
    /// `gain / reference-sum × 100` over the qualifying rows, or `None`.
    pub gain_pct: Option<f64>,
}

/// The full holdings report for a scope at `as_of`.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingsReport {
    /// Snapshot date.
    pub as_of: String,
    /// Base valuation commodity (`PriceDb` base, else `"$"`).
    pub base: String,
    /// Holdings with `shares > 0`, sorted by market value desc (unpriced last,
    /// then by symbol).
    pub holdings: Vec<Holding>,
    /// Portfolio totals.
    pub totals: HoldingsTotals,
    /// `gain_pct > 0` only, sorted desc, ≤ 5.
    pub top_gainers: Vec<Holding>,
    /// `gain_pct < 0` only, sorted asc, ≤ 5.
    pub top_losers: Vec<Holding>,
    /// Scope-local warnings.
    pub warnings: Vec<HoldingsWarning>,
}
