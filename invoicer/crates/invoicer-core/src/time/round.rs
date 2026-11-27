//! Rounding for billing (plan 46 B-round, T11): never stored, applied when
//! an invoice row is made, with the actual and the rounded hours both shown.
//!
//! A customer note sets `time_increment` (minutes; none by default),
//! `time_rounding` (`up`, the default, or `nearest`) and `time_round_scope`:
//! `row` (the default: round the grouped row, ABA 93-379 friendly), `day`
//! (each account-day) or `entry`.

use ledgeline_core::time::TimeQty;
use serde::Serialize;
use std::collections::BTreeMap;

/// Which way a quantity rounds to the increment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// Up to the next increment.
    #[default]
    Up,
    /// To the nearest increment, half up.
    Nearest,
}

/// What is rounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// Each invoice row's total.
    #[default]
    Row,
    /// Each day's total within a row.
    Day,
    /// Each entry.
    Entry,
}

/// A customer's rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct Rounding {
    /// The increment in minutes; `None` rounds nothing.
    pub increment: Option<u32>,
    /// Up or nearest.
    pub direction: Direction,
    /// Row, day or entry.
    pub scope: Scope,
}

impl Rounding {
    /// From the note's keys; anything unreadable is the default.
    #[must_use]
    pub fn of_fields(increment: Option<u32>, direction: Option<&str>, scope: Option<&str>) -> Self {
        Self {
            increment: increment.filter(|minutes| *minutes > 0),
            direction: match direction.map(str::to_ascii_lowercase).as_deref() {
                Some("nearest") => Direction::Nearest,
                _ => Direction::Up,
            },
            scope: match scope.map(str::to_ascii_lowercase).as_deref() {
                Some("day") => Scope::Day,
                Some("entry") => Scope::Entry,
                _ => Scope::Row,
            },
        }
    }

    /// `quantity` rounded to the increment.
    #[must_use]
    pub fn round(&self, quantity: TimeQty) -> TimeQty {
        let Some(increment) = self.increment else {
            return quantity;
        };
        let step = TimeQty::from_minutes(i64::from(increment)).nanos();
        let nanos = quantity.nanos();
        if nanos <= 0 || step <= 0 {
            return quantity;
        }
        let whole = nanos / step;
        let rest = nanos % step;
        let steps = match self.direction {
            Direction::Up if rest > 0 => whole + 1,
            Direction::Nearest if rest * 2 >= step => whole + 1,
            _ => whole,
        };
        TimeQty::from_nanos(steps * step)
    }

    /// A row's billed quantity from its entries (`(date, quantity)`), by the
    /// scope.
    #[must_use]
    pub fn row(&self, entries: &[(&str, TimeQty)]) -> TimeQty {
        match self.scope {
            Scope::Row => self.round(TimeQty::sum(entries.iter().map(|(_, q)| *q))),
            Scope::Entry => TimeQty::sum(entries.iter().map(|(_, q)| self.round(*q))),
            Scope::Day => {
                let mut days: BTreeMap<&str, TimeQty> = BTreeMap::new();
                for (date, quantity) in entries {
                    let day = days.entry(date).or_default();
                    *day = day.plus(*quantity);
                }
                TimeQty::sum(days.into_values().map(|q| self.round(q)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(minutes: i64) -> TimeQty {
        TimeQty::from_minutes(minutes)
    }

    /// ABA 93-379: three 2-minute emails are one 6-minute row by the row,
    /// three increments by the entry.
    #[test]
    fn scopes_round_rows_days_and_entries() {
        let six_up = Rounding::of_fields(Some(6), None, None);
        let emails = [
            ("2026-11-02", m(2)),
            ("2026-11-02", m(2)),
            ("2026-11-03", m(2)),
        ];
        assert_eq!(six_up.row(&emails), m(6));
        let by_entry = Rounding {
            scope: Scope::Entry,
            ..six_up
        };
        assert_eq!(by_entry.row(&emails), m(18));
        let by_day = Rounding {
            scope: Scope::Day,
            ..six_up
        };
        assert_eq!(by_day.row(&emails), m(12));
        let nearest = Rounding::of_fields(Some(15), Some("nearest"), Some("row"));
        assert_eq!(nearest.round(m(22)), m(15));
        assert_eq!(nearest.round(TimeQty::from_seconds(22 * 60 + 30)), m(30));
        assert_eq!(Rounding::default().row(&emails), m(6));
        assert_eq!(six_up.round(m(12)), m(12));
        assert_eq!(Rounding::of_fields(Some(0), None, None).increment, None);
    }
}
