//! "What if the same money had gone into X instead?" — the benchmark line drawn
//! over the Stocks tab's value-over-time chart.
//!
//! The simulation replays the portfolio's own dated contributions
//! ([`super::engine::holdings_flows`]) into one benchmark security, buying and
//! selling it at its dividend-adjusted close, so the line answers the question a
//! reader actually has: "given what I put in and took out, and when, would I
//! have more or less by now in an index fund?". It is deliberately NOT a
//! percentage-return comparison — a return series ignores timing, and timing is
//! most of the difference between two portfolios fed the same paychecks.
//!
//! # Where the two lines start
//!
//! Always together, at the chart's left edge. The simulated account is SEEDED
//! with the portfolio's actual market value at the first point, converted into
//! benchmark units at that date's close; only flows after that point are then
//! applied. For a window that opens after the portfolio did (twelve months, five
//! years) that is the only honest start — the older contributions are already
//! inside the seed, at what they had become — and for "all time" it is the same
//! rule applied at the first bucket.
//!
//! A benchmark with no close on or before the first point (a fund younger than
//! the window) starts at the first point it CAN price, seeded with the
//! portfolio's value there. Points before it are `None`: a gap, never a zero.
//!
//! # Numbers
//!
//! The arithmetic is `f64`. Every input has already crossed the display
//! boundary: adjusted closes arrive from Yahoo as binary floats, and the output
//! is a chart line, not a figure anyone reconciles. Exact decimals would only
//! lend a hypothetical line precision it does not have.

use super::engine::DatedFlow;
use super::series::HoldingsPoint;

/// One benchmark close: `(YYYY-MM-DD, dividend-adjusted close)`.
pub type Close = (String, f64);

/// A simulated benchmark line, index-aligned to the series points it was built
/// against.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkLine {
    /// The simulated account's value at each point; `None` before the first
    /// point the benchmark can price.
    pub values: Vec<Option<f64>>,
    /// Index of the first `Some` in `values` — where the line was seeded.
    pub start: Option<usize>,
}

/// The latest positive close on or before `date`, from `closes` sorted
/// ascending by date. A weekend, a holiday, or a point dated today (whose close
/// has not happened) all read the previous session's close.
fn close_at(closes: &[Close], date: &str) -> Option<f64> {
    let after = closes.partition_point(|(day, _)| day.as_str() <= date);
    closes[..after]
        .iter()
        .rev()
        .map(|&(_, close)| close)
        .find(|close| close.is_finite() && *close > 0.0)
}

