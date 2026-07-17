//! `Dec` — an exact base-10 decimal used throughout the engine.
//!
//! A `Dec` is `mantissa / 10^places`. This mirrors the TypeScript `Dec = {m:
//! bigint; p: number}` in `web/src/lib/domain/money.ts` and, crucially, the
//! semantics of the Haskell `Data.Decimal` used by hledger:
//!
//! - addition/subtraction align to the larger scale and do **not** normalize
//!   (trailing zeros are kept),
//! - multiplication adds the scales and then **normalizes** (strips trailing
//!   zeros down to — but not below — zero decimal places).
//!
//! Matching those rules exactly is what lets our inferred balancing amounts
//! reproduce hledger's `decimalMantissa`/`decimalPlaces` byte-for-byte.

use std::cmp::Ordering;
use thiserror::Error;

/// Errors produced by exact-decimal parsing/arithmetic.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum DecError {
    /// A checked arithmetic operation overflowed `i128`.
    #[error("decimal arithmetic overflow")]
    Overflow,
    /// A numeric literal could not be parsed.
    #[error("invalid numeric literal: '{0}'")]
    InvalidNumber(String),
}

/// An exact decimal value: `mantissa / 10^places`.
///
/// Equality and ordering are by numeric **value** (so `1.50` equals `1.5`),
/// while `mantissa`/`places` are preserved verbatim for wire serialization.
#[derive(Debug, Clone, Copy)]
pub struct Dec {
    /// Signed significand.
    pub mantissa: i128,
    /// Number of base-10 fractional digits.
    pub places: u32,
}

// `add`/`sub`/`mul`/`neg` deliberately return `Result` (all arithmetic is
// overflow-checked), so they cannot implement the infallible `std::ops` traits;
// the conventional names are kept to match the TS engine and the task contract.
#[allow(clippy::should_implement_trait)]
impl Dec {
    /// Build directly from a mantissa and a scale. Never from a float.
    #[must_use]
    pub const fn new(mantissa: i128, places: u32) -> Self {
        Self { mantissa, places }
    }

