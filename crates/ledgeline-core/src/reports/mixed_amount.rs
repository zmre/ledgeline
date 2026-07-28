//! `MixedAmount` — a multi-commodity amount (`commodity → exact quantity`).
//!
//! Port of the `MixedAmount = Map<string, Dec>` operations in
//! `web/src/lib/domain/money.ts` (`maAdd`/`maNeg`/`maIsZero`). Backed by a
//! `BTreeMap<Commodity, Dec>` so iteration is deterministic (lexical by
//! commodity). Following the TS contract, a commodity that nets to exactly zero
//! is dropped from results — the empty map is the additive identity.
//!
//! `Dec` equality/ordering is by numeric value, so two `MixedAmount`s comparing
//! equal may still carry different `mantissa`/`places` representations; the
//! golden tests canonicalize (strip trailing zeros) before comparing wire
//! numbers.

use crate::decimal::{Dec, DecError};
use crate::model::Commodity;
use std::collections::BTreeMap;

/// A commodity-keyed bag of exact quantities, zero commodities dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MixedAmount(BTreeMap<Commodity, Dec>);

impl MixedAmount {
    /// The empty (zero) mixed amount.
    #[must_use]
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// A single-commodity amount. A zero quantity yields the empty map (matching
    /// the zero-dropping contract).
    #[must_use]
    pub fn single(commodity: Commodity, qty: Dec) -> Self {
        let mut map = BTreeMap::new();
        if !qty.is_zero() {
            map.insert(commodity, qty);
        }
        Self(map)
    }

    /// True when no commodity is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// True when every commodity is exactly zero (`maIsZero`). Because results
    /// drop zeros, an empty map is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.values().all(Dec::is_zero)
    }

    /// This commodity's quantity, if present.
    #[must_use]
    pub fn get(&self, commodity: &Commodity) -> Option<Dec> {
        self.0.get(commodity).copied()
    }

    /// Iterate `(commodity, quantity)` pairs in lexical commodity order.
    pub fn iter(&self) -> impl Iterator<Item = (&Commodity, &Dec)> {
        self.0.iter()
    }

    /// Add `qty` into `commodity` in place, **without** pruning zeros — callers
    /// prune once at the end, matching `accountTotals`' single final sweep.
    ///
    /// # Errors
    /// Returns [`DecError`] on decimal overflow.
    pub fn accumulate(&mut self, commodity: &Commodity, qty: Dec) -> Result<(), DecError> {
        match self.0.get(commodity).copied() {
            Some(prev) => self.0.insert(commodity.clone(), prev.add(qty)?),
            None => self.0.insert(commodity.clone(), qty),
        };
        Ok(())
    }

    /// Drop every commodity whose quantity is exactly zero.
    pub fn drop_zeros(&mut self) {
        self.0.retain(|_, qty| !qty.is_zero());
    }

    /// Add `other` into `self` commodity-wise, dropping the commodities that net
    /// to zero — [`MixedAmount::ma_add`] without its clone of the accumulator.
    ///
    /// The zero-pruning is what separates this from a bare
    /// [`MixedAmount::accumulate`] loop, and it has to happen HERE rather than
    /// once at the end: a commodity that nets to zero and is later re-added must
    /// come back at the new addend's scale, exactly as `maAdd` gives it.
    ///
    /// # Errors
    /// Returns [`DecError`] on decimal overflow.
    pub fn ma_add_assign(&mut self, other: &MixedAmount) -> Result<(), DecError> {
        for (commodity, qty) in &other.0 {
            self.accumulate(commodity, *qty)?;
        }
        self.drop_zeros();
        Ok(())
    }

    /// Commodity-wise sum; zero commodities dropped from the result (`maAdd`).
    ///
    /// # Errors
    /// Returns [`DecError`] on decimal overflow.
    pub fn ma_add(&self, other: &MixedAmount) -> Result<MixedAmount, DecError> {
        let mut out = self.clone();
        out.ma_add_assign(other)?;
        Ok(out)
    }

    /// Negate every commodity (`maNeg`). Does not prune (inputs are already
    /// zero-free in practice).
    ///
    /// # Errors
    /// Returns [`DecError`] if a mantissa is `i128::MIN`.
    pub fn ma_neg(&self) -> Result<MixedAmount, DecError> {
        let mut out = BTreeMap::new();
        for (commodity, qty) in &self.0 {
            out.insert(commodity.clone(), qty.neg()?);
        }
        Ok(MixedAmount(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(symbol: &str) -> Commodity {
        Commodity(symbol.into())
    }

    fn ma(entries: &[(&str, Dec)]) -> MixedAmount {
        let mut out = MixedAmount::new();
        for (symbol, qty) in entries {
            out.accumulate(&c(symbol), *qty).unwrap();
        }
        out
    }

    /// `ma_add` is now `clone` + [`MixedAmount::ma_add_assign`], so the two must
    /// agree exactly — including on the REPRESENTATION, not just the value.
    #[test]
    fn ma_add_assign_matches_ma_add_including_scale() {
        let cases = [
            (
                vec![("$", Dec::new(1000, 2))],
                vec![("$", Dec::new(250, 2))],
            ),
            // Nets to zero: the commodity has to be dropped, not kept at zero.
            (
                vec![("$", Dec::new(500, 2))],
                vec![("$", Dec::new(-500, 2))],
            ),
            // Disjoint commodities merge.
            (
                vec![("$", Dec::new(1, 0))],
                vec![("EUR", Dec::new(2, 0)), ("AAPL", Dec::new(3, 0))],
            ),
            // Adding into the empty accumulator keeps the addend's scale.
            (vec![], vec![("$", Dec::new(3000, 3))]),
            (vec![("$", Dec::new(0, 4))], vec![("$", Dec::new(30, 1))]),
        ];
        for (left, right) in cases {
            let (a, b) = (ma(&left), ma(&right));
            let mut in_place = a.clone();
            in_place.ma_add_assign(&b).unwrap();
            let cloned = a.ma_add(&b).unwrap();
            assert_eq!(in_place, cloned, "{left:?} + {right:?}");
            let representations = |m: &MixedAmount| {
                m.iter()
                    .map(|(commodity, qty)| (commodity.clone(), qty.mantissa, qty.places))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                representations(&in_place),
                representations(&cloned),
                "{left:?} + {right:?} representation"
            );
        }
    }

    /// Pruning has to happen per addition: a commodity that nets to zero and is
    /// then re-added comes back at the NEW addend's scale. Deferring the sweep to
    /// the end would leave the stale scale behind and change the wire output.
    #[test]
    fn a_commodity_that_nets_to_zero_returns_at_the_new_scale() {
        let mut acc = ma(&[("$", Dec::new(500, 3))]);
        acc.ma_add_assign(&ma(&[("$", Dec::new(-500, 3))])).unwrap();
        assert!(acc.is_empty());
        acc.ma_add_assign(&ma(&[("$", Dec::new(30, 1))])).unwrap();
        let (_, qty) = acc.iter().next().unwrap();
        assert_eq!((qty.mantissa, qty.places), (30, 1));
    }
}
