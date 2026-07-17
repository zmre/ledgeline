//! Period/bucket math — port of `web/src/lib/reports/periods.ts`.
//!
//! Pure string + integer date arithmetic (Howard Hinnant's civil-day
//! algorithms); never any `Date`/timezone parsing. Divisions use
//! `div_euclid`/`rem_euclid`, which coincide with the TS `Math.floor` since all
//! divisors are positive.
//!
//! `today()` is intentionally not ported — it is a UI default (local wall
//! clock) that no report consumes, and it is the one function the TS module
//! documents as its only permitted `Date` use.

use super::ReportError;
use std::cmp::Ordering;

/// A bucketing interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interval {
    /// One day per bucket.
    Daily,
    /// ISO-8601 weeks (Mon–Sun; W01 contains Jan 4).
    Weekly,
    /// Calendar months.
    Monthly,
    /// Calendar quarters.
    Quarterly,
    /// Calendar years.
    Yearly,
}

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// `(year, month, day)` from a well-formed ISO date. Malformed slices yield 0
/// (unreachable in report flow, where dates always come from the journal or
/// bucket math).
fn parts(date: &str) -> (i64, i64, i64) {
    let field = |range: std::ops::Range<usize>| -> i64 {
        date.get(range).and_then(|s| s.parse().ok()).unwrap_or(0)
    };
    (field(0..4), field(5..7), field(8..10))
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    if month == 2 && is_leap(year) {
        29
    } else if (1..=12).contains(&month) {
        MONTH_DAYS[(month - 1) as usize]
    } else {
        0
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2).div_euclid(5) + day - 1;
    let doe = yoe * 365 + yoe.div_euclid(4) - yoe.div_euclid(100) + doy;
    era * 146097 + doe - 719468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe.div_euclid(1460) + doe.div_euclid(36524) - doe.div_euclid(146096))
        .div_euclid(365);
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe.div_euclid(4) - yoe.div_euclid(100));
    let mp = (5 * doy + 2).div_euclid(153);
    let day = doy - (153 * mp + 2).div_euclid(5) + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}

fn to_iso(year: i64, month: i64, day: i64) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

/// ISO weekday for a `days_from_civil` day number: 1 = Monday … 7 = Sunday.
fn iso_weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(7) + 1
}

/// Monday day-number of ISO week `week` in ISO week-year `year`.
fn iso_week_monday(year: i64, week: i64) -> i64 {
    let jan4 = days_from_civil(year, 1, 4);
    jan4 - (iso_weekday(jan4) - 1) + (week - 1) * 7
}

/// Bucket key containing `date`, e.g. `"2026-07"`, `"2026-Q3"`, `"2026-W28"`.
#[must_use]
pub fn bucket_key(date: &str, interval: Interval) -> String {
    let (y, m, d) = parts(date);
    match interval {
        Interval::Daily => date.to_string(),
        Interval::Weekly => {
            let days = days_from_civil(y, m, d);
            let thursday = days + (4 - iso_weekday(days));
            let (wy, _, _) = civil_from_days(thursday);
            let week = (thursday - days_from_civil(wy, 1, 1)).div_euclid(7) + 1;
            format!("{wy:04}-W{week:02}")
        }
        Interval::Monthly => format!("{y:04}-{m:02}"),
        Interval::Quarterly => format!("{y:04}-Q{}", (m - 1).div_euclid(3) + 1),
        Interval::Yearly => format!("{y:04}"),
    }
}

