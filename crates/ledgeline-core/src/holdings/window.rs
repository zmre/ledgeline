//! The value-over-time chart's window: how many buckets a `since` date spans,
//! and which interval an open-ended window ("all time", year to date) is drawn
//! at.
//!
//! Both Holdings series endpoints take the SPA's gain period either as a bucket
//! count (`12mo` is twelve month-ends, as it always was) or as a start date. A
//! start date is the principled form for anything the SPA cannot count by
//! itself — "all time" begins at the scope's first holding activity, which only
//! the engine knows — and this module is the arithmetic that turns it back into
//! the `(interval, count)` pair `holdings_series` already takes, so neither
//! series engine grows a second way to be asked for a window.

use std::cmp::Ordering;

use crate::reports::periods::{bucket_key, bucket_start, days_between, months_between, parts};
use crate::reports::{Interval, ReportError, bucket_end, compare_iso};

/// How few points an auto-chosen interval aims to plot at least: fewer than
/// this and the line is a couple of segments that say less than the table.
const MIN_POINTS: usize = 6;

/// How many points an auto-chosen interval may plot at most. Ten years of
/// months; past it the interval coarsens to quarters, then years.
const MAX_POINTS: usize = 120;

/// Number of `interval` buckets from the one containing `since` through the one
/// containing `as_of`, inclusive — the `count` that makes `last_n_buckets(as_of,
/// interval, count)` start at `since`'s bucket. `1` when `since` is on or after
/// `as_of`.
///
/// Pure arithmetic on the bucket boundaries, never a walk, so an absurd span
/// costs nothing to measure; callers compare the answer against
/// [`crate::reports::MAX_BUCKETS`] before asking for that many buckets.
#[must_use]
pub fn series_count(since: &str, as_of: &str, interval: Interval) -> usize {
    if since >= as_of {
        return 1;
    }
    let span = match interval {
        Interval::Daily => days_between(since, as_of),
        Interval::Weekly => {
            let monday = |date: &str| bucket_start(&bucket_key(date, Interval::Weekly));
            match (monday(since), monday(as_of)) {
                (Ok(from), Ok(to)) => days_between(&from, &to) / 7,
                // Unreachable: a weekly key always parses back.
                _ => days_between(since, as_of) / 7,
            }
        }
        Interval::Monthly => months_between(since, as_of),
        Interval::Quarterly => {
            let quarter = |date: &str| {
                let (year, month, _) = parts(date);
                year * 4 + (month - 1).div_euclid(3)
            };
            quarter(as_of) - quarter(since)
        }
        Interval::Yearly => parts(as_of).0 - parts(since).0,
    };
    usize::try_from(span).map_or(1, |span| span.saturating_add(1))
}

/// Which points a value-over-time series takes: `count` buckets of `interval`
/// ending at the scope's `as_of`, the first taken at `start` when there is one
/// (see [`series_dates`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesWindow {
    /// The bucket grain.
    pub interval: Interval,
    /// How many buckets, the last containing `as_of`.
    pub count: usize,
    /// For a window asked for from a date: that date, where the first point is
    /// taken — the gain period's reference. `None` for a counted window.
    pub start: Option<String>,
}

impl SeriesWindow {
    /// The last `count` buckets, each point at its bucket's end.
    #[must_use]
    pub fn counted(interval: Interval, count: usize) -> Self {
        Self {
            interval,
            count,
            start: None,
        }
    }
}

/// The date each bucket's point is taken at: the bucket's last day, clamped so
/// the final point never overshoots `as_of`.
///
/// With a `start` (a window asked for by date, `since=YYYY-MM-DD`), the FIRST
/// point is taken at `start` itself rather than at the end of `start`'s bucket:
/// the window is the gain period's, so its first point must be the value the
/// gain is measured against — `mv(start)` — and the benchmark overlay is seeded
/// there too. A twelve-month window from 2025-09-28 is then 2025-09-28 followed
/// by twelve month-ends, not 2025-09-30. A single-bucket window keeps its one
/// point at `as_of`: moving it would leave no point at the window's end.
///
/// # Errors
/// [`ReportError`] for an unparseable bucket key (unreachable for keys from
/// `last_n_buckets`).
pub fn series_dates(
    keys: &[String],
    as_of: &str,
    start: Option<&str>,
) -> Result<Vec<String>, ReportError> {
    let mut dates = keys
        .iter()
        .map(|key| {
            let end = bucket_end(key)?;
            Ok(if compare_iso(&end, as_of) == Ordering::Greater {
                as_of.to_string()
            } else {
                end
            })
        })
        .collect::<Result<Vec<String>, ReportError>>()?;
    if let (Some(start), [first, _, ..]) = (start, dates.as_mut_slice())
        && compare_iso(start, first) == Ordering::Less
    {
        *first = start.to_string();
    }
    Ok(dates)
}

