//! Holdings-over-time series — port of `web/src/lib/holdings/series.ts`.
//!
//! A portfolio snapshot at each of the last `count` period boundaries is
//! [`compute_holdings`] mapped over a date series: only `as_of` time-travels; the
//! account scope is unchanged. Reusing `compute_holdings` unchanged keeps the
//! totals math a single source of truth (at the cost of one full recompute per
//! point, which is fine at `count ≈ 12`).

use std::cmp::Ordering;

use crate::decimal::Dec;
use crate::model::{AccountDeclaration, Commodity, PriceDirective, Transaction};
use crate::reports::{
    Interval, ReportError, bucket_end, bucket_label, compare_iso, last_n_buckets,
};

use super::engine::compute_holdings;
use super::types::HoldingsScope;

/// One point in a [`HoldingsSeries`].
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingsPoint {
    /// Snapshot date: the bucket's last day, clamped so the final point never
    /// overshoots `scope.as_of`.
    pub date: String,
    /// Bucket key (e.g. `"2026-07"`), for axis labels.
    pub bucket: String,
    /// Human bucket label (e.g. `"Jul 2026"`).
    pub label: String,
    /// Total priced market value at `date`, in the base commodity.
    pub market_value: Dec,
    /// Total cost basis at `date`; `None` when any held lot is tainted/unpriced
    /// (same refusal as `HoldingsReport.totals.basis`).
    pub basis: Option<Dec>,
}

/// A market-value (and basis) trend over the last `count` period boundaries.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingsSeries {
    /// Base valuation commodity.
    pub base: String,
    /// Oldest → newest, length = `count`.
    pub points: Vec<HoldingsPoint>,
    /// True when at least one point has a non-null basis.
    pub has_basis: bool,
}

/// Portfolio market value (and cost basis) at each of the last `count` period
/// boundaries ending at `scope.as_of`, oldest first. Port of the TS
/// `holdingsSeries`.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow or an unrecognized bucket key
/// (unreachable for the intervals here).
pub fn holdings_series(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
    scope: &HoldingsScope,
    interval: Interval,
    count: usize,
) -> Result<HoldingsSeries, ReportError> {
    let keys = last_n_buckets(&scope.as_of, interval, count)?;
    let mut base = "$".to_string();
    let mut has_basis = false;
    let mut points = Vec::with_capacity(keys.len());
    for key in keys {
        let end = bucket_end(&key)?;
        let date = if compare_iso(&end, &scope.as_of) == Ordering::Greater {
            scope.as_of.clone()
        } else {
            end
        };
        let point_scope = HoldingsScope {
            accounts: scope.accounts.clone(),
            mode: scope.mode,
            as_of: date.clone(),
            // The trend tracks market value/basis only; gain windowing is a
            // per-snapshot concern and never applies to a series point.
            gain_since: None,
        };
        let report = compute_holdings(txns, prices, accounts, commodity_tags, &point_scope)?;
        base = report.base;
        if report.totals.basis.is_some() {
            has_basis = true;
        }
        points.push(HoldingsPoint {
            date,
            bucket: key.clone(),
            label: bucket_label(&key),
            market_value: report.totals.market_value,
            basis: report.totals.basis,
        });
    }
    Ok(HoldingsSeries {
        base,
        points,
        has_basis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdings::test_helpers::{buy, posting, scope, txn, usd};
    use crate::holdings::types::ScopeMode;

    // VTI: 10 @ $200 on 2025-02-10, +10 @ $220 on 2025-04-10; priced $250 from
    // 2025-01.
    fn txns() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2025-02-10",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 20000, true),
                    posting("assets:broker:cash", vec![usd(-200_000)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2025-04-10",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 22000, true),
                    posting("assets:broker:cash", vec![usd(-220_000)], &[]),
                ],
                &[],
            ),
        ]
    }

    fn prices() -> Vec<PriceDirective> {
        vec![crate::holdings::test_helpers::pd(
            "2025-01-01",
            "VTI",
            25000,
            "$",
        )]
    }

    fn values(series: &HoldingsSeries) -> Vec<f64> {
        series
            .points
            .iter()
            .map(|p| p.market_value.floating_point())
            .collect()
    }

    #[test]
    fn snapshots_market_value_at_each_month_end_ending_at_asof() {
        let series = holdings_series(
            &txns(),
            &prices(),
            &[],
            &[],
            &scope("2025-05-15", ScopeMode::Include, &[]),
            Interval::Monthly,
            5,
        )
        .unwrap();
        assert_eq!(series.base, "$");
        let buckets: Vec<&str> = series.points.iter().map(|p| p.bucket.as_str()).collect();
        assert_eq!(
            buckets,
            ["2025-01", "2025-02", "2025-03", "2025-04", "2025-05"]
        );
        // Final point clamps to asOf, not the month's last day.
        assert_eq!(series.points.last().unwrap().date, "2025-05-15");
        assert_eq!(series.points[0].date, "2025-01-31");

        assert_eq!(values(&series), vec![0.0, 2500.0, 2500.0, 5000.0, 5000.0]);
    }

    #[test]
    fn tracks_cost_basis_and_flags_availability() {
        let series = holdings_series(
            &txns(),
            &prices(),
            &[],
            &[],
            &scope("2025-05-15", ScopeMode::Include, &[]),
            Interval::Monthly,
            5,
        )
        .unwrap();
        assert!(series.has_basis);
        let basis: Vec<Option<f64>> = series
            .points
            .iter()
            .map(|p| p.basis.map(|b| b.floating_point()))
            .collect();
        assert_eq!(
            basis,
            vec![
                Some(0.0),
                Some(2000.0),
                Some(2000.0),
                Some(4200.0),
                Some(4200.0)
            ]
        );
    }

    #[test]
    fn respects_exclude_scoping() {
        let series = holdings_series(
            &txns(),
            &prices(),
            &[],
            &[],
            &scope("2025-05-15", ScopeMode::Exclude, &["assets:broker:vti"]),
            Interval::Monthly,
            3,
        )
        .unwrap();
        assert!(series.points.iter().all(|p| p.market_value.is_zero()));
        // No holdings ⇒ the empty-portfolio basis total is a (non-null) zero.
        assert!(
            series
                .points
                .iter()
                .all(|p| p.basis.is_some_and(|b| b.is_zero()))
        );
    }

    #[test]
    fn time_travels_earlier_asof_never_sees_later_buys() {
        let series = holdings_series(
            &txns(),
            &prices(),
            &[],
            &[],
            &scope("2025-03-31", ScopeMode::Include, &[]),
            Interval::Monthly,
            2,
        )
        .unwrap();
        let buckets: Vec<&str> = series.points.iter().map(|p| p.bucket.as_str()).collect();
        assert_eq!(buckets, ["2025-02", "2025-03"]);
        assert_eq!(values(&series), vec![2500.0, 2500.0]); // second buy (Apr) is in the future
    }
}