    /// The value zero at scale 0.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            mantissa: 0,
            places: 0,
        }
    }

    /// True when the value is exactly zero (regardless of scale).
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.mantissa == 0
    }

    /// Negate the value (checked; only `i128::MIN` overflows).
    #[must_use = "returns the negated value"]
    pub fn neg(self) -> Result<Self, DecError> {
        self.mantissa
            .checked_neg()
            .map(|m| Self::new(m, self.places))
            .ok_or(DecError::Overflow)
    }

    /// Absolute value (checked).
    pub fn abs(self) -> Result<Self, DecError> {
        if self.mantissa < 0 {
            self.neg()
        } else {
            Ok(self)
        }
    }

    /// Rescale to exactly `target` fractional places by padding with zeros.
    ///
    /// `target` must be `>=` the current scale; shrinking would lose precision
    /// and is rejected.
    fn rescaled(self, target: u32) -> Result<Self, DecError> {
        if target < self.places {
            return Err(DecError::Overflow);
        }
        let factor = pow10(target - self.places)?;
        let mantissa = self
            .mantissa
            .checked_mul(factor)
            .ok_or(DecError::Overflow)?;
        Ok(Self::new(mantissa, target))
    }

    /// Exact addition. Result scale is `max(self.places, other.places)`; no
    /// normalization (matching `Data.Decimal`).
    pub fn add(self, other: Self) -> Result<Self, DecError> {
        let places = self.places.max(other.places);
        let a = self.rescaled(places)?;
        let b = other.rescaled(places)?;
        let mantissa = a
            .mantissa
            .checked_add(b.mantissa)
            .ok_or(DecError::Overflow)?;
        Ok(Self::new(mantissa, places))
    }

    /// Exact subtraction (see [`Dec::add`]).
    pub fn sub(self, other: Self) -> Result<Self, DecError> {
        self.add(other.neg()?)
    }

    /// Exact multiplication. Scales add, then the result is normalized (trailing
    /// zeros stripped), matching `Data.Decimal`'s `normalizeDecimal`.
    pub fn mul(self, other: Self) -> Result<Self, DecError> {
        let mantissa = self
            .mantissa
            .checked_mul(other.mantissa)
            .ok_or(DecError::Overflow)?;
        let places = self
            .places
            .checked_add(other.places)
            .ok_or(DecError::Overflow)?;
        Ok(Self::new(mantissa, places).normalized())
    }

    /// Strip trailing decimal zeros down to (but not below) scale 0. Zero
    /// normalizes to scale 0.
    #[must_use]
    pub fn normalized(self) -> Self {
        let mut mantissa = self.mantissa;
        let mut places = self.places;
        while places > 0 && mantissa % 10 == 0 {
            mantissa /= 10;
            places -= 1;
        }
        Self::new(mantissa, places)
    }

    /// Display-only conversion to `f64` (`mantissa / 10^places`). Never used for
    /// arithmetic or equality.
    #[must_use]
    pub fn floating_point(&self) -> f64 {
        (self.mantissa as f64) / 10f64.powi(self.places as i32)
    }

    /// Parse a numeric literal using `decimal_mark` as the decimal separator.
    ///
    /// Any other of `.`/`,`/`_`/space is treated as a digit-group separator and
    /// discarded. The resulting `places` equals the count of written fractional
    /// digits (so `"5.00"` yields scale 2, not the normalized scale 0).
    pub fn parse(input: &str, decimal_mark: char) -> Result<Self, DecError> {
        let trimmed = input.trim();
        let (negative, body) = match trimmed.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        };
        let body = body.trim();
        if body.is_empty() {
            return Err(DecError::InvalidNumber(input.to_string()));
        }
        let is_allowed =
            |c: char| c.is_ascii_digit() || c == decimal_mark || matches!(c, '.' | ',' | ' ' | '_');
        if !body.chars().all(is_allowed) {
            return Err(DecError::InvalidNumber(input.to_string()));
        }

        let (int_src, frac_src) = match body.rfind(decimal_mark) {
            Some(pos) => (&body[..pos], &body[pos + decimal_mark.len_utf8()..]),
            None => (body, ""),
        };
        let int_digits: String = int_src.chars().filter(char::is_ascii_digit).collect();
        let frac_digits: String = frac_src.chars().filter(char::is_ascii_digit).collect();
        let places = u32::try_from(frac_digits.len())
            .map_err(|_| DecError::InvalidNumber(input.to_string()))?;

        let combined = format!("{int_digits}{frac_digits}");
        if combined.is_empty() {
            return Err(DecError::InvalidNumber(input.to_string()));
        }
        let magnitude: i128 = combined
            .parse()
            .map_err(|_| DecError::InvalidNumber(input.to_string()))?;
        let mantissa = if negative {
            magnitude.checked_neg().ok_or(DecError::Overflow)?
        } else {
            magnitude
        };
        // hledger reads at most `MAX_PARSE_PLACES` fractional digits, rounding the
        // remainder half-to-even; match that so parsed prices/amounts agree
        // byte-for-byte (e.g. a 13-place price stores 10 places).
        Ok(Self::new(mantissa, places).rounded_half_even(MAX_PARSE_PLACES))
    }

    /// Round to `target` fractional places using round-half-to-even (banker's
    /// rounding), matching `Data.Decimal`. Returns `self` unchanged when
    /// `target >= places`.
    #[must_use]
    fn rounded_half_even(self, target: u32) -> Self {
        if target >= self.places {
            return self;
        }
        let drop = self.places - target;
        let Ok(divisor) = pow10(drop) else {
            return self;
        };
        let quotient = self.mantissa / divisor;
        let remainder = (self.mantissa % divisor).abs();
        let half = divisor / 2; // exact: `divisor` is a power of ten >= 10
        let round_away = remainder > half || (remainder == half && quotient % 2 != 0);
        let adjusted = if round_away {
            if self.mantissa >= 0 {
                quotient + 1
            } else {
                quotient - 1
            }
        } else {
            quotient
        };
        Self::new(adjusted, target)
    }
}

/// hledger reads numbers with at most this many fractional digits (rounding the
/// remainder half-to-even), so parsing caps to match its stored precision.
const MAX_PARSE_PLACES: u32 = 10;

/// `10^exp` as an `i128`, checked for overflow.
fn pow10(exp: u32) -> Result<i128, DecError> {
    10i128.checked_pow(exp).ok_or(DecError::Overflow)
}