/// A recognized bucket key, parsed once for `bucket_start`/`bucket_end`/`label`.
enum ParsedKey<'a> {
    Day(&'a str),
    Year(i64),
    Month(i64, i64),
    Quarter(i64, i64),
    Week(i64, i64),
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn parse_bucket_key(key: &str) -> Option<ParsedKey<'_>> {
    let bytes = key.as_bytes();
    // Day: YYYY-MM-DD
    if key.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && all_digits(&key[0..4])
        && all_digits(&key[5..7])
        && all_digits(&key[8..10])
    {
        return Some(ParsedKey::Day(key));
    }
    // Year: YYYY
    if key.len() == 4 && all_digits(key) {
        return Some(ParsedKey::Year(key.parse().ok()?));
    }
    if key.len() == 7 && bytes[4] == b'-' && all_digits(&key[0..4]) {
        // Month: YYYY-MM
        if all_digits(&key[5..7]) {
            return Some(ParsedKey::Month(
                key[0..4].parse().ok()?,
                key[5..7].parse().ok()?,
            ));
        }
        // Quarter: YYYY-Q[1-4]
        if bytes[5] == b'Q' && (b'1'..=b'4').contains(&bytes[6]) {
            return Some(ParsedKey::Quarter(
                key[0..4].parse().ok()?,
                i64::from(bytes[6] - b'0'),
            ));
        }
    }
    // Week: YYYY-Wnn
    if key.len() == 8
        && bytes[4] == b'-'
        && bytes[5] == b'W'
        && all_digits(&key[0..4])
        && all_digits(&key[6..8])
    {
        return Some(ParsedKey::Week(
            key[0..4].parse().ok()?,
            key[6..8].parse().ok()?,
        ));
    }
    None
}

/// Human label for a bucket key: `"2026-07" → "Jul 2026"`, `"2026-Q3" → "Q3
/// 2026"`, `"2026-W28" → "W28 2026"`. Yearly and daily keys label themselves.
#[must_use]
pub fn bucket_label(key: &str) -> String {
    match parse_bucket_key(key) {
        Some(ParsedKey::Month(y, m)) if (1..=12).contains(&m) => {
            format!("{} {y:04}", MONTH_NAMES[(m - 1) as usize])
        }
        Some(ParsedKey::Quarter(y, q)) => format!("Q{q} {y:04}"),
        Some(ParsedKey::Week(y, w)) => format!("W{w:02} {y:04}"),
        _ => key.to_string(),
    }
}

/// First date in a bucket (companion to [`bucket_end`]).
///
/// # Errors
/// Returns [`ReportError::InvalidBucketKey`] for an unrecognized key.
pub fn bucket_start(key: &str) -> Result<String, ReportError> {
    match parse_bucket_key(key) {
        Some(ParsedKey::Day(d)) => Ok(d.to_string()),
        Some(ParsedKey::Year(y)) => Ok(format!("{y:04}-01-01")),
        Some(ParsedKey::Month(y, m)) => Ok(format!("{y:04}-{m:02}-01")),
        Some(ParsedKey::Quarter(y, q)) => Ok(to_iso(y, (q - 1) * 3 + 1, 1)),
        Some(ParsedKey::Week(y, w)) => {
            let (yy, mm, dd) = civil_from_days(iso_week_monday(y, w));
            Ok(to_iso(yy, mm, dd))
        }
        None => Err(ReportError::InvalidBucketKey(key.to_string())),
    }
}

/// Last date in a bucket (leap-aware; weekly buckets end on Sunday).
///
/// # Errors
/// Returns [`ReportError::InvalidBucketKey`] for an unrecognized key.
pub fn bucket_end(key: &str) -> Result<String, ReportError> {
    match parse_bucket_key(key) {
        Some(ParsedKey::Day(d)) => Ok(d.to_string()),
        Some(ParsedKey::Year(y)) => Ok(format!("{y:04}-12-31")),
        Some(ParsedKey::Month(y, m)) => Ok(to_iso(y, m, days_in_month(y, m))),
        Some(ParsedKey::Quarter(y, q)) => {
            let m = q * 3;
            Ok(to_iso(y, m, days_in_month(y, m)))
        }
        Some(ParsedKey::Week(y, w)) => {
            let (yy, mm, dd) = civil_from_days(iso_week_monday(y, w) + 6);
            Ok(to_iso(yy, mm, dd))
        }
        None => Err(ReportError::InvalidBucketKey(key.to_string())),
    }
}

