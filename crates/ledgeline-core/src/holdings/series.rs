//! Holdings-over-time series — port of `web/src/lib/holdings/series.ts`.
//!
//! A portfolio snapshot at each of the last `count` period boundaries: only
//! `as_of` time-travels; the account scope is unchanged. The totals math stays a
//! single source of truth because every point IS a `compute_holdings` report —
//! but the whole series is produced by ONE date-ordered replay of the
//! average-cost pools (`engine::holdings_at_each`) rather than one full
//! recompute per point, which used to make the endpoint cost `count ×
//! compute_holdings` and made it the slowest one measured.

use crate::decimal::Dec;
use crate::model::{AccountDeclaration, Commodity, PriceDirective, Transaction};
use crate::reports::{ReportError, bucket_label, last_n_buckets};

use super::engine::{HoldingsFlows, holdings_at_each_with_flows};
use super::types::HoldingsScope;
use super::window::{SeriesWindow, series_dates};

/// One point in a [`HoldingsSeries`].
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingsPoint {
    /// Snapshot date: the bucket's last day, clamped so the final point never
    /// overshoots `scope.as_of` — or, for the first point of a window asked
    /// for from a `start` date, that date (see [`super::window::series_dates`]).
    pub date: String,
    /// Bucket key (e.g. `"2026-07"`), for axis labels.
    pub bucket: String,
    /// Human bucket label (e.g. `"Jul 2026"`).
    pub label: String,
    /// Total priced market value at `date`, in the base commodity.
    pub market_value: Dec,
    /// Total cost basis at `date` — the PARTIAL sum over priced holdings with a
    /// known basis (same rule as `HoldingsReport.totals.basis`); `None` only when
    /// none qualify.
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

/// Portfolio market value (and cost basis) at each of `window`'s points: the
/// last `window.count` period boundaries ending at `scope.as_of`, oldest
/// first, the first taken at `window.start` when there is one (see
/// [`super::window::series_dates`]). Port of the TS `holdingsSeries`.
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
    window: &SeriesWindow,
) -> Result<HoldingsSeries, ReportError> {
    series_and_flows(txns, prices, accounts, commodity_tags, scope, window, false)
        .map(|(series, _)| series)
}

/// [`holdings_series`] together with the [`holdings_flows`] over
/// `(first point, scope.as_of]` — the pair a benchmark comparison simulates
/// from — built from one set of inputs and recorded by one replay.
///
/// # Errors
/// As [`holdings_series`].
pub fn holdings_series_and_flows(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
    scope: &HoldingsScope,
    window: &SeriesWindow,
) -> Result<(HoldingsSeries, HoldingsFlows), ReportError> {
    series_and_flows(txns, prices, accounts, commodity_tags, scope, window, true)
        .map(|(series, flows)| (series, flows.unwrap_or_default()))
}

fn series_and_flows(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    commodity_tags: &[(Commodity, Vec<(String, String)>)],
    scope: &HoldingsScope,
    window: &SeriesWindow,
    with_flows: bool,
) -> Result<(HoldingsSeries, Option<HoldingsFlows>), ReportError> {
    let keys = last_n_buckets(&scope.as_of, window.interval, window.count)?;
    // Each bucket's last day (the first one at `window.start`, when given),
    // clamped so the final point never overshoots `scope.as_of`. Ascending,
    // which is what lets one replay serve them all.
    let dates = series_dates(&keys, &scope.as_of, window.start.as_deref())?;
    // The valuation commodity is resolved ONCE, from the scope's own `as_of`,
    // and every point is pinned to it. Each point is a different snapshot
    // holding a different set of symbols, so left to choose for itself an early
    // (or empty) bucket could legitimately land on a different commodity than
    // the last one — and a trend line whose units change partway along is worse
    // than no trend.
    let flows_after = with_flows.then(|| dates.first().cloned()).flatten();
    let (_, reports, flows) = holdings_at_each_with_flows(
        txns,
        prices,
        accounts,
        commodity_tags,
        scope,
        &dates,
        flows_after.as_deref(),
    )?;

    let mut base = "$".to_string();
    let mut has_basis = false;
    let mut points = Vec::with_capacity(keys.len());
    for ((key, date), report) in keys.iter().zip(dates).zip(reports) {
        base = report.base;
        if report.totals.basis.is_some() {
            has_basis = true;
        }
        points.push(HoldingsPoint {
            date,
            bucket: key.clone(),
            label: bucket_label(key),
            market_value: report.totals.market_value,
            basis: report.totals.basis,
        });
    }
    let series = HoldingsSeries {
        base,
        points,
        has_basis,
    };
    Ok((series, flows))
}

