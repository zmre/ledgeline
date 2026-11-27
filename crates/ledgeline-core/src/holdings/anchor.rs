//! The value chart's y-axis anchor (plan 47 D1/D2): the value range over a
//! horizon at least as long as the period shown, so a quiet week is drawn on
//! the scale of the year around it instead of being stretched into drama.
//!
//! The engine answers the RANGE only — `min`/`max` of real values. Padding is
//! presentation (D3) and stays in the SPA.

use crate::decimal::Dec;
use crate::reports::periods::add_months;

use super::series::HoldingsPoint;
use super::window::{SeriesWindow, auto_interval, series_count};

/// How far back the anchor looks (D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorHorizon {
    /// The last year: every period shorter than a year anchors here.
    OneYear,
    /// The last five years.
    FiveYears,
    /// Since the scope's first holding activity.
    All,
}

impl AnchorHorizon {
    /// Parse the wire spelling: `1yr`, `5yr` or `all`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "1yr" => Some(Self::OneYear),
            "5yr" => Some(Self::FiveYears),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// The horizon's first day for a chart ending at `as_of`: a calendar year
    /// (or five) back, or `inception` for [`AnchorHorizon::All`] — `None` when
    /// nothing in scope has ever been held, so there is no horizon to add.
    #[must_use]
    pub fn start(self, as_of: &str, inception: impl FnOnce() -> Option<String>) -> Option<String> {
        match self {
            Self::OneYear => Some(add_months(as_of, -12)),
            Self::FiveYears => Some(add_months(as_of, -60)),
            Self::All => inception().map(|first| first.min(as_of.to_string())),
        }
    }
}

/// The window an anchor series is sampled over: `start` through `as_of` at the
/// interval an open-ended chart of that span would use ([`auto_interval`]),
/// the first point taken at `start` itself.
#[must_use]
pub fn anchor_window(start: &str, as_of: &str) -> SeriesWindow {
    let interval = auto_interval(start, as_of);
    SeriesWindow {
        interval,
        count: series_count(start, as_of, interval),
        start: Some(start.to_string()),
    }
}

/// A closed value range, in the series' base commodity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueRange {
    pub min: Dec,
    pub max: Dec,
}

/// The min and max market value over every series in `series` (D2): each
/// series first drops its LEADING points that are `<= 0` — the empty months
/// before an opening balance, which would otherwise pin the axis at zero under
/// a portfolio that never was zero — and the rest join one min/max. `None`
/// when no series has a point `> 0`.
///
/// Only leading points are skipped. A position that is sold down to nothing
/// LATER really was worth zero then, and the axis must reach it.
#[must_use]
pub fn anchor_range<'a>(
    series: impl IntoIterator<Item = &'a [HoldingsPoint]>,
) -> Option<ValueRange> {
    series
        .into_iter()
        .flat_map(|points| {
            points
                .iter()
                .skip_while(|point| point.market_value.mantissa <= 0)
                .map(|point| point.market_value)
        })
        .fold(None, |range: Option<ValueRange>, value| {
            Some(match range {
                None => ValueRange {
                    min: value,
                    max: value,
                },
                Some(range) => ValueRange {
                    min: range.min.min(value),
                    max: range.max.max(value),
                },
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizons_count_calendar_years_back() {
        let never = || -> Option<String> { panic!("only `all` reads inception") };
        assert_eq!(
            AnchorHorizon::OneYear.start("2026-11-26", never).as_deref(),
            Some("2025-11-26")
        );
        assert_eq!(
            AnchorHorizon::FiveYears
                .start("2024-02-29", never)
                .as_deref(),
            Some("2019-02-28")
        );
        assert_eq!(
            AnchorHorizon::All
                .start("2026-11-26", || Some("2020-03-04".to_string()))
                .as_deref(),
            Some("2020-03-04")
        );
        assert_eq!(AnchorHorizon::All.start("2026-11-26", || None), None);
    }

    #[test]
    fn only_the_wire_spellings_parse() {
        assert_eq!(AnchorHorizon::parse("1yr"), Some(AnchorHorizon::OneYear));
        assert_eq!(AnchorHorizon::parse("5yr"), Some(AnchorHorizon::FiveYears));
        assert_eq!(AnchorHorizon::parse("all"), Some(AnchorHorizon::All));
        assert_eq!(AnchorHorizon::parse("1y"), None);
        assert_eq!(AnchorHorizon::parse(""), None);
    }

    #[test]
    fn the_anchor_window_starts_at_the_horizon() {
        let window = anchor_window("2025-11-26", "2026-11-26");
        assert_eq!(window.start.as_deref(), Some("2025-11-26"));
        assert_eq!(window.count, 13);
    }
}
