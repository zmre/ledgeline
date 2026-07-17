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

    /// Commodity-wise sum; zero commodities dropped from the result (`maAdd`).
    ///
    /// # Errors
    /// Returns [`DecError`] on decimal overflow.
    pub fn ma_add(&self, other: &MixedAmount) -> Result<MixedAmount, DecError> {
        let mut out = self.0.clone();
        for (commodity, qty) in &other.0 {
            match out.get(commodity).copied() {
                Some(prev) => out.insert(commodity.clone(), prev.add(*qty)?),
                None => out.insert(commodity.clone(), *qty),
            };
        }
        out.retain(|_, qty| !qty.is_zero());
        Ok(MixedAmount(out))
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
