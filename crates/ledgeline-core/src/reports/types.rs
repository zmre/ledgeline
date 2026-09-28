//! Report result shapes — Rust equivalents of `web/src/lib/reports/types.ts`.
//!
//! Serde-free for now (JSON serialization is a later endpoint task). Sign
//! conventions match hledger's `bs`/`is` presentation: liabilities (bs) and
//! revenues (is) rows/totals are shown sign-flipped positive; grand totals are
//! nets (`assets − liabilities(displayed)`, `revenues(displayed) − expenses`).
//! `PeriodReport` values keep natural signs.

use super::mixed_amount::MixedAmount;
use crate::model::Commodity;

/// One row of a sectioned report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    /// Full, colon-delimited account name (already clamped to the report depth).
    pub account: String,
    /// Number of `:`-separated segments in `account`.
    pub depth: usize,
    /// Direct total of postings to exactly this (clamped) account name.
    pub own: MixedAmount,
    /// Rolled-up total including all sub-accounts.
    pub inclusive: MixedAmount,
}

/// A titled group of rows plus its subtree total (`Assets`, `Liabilities`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Section title.
    pub title: String,
    /// Member rows, sorted by account name.
    pub rows: Vec<ReportRow>,
    /// Total across the section's depth-1 roots (sign-flipped for
    /// liabilities/revenues, matching the rows).
    pub total: MixedAmount,
}

/// Balance sheet / income statement. `as_of` for point-in-time, `from`/`to` for
/// ranges (all inclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionedReport {
    /// Point-in-time date (balance sheet).
    pub as_of: Option<String>,
    /// Inclusive range start (income statement).
    pub from: Option<String>,
    /// Inclusive range end (income statement).
    pub to: Option<String>,
    /// The report's sections, in presentation order.
    pub sections: Vec<Section>,
    /// Net grand total across sections.
    pub grand_total: MixedAmount,
}

/// Extra result info (contract extension, see `plans/06-reports-engine.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReportMeta {
    /// Commodities skipped during valuation because no direct price to the
    /// target existed (sorted, deduped).
    pub unpriced: Vec<Commodity>,
}

/// Which side of the balance sheet a net-worth row's balance comes from.
///
/// Decided by the EFFECTIVE declared type of the accounts rolled into the row —
/// the same membership test that decides whether an account is in net worth at
/// all — never by the row's sign or name. An overdrawn checking account is
/// still an `Asset` row (it just holds a negative balance), and a credit card
/// carrying a refund is still a `Liability` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKind {
    /// Every account rolled into the row is an asset (cash included).
    Asset,
    /// Every account rolled into the row is a liability.
    Liability,
    /// The row's subtree holds both — possible only when declared types nest a
    /// liability under an asset parent (or the reverse) and the depth clamp
    /// folds them into one row.
    Mixed,
}

impl RowKind {
    /// The wire spelling (`"asset"` / `"liability"` / `"mixed"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Liability => "liability",
            Self::Mixed => "mixed",
        }
    }
}

/// One row of a period report: one `MixedAmount` per bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodRow {
    /// Full account name.
    pub account: String,
    /// Number of `:`-separated segments in `account`.
    pub depth: usize,
    /// One value per bucket, oldest → newest.
    pub values: Vec<MixedAmount>,
    /// Balance-sheet side, for net worth only; `None` for every other period
    /// report (and then absent from the wire, so their bytes do not move).
    pub kind: Option<RowKind>,
}

/// Cash flow / net worth: one column per bucket, oldest → newest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodReport {
    /// Bucket keys, oldest → newest.
    pub buckets: Vec<String>,
    /// Rows (union of accounts across buckets, sorted).
    pub rows: Vec<PeriodRow>,
    /// One net total per bucket.
    pub totals: Vec<MixedAmount>,
    /// Present only when noteworthy (e.g. unpriced commodities in net worth).
    pub meta: Option<ReportMeta>,
}