/// The `n` consecutive bucket keys ending with the bucket containing `end`,
/// oldest → newest. Empty when `n == 0`.
///
/// # Errors
/// Returns [`ReportError::InvalidBucketKey`] if bucket math ever yields an
/// unrecognized key (unreachable for the intervals here).
pub fn last_n_buckets(end: &str, interval: Interval, n: usize) -> Result<Vec<String>, ReportError> {
    let mut out = Vec::with_capacity(n);
    let mut key = bucket_key(end, interval);
    for _ in 0..n {
        out.push(key.clone());
        let (y, m, d) = parts(&bucket_start(&key)?);
        let (py, pm, pd) = civil_from_days(days_from_civil(y, m, d) - 1);
        key = bucket_key(&to_iso(py, pm, pd), interval);
    }
    out.reverse();
    Ok(out)
}

/// The bucket key immediately following `key` for the same `interval` (the
/// bucket containing the day after `key`'s last day). Companion to
/// [`last_n_buckets`], for forward iteration (used by the budget report to walk
/// a periodic rule's occurrences across a report span).
///
/// # Errors
/// Returns [`ReportError::InvalidBucketKey`] for an unrecognized key.
pub fn next_bucket(key: &str, interval: Interval) -> Result<String, ReportError> {
    let (y, m, d) = parts(&bucket_end(key)?);
    let (ny, nm, nd) = civil_from_days(days_from_civil(y, m, d) + 1);
    Ok(bucket_key(&to_iso(ny, nm, nd), interval))
}