impl PartialEq for Dec {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Dec {}

impl PartialOrd for Dec {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Dec {
    fn cmp(&self, other: &Self) -> Ordering {
        let places = self.places.max(other.places);
        match (self.rescaled(places), other.rescaled(places)) {
            (Ok(a), Ok(b)) => a.mantissa.cmp(&b.mantissa),
            // Only reachable for astronomically large values; fall back to a
            // finite float comparison so ordering stays total and panic-free.
            _ => self
                .floating_point()
                .partial_cmp(&other.floating_point())
                .unwrap_or(Ordering::Equal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grouped_dollar() {
        let d = Dec::parse("5,000.00", '.').unwrap();
        assert_eq!(d, Dec::new(500000, 2));
        assert_eq!(d.places, 2);
    }

    #[test]
    fn parses_negative_dollar() {
        assert_eq!(Dec::parse("-450.00", '.').unwrap(), Dec::new(-45000, 2));
    }

    #[test]
    fn parses_comma_decimal_eur() {
        // "1.000,00" with comma decimal mark -> 1000.00, scale 2.
        assert_eq!(Dec::parse("1.000,00", ',').unwrap(), Dec::new(100000, 2));
        // "645,00" -> 645.00, scale 2.
        assert_eq!(Dec::parse("645,00", ',').unwrap(), Dec::new(64500, 2));
    }

    #[test]
    fn parses_integer_and_fraction() {
        assert_eq!(Dec::parse("10", '.').unwrap(), Dec::new(10, 0));
        assert_eq!(Dec::parse("4.5", '.').unwrap(), Dec::new(45, 1));
        assert_eq!(Dec::parse("0.005", '.').unwrap(), Dec::new(5, 3));
        assert_eq!(Dec::parse("1.0850", '.').unwrap(), Dec::new(10850, 4));
    }

    #[test]
    fn rejects_garbage() {
        assert!(Dec::parse("abc", '.').is_err());
        assert!(Dec::parse("", '.').is_err());
        assert!(Dec::parse("$5", '.').is_err());
    }

    #[test]
    fn add_keeps_max_scale_without_normalizing() {
        // 5000.00 + 10000.00 + (-450.00) = 14550.00, scale 2 (not normalized).
        let sum = Dec::new(500000, 2)
            .add(Dec::new(1000000, 2))
            .unwrap()
            .add(Dec::new(-45000, 2))
            .unwrap();
        assert_eq!(sum.mantissa, 1455000);
        assert_eq!(sum.places, 2);
    }

    #[test]
    fn add_aligns_differing_scales() {
        let sum = Dec::new(5, 0).add(Dec::new(25, 2)).unwrap();
        assert_eq!(sum, Dec::new(525, 2));
        assert_eq!(sum.places, 2);
    }

    #[test]
    fn mul_normalizes_trailing_zeros() {
        // 10 * 220.00 = 2200 at scale 0 (matches hledger's normalizeDecimal).
        let product = Dec::new(10, 0).mul(Dec::new(22000, 2)).unwrap();
        assert_eq!(product, Dec::new(2200, 0));
        assert_eq!(product.places, 0);

        // 1000.00 * 1.0850 = 1085 at scale 0.
        let p2 = Dec::new(100000, 2).mul(Dec::new(10850, 4)).unwrap();
        assert_eq!(p2, Dec::new(1085, 0));
        assert_eq!(p2.places, 0);

        // -12 * 205.60 = -2467.2 at scale 1.
        let p3 = Dec::new(-12, 0).mul(Dec::new(20560, 2)).unwrap();
        assert_eq!(p3, Dec::new(-24672, 1));
        assert_eq!(p3.places, 1);
    }

    #[test]
    fn neg_and_abs() {
        assert_eq!(Dec::new(-45000, 2).neg().unwrap(), Dec::new(45000, 2));
        assert_eq!(Dec::new(-45000, 2).abs().unwrap(), Dec::new(45000, 2));
        assert_eq!(Dec::new(45000, 2).abs().unwrap(), Dec::new(45000, 2));
    }

    #[test]
    fn zero_and_ordering() {
        assert!(Dec::zero().is_zero());
        assert!(Dec::new(0, 5).is_zero());
        assert_eq!(Dec::new(0, 5).normalized(), Dec::new(0, 0));
        assert!(Dec::new(500, 2) < Dec::new(6, 0));
        assert_eq!(Dec::new(50, 1), Dec::new(500, 2));
        assert!(Dec::new(-1, 0) < Dec::new(0, 0));
    }

    #[test]
    fn floating_point_is_display_only() {
        assert!((Dec::new(10850, 4).floating_point() - 1.085).abs() < 1e-12);
        assert!((Dec::new(-1455000, 2).floating_point() - (-14550.0)).abs() < 1e-9);
    }

    #[test]
    fn parse_caps_at_ten_places_half_even() {
        // > 10 places: dropped digits round half-to-even (verified against
        // hledger 1.52's stored market-price precision).
        assert_eq!(
            Dec::parse("289.3599853515625", '.').unwrap(),
            Dec::new(2_893_599_853_516, 10) // ...5625 rounds up
        );
        assert_eq!(
            Dec::parse("1.1234567890123", '.').unwrap(),
            Dec::new(11_234_567_890, 10) // ...123 rounds down
        );
        // Exact-half ties round to even.
        assert_eq!(
            Dec::parse("1.00000000005", '.').unwrap(),
            Dec::new(10_000_000_000, 10) // 10th digit 0 (even) stays
        );
        assert_eq!(
            Dec::parse("1.00000000015", '.').unwrap(),
            Dec::new(10_000_000_002, 10) // 10th digit 1 (odd) -> 2
        );
        assert_eq!(
            Dec::parse("-1.00000000015", '.').unwrap(),
            Dec::new(-10_000_000_002, 10) // symmetric for negatives
        );
        // <= 10 places is left exactly as written.
        assert_eq!(
            Dec::parse("1.123456789", '.').unwrap(),
            Dec::new(1_123_456_789, 9)
        );
        assert_eq!(Dec::parse("5.00", '.').unwrap(), Dec::new(500, 2));
    }
}
