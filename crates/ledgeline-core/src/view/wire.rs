//! The value shapes every journal-view payload shares: an exact decimal as a
//! mantissa STRING, and an amount style exactly as the SPA's domain
//! `AmountStyle` (`web/src/lib/domain/types.ts`) spells it.
//!
//! The mantissa travels as a string because JSON numbers are doubles on the
//! other side: a mantissa past 2^53 would silently lose digits in the browser,
//! and the hledger-compatible `/transactions` wire (which sends it as a number)
//! has to refuse such amounts outright.

use crate::decimal::Dec;
use crate::model::{Amount, AmountStyle, Commodity, CommoditySide, DigitGroups, Journal};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

/// An exact decimal: `m / 10^p`, `m` as decimal text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireDec {
    /// The signed mantissa, as decimal text.
    pub m: String,
    /// Decimal places.
    pub p: u32,
}

impl From<Dec> for WireDec {
    fn from(value: Dec) -> Self {
        Self {
            m: value.mantissa.to_string(),
            p: value.places,
        }
    }
}

/// An amount style in the SPA's shape: `{side, spaced, precision, decimalPoint,
/// digitGroups}`. Group sizes are always positive; a grouping with none left is
/// `null` (exactly how `normalize.ts` decodes an `astyle`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireStyle {
    /// `"L"` or `"R"`.
    pub side: &'static str,
    /// A space between symbol and number.
    pub spaced: bool,
    /// Display precision.
    pub precision: u32,
    /// The decimal mark, `"."` when the style names none.
    pub decimal_point: String,
    /// `[separator, sizes]`, or `null` for no grouping.
    pub digit_groups: Option<(String, Vec<u8>)>,
}

impl From<&AmountStyle> for WireStyle {
    fn from(style: &AmountStyle) -> Self {
        Self {
            side: match style.side {
                CommoditySide::Left => "L",
                CommoditySide::Right => "R",
            },
            spaced: style.spaced,
            precision: style.precision,
            decimal_point: style.decimal_mark.unwrap_or('.').to_string(),
            digit_groups: style
                .digit_groups
                .as_deref()
                .map(|groups| (groups.mark.to_string(), groups.sizes.clone())),
        }
    }
}

/// The journal's declared `commodity`/`D` styles, a later declaration winning
/// (the `/api/journal` `commodityStyles` rule).
#[derive(Debug, Default)]
pub struct DeclaredStyles {
    styles: HashMap<String, AmountStyle>,
}

impl DeclaredStyles {
    /// Collect `journal`'s declared styles.
    #[must_use]
    pub fn of(journal: &Journal) -> Self {
        Self {
            styles: journal
                .commodity_styles
                .iter()
                .map(|(Commodity(symbol), style)| (symbol.clone(), normalized(style)))
                .collect(),
        }
    }

    /// The style `amount` DISPLAYS in — the SPA's `normalize.ts` `amountStyle`.
    ///
    /// A declared style is canonical, but its precision is a FLOOR, never a
    /// cap: a 10.25-share lot under `commodity 1.0 VTI` must not read as 10.3.
    /// Without a declaration the amount's own as-written style applies.
    #[must_use]
    pub fn display_style(&self, amount: &Amount) -> AmountStyle {
        let places = amount.quantity.places;
        match self.styles.get(amount.commodity.0.as_str()) {
            Some(declared) if places <= declared.precision => declared.clone(),
            Some(declared) => AmountStyle {
                precision: places,
                ..declared.clone()
            },
            None => normalized(&amount.style),
        }
    }
}

/// A style as the SPA decodes it: non-positive group sizes dropped, and a
/// grouping with no sizes left treated as no grouping at all.
fn normalized(style: &AmountStyle) -> AmountStyle {
    let groups = style.digit_groups.as_deref().and_then(|groups| {
        if groups.sizes.iter().all(|size| *size > 0) {
            return Some(Arc::new(groups.clone()));
        }
        let sizes: Vec<u8> = groups
            .sizes
            .iter()
            .copied()
            .filter(|size| *size > 0)
            .collect();
        (!sizes.is_empty()).then(|| {
            Arc::new(DigitGroups {
                mark: groups.mark,
                sizes,
            })
        })
    });
    AmountStyle {
        digit_groups: groups,
        ..style.clone()
    }
}

/// The SPA's default style for a commodity it has no amount for
/// (`format/amounts.ts` `DEFAULT_AMOUNT_STYLE`).
#[must_use]
pub fn default_style() -> AmountStyle {
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

/// A decimal as plain text — sign, digits, point, no grouping and no symbol —
/// for diagnostic messages (`checks/rules.ts` `decToString`).
#[must_use]
pub fn plain_decimal(value: Dec) -> String {
    let places = value.places as usize;
    let digits = format!(
        "{:0>width$}",
        value.mantissa.unsigned_abs(),
        width = places + 1
    );
    let (whole, frac) = digits.split_at(digits.len() - places);
    let sign = if value.mantissa < 0 { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{frac}")
    }
}

/// `Number(m) / 10 ** p` — the SPA's display-boundary `toNumber`. Both halves
/// round to nearest exactly as JavaScript does, and `10^p` is exact for every
/// `p` up to 22.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
pub fn to_number(value: Dec) -> f64 {
    value.mantissa as f64 / 10f64.powi(value.places as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_decimal_pads_and_signs() {
        assert_eq!(plain_decimal(Dec::new(-6_735_456, 3)), "-6735.456");
        assert_eq!(plain_decimal(Dec::new(5, 3)), "0.005");
        assert_eq!(plain_decimal(Dec::new(-5, 0)), "-5");
        assert_eq!(plain_decimal(Dec::new(0, 2)), "0.00");
    }

    #[test]
    fn to_number_matches_javascript() {
        assert!((to_number(Dec::new(12_345, 2)) - 123.45).abs() < f64::EPSILON);
        assert!(to_number(Dec::new(-1, 0)) < 0.0);
    }
}
