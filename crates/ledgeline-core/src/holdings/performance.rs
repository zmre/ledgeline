//! Performance — the Stocks chart's flow-adjusted view (plan 47 A1b/A1c/A1e).
//!
//! A value chart moves when you buy and sell. A performance chart must not:
//! selling 999 of 1,000 shares while the price rises is a falling VALUE and a
//! positive RETURN. This module turns a value series plus its per-bucket net
//! flows into a cumulative time-weighted return (TWR), chain-linked from one
//! Modified Dietz return per bucket.
//!
//! # Flow sign
//!
//! A flow is money INTO the scoped holdings: a buy is positive (at the cost
//! paid), a sell negative (at its proceeds), a transfer into scope positive and
//! out of scope negative — the convention [`super::engine::holdings_flows`]
//! already records for benchmarks.
//!
//! # The per-bucket return
//!
//! With `V0`/`V1` the value at the bucket's ends and `F` its net flow:
//!
//! - both ends held: `r = (V1 − V0 − F) / (V0 + F/2)` — Modified Dietz, the
//!   flow assumed mid-bucket;
//! - opening from nothing (`V0 ≤ 0`): `r = (V1 − F) / F`. There was no capital
//!   before the flow, so there is nothing to time-weight against: the flow IS
//!   the capital, and TWR ignores time out of the market. The mid-bucket
//!   `F/2` would double the first bucket's return (buy $1,000, end the month
//!   at $1,100: +20 % instead of +10 %);
//! - closing to nothing (`V1 ≤ 0`): `r = (V1 − V0 − F) / V0`, the mirror case
//!   — the capital was `V0` until it left;
//! - a non-positive denominator (no capital at all, or flows that make the
//!   Dietz weighting meaningless) contributes `r = 0`.
//!
//! The arithmetic is exact `Dec` up to the one ratio per bucket, which crosses
//! the display boundary as `f64`; every result is finite.

use crate::decimal::Dec;
use crate::reports::ReportError;

use super::engine::DatedFlow;
use super::series::HoldingsPoint;

/// Net flow per point: entry `i` sums the `flows` dated in
/// `(dates[i − 1], dates[i]]`; entry 0 is always zero. `dates` and `flows`
/// ascending. A flow outside `(dates[0], last]` is ignored.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn bucket_flows(dates: &[&str], flows: &[DatedFlow]) -> Result<Vec<Dec>, ReportError> {
    let mut out = Vec::with_capacity(dates.len());
    let mut pending = flows.iter().peekable();
    for (index, date) in dates.iter().enumerate() {
        let mut sum = Dec::zero();
        while let Some(flow) = pending.next_if(|flow| flow.date.as_str() <= *date) {
            // Flows on or before the first point are already inside its value.
            if index > 0 {
                sum = sum.add(flow.amount)?;
            }
        }
        out.push(sum);
    }
    Ok(out)
}

/// The bucket's return (see the module docs), `0` when it is undefined.
fn bucket_return(start: Dec, end: Dec, flow: Dec) -> Result<f64, ReportError> {
    let gain = end.sub(start)?.sub(flow)?;
    // Both sides doubled so `F/2` stays exact: r = 2·gain / (2·V0 + F).
    let (numerator, denominator) = if start.mantissa <= 0 {
        (gain, flow)
    } else if end.mantissa <= 0 {
        (gain, start)
    } else {
        (gain.add(gain)?, start.add(start)?.add(flow)?)
    };
    if denominator.mantissa <= 0 {
        return Ok(0.0);
    }
    let ratio = numerator.floating_point() / denominator.floating_point();
    Ok(if ratio.is_finite() { ratio } else { 0.0 })
}

/// The cumulative growth index at each point (`1.0` = where the chain
/// started), `None` until something is held.
///
/// The chain starts at the first point with a value `> 0`: at `1.0` when that
/// is the first point (or the holding arrived with no flow, like an opening
/// balance — nothing to measure a return against), else at `1 + r` of the
/// bucket that bought it. Once started it never goes back to `None`: a
/// position sold to nothing holds its last index (`r = 0` while empty), and a
/// later re-purchase continues the same chain.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn growth_index(values: &[Dec], flows: &[Dec]) -> Result<Vec<Option<f64>>, ReportError> {
    let mut out: Vec<Option<f64>> = Vec::with_capacity(values.len());
    let mut index: Option<f64> = None;
    for (i, (&value, &flow)) in values.iter().zip(flows).enumerate() {
        index = match (index, i.checked_sub(1)) {
            (None, None) => (value.mantissa > 0).then_some(1.0),
            (None, Some(previous)) if value.mantissa > 0 => {
                Some(1.0 + bucket_return(values[previous], value, flow)?)
            }
            (None, Some(_)) => None,
            (Some(current), Some(previous)) => {
                Some(current * (1.0 + bucket_return(values[previous], value, flow)?))
            }
            // Unreachable: the index is only ever set after the first point.
            (Some(current), None) => Some(current),
        };
        out.push(index.filter(|index| index.is_finite()));
    }
    Ok(out)
}