/// The interval an open-ended window from `since` to `as_of` is drawn at.
///
/// Months once the window holds at least [`MIN_POINTS`] of them — the
/// granularity the default twelve-month chart has always used — coarsening to
/// quarters and then years so no chart plots more than [`MAX_POINTS`]. A window
/// too short for six months is drawn weekly, and one too short for six weeks
/// daily, so a young portfolio or an early-January year-to-date still gets a
/// line rather than two dots.
#[must_use]
pub fn auto_interval(since: &str, as_of: &str) -> Interval {
    let count = |interval| series_count(since, as_of, interval);
    if count(Interval::Monthly) >= MIN_POINTS {
        [Interval::Monthly, Interval::Quarterly]
            .into_iter()
            .find(|&interval| count(interval) <= MAX_POINTS)
            .unwrap_or(Interval::Yearly)
    } else if count(Interval::Weekly) >= MIN_POINTS {
        Interval::Weekly
    } else {
        Interval::Daily
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reports::last_n_buckets;

    #[test]
    fn counts_buckets_inclusive_of_both_ends() {
        assert_eq!(series_count("2026-09-21", "2026-09-28", Interval::Daily), 8);
        assert_eq!(
            series_count("2026-01-15", "2026-09-28", Interval::Monthly),
            9
        );
        assert_eq!(
            series_count("2025-12-31", "2026-09-28", Interval::Quarterly),
            4
        );
        assert_eq!(
            series_count("2020-06-01", "2026-09-28", Interval::Yearly),
            7
        );
    }

    #[test]
    fn weekly_counts_iso_weeks_not_seven_day_spans() {
        // Sunday 2026-09-27 and Monday 2026-09-28 are in different ISO weeks.
        assert_eq!(
            series_count("2026-09-27", "2026-09-28", Interval::Weekly),
            2
        );
        assert_eq!(
            series_count("2026-09-29", "2026-10-04", Interval::Weekly),
            1
        );
    }

    #[test]
    fn a_since_on_or_after_as_of_is_one_bucket() {
        assert_eq!(
            series_count("2026-09-28", "2026-09-28", Interval::Monthly),
            1
        );
        assert_eq!(series_count("2027-01-01", "2026-09-28", Interval::Daily), 1);
    }

    /// The contract callers rely on: feeding the count back to
    /// `last_n_buckets` starts the series at `since`'s own bucket.
    #[test]
    fn the_count_starts_last_n_buckets_at_since() {
        for interval in [
            Interval::Daily,
            Interval::Weekly,
            Interval::Monthly,
            Interval::Quarterly,
            Interval::Yearly,
        ] {
            let since = "2024-02-29";
            let keys = last_n_buckets(
                "2026-09-28",
                interval,
                series_count(since, "2026-09-28", interval),
            )
            .unwrap();
            assert_eq!(keys[0], bucket_key(since, interval), "{interval:?}");
        }
    }

    fn keys(as_of: &str, since: &str, interval: Interval) -> Vec<String> {
        last_n_buckets(as_of, interval, series_count(since, as_of, interval)).unwrap()
    }

    #[test]
    fn a_dated_window_takes_its_first_point_at_the_start_itself() {
        let monthly = keys("2026-09-28", "2025-09-28", Interval::Monthly);
        let dates = series_dates(&monthly, "2026-09-28", Some("2025-09-28")).unwrap();
        assert_eq!(dates.len(), 13);
        assert_eq!(dates[0], "2025-09-28");
        assert_eq!(dates[1], "2025-10-31");
        assert_eq!(dates.last().unwrap(), "2026-09-28");

        // Year to date from the prior year's last day, drawn quarterly: the
        // first point is Dec 31 (already its bucket's end), then quarter-ends.
        let quarterly = keys("2026-09-28", "2025-12-31", Interval::Quarterly);
        let dates = series_dates(&quarterly, "2026-09-28", Some("2025-12-31")).unwrap();
        assert_eq!(
            dates,
            ["2025-12-31", "2026-03-31", "2026-06-30", "2026-09-28"]
        );

        // A weekly window starting mid-week starts mid-week.
        let weekly = keys("2026-09-28", "2026-06-24", Interval::Weekly);
        let dates = series_dates(&weekly, "2026-09-28", Some("2026-06-24")).unwrap();
        assert_eq!(dates[0], "2026-06-24");
        assert_eq!(dates[1], "2026-07-05");
    }

    #[test]
    fn without_a_start_every_point_is_its_buckets_end() {
        let monthly = last_n_buckets("2026-09-28", Interval::Monthly, 3).unwrap();
        assert_eq!(
            series_dates(&monthly, "2026-09-28", None).unwrap(),
            ["2026-07-31", "2026-08-31", "2026-09-28"]
        );
        // One bucket: its point stays at as_of whatever the start.
        let one = last_n_buckets("2026-09-28", Interval::Monthly, 1).unwrap();
        assert_eq!(
            series_dates(&one, "2026-09-28", Some("2026-09-02")).unwrap(),
            ["2026-09-28"]
        );
    }

    #[test]
    fn auto_prefers_months_then_coarsens_past_ten_years() {
        assert_eq!(auto_interval("2021-01-01", "2026-09-28"), Interval::Monthly);
        // 2012-10 .. 2026-09 is 168 months (> 120) but 56 quarters.
        assert_eq!(
            auto_interval("2012-10-01", "2026-09-28"),
            Interval::Quarterly
        );
        assert_eq!(auto_interval("1990-01-01", "2026-09-28"), Interval::Yearly);
    }

    #[test]
    fn auto_goes_finer_when_months_would_be_too_few_points() {
        // Year to date in late April: Dec..Apr is five months, so weekly.
        assert_eq!(auto_interval("2025-12-31", "2026-04-25"), Interval::Weekly);
        // …and in June it is seven (Dec..Jun), so monthly.
        assert_eq!(auto_interval("2025-12-31", "2026-06-02"), Interval::Monthly);
        // Early January: a handful of days.
        assert_eq!(auto_interval("2025-12-31", "2026-01-09"), Interval::Daily);
    }
}
