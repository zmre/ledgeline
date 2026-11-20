//! Money, percentages and quantities as the bill table writes them, in the
//! journal's own commodity style. All arithmetic is `ledgeline_core`'s exact
//! [`Dec`]; rounding is to the commodity's precision, half away from zero.

use ledgeline_core::Dec;
use ledgeline_core::decimal::DecError;
use ledgeline_core::display::format_amount;
use ledgeline_core::model::{AmountStyle, Commodity, CommoditySide, DigitGroups, Journal};
use ledgeline_core::parse::JournalStyles;
use std::sync::Arc;
use thiserror::Error;

/// Why a cell isn't the number it should be.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NumberError {
    /// Not an amount in the invoice's commodity.
    #[error("`{text}` is not an amount in {commodity}")]
    NotMoney {
        /// The cell text.
        text: String,
        /// The invoice's commodity.
        commodity: String,
    },
    /// Not a number.
    #[error("`{0}` is not a number")]
    NotNumber(String),
    /// Not a percentage.
    #[error("`{0}` is not a percentage")]
    NotPercent(String),
    /// Too large to compute with.
    #[error("the amounts are too large")]
    Overflow,
}

impl From<DecError> for NumberError {
    fn from(_: DecError) -> Self {
        Self::Overflow
    }
}

/// The invoice's commodity and how the journal writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyStyle {
    commodity: Commodity,
    style: AmountStyle,
}

/// A parsed money cell: the value, and whether a sign was written (`-$800`,
/// `$-800`, `+$50`), which is what makes a bare amount row a fixed
/// adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrittenMoney {
    /// The value.
    pub value: Dec,
    /// A `-` or `+` was written.
    pub signed: bool,
}

impl MoneyStyle {
    /// `commodity` in `style`.
    #[must_use]
    pub fn new(commodity: &str, style: AmountStyle) -> Self {
        Self {
            commodity: Commodity(commodity.to_string()),
            style,
        }
    }

    /// `commodity` as the journal declares it (`commodity $1,000.00`), or
    /// [`MoneyStyle::fallback`] when it declares no style for it.
    #[must_use]
    pub fn from_styles(styles: &JournalStyles<'_>, commodity: &str) -> Self {
        let key = Commodity(commodity.to_string());
        styles.declared_style(&key).map_or_else(
            || Self::fallback(commodity),
            |style| Self::new(commodity, style.clone()),
        )
    }

    /// [`MoneyStyle::from_styles`] for a loaded journal.
    #[must_use]
    pub fn of_journal(journal: &Journal, commodity: &str) -> Self {
        Self::from_styles(&JournalStyles::of(journal), commodity)
    }

    /// The style used when the journal declares none: symbol on the left,
    /// unspaced, `.` decimal mark, `,` thousands, two places.
    #[must_use]
    pub fn fallback(commodity: &str) -> Self {
        Self::new(
            commodity,
            AmountStyle {
                side: CommoditySide::Left,
                spaced: false,
                decimal_mark: Some('.'),
                digit_groups: Some(Arc::new(DigitGroups {
                    mark: ',',
                    sizes: vec![3],
                })),
                precision: 2,
            },
        )
    }

    /// The commodity symbol.
    #[must_use]
    pub fn commodity(&self) -> &str {
        &self.commodity.0
    }

    /// The display style.
    #[must_use]
    pub fn style(&self) -> &AmountStyle {
        &self.style
    }

    /// Fractional digits every computed amount is rounded to.
    #[must_use]
    pub fn precision(&self) -> u32 {
        self.style.precision
    }

    /// The decimal mark numbers in the table are read with.
    #[must_use]
    pub fn decimal_mark(&self) -> char {
        self.style.decimal_mark.unwrap_or('.')
    }

    /// `value` rounded to the commodity's precision, half away from zero.
    ///
    /// # Errors
    /// [`NumberError::Overflow`].
    pub fn round(&self, value: Dec) -> Result<Dec, NumberError> {
        Ok(value.rounded(self.precision())?)
    }

