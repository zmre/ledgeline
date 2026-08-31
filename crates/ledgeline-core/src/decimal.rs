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
    /// [`Dec::div_int`] was asked to divide by a count of zero — the mean of no
    /// values, which is not a number rather than an infinite one.
    #[error("cannot divide by a count of zero")]
    DivideByZero,
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

    /// Divide by a positive whole count, keeping this value's scale and rounding
    /// half away from zero.
    ///
    /// Deliberately NOT general division. Money divided by money is a ratio and
    /// belongs in floating point at a chart boundary; money divided by a *count*
    /// is money, and this is the one place the engine needs it — the mean of a
    /// few periods' actuals, which the budget editor shows beside the amount box.
    ///
    /// # Why the scale does not grow
    /// The result is another figure in the same commodity, shown in the same
    /// column as the periods it averages. Widening the scale to make the division
    /// exact would print `$553.3333333333` under three amounts written to the
    /// cent, which is precision the inputs never had.
    ///
    /// # Why half away from zero
    /// It is what `Data.Decimal`'s `roundTo` does, and what a reader expects of a
    /// displayed average: `$100.335` over three periods reads as `$100.34`, not
    /// as `$100.33`. Banker's rounding is for repeated accumulation, and nothing
    /// accumulates this.
    ///
    /// # Errors
    /// [`DecError::DivideByZero`] when `count` is zero; the arithmetic itself
    /// cannot overflow, since `|mantissa / count| <= |mantissa|`.
    pub fn div_int(self, count: u32) -> Result<Self, DecError> {
        let divisor = i128::from(count);
        if divisor == 0 {
            return Err(DecError::DivideByZero);
        }
        let quotient = self.mantissa / divisor;
        let remainder = self.mantissa % divisor;
        // `remainder` carries the sign of `mantissa`, so the correction is in the
        // same direction as the value — which is what "away from zero" means.
        let rounded = if remainder.unsigned_abs() * 2 >= divisor.unsigned_abs() {
            quotient + self.mantissa.signum()
        } else {
            quotient
        };
        Ok(Self::new(rounded, self.places))
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

    /// Display-boundary conversion to `f64` (`mantissa / 10^places`), lossy
    /// above 2^53.
    ///
    /// Used for exactly two things: the wire's convenience number field
    /// (`wire.rs`), and the percent-change arithmetic the reports publish as
    /// `f64` (`insights::pct_change`, `holdings::engine::gain_pct`). A percent is
    /// already a lossy summary and is never added back into a balance, so that
    /// arithmetic is contained.
    ///
    /// What it must NOT do is decide anything: no equality, no ordering, no
    /// threshold, no ranking that truncates. Every such site is exact `Dec`
    /// ([`Ord`] included). This doc previously read "Never used for arithmetic or
    /// equality" — false in both halves at the time, and a doc that lies about an
    /// invariant is how wrong tests get written (DRY-5).
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
        Self::parse_with_mark(input, Some(decimal_mark))
    }

    /// Parse a numeric literal, where `decimal_mark` may be `None`.
    ///
    /// `None` means the literal has **no** decimal mark, so every `.`/`,` in it
    /// is a digit-group separator and the result is a whole number: hledger
    /// reads `1.2.3` as `123` and `1.234.567` as `1234567`, because a repeated
    /// separator cannot be a decimal point. Passing `Some(mark)` behaves exactly
    /// like [`Dec::parse`].
    pub fn parse_with_mark(input: &str, decimal_mark: Option<char>) -> Result<Self, DecError> {
        let trimmed = input.trim();
        // Scientific notation: split off an `e`/`E` exponent (optionally signed).
        // hledger evaluates `1.05e2` to 105 and `31415926e-7` to 3.1415926.
        let (mantissa_input, exponent) = match trimmed.find(['e', 'E']) {
            Some(pos) => {
                let exponent = trimmed[pos + 1..]
                    .parse::<i32>()
                    .map_err(|_| DecError::InvalidNumber(input.to_string()))?;
                (&trimmed[..pos], exponent)
            }
            None => (trimmed, 0),
        };
        let (negative, body) = match mantissa_input.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (
                false,
                mantissa_input.strip_prefix('+').unwrap_or(mantissa_input),
            ),
        };
        let body = body.trim();
        if body.is_empty() {
            return Err(DecError::InvalidNumber(input.to_string()));
        }
        // U+00A0 NO-BREAK SPACE joins the plain space as a digit-group
        // separator: hledger accepts `1\u{a0}000.00` exactly as it accepts
        // `1 000.00`.
        let is_allowed = |c: char| {
            c.is_ascii_digit()
                || decimal_mark == Some(c)
                || matches!(c, '.' | ',' | ' ' | '_' | '\u{a0}')
        };
        if !body.chars().all(is_allowed) {
            return Err(DecError::InvalidNumber(input.to_string()));
        }

        // With no decimal mark every separator is a digit group, so the whole
        // body is the integer part.
        let split = decimal_mark.and_then(|mark| body.rfind(mark).map(|pos| (mark, pos)));
        let (int_src, frac_src) = match split {
            Some((mark, pos)) => (&body[..pos], &body[pos + mark.len_utf8()..]),
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
        // `combined` is non-empty and all ASCII digits by construction (both
        // halves were filtered with `is_ascii_digit`), so the only way this can
        // fail is magnitude. hledger's mantissa is arbitrary-precision and ours
        // is `i128`, so report the real reason rather than the misleading
        // "invalid numeric literal".
        let magnitude: i128 = combined.parse().map_err(|_| DecError::Overflow)?;
        let mantissa = if negative {
            magnitude.checked_neg().ok_or(DecError::Overflow)?
        } else {
            magnitude
        };
        // hledger reads at most `MAX_PARSE_PLACES` fractional digits, rounding the
        // remainder half-to-even; match that so parsed prices/amounts agree
        // byte-for-byte (e.g. a 13-place price stores 10 places).
        let base = Self::new(mantissa, places);
        Ok(apply_exponent(base, exponent)?.rounded_half_even(MAX_PARSE_PLACES))
    }

    /// Round to `target` fractional places using round-half-to-even (banker's
    /// rounding), matching `Data.Decimal`. Returns `self` unchanged when
    /// `target >= places`, and otherwise **always** returns a value at scale
    /// `target` — see the closed-cap argument below.
    #[must_use]
    fn rounded_half_even(self, target: u32) -> Self {
        if target >= self.places {
            return self;
        }
        let drop = self.places - target;
        let Ok(divisor) = pow10(drop) else {
            // `pow10` has no answer from `drop == 39` up, and this arm used to
            // `return self` — so the scales the cap exists to refuse were exactly
            // the ones that skipped it. `1e-2147483648` parsed to
            // `places = 2_147_483_648`, which rode out as `style.precision` on the
            // wire and as a `"0".repeat(places)` allocation in every renderer
            // downstream. The cap has to fail CLOSED.
            //
            // Zero at `target` is not a fallback here, it is the exact answer.
            // The mantissa is an `i128`, so `|mantissa| <= 2^127 < 5 × 10^38`,
            // i.e. strictly under `10^39 / 2 <= 10^drop / 2`. Dividing by
            // `10^places` gives `|self| < 10^-target / 2`: strictly less than half
            // a unit in the last place we are keeping. Half-even rounds that to
            // zero, which is precisely what the arithmetic below would compute if
            // `i128` could hold `10^drop`.
            //
            // So no magnitude is being discarded. A bounded mantissa spread over a
            // scale this large IS a vanishing value; the problem was never a large
            // number wearing the wrong scale, because `i128` cannot hold one.
            return Self::new(0, target);
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
///
/// Public so the edit wire can reject a `places` the parser could never have
/// produced, instead of letting it through to be rendered and then bounced by
/// the round-trip guard with a misleading message (SEC-5).
pub const MAX_PARSE_PLACES: u32 = 10;

/// The widest fractional precision any renderer of a [`Dec`] will lay out.
///
/// Every exact renderer in the engine pads the integer side with
/// `"0".repeat(places)`, so an unclamped `places` turns a handful of input bytes
/// into a proportional allocation — `1e-2147483648` asks for 2.1 GB. Clamping is
/// what makes those renderers total.
///
/// Nothing the engine itself produces comes close: [`Dec::parse`] caps at
/// [`MAX_PARSE_PLACES`] and [`Dec::mul`] at most sums its operands' scales. Only a
/// [`Dec`] built directly from unvalidated input — a wire payload, a bank
/// statement's `BALAMT` — can exceed it. 255 is hledger's own maximum displayed
/// precision, so the clamp cannot truncate a value hledger could have written.
///
/// Lives here, beside the type it bounds, because three renderers need it
/// (`edit::render_dec`, `assertions::render_dec`, `convert::ofx::render`) and two
/// of them had grown their own copy of the number while the third had none.
pub const MAX_RENDER_PLACES: u32 = 255;

/// `10^exp` as an `i128`, checked for overflow.
fn pow10(exp: u32) -> Result<i128, DecError> {
    10i128.checked_pow(exp).ok_or(DecError::Overflow)
}

/// Apply a base-10 `exponent` (scientific notation) to a parsed decimal: the
/// value becomes `mantissa × 10^exponent`. When the exponent exceeds the
/// fractional places the scale drops to zero and the mantissa is scaled up.
fn apply_exponent(dec: Dec, exponent: i32) -> Result<Dec, DecError> {
    if exponent == 0 {
        return Ok(dec);
    }
    let net = i64::from(dec.places) - i64::from(exponent);
    if net >= 0 {
        let places = u32::try_from(net).map_err(|_| DecError::Overflow)?;
        Ok(Dec::new(dec.mantissa, places))
    } else {
        let shift = u32::try_from(-net).map_err(|_| DecError::Overflow)?;
        let factor = pow10(shift)?;
        let mantissa = dec.mantissa.checked_mul(factor).ok_or(DecError::Overflow)?;
        Ok(Dec::new(mantissa, 0))
    }
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

/// Number of decimal digits in `value`; zero has none.
///
/// `x × 10^exp` has exactly `digits(x) + exp` of them, which is what lets
/// [`cmp_scaled`] settle a comparison without performing the multiplication.
fn digits(value: u128) -> u32 {
    value.checked_ilog10().map_or(0, |log| log + 1)
}

/// Compare `x × 10^exp` against `y` exactly, for magnitudes whose scaled form
/// may not fit in a `u128`.
///
/// The decimal digit count decides every comparison but one: a number with more
/// digits is the larger, and scaling by a power of ten shifts that count by
/// exactly `exp`. When the counts agree the product has as many digits as `y`,
/// so either it fits in a `u128` and is compared outright, or it exceeds
/// `u128::MAX` — and `y` does not — making it the larger. Nothing here can
/// overflow, so there is no inexact path to fall back to.
fn cmp_scaled(x: u128, exp: u32, y: u128) -> Ordering {
    if x == 0 || y == 0 {
        // `x × 10^exp` is zero exactly when `x` is, so the raw compare is right.
        return x.cmp(&y);
    }
    match (u64::from(digits(x)) + u64::from(exp)).cmp(&u64::from(digits(y))) {
        Ordering::Equal => match 10u128
            .checked_pow(exp)
            .and_then(|factor| x.checked_mul(factor))
        {
            Some(scaled) => scaled.cmp(&y),
            None => Ordering::Greater,
        },
        counts => counts,
    }
}

/// Total, **exact** ordering by numeric value — no float path at any magnitude.
///
/// This used to rescale both sides to the larger scale and, when that overflowed
/// `i128`, compare `f64`s with `unwrap_or(Ordering::Equal)`. Two values 0.3
/// apart then reported `Equal`, and because [`PartialEq`] is defined in terms of
/// this, every `BTreeMap<_, Dec>` key, every `sort`, and every `dedup` in the
/// engine inherited it (DRY-5). Comparing the sign first and then the magnitudes
/// removes the multiplication that overflowed, so the fallback has nothing left
/// to be a fallback for.
impl Ord for Dec {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.mantissa.signum().cmp(&other.mantissa.signum()) {
            Ordering::Equal => {}
            sign => return sign,
        }
        // Same sign (or both zero). `unsigned_abs` is total where `neg` is not:
        // it handles `i128::MIN`, which has no positive counterpart.
        let (x, y) = (self.mantissa.unsigned_abs(), other.mantissa.unsigned_abs());
        // Comparing `x / 10^p` against `y / 10^q` is comparing `x × 10^q`
        // against `y × 10^p`; dividing through by the common `10^min(p, q)`
        // leaves one scaling, on whichever side carries the smaller scale.
        let magnitude = if self.places <= other.places {
            cmp_scaled(x, other.places - self.places, y)
        } else {
            cmp_scaled(y, self.places - other.places, x).reverse()
        };
        // Below zero, the larger magnitude is the smaller value.
        if self.mantissa < 0 {
            magnitude.reverse()
        } else {
            magnitude
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Stripping trailing zeros is a CANONICAL form: for a value `m / 10^p`, the
    /// representation with either `p == 0` or `10 ∤ m` is unique. (If
    /// `m₁/10^p₁ = m₂/10^p₂` with both canonical and `p₁ < p₂`, then
    /// `m₂ = m₁ × 10^(p₂−p₁)` is divisible by ten, contradicting `p₂`'s
    /// canonicity.) So two `Dec`s are numerically equal exactly when their
    /// normalized forms match field-for-field — an equality oracle that never
    /// rescales and therefore cannot overflow, which is what makes it a valid
    /// check on the very inputs the old `Ord` got wrong.
    fn same_value(a: Dec, b: Dec) -> bool {
        let (a, b) = (a.normalized(), b.normalized());
        a.mantissa == b.mantissa && a.places == b.places
    }

    proptest! {
        /// `Ord` is a total order that agrees with numeric value at EVERY
        /// magnitude — including the range where aligning the two scales
        /// overflows `i128`, which is precisely where it used to compare `f64`s
        /// and collapse distinct values to `Equal`.
        #[test]
        fn ord_is_exact_and_antisymmetric_at_any_magnitude(
            ma in any::<i128>(),
            pa in 0u32..80,
            mb in any::<i128>(),
            pb in 0u32..80,
        ) {
            let (a, b) = (Dec::new(ma, pa), Dec::new(mb, pb));

            prop_assert_eq!(a.cmp(&b), b.cmp(&a).reverse());
            prop_assert_eq!(a.cmp(&a), Ordering::Equal);
            // The heart of it: `Equal` must mean equal, never "too large to tell".
            prop_assert_eq!(a == b, same_value(a, b));
        }

        /// Transitivity, over triples spanning the same overflow-prone range.
        /// A comparator that reports unequal values as `Equal` breaks this, and
        /// `BTreeMap` silently corrupts when its key comparator is not a total
        /// order.
        #[test]
        fn ord_is_transitive(
            ma in any::<i128>(), pa in 0u32..80,
            mb in any::<i128>(), pb in 0u32..80,
            mc in any::<i128>(), pc in 0u32..80,
        ) {
            let (a, b, c) = (Dec::new(ma, pa), Dec::new(mb, pb), Dec::new(mc, pc));
            let mut sorted = [a, b, c];
            sorted.sort();
            prop_assert!(sorted[0] <= sorted[1]);
            prop_assert!(sorted[1] <= sorted[2]);
            prop_assert!(sorted[0] <= sorted[2]);
        }

        /// The same exactness check, but with the generator aimed squarely at
        /// the overflow zone.
        ///
        /// This one earns its keep where the uniform version above does not:
        /// mantissas within a factor of ten of `i128::MAX` make aligning two
        /// different scales overflow every time, so every case here takes the
        /// path the old implementation resolved with `f64`. Uniform `i128`s
        /// essentially never land there — verified by reverting `Ord` and
        /// watching the uniform properties still pass while these fail.
        #[test]
        fn ord_is_exact_where_scales_cannot_be_aligned(
            ma in (i128::MAX / 10)..=i128::MAX,
            pa in 0u32..4,
            mb in (i128::MAX / 10)..=i128::MAX,
            pb in 0u32..4,
        ) {
            let (a, b) = (Dec::new(ma, pa), Dec::new(mb, pb));
            prop_assert_eq!(a == b, same_value(a, b));
            prop_assert_eq!(a.cmp(&b), b.cmp(&a).reverse());
            // Negatives take the mirrored branch.
            let (na, nb) = (Dec::new(-ma, pa), Dec::new(-mb, pb));
            prop_assert_eq!(na == nb, same_value(na, nb));
            prop_assert_eq!(na.cmp(&nb), a.cmp(&b).reverse());
        }

        /// Ordering agrees with subtraction wherever subtraction is defined —
        /// an independent cross-check against the exact arithmetic the engine
        /// actually runs on money.
        #[test]
        fn ord_agrees_with_subtraction(
            ma in -(10i128.pow(30))..10i128.pow(30),
            pa in 0u32..12,
            mb in -(10i128.pow(30))..10i128.pow(30),
            pb in 0u32..12,
        ) {
            let (a, b) = (Dec::new(ma, pa), Dec::new(mb, pb));
            if let Ok(difference) = a.sub(b) {
                prop_assert_eq!(a.cmp(&b), difference.cmp(&Dec::zero()));
            }
        }
    }

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

    #[test]
    fn the_parse_scale_cap_fails_closed_past_what_pow10_can_build() {
        // `pow10` has no answer from `drop == 39` up, and the cap used to
        // `return self` there — so the scales it exists to refuse were exactly
        // the ones that skipped it. The resulting `places` became
        // `style.precision` on the wire and a `"0".repeat(places)` allocation in
        // every renderer downstream.
        let poisoned = Dec::parse("1e-2147483648", '.').expect("a well-formed literal");
        assert_eq!(poisoned.places, MAX_PARSE_PLACES);
        assert_eq!(poisoned.mantissa, 0);

        // The same thing spelled without an exponent: 49 written fractional
        // digits is the first scale whose drop to ten places exceeds 38.
        let literal = format!("0.{}1", "0".repeat(48));
        let boundary = Dec::parse(&literal, '.').expect("49 fractional digits is well-formed");
        assert_eq!((boundary.mantissa, boundary.places), (0, MAX_PARSE_PLACES));
    }

    #[test]
    fn the_closed_scale_cap_is_the_arithmetically_correct_answer() {
        // Zero at the target is not a compromise; it is what half-even rounding
        // computes. The premise is the mantissa's own bound: an `i128` cannot
        // reach half of 10^39, so with `drop >= 39` the value is strictly under
        // half a unit in the last kept place, whatever the mantissa.
        //
        // Checked as `|m| / 10^38 < 5` rather than `|m| < 5 × 10^38` because
        // 5 × 10^38 does not fit in a `u128` either. The two are the same claim:
        // 5 × 10^38 is a multiple of 10^38, so the floor division is exact here.
        assert!(i128::MAX.unsigned_abs() / 10u128.pow(38) < 5);
        assert!(i128::MIN.unsigned_abs() / 10u128.pow(38) < 5);
        for mantissa in [i128::MAX, i128::MIN, 1, -1] {
            let capped = Dec::new(mantissa, 49).rounded_half_even(10);
            assert_eq!((capped.mantissa, capped.places), (0, 10), "{mantissa}");
        }

        // One place shallower the divisor still fits, so the ordinary path runs
        // — and it does NOT collapse to zero. That is what shows the closed arm
        // draws the line where the arithmetic does, rather than early: it takes
        // over exactly when the correct answer has become zero anyway.
        let widest = Dec::new(i128::MAX, 48).rounded_half_even(10);
        assert_eq!((widest.mantissa, widest.places), (2, 10));
    }

    proptest! {
        /// The cap is TOTAL: past the target, the result is AT the target for
        /// every scale a `u32` can express. This is the property the old
        /// `return self` broke, and it breaks it for ~99.9999% of the `places`
        /// range — every value from 49 up — so a uniform generator finds it
        /// immediately.
        #[test]
        fn rounding_never_returns_a_scale_wider_than_its_target(
            mantissa: i128,
            places: u32,
            target in 0u32..=MAX_PARSE_PLACES,
        ) {
            let rounded = Dec::new(mantissa, places).rounded_half_even(target);
            prop_assert_eq!(rounded.places, places.min(target));
        }

        /// And nothing that reaches the closed arm was a value worth keeping:
        /// its magnitude is under half a unit in the last kept place, so the
        /// zero it becomes is the correctly rounded value and not a lost one.
        #[test]
        fn a_capped_value_was_already_smaller_than_half_an_ulp(
            mantissa: i128,
            extra in 39u32..=200,
            target in 0u32..=MAX_PARSE_PLACES,
        ) {
            let value = Dec::new(mantissa, target + extra);
            prop_assert_eq!(value.rounded_half_even(target).mantissa, 0);
            // Zero because the value really was under half a unit in the last
            // kept place, not because the cap gave up. `|m| < 10^39 / 2` holds
            // for EVERY `i128`, and a larger `extra` only shrinks the value
            // further, so the tightest case is the one checked. Spelled as a
            // division because neither 10^39 nor 5 x 10^38 fits in a `u128`.
            prop_assert!(mantissa.unsigned_abs() / 10u128.pow(38) < 5);
        }
    }

    #[test]
    fn parse_with_no_decimal_mark_treats_every_separator_as_a_group() {
        // PARSE-3: a repeated separator cannot be a decimal point, so hledger
        // reads the whole literal as a whole number.
        assert_eq!(
            Dec::parse_with_mark("1.2.3", None).unwrap(),
            Dec::new(123, 0)
        );
        assert_eq!(
            Dec::parse_with_mark("1.234.567", None).unwrap(),
            Dec::new(1_234_567, 0)
        );
        assert_eq!(
            Dec::parse_with_mark("1,234,567", None).unwrap(),
            Dec::new(1_234_567, 0)
        );
        assert_eq!(
            Dec::parse_with_mark("-1.2.3", None).unwrap(),
            Dec::new(-123, 0)
        );
        // `Some(mark)` is unchanged from `parse`.
        assert_eq!(
            Dec::parse_with_mark("1,50", Some(',')).unwrap(),
            Dec::new(150, 2)
        );
    }

    #[test]
    fn parses_space_and_nbsp_digit_groups() {
        // PARSE-4: hledger accepts both the plain space and U+00A0 as digit
        // group separators (`1 000.00`, `1\u{a0}000.00`).
        assert_eq!(Dec::parse("1 000.00", '.').unwrap(), Dec::new(100_000, 2));
        assert_eq!(
            Dec::parse("1\u{a0}000.00", '.').unwrap(),
            Dec::new(100_000, 2)
        );
        assert_eq!(
            Dec::parse_with_mark("1 000 000", None).unwrap(),
            Dec::new(1_000_000, 0)
        );
    }

    /// The two values the old float fallback collapsed together.
    ///
    /// `HUGE` is `i128::MAX / 10 + 1` at scale 0; `i128::MAX` at scale 1 is
    /// `0.3` less. Aligning them to a common scale multiplies `HUGE` by ten,
    /// which overflows `i128` — the only door into the old fallback.
    const HUGE: i128 = i128::MAX / 10 + 1;

    #[test]
    fn ord_is_exact_where_rescaling_overflows() {
        let bigger = Dec::new(HUGE, 0);
        let smaller = Dec::new(i128::MAX, 1);

        // The premises. Rescaling to the common scale really does overflow...
        assert!(HUGE.checked_mul(10).is_none());
        // ...and both values really do share one `f64` image, so the old
        // `partial_cmp(...).unwrap_or(Equal)` fallback returned `Equal` for two
        // values a full 0.3 apart.
        assert_eq!(bigger.floating_point(), smaller.floating_point());

        // Exact ordering, in both directions and via every derived operator.
        assert_eq!(bigger.cmp(&smaller), Ordering::Greater);
        assert_eq!(smaller.cmp(&bigger), Ordering::Less);
        assert!(bigger > smaller);
        assert_ne!(bigger, smaller);

        // `PartialEq` is `cmp == Equal`, so the collapse used to make these two
        // distinct values collide as `BTreeMap` keys and vanish under `dedup`.
        let mut map = std::collections::BTreeMap::new();
        map.insert(bigger, "bigger");
        map.insert(smaller, "smaller");
        assert_eq!(map.len(), 2, "two different values must be two keys");
        let mut values = vec![bigger, smaller];
        values.dedup();
        assert_eq!(values.len(), 2);

        // Negatives mirror exactly: magnitude order inverts below zero.
        let (neg_bigger, neg_smaller) = (Dec::new(-HUGE, 0), Dec::new(-i128::MAX, 1));
        assert_eq!(neg_bigger.cmp(&neg_smaller), Ordering::Less);
        assert!(neg_bigger < neg_smaller);

        // Sign alone settles a cross-zero pair the old code could not scale.
        assert!(neg_bigger < bigger);
        assert!(neg_bigger < Dec::zero());
        assert!(bigger > Dec::zero());
    }

    #[test]
    fn ord_survives_extreme_scales_without_overflowing() {
        // `places` is a `u32`, so the scale difference alone can dwarf any
        // power of ten an `i128` can hold. Digit counting settles these without
        // ever attempting the multiplication.
        let tiny = Dec::new(i128::MAX, u32::MAX);
        let one = Dec::new(1, 0);
        assert!(tiny < one);
        assert!(tiny > Dec::zero());
        assert_eq!(tiny.cmp(&tiny), Ordering::Equal);

        // `i128::MIN` has no positive counterpart, so `neg`/`abs` fail on it;
        // ordering must not.
        let min = Dec::new(i128::MIN, 0);
        assert!(min < Dec::zero());
        assert!(min < Dec::new(i128::MIN + 1, 0));
        assert_eq!(min.cmp(&min), Ordering::Equal);
    }

    #[test]
    fn ord_agrees_with_exact_rational_ordering() {
        // Cross-check the digit-count shortcut against ordinary same-scale
        // integer math over a spread of scales and signs. Every pair here is
        // small enough that rescaling cannot overflow, so this pins the fast
        // path to the arithmetic the old implementation used.
        let mantissas = [-1_000_001i128, -999, -1, 0, 1, 999, 1_000_001];
        for &ma in &mantissas {
            for pa in 0u32..6 {
                for &mb in &mantissas {
                    for pb in 0u32..6 {
                        let scale = pa.max(pb);
                        let lhs = ma * 10i128.pow(scale - pa);
                        let rhs = mb * 10i128.pow(scale - pb);
                        assert_eq!(
                            Dec::new(ma, pa).cmp(&Dec::new(mb, pb)),
                            lhs.cmp(&rhs),
                            "{ma}e-{pa} vs {mb}e-{pb}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn oversized_mantissa_reports_overflow_not_invalid() {
        // PARSE-9: hledger's mantissa is arbitrary-precision; ours is i128. The
        // 42-digit literal hledger accepts must fail as Overflow, not as the
        // misleading "invalid numeric literal".
        let huge = "123456789012345678901234567890123456789012";
        assert_eq!(Dec::parse(huge, '.'), Err(DecError::Overflow));
        // Genuinely malformed input still reports InvalidNumber.
        assert_eq!(
            Dec::parse("12x4", '.'),
            Err(DecError::InvalidNumber("12x4".to_string()))
        );
    }

    /// The mean of a few periods' money: same scale in, same scale out.
    #[test]
    fn div_int_keeps_the_scale_it_was_given() {
        // $1,659.00 over three months is exactly $553.00 — and stays at 2dp
        // rather than normalizing to `553`, so it prints in the same column as
        // the figures it averages.
        let sum = Dec::new(165_900, 2);
        assert_eq!(sum.div_int(3).unwrap(), Dec::new(55_300, 2));
        assert_eq!(sum.div_int(3).unwrap().places, 2);
        // A whole-dollar figure stays whole-dollar.
        assert_eq!(Dec::new(900, 0).div_int(4).unwrap(), Dec::new(225, 0));
    }

    #[test]
    fn div_int_rounds_half_away_from_zero() {
        // 301 / 3 = 100.333… → 100.33 at 2dp (remainder under half).
        assert_eq!(Dec::new(30_100, 2).div_int(3).unwrap(), Dec::new(10_033, 2));
        // 302 / 3 = 100.666… → 100.67 (remainder over half).
        assert_eq!(Dec::new(30_200, 2).div_int(3).unwrap(), Dec::new(10_067, 2));
        // Exactly half rounds AWAY from zero, in both directions — the property
        // banker's rounding would break.
        assert_eq!(Dec::new(5, 0).div_int(2).unwrap(), Dec::new(3, 0));
        assert_eq!(Dec::new(-5, 0).div_int(2).unwrap(), Dec::new(-3, 0));
        // And a negative average (income, as hledger writes it) is symmetric.
        assert_eq!(
            Dec::new(-30_100, 2).div_int(3).unwrap(),
            Dec::new(-10_033, 2)
        );
    }

    #[test]
    fn div_int_by_one_is_the_value_itself() {
        for value in [Dec::new(0, 0), Dec::new(1234, 2), Dec::new(-7, 3)] {
            assert_eq!(value.div_int(1).unwrap(), value);
        }
    }

    #[test]
    fn div_int_refuses_a_count_of_zero() {
        // The mean of no values is not a number, and reporting one would be a
        // confident answer to a question nobody can answer.
        assert_eq!(Dec::new(100, 2).div_int(0), Err(DecError::DivideByZero));
    }

    proptest! {
        /// Division by a count can never overflow, and never widens the scale —
        /// the two claims `div_int`'s docs make about it being total.
        #[test]
        fn div_int_is_total_and_scale_preserving(mantissa: i128, places in 0u32..12, count in 1u32..1000) {
            let value = Dec::new(mantissa, places);
            let mean = value.div_int(count).expect("division by a positive count cannot fail");
            prop_assert_eq!(mean.places, places);
            // |mean| <= |value|, because dividing by at least one never grows a
            // magnitude — and the rounding step adds at most one unit, which
            // cannot push it past the original.
            prop_assert!(mean.mantissa.unsigned_abs() <= value.mantissa.unsigned_abs());
        }
    }
}
