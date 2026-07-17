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

/// One row of a period report: one `MixedAmount` per bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodRow {
    /// Full account name.
    pub account: String,
    /// Number of `:`-separated segments in `account`.
    pub depth: usize,
    /// One value per bucket, oldest → newest.
    pub values: Vec<MixedAmount>,
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