#[cfg(test)]
mod replay_identity;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdings::test_helpers::{amt, buy, pd, posting, scope, txn, usd, with_cost};
    use crate::holdings::types::ScopeMode;
    use crate::reports::Interval;

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
        vec![pd("2025-01-01", "VTI", 25000, "$")]
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
            &SeriesWindow::counted(Interval::Monthly, 5),
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
            &SeriesWindow::counted(Interval::Monthly, 5),
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
            &SeriesWindow::counted(Interval::Monthly, 3),
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

    /// HOLD-3. The scope holds a `$`-priced symbol early and a EUR-priced one at
    /// `as_of`, so left to choose for itself each point would pick whichever
    /// commodity prices ITS OWN holdings — plotting dollars and euros on one
    /// line. The base is resolved once, from `scope.as_of`, and pinned.
    #[test]
    fn every_point_is_valued_in_one_pinned_commodity() {
        let txns = vec![
            txn(
                1,
                "2025-01-05",
                vec![
                    buy("assets:broker:us", "USDSYM", 10, 10000, true),
                    posting("assets:broker:cash", vec![usd(-100_000)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2025-03-05",
                vec![
                    posting("assets:broker:us", vec![amt("USDSYM", -10, 0)], &[]),
                    posting("assets:broker:cash", vec![usd(100_000)], &[]),
                ],
                &[],
            ),
            txn(
                3,
                "2025-04-05",
                vec![
                    posting(
                        "assets:broker:eu",
                        vec![with_cost(amt("EURSYM", 10, 0), 5000, true, "EUR")],
                        &[],
                    ),
                    posting("assets:broker:cash", vec![amt("EUR", -50000, 2)], &[]),
                ],
                &[],
            ),
        ];
        let prices = vec![
            pd("2025-01-01", "USDSYM", 10000, "$"),
            pd("2025-04-01", "EURSYM", 5000, "EUR"),
        ];
        let series = holdings_series(
            &txns,
            &prices,
            &[],
            &[],
            &scope("2025-05-31", ScopeMode::Include, &[]),
            &SeriesWindow::counted(Interval::Monthly, 5),
        )
        .unwrap();
        // At `as_of` only EURSYM is held, and nothing connects it to `$`.
        assert_eq!(series.base, "EUR");
        // January–March hold only the `$`-priced symbol, which EUR cannot value:
        // an honest zero, NOT a $1,000 point smuggled onto a euro axis.
        assert_eq!(values(&series), vec![0.0, 0.0, 0.0, 500.0, 500.0]);
    }

    #[test]
    fn time_travels_earlier_asof_never_sees_later_buys() {
        let series = holdings_series(
            &txns(),
            &prices(),
            &[],
            &[],
            &scope("2025-03-31", ScopeMode::Include, &[]),
            &SeriesWindow::counted(Interval::Monthly, 2),
        )
        .unwrap();
        let buckets: Vec<&str> = series.points.iter().map(|p| p.bucket.as_str()).collect();
        assert_eq!(buckets, ["2025-02", "2025-03"]);
        assert_eq!(values(&series), vec![2500.0, 2500.0]); // second buy (Apr) is in the future
    }

    #[test]
    fn one_replay_answers_exactly_what_the_series_and_flows_answer_apart() {
        let mut journal = txns();
        journal.push(txn(
            3,
            "2025-05-01",
            vec![
                crate::holdings::test_helpers::sell("assets:broker:vti", "VTI", 5),
                posting("assets:broker:cash", vec![usd(125_000)], &[]),
            ],
            &[],
        ));
        let sc = scope("2025-05-15", ScopeMode::Include, &[]);
        for window in [
            SeriesWindow::counted(Interval::Monthly, 5),
            SeriesWindow::counted(Interval::Monthly, 2),
            SeriesWindow::counted(Interval::Daily, 1),
        ] {
            let (series, flows) =
                holdings_series_and_flows(&journal, &prices(), &[], &[], &sc, &window).unwrap();
            let apart = holdings_series(&journal, &prices(), &[], &[], &sc, &window).unwrap();
            let after = apart.points.first().map(|point| point.date.as_str());
            let apart_flows =
                crate::holdings::holdings_flows(&journal, &prices(), &[], &[], &sc, after).unwrap();
            assert_eq!(series, apart);
            assert_eq!(flows, apart_flows);
        }
    }
}