/// Simulate putting the portfolio's money into one benchmark instead.
///
/// - `points`: the portfolio's value-over-time series (ascending dates).
/// - `flows`: the portfolio's dated net contributions (ascending); only those
///   AFTER the seeding point are applied, since everything before it is already
///   in the seed.
/// - `closes`: the benchmark's dividend-adjusted closes (ascending).
///
/// A withdrawal larger than the simulated account holds empties it rather than
/// driving it negative: the benchmark portfolio cannot sell shares it does not
/// own, and a below-zero line would claim a debt nobody took on.
#[must_use]
pub fn simulate_benchmark(
    points: &[HoldingsPoint],
    flows: &[DatedFlow],
    closes: &[Close],
) -> BenchmarkLine {
    let Some(start) = points
        .iter()
        .position(|point| close_at(closes, &point.date).is_some())
    else {
        return BenchmarkLine {
            values: vec![None; points.len()],
            start: None,
        };
    };

    let seed_date = points[start].date.as_str();
    let seed_close = close_at(closes, seed_date).unwrap_or(1.0); // Some by `position`
    let mut units = (points[start].market_value.floating_point() / seed_close).max(0.0);
    let mut pending = flows
        .iter()
        .skip_while(|flow| flow.date.as_str() <= seed_date)
        .peekable();

    let mut values = vec![None; start];
    for point in &points[start..] {
        while let Some(flow) = pending.next_if(|flow| flow.date <= point.date) {
            // Every flow here postdates the seed, which is itself priced, so a
            // close always exists; the guard is for a malformed close list.
            if let Some(close) = close_at(closes, &flow.date) {
                units = (units + flow.amount.floating_point() / close).max(0.0);
            }
        }
        values.push(close_at(closes, &point.date).map(|close| units * close));
    }
    BenchmarkLine {
        values,
        start: Some(start),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decimal::Dec;

    fn point(date: &str, value: i64) -> HoldingsPoint {
        HoldingsPoint {
            date: date.to_string(),
            bucket: date.to_string(),
            label: date.to_string(),
            market_value: Dec::new(i128::from(value), 0),
            basis: None,
        }
    }

    fn flow(date: &str, amount: i64) -> DatedFlow {
        DatedFlow {
            date: date.to_string(),
            amount: Dec::new(i128::from(amount), 0),
        }
    }

    fn closes(rows: &[(&str, f64)]) -> Vec<Close> {
        rows.iter().map(|&(d, c)| (d.to_string(), c)).collect()
    }

    fn approx(values: &[Option<f64>]) -> Vec<Option<i64>> {
        values
            .iter()
            .map(|v| v.map(|v| (v * 100.0).round() as i64))
            .collect()
    }

    #[test]
    fn both_lines_start_together_and_diverge_with_the_benchmark() {
        let points = [
            point("2026-01-31", 1000),
            point("2026-02-28", 1100),
            point("2026-03-31", 900),
        ];
        let prices = closes(&[
            ("2026-01-30", 100.0),
            ("2026-02-27", 120.0),
            ("2026-03-31", 150.0),
        ]);
        let line = simulate_benchmark(&points, &[], &prices);
        assert_eq!(line.start, Some(0));
        // Seeded with 10 units at $100; no flows; the benchmark's price moves it.
        assert_eq!(
            approx(&line.values),
            vec![Some(100_000), Some(120_000), Some(150_000)]
        );
    }

    #[test]
    fn in_window_flows_buy_and_sell_benchmark_units_at_their_own_dates_close() {
        let points = [
            point("2026-01-31", 1000),
            point("2026-02-28", 0),
            point("2026-03-31", 0),
        ];
        let prices = closes(&[
            ("2026-01-31", 100.0),
            ("2026-02-10", 50.0),
            ("2026-02-28", 60.0),
            ("2026-03-15", 80.0),
            ("2026-03-31", 100.0),
        ]);
        let flows = [
            flow("2026-01-15", 999_999), // before the seed: already inside it, ignored
            flow("2026-02-10", 500),     // +10 units at $50 → 20 units
            flow("2026-03-15", -800),    // −10 units at $80 → 10 units
        ];
        let line = simulate_benchmark(&points, &flows, &prices);
        assert_eq!(
            approx(&line.values),
            vec![Some(100_000), Some(120_000), Some(100_000)]
        );
    }

    #[test]
    fn a_flow_on_a_day_without_a_close_uses_the_previous_session() {
        let points = [point("2026-01-02", 0), point("2026-01-10", 0)];
        let prices = closes(&[("2026-01-02", 10.0), ("2026-01-09", 20.0)]);
        // Saturday 2026-01-03: priced at Friday's $10 → 10 units; the point on
        // Saturday the 10th reads Friday the 9th's $20.
        let line = simulate_benchmark(&points, &[flow("2026-01-03", 100)], &prices);
        assert_eq!(approx(&line.values), vec![Some(0), Some(20_000)]);
    }

    #[test]
    fn a_benchmark_younger_than_the_window_starts_where_it_can_be_priced() {
        let points = [
            point("2010-12-31", 500),
            point("2011-06-30", 800),
            point("2011-12-31", 900),
        ];
        let prices = closes(&[("2011-01-28", 40.0), ("2011-12-30", 50.0)]);
        let flows = [flow("2011-03-01", 100_000)]; // before its first close: unpriceable, but seed is later
        let line = simulate_benchmark(&points, &flows, &prices);
        assert_eq!(line.start, Some(1));
        // Seeded on 2011-06-30 with $800 at $40 = 20 units; the March flow is
        // before the seed, so it is already inside the $800.
        assert_eq!(
            approx(&line.values),
            vec![None, Some(80_000), Some(100_000)]
        );
    }

    #[test]
    fn no_close_at_all_is_an_all_gap_line() {
        let points = [point("2026-01-31", 1000), point("2026-02-28", 1000)];
        let line = simulate_benchmark(&points, &[], &[]);
        assert_eq!(line.start, None);
        assert_eq!(line.values, vec![None, None]);
    }

    #[test]
    fn withdrawals_beyond_the_simulated_balance_empty_it_rather_than_go_negative() {
        let points = [
            point("2026-01-31", 100),
            point("2026-02-28", 0),
            point("2026-03-31", 0),
        ];
        let prices = closes(&[
            ("2026-01-31", 10.0),
            ("2026-02-28", 5.0),
            ("2026-03-31", 10.0),
        ]);
        let flows = [flow("2026-02-15", -1_000), flow("2026-03-01", 50)];
        let line = simulate_benchmark(&points, &flows, &prices);
        // 10 units, then a −$1000 sale at $10 would be −100 units: empty at 0.
        // The March buy of $50 at Feb's $5 close = 10 units, worth $100 at $10.
        assert_eq!(
            approx(&line.values),
            vec![Some(10_000), Some(0), Some(10_000)]
        );
    }

    #[test]
    fn ignores_non_positive_closes() {
        let prices = closes(&[("2026-01-01", 10.0), ("2026-01-02", 0.0)]);
        assert_eq!(close_at(&prices, "2026-01-02"), Some(10.0));
        assert_eq!(close_at(&prices, "2025-12-31"), None);
    }
}