/// Fill each point's `flow` from `flows` and its `return_pct` from the chain
/// ([`growth_index`] − 1).
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn attach_returns(
    points: &mut [HoldingsPoint],
    flows: &[DatedFlow],
) -> Result<(), ReportError> {
    let dates: Vec<&str> = points.iter().map(|point| point.date.as_str()).collect();
    let per_point = bucket_flows(&dates, flows)?;
    attach_point_flows(points, &per_point)
}

/// [`attach_returns`] for flows already bucketed per point.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow.
pub fn attach_point_flows(points: &mut [HoldingsPoint], flows: &[Dec]) -> Result<(), ReportError> {
    let values: Vec<Dec> = points.iter().map(|point| point.market_value).collect();
    let index = growth_index(&values, flows)?;
    for ((point, &flow), index) in points.iter_mut().zip(flows).zip(index) {
        point.flow = flow;
        point.return_pct = index.map(|index| index - 1.0);
    }
    Ok(())
}

/// A closed range of returns, as fractions (`0.1` = +10 %).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReturnRange {
    pub min: f64,
    pub max: f64,
}

/// The Performance chart's y-axis anchor (A1c): the return index over the
/// anchor horizon, REBASED to the shown window's start, unioned with the shown
/// points' own returns. `None` when neither has a return.
///
/// The rebase point is where the shown chain starts — its first point with a
/// return — and the horizon's index there is read from the latest horizon
/// point on or before that date that has one (its first such point when none
/// does, e.g. a holding bought between two coarse horizon points). Each horizon
/// point then reads `index / base − 1`: "how far from the window's start had
/// this already been".
#[must_use]
pub fn return_anchor_range(
    horizon: &[HoldingsPoint],
    shown: &[HoldingsPoint],
) -> Option<ReturnRange> {
    let shown_returns = shown.iter().filter_map(|point| point.return_pct);
    let rebased: Vec<f64> = shown
        .iter()
        .find(|point| point.return_pct.is_some())
        .and_then(|start| {
            let indexed = || {
                horizon
                    .iter()
                    .filter_map(|point| point.return_pct.map(|r| (point.date.as_str(), 1.0 + r)))
            };
            indexed()
                .take_while(|(date, _)| *date <= start.date.as_str())
                .last()
                .or_else(|| indexed().next())
                .map(|(_, base)| base)
                .filter(|base| base.is_finite() && *base > 0.0)
                .map(|base| indexed().map(|(_, index)| index / base - 1.0).collect())
        })
        .unwrap_or_default();
    shown_returns
        .chain(rebased)
        .filter(|value| value.is_finite())
        .fold(None, |range: Option<ReturnRange>, value| {
            Some(match range {
                None => ReturnRange {
                    min: value,
                    max: value,
                },
                Some(range) => ReturnRange {
                    min: range.min.min(value),
                    max: range.max.max(value),
                },
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(whole: i64) -> Dec {
        Dec::new(i128::from(whole), 0)
    }

    fn flow(date: &str, amount: i64) -> DatedFlow {
        DatedFlow {
            date: date.to_string(),
            amount: d(amount),
        }
    }

    fn index(values: &[i64], flows: &[i64]) -> Vec<Option<f64>> {
        let values: Vec<Dec> = values.iter().copied().map(d).collect();
        let flows: Vec<Dec> = flows.iter().copied().map(d).collect();
        growth_index(&values, &flows).unwrap()
    }

    fn close(actual: Option<f64>, expected: f64) -> bool {
        actual.is_some_and(|actual| (actual - expected).abs() < 1e-9)
    }

    fn point(date: &str, value: i64, return_pct: Option<f64>) -> HoldingsPoint {
        HoldingsPoint {
            date: date.to_string(),
            bucket: date.to_string(),
            label: date.to_string(),
            market_value: d(value),
            basis: None,
            flow: Dec::zero(),
            return_pct,
        }
    }

    #[test]
    fn flows_land_in_the_bucket_ending_at_or_after_them() {
        let flows = [
            flow("2026-01-31", 7), // on the first point: inside its value
            flow("2026-02-01", 10),
            flow("2026-02-28", 5),
            flow("2026-03-15", -20),
            flow("2026-04-02", 99), // after the last point
        ];
        let got = bucket_flows(&["2026-01-31", "2026-02-28", "2026-03-31"], &flows).unwrap();
        assert_eq!(got, vec![d(0), d(15), d(-20)]);
    }

    #[test]
    fn no_flows_is_plain_price_return_chain_linked() {
        let got = index(&[100, 110, 99], &[0, 0, 0]);
        assert!(close(got[0], 1.0));
        assert!(close(got[1], 1.1));
        assert!(close(got[2], 0.99));
    }

    /// The user's scenario: 1,000 shares at $100, 999 sold mid-bucket at $105
    /// while the price ends at $110. Value collapses; the return is positive.
    #[test]
    fn selling_almost_everything_while_the_price_rises_is_a_positive_return() {
        let got = index(&[100_000, 110], &[0, -104_895]);
        let r = got[1].unwrap() - 1.0;
        // (110 − 100000 + 104895) / (100000 − 52447.5) = 5005 / 47552.5
        assert!((r - 5005.0 / 47_552.5).abs() < 1e-12, "{r}");
        assert!(r > 0.0);
    }

    #[test]
    fn a_mid_window_first_purchase_starts_the_chain_from_its_own_cost() {
        // Nothing held, then $1,000 bought and worth $1,100 at bucket end,
        // then flat.
        let got = index(&[0, 1_100, 1_100], &[0, 1_000, 0]);
        assert_eq!(got[0], None);
        assert!(close(got[1], 1.1), "{:?}", got[1]);
        assert!(close(got[2], 1.1));
    }

    #[test]
    fn an_opening_balance_without_a_flow_starts_at_zero_return() {
        let got = index(&[0, 500, 550], &[0, 0, 0]);
        assert_eq!(got[0], None);
        assert!(close(got[1], 1.0));
        assert!(close(got[2], 1.1));
    }

    #[test]
    fn a_reinvested_dividend_is_a_small_inflow_not_a_gain() {
        // $10,000 → $10,050 where $50 was a reinvested dividend: 0 % return.
        let got = index(&[10_000, 10_050], &[0, 50]);
        assert!(close(got[1], 1.0));
    }

    #[test]
    fn selling_out_holds_the_chain_and_a_rebuy_continues_it() {
        // 100 → sold for 120 (+20 %) → empty → bought 50, worth 55 (+10 %).
        let got = index(&[100, 0, 0, 55], &[0, -120, 0, 50]);
        assert!(close(got[1], 1.2));
        assert!(close(got[2], 1.2));
        assert!(close(got[3], 1.32));
    }

    #[test]
    fn degenerate_denominators_contribute_nothing_and_never_nan() {
        // V0 > 0, flow so negative that V0 + F/2 ≤ 0 while V1 stays > 0.
        let got = index(&[100, 10], &[0, -300]);
        assert!(close(got[1], 1.0));
        assert!(index(&[0, 0, 0], &[0, 0, 0]).iter().all(Option::is_none));
        assert!(index(&[0, 0], &[0, -5]).iter().all(Option::is_none));
    }

    #[test]
    fn attach_fills_flow_and_return_on_every_point() {
        let mut points = vec![
            point("2026-01-31", 100, None),
            point("2026-02-28", 210, None),
        ];
        attach_returns(&mut points, &[flow("2026-02-10", 100)]).unwrap();
        assert_eq!(points[0].flow, d(0));
        assert_eq!(points[1].flow, d(100));
        assert!(close(points[0].return_pct, 0.0));
        // (210 − 100 − 100) / (100 + 50)
        assert!(close(points[1].return_pct, 10.0 / 150.0));
    }

    #[test]
    fn the_return_anchor_is_rebased_to_the_window_start() {
        // Horizon index: 1.0, 1.2, 1.5, 1.35 (−10 % in the last bucket).
        let horizon = [
            point("2025-12-31", 1, Some(0.0)),
            point("2026-01-31", 1, Some(0.2)),
            point("2026-02-28", 1, Some(0.5)),
            point("2026-03-31", 1, Some(0.35)),
        ];
        // The window opens on 2026-02-28 and has fallen 10 % since.
        let shown = [
            point("2026-02-28", 1, Some(0.0)),
            point("2026-03-31", 1, Some(-0.1)),
        ];
        let range = return_anchor_range(&horizon, &shown).unwrap();
        // Rebased on 1.5: 1/1.5 − 1 = −1/3 is the floor, 0 the ceiling.
        assert!((range.min - (1.0 / 1.5 - 1.0)).abs() < 1e-12);
        assert!(range.max.abs() < 1e-12);
    }

    #[test]
    fn the_return_anchor_rebases_between_coarse_horizon_points() {
        let horizon = [
            point("2026-01-31", 1, Some(0.0)),
            point("2026-02-28", 1, Some(1.0)),
        ];
        // Window starts mid-February: rebased on January's 1.0.
        let shown = [point("2026-02-14", 1, Some(0.0))];
        let range = return_anchor_range(&horizon, &shown).unwrap();
        assert_eq!(range, ReturnRange { min: 0.0, max: 1.0 });
    }

    #[test]
    fn no_returns_anywhere_is_no_anchor() {
        let horizon = [point("2026-01-31", 0, None)];
        let shown = [point("2026-02-28", 0, None)];
        assert_eq!(return_anchor_range(&horizon, &shown), None);
        assert_eq!(return_anchor_range(&[], &[]), None);
    }
}
