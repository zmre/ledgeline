//! Display formatting for amounts — the engine's copy of the SPA's
//! `web/src/lib/domain/money.ts` `displayPlaces` / `formatDec` /
//! `formatAmount` (plan 26 B2).
//!
//! Search text, check messages and any future non-web UI need to render an
//! amount EXACTLY as the web UI does, so this is a port, not a redesign:
//! `fixtures/display-format/cases.json` — generated from the TypeScript, and
//! asserted by both `web/src/lib/domain/displayFormat.golden.test.ts` and
//! `tests/display_format.rs` — is what keeps the two from drifting.
//!
//! This is a DISPLAY boundary. Exact [`Dec`]s keep their full precision
//! everywhere else; rounding happens here and nowhere upstream.
//!
//! Unlike `edit::render_amount` (which writes journal text and must round-trip
//! through the parser), this renders for people: money is capped at two
//! decimals, and a commodity-less amount carries no symbol.

use crate::decimal::Dec;
use crate::model::{AmountStyle, CommoditySide, DigitGroups};

/// Display cap for MONEY: never more than two decimal places, whatever the
/// style or the value's own precision says. Its unit of account is the cent; a
/// third decimal on a balance is noise.
pub const MAX_DISPLAY_DECIMALS: u32 = 2;

/// Display cap for NON-money quantities (unit/share counts, per-unit rates):
/// 8 places is the finest subdivision in wide use (a satoshi is 1e-8 BTC).
pub const MAX_QUANTITY_DECIMALS: u32 = 8;

/// The decimal mark used when a style names none (`asdecimalmark: null`),
/// exactly as the SPA decodes such a style.
const DEFAULT_DECIMAL_MARK: char = '.';

/// `d` rounded half away from zero to `places`, for display.
///
/// [`Dec::rounded`] can only fail on overflow. Scaling DOWN overflows only when
/// `10^(d.places - places)` exceeds `i128`, i.e. by more than 38 digits — and
/// every `i128` mantissa is then below half the divisor, so the value rounds to
/// zero. Scaling UP past `i128` cannot be represented at all, so the value
/// renders at its own scale instead (no digit is invented or lost).
fn round_display(d: Dec, places: u32) -> Dec {
    d.rounded(places).unwrap_or(if places < d.places {
        Dec::new(0, places)
    } else {
        d
    })
}

/// How many decimal places to render `d` at: `min(precision, max_decimals)`,
/// with one exception.
///
/// Rounding may COMPRESS a number; it must never DELETE one. When the cap would
/// round a non-zero value to zeros — a `$0.00012345` price shown as `0.00` —
/// the places are relaxed to the value's own precision, bounded by
/// [`MAX_QUANTITY_DECIMALS`], but only if that actually reveals a digit (a value
/// below 1e-8 keeps the short zero rather than a row of zeros).
#[must_use]
pub fn display_places(d: Dec, precision: u32, max_decimals: u32) -> u32 {
    let places = precision.min(max_decimals);
    if d.is_zero() || !round_display(d, places).is_zero() {
        return places;
    }
    let relaxed = d.places.min(MAX_QUANTITY_DECIMALS);
    if relaxed > places && !round_display(d, relaxed).is_zero() {
        relaxed
    } else {
        places
    }
}