    /// `value` written as the table writes money: the journal's side,
    /// spacing, marks, grouping and precision, with the sign in front of the
    /// symbol (`-$250.00`), as a reader writes it on an invoice.
    #[must_use]
    pub fn format(&self, value: Dec) -> String {
        let rounded = value.rounded(self.precision()).unwrap_or(value);
        let magnitude = rounded.abs().unwrap_or(rounded);
        let text = format_amount(self.commodity(), magnitude, &self.style, self.precision());
        if rounded.mantissa < 0 {
            format!("-{text}")
        } else {
            text
        }
    }

    /// Read a money cell. The symbol may be on either side or absent, and
    /// the sign before or after it: `-$800`, `$-800`, `800`, `1.234,56 €`.
    ///
    /// # Errors
    /// [`NumberError::NotMoney`] for anything else, including another
    /// commodity's symbol.
    pub fn parse(&self, text: &str) -> Result<WrittenMoney, NumberError> {
        let not_money = || NumberError::NotMoney {
            text: text.to_string(),
            commodity: self.commodity().to_string(),
        };
        let symbol = self.commodity();
        let strip_symbol = |s: &'_ str| -> String {
            let s = s.trim();
            let s = if symbol.is_empty() {
                s
            } else {
                s.strip_prefix(symbol)
                    .or_else(|| s.strip_suffix(symbol))
                    .unwrap_or(s)
            };
            s.trim().to_string()
        };
        let body = plain(text);
        let (outer_negative, outer_signed, rest) = split_sign(body);
        let rest = strip_symbol(rest);
        let (inner_negative, inner_signed, rest) = split_sign(&rest);
        if outer_signed && inner_signed {
            return Err(not_money());
        }
        let number = strip_symbol(rest);
        if number.is_empty() || number.starts_with(['-', '+']) {
            return Err(not_money());
        }
        let magnitude = Dec::parse(&number, self.decimal_mark()).map_err(|_| not_money())?;
        let value = if outer_negative || inner_negative {
            magnitude.neg()?
        } else {
            magnitude
        };
        Ok(WrittenMoney {
            value,
            signed: outer_signed || inner_signed,
        })
    }
}

/// A leading `-`, `+` or `−` (U+2212): `(negative, signed, rest)`.
fn split_sign(text: &str) -> (bool, bool, &str) {
    let text = text.trim();
    text.strip_prefix(['-', '\u{2212}'])
        .map(|rest| (true, true, rest))
        .or_else(|| text.strip_prefix('+').map(|rest| (false, true, rest)))
        .unwrap_or((false, false, text))
}

/// A cell's text with what doesn't change its meaning removed: surrounding
/// whitespace, `&nbsp;` padding, and bold markers (`**$20.00**`).
#[must_use]
pub fn plain(text: &str) -> &str {
    let text = text.trim();
    let text = strip_nbsp(text);
    unbold(text).map_or(text, |inner| strip_nbsp(inner.trim()))
}

fn strip_nbsp(text: &str) -> &str {
    let mut text = text.trim_matches(|c: char| c.is_whitespace());
    loop {
        let next = text
            .strip_prefix("&nbsp;")
            .or_else(|| text.strip_suffix("&nbsp;"))
            .map(|rest| rest.trim_matches(|c: char| c.is_whitespace()));
        match next {
            Some(rest) => text = rest,
            None => return text,
        }
    }
}

/// `**x**` or `__x__` with non-empty `x`: the inner text.
#[must_use]
pub fn unbold(text: &str) -> Option<&str> {
    ["**", "__"].iter().find_map(|mark| {
        text.strip_prefix(mark)
            .and_then(|rest| rest.strip_suffix(mark))
            .filter(|inner| !inner.trim().is_empty())
    })
}

/// A cell that reads as empty: blank, or only `&nbsp;`.
#[must_use]
pub fn is_blank(text: &str) -> bool {
    strip_nbsp(text.trim()).is_empty()
}

/// A written percentage: `-10` for `-10%`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Percent(Dec);

impl Percent {
    /// The percentage as written (`8.25` for `8.25%`).
    #[must_use]
    pub fn value(self) -> Dec {
        self.0
    }

    /// The multiplier (`0.0825` for `8.25%`).
    #[must_use]
    pub fn fraction(self) -> Dec {
        Dec::new(self.0.mantissa, self.0.places + 2)
    }