/// Lexical ISO-date comparison.
#[must_use]
pub fn compare_iso(a: &str, b: &str) -> Ordering {
    a.cmp(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_key_daily_monthly_quarterly_yearly() {
        assert_eq!(bucket_key("2026-07-08", Interval::Daily), "2026-07-08");
        assert_eq!(bucket_key("2026-07-08", Interval::Monthly), "2026-07");
        assert_eq!(bucket_key("2026-01-31", Interval::Quarterly), "2026-Q1");
        assert_eq!(bucket_key("2026-03-31", Interval::Quarterly), "2026-Q1");
        assert_eq!(bucket_key("2026-04-01", Interval::Quarterly), "2026-Q2");
        assert_eq!(bucket_key("2026-10-01", Interval::Quarterly), "2026-Q4");
        assert_eq!(bucket_key("2026-07-08", Interval::Yearly), "2026");
    }

    #[test]
    fn bucket_key_iso_weeks_incl_year_boundaries() {
        assert_eq!(bucket_key("2026-07-08", Interval::Weekly), "2026-W28");
        assert_eq!(bucket_key("2026-01-01", Interval::Weekly), "2026-W01");
        assert_eq!(bucket_key("2025-12-29", Interval::Weekly), "2026-W01");
        assert_eq!(bucket_key("2024-12-29", Interval::Weekly), "2024-W52");
        assert_eq!(bucket_key("2024-12-30", Interval::Weekly), "2025-W01");
        assert_eq!(bucket_key("2021-01-01", Interval::Weekly), "2020-W53");
        assert_eq!(bucket_key("2020-12-31", Interval::Weekly), "2020-W53");
        assert_eq!(bucket_key("2019-12-30", Interval::Weekly), "2020-W01");
        assert_eq!(bucket_key("2020-02-29", Interval::Weekly), "2020-W09");
        assert_eq!(bucket_key("2015-12-28", Interval::Weekly), "2015-W53");
        assert_eq!(bucket_key("2016-01-03", Interval::Weekly), "2015-W53");
        assert_eq!(bucket_key("2016-01-04", Interval::Weekly), "2016-W01");
    }

    #[test]
    fn labels_each_key_format() {
        assert_eq!(bucket_label("2026-07"), "Jul 2026");
        assert_eq!(bucket_label("2026-01"), "Jan 2026");
        assert_eq!(bucket_label("2026-Q3"), "Q3 2026");
        assert_eq!(bucket_label("2026-W05"), "W05 2026");
        assert_eq!(bucket_label("2026"), "2026");
        assert_eq!(bucket_label("2026-07-08"), "2026-07-08");
    }

    #[test]
    fn month_ends_including_leap_years() {
        assert_eq!(bucket_end("2024-02").unwrap(), "2024-02-29");
        assert_eq!(bucket_end("2023-02").unwrap(), "2023-02-28");
        assert_eq!(bucket_end("2100-02").unwrap(), "2100-02-28");
        assert_eq!(bucket_end("2000-02").unwrap(), "2000-02-29");
        assert_eq!(bucket_end("2026-01").unwrap(), "2026-01-31");
        assert_eq!(bucket_end("2026-04").unwrap(), "2026-04-30");
        assert_eq!(bucket_end("2026-12").unwrap(), "2026-12-31");
        assert_eq!(bucket_start("2026-12").unwrap(), "2026-12-01");
    }

    #[test]
    fn quarter_and_year_bounds() {
        assert_eq!(bucket_end("2026-Q1").unwrap(), "2026-03-31");
        assert_eq!(bucket_end("2024-Q1").unwrap(), "2024-03-31");
        assert_eq!(bucket_end("2026-Q2").unwrap(), "2026-06-30");
        assert_eq!(bucket_end("2026-Q3").unwrap(), "2026-09-30");
        assert_eq!(bucket_end("2026-Q4").unwrap(), "2026-12-31");
        assert_eq!(bucket_start("2026-Q3").unwrap(), "2026-07-01");
        assert_eq!(bucket_end("2026").unwrap(), "2026-12-31");
        assert_eq!(bucket_start("2026").unwrap(), "2026-01-01");
        assert_eq!(bucket_end("2026-07-08").unwrap(), "2026-07-08");
        assert_eq!(bucket_start("2026-07-08").unwrap(), "2026-07-08");
    }

    #[test]
    fn iso_week_bounds_across_year_boundaries() {
        assert_eq!(bucket_start("2026-W28").unwrap(), "2026-07-06");
        assert_eq!(bucket_end("2026-W28").unwrap(), "2026-07-12");
        assert_eq!(bucket_start("2026-W01").unwrap(), "2025-12-29");
        assert_eq!(bucket_end("2026-W01").unwrap(), "2026-01-04");
        assert_eq!(bucket_start("2020-W01").unwrap(), "2019-12-30");
        assert_eq!(bucket_end("2020-W53").unwrap(), "2021-01-03");
    }

    #[test]
    fn rejects_unrecognized_keys() {
        assert_eq!(
            bucket_end("garbage"),
            Err(ReportError::InvalidBucketKey("garbage".into()))
        );
        assert_eq!(
            bucket_start("2026-Q5"),
            Err(ReportError::InvalidBucketKey("2026-Q5".into()))
        );
    }

    #[test]
    fn last_n_buckets_walks_intervals() {
        assert_eq!(
            last_n_buckets("2026-02-15", Interval::Monthly, 4).unwrap(),
            ["2025-11", "2025-12", "2026-01", "2026-02"]
        );
        assert_eq!(
            last_n_buckets("2026-02-15", Interval::Quarterly, 5).unwrap(),
            ["2025-Q1", "2025-Q2", "2025-Q3", "2025-Q4", "2026-Q1"]
        );
        assert_eq!(
            last_n_buckets("2026-02-15", Interval::Yearly, 3).unwrap(),
            ["2024", "2025", "2026"]
        );
        assert_eq!(
            last_n_buckets("2026-01-01", Interval::Weekly, 3).unwrap(),
            ["2025-W51", "2025-W52", "2026-W01"]
        );
        assert_eq!(
            last_n_buckets("2021-01-04", Interval::Weekly, 3).unwrap(),
            ["2020-W52", "2020-W53", "2021-W01"]
        );
        assert_eq!(
            last_n_buckets("2024-03-01", Interval::Daily, 3).unwrap(),
            ["2024-02-28", "2024-02-29", "2024-03-01"]
        );
        assert!(
            last_n_buckets("2026-07-08", Interval::Monthly, 0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn compares_iso_lexically() {
        assert_eq!(compare_iso("2026-01-31", "2026-02-01"), Ordering::Less);
        assert_eq!(compare_iso("2026-02-01", "2026-02-01"), Ordering::Equal);
        assert_eq!(compare_iso("2026-02-02", "2026-02-01"), Ordering::Greater);
    }
}