/// Group integer digits right to left; the last group size repeats (hledger
/// semantics). A zero size stops grouping, as an empty size list does.
fn group_digits(int_digits: &str, groups: &DigitGroups) -> String {
    if groups.sizes.is_empty() {
        return int_digits.to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    let mut rest = int_digits;
    let mut index = 0;
    while !rest.is_empty() {
        let size = usize::from(groups.sizes[index.min(groups.sizes.len() - 1)]);
        if size == 0 || rest.len() <= size {
            parts.push(rest);
            break;
        }
        let (head, tail) = rest.split_at(rest.len() - size);
        parts.push(tail);
        rest = head;
        index += 1;
    }
    parts.reverse();
    parts.join(&groups.mark.to_string())
}

/// Format a quantity per `style`: sign, digit grouping, decimal mark, and the
/// places [`display_places`] picks. `-` for negatives, after any symbol (the
/// symbol is [`format_amount`]'s business): `$-1,234.56`.
#[must_use]
pub fn format_dec(d: Dec, style: &AmountStyle, max_decimals: u32) -> String {
    let rounded = round_display(d, display_places(d, style.precision, max_decimals));
    let negative = rounded.mantissa < 0;
    let places = rounded.places as usize;
    let digits = format!(
        "{:0>width$}",
        rounded.mantissa.unsigned_abs(),
        width = places + 1
    );
    let (int_digits, frac_digits) = digits.split_at(digits.len() - places);
    let int_part = style.digit_groups.as_deref().map_or_else(
        || int_digits.to_string(),
        |groups| group_digits(int_digits, groups),
    );
    let frac_part = if frac_digits.is_empty() {
        String::new()
    } else {
        format!(
            "{}{frac_digits}",
            style.decimal_mark.unwrap_or(DEFAULT_DECIMAL_MARK)
        )
    };
    format!("{}{int_part}{frac_part}", if negative { "-" } else { "" })
}

/// Format `qty` of `commodity` honouring the style's side, spacing, precision
/// and grouping: `$-1,234.56`, `45,00 EUR`. A commodity-less amount is just the
/// number. Pass [`MAX_DISPLAY_DECIMALS`] for money and [`MAX_QUANTITY_DECIMALS`]
/// for a unit count.
#[must_use]
pub fn format_amount(commodity: &str, qty: Dec, style: &AmountStyle, max_decimals: u32) -> String {
    let number = format_dec(qty, style, max_decimals);
    if commodity.is_empty() {
        return number;
    }
    let space = if style.spaced { " " } else { "" };
    match style.side {
        CommoditySide::Left => format!("{commodity}{space}{number}"),
        CommoditySide::Right => format!("{number}{space}{commodity}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn usd() -> AmountStyle {
        AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: Some('.'),
            digit_groups: Some(Arc::new(DigitGroups {
                mark: ',',
                sizes: vec![3],
            })),
            precision: 2,
        }
    }

    #[test]
    fn money_is_capped_at_two_places_and_grouped() {
        let style = usd();
        assert_eq!(
            format_amount("$", Dec::new(-123_456_789, 3), &style, MAX_DISPLAY_DECIMALS),
            "$-123,456.79"
        );
        assert_eq!(
            format_amount("$", Dec::new(5, 0), &style, MAX_DISPLAY_DECIMALS),
            "$5.00"
        );
    }

    #[test]
    fn a_value_the_cap_would_zero_is_relaxed_not_deleted() {
        let style = usd();
        assert_eq!(
            display_places(Dec::new(12_345, 8), 2, MAX_DISPLAY_DECIMALS),
            8
        );
        assert_eq!(
            format_dec(Dec::new(12_345, 8), &style, MAX_DISPLAY_DECIMALS),
            "0.00012345"
        );
        // Below 1e-8 there is no digit to reveal: keep the short zero.
        assert_eq!(
            format_dec(Dec::new(1, 10), &style, MAX_DISPLAY_DECIMALS),
            "0.00"
        );
    }

    #[test]
    fn half_away_from_zero_both_signs() {
        let style = usd();
        assert_eq!(
            format_dec(Dec::new(5, 3), &style, MAX_DISPLAY_DECIMALS),
            "0.01"
        );
        assert_eq!(
            format_dec(Dec::new(-5, 3), &style, MAX_DISPLAY_DECIMALS),
            "-0.01"
        );
        // Would round to zero at two places, so the places are relaxed.
        assert_eq!(
            format_dec(Dec::new(-4, 3), &style, MAX_DISPLAY_DECIMALS),
            "-0.004"
        );
        // Below 1e-8 it really is zero: no stray minus sign.
        assert_eq!(
            format_dec(Dec::new(-1, 10), &style, MAX_DISPLAY_DECIMALS),
            "0.00"
        );
    }

    #[test]
    fn indian_grouping_repeats_the_last_size() {
        let groups = DigitGroups {
            mark: ',',
            sizes: vec![3, 2],
        };
        assert_eq!(group_digits("123456789", &groups), "12,34,56,789");
    }

    #[test]
    fn right_side_spaced_with_comma_mark() {
        let style = AmountStyle {
            side: CommoditySide::Right,
            spaced: true,
            decimal_mark: Some(','),
            digit_groups: Some(Arc::new(DigitGroups {
                mark: '.',
                sizes: vec![3],
            })),
            precision: 2,
        };
        assert_eq!(
            format_amount("EUR", Dec::new(123_456, 2), &style, MAX_DISPLAY_DECIMALS),
            "1.234,56 EUR"
        );
        assert_eq!(
            format_amount("", Dec::new(123_456, 2), &style, MAX_DISPLAY_DECIMALS),
            "1.234,56"
        );
    }

    #[test]
    fn unrepresentable_rescale_renders_at_the_values_own_scale() {
        let style = AmountStyle {
            precision: 2,
            ..usd()
        };
        let huge = Dec::new(i128::MAX, 0);
        let text = format_dec(huge, &style, MAX_DISPLAY_DECIMALS);
        assert!(
            text.ends_with("727"),
            "rendered without inventing digits: {text}"
        );
    }
}