    /// `None` when `text` has no `%` (it isn't trying to be a percentage);
    /// otherwise the percentage or why it isn't one.
    ///
    /// # Errors
    /// [`NumberError::NotPercent`] for `%` text that isn't a number.
    pub fn parse(text: &str, decimal_mark: char) -> Option<Result<Self, NumberError>> {
        let body = plain(text).strip_suffix('%')?;
        let (negative, _, number) = split_sign(body);
        let not = || NumberError::NotPercent(text.to_string());
        let number = number.trim();
        Some(
            (!number.is_empty() && !number.starts_with(['-', '+']))
                .then_some(number)
                .ok_or_else(not)
                .and_then(|n| Dec::parse(n, decimal_mark).map_err(|_| not()))
                .and_then(|n| if negative { Ok(n.neg()?) } else { Ok(n) })
                .map(Self),
        )
    }
}

/// Read a quantity cell: a plain number, signed or not.
///
/// # Errors
/// [`NumberError::NotNumber`].
pub fn parse_quantity(text: &str, decimal_mark: char) -> Result<Dec, NumberError> {
    let (negative, _, number) = split_sign(plain(text));
    let not = || NumberError::NotNumber(text.to_string());
    if number.is_empty() || number.starts_with(['-', '+']) {
        return Err(not());
    }
    let magnitude = Dec::parse(number, decimal_mark).map_err(|_| not())?;
    Ok(if negative {
        magnitude.neg()?
    } else {
        magnitude
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd() -> MoneyStyle {
        MoneyStyle::fallback("$")
    }

    fn eur() -> MoneyStyle {
        MoneyStyle::new(
            "€",
            AmountStyle {
                side: CommoditySide::Right,
                spaced: true,
                decimal_mark: Some(','),
                digit_groups: Some(Arc::new(DigitGroups {
                    mark: '.',
                    sizes: vec![3],
                })),
                precision: 2,
            },
        )
    }

    #[test]
    fn both_sign_placements_read_the_same() {
        let a = usd().parse("-$800").expect("reads");
        let b = usd().parse("$-800").expect("reads");
        assert_eq!(a, b);
        assert_eq!(a.value, Dec::new(-800, 0));
        assert!(a.signed);
        assert!(!usd().parse("$8,000").expect("reads").signed);
        assert!(usd().parse("+$50").expect("reads").signed);
        assert_eq!(
            usd().parse("**$16,700**").expect("reads").value,
            Dec::new(16_700, 0)
        );
        assert_eq!(
            usd().parse("1,234.5").expect("reads").value,
            Dec::new(12_345, 1)
        );
    }

    #[test]
    fn other_commodities_and_double_signs_are_not_money() {
        for text in ["€800", "-$-800", "$", "abc", "", "$ 8 x"] {
            assert!(usd().parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn formatting_puts_the_sign_before_the_symbol() {
        assert_eq!(usd().format(Dec::new(-25_000, 2)), "-$250.00");
        assert_eq!(usd().format(Dec::new(1_648_221, 2)), "$16,482.21");
        assert_eq!(eur().format(Dec::new(-123_456, 2)), "-1.234,56 €");
        assert_eq!(
            eur().parse("1.234,56 €").expect("reads").value,
            Dec::new(123_456, 2)
        );
    }

    #[test]
    fn percentages() {
        let p = Percent::parse("-10%", '.').expect("is one").expect("reads");
        assert_eq!(p.fraction(), Dec::new(-1, 1));
        let p = Percent::parse("8.25%", '.')
            .expect("is one")
            .expect("reads");
        assert_eq!(p.fraction(), Dec::new(825, 4));
        assert!(Percent::parse("$5", '.').is_none());
        assert!(Percent::parse("x%", '.').expect("is one").is_err());
    }

    #[test]
    fn nbsp_and_bold_are_not_content() {
        assert!(is_blank("&nbsp;"));
        assert!(is_blank(" &nbsp; &nbsp; "));
        assert_eq!(plain("**Subtotal**"), "Subtotal");
        assert_eq!(plain("&nbsp;10"), "10");
    }
}
