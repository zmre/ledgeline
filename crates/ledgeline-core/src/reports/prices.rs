//! Market-price database + valuation — port of
//! `web/src/lib/reports/prices.ts`.
//!
//! Direct conversions only: a commodity is valued via the latest `P` directive
//! dated ≤ `as_of` that prices it directly in the target commodity. Commodities
//! without such a price are SKIPPED (never guessed) and reported via the
//! optional [`ValuationMeta`] out-param.

use super::mixed_amount::MixedAmount;
use crate::decimal::{Dec, DecError};
use crate::model::{Amount, Commodity, PriceDirective};
use std::collections::BTreeMap;

/// A market-price lookup table built from `P` directives.
#[derive(Debug, Clone)]
pub struct PriceDb {
    /// Directives per priced commodity, stable-sorted ascending by date (so the
    /// last-declared wins for equal dates on the reverse scan).
    by_commodity: BTreeMap<Commodity, Vec<PriceDirective>>,
    /// Default valuation target: the most frequent price commodity (ties broken
    /// lexically); `None` when there are no directives.
    base: Option<Commodity>,
}

impl PriceDb {
    /// Build a [`PriceDb`] from directives (in journal/declaration order).
    #[must_use]
    pub fn build(directives: &[PriceDirective]) -> PriceDb {
        let mut by_commodity: BTreeMap<Commodity, Vec<PriceDirective>> = BTreeMap::new();
        for directive in directives {
            by_commodity
                .entry(directive.commodity.clone())
                .or_default()
                .push(directive.clone());
        }
        // Stable sort: same-date directives keep journal order.
        for list in by_commodity.values_mut() {
            list.sort_by(|a, b| a.date.cmp(&b.date));
        }

        let mut counts: BTreeMap<Commodity, usize> = BTreeMap::new();
        let mut base: Option<Commodity> = None;
        for directive in directives {
            let target = &directive.price.commodity;
            let count = {
                let slot = counts.entry(target.clone()).or_insert(0);
                *slot += 1;
                *slot
            };
            let base_count = base
                .as_ref()
                .and_then(|b| counts.get(b))
                .copied()
                .unwrap_or(0);
            let replace = match &base {
                None => true,
                Some(b) => count > base_count || (count == base_count && target < b),
            };
            if replace {
                base = Some(target.clone());
            }
        }
        PriceDb { by_commodity, base }
    }

    /// The latest directive for `commodity` dated ≤ `as_of` that also satisfies
    /// `matches`, scanning newest-first (last-declared wins on ties).
    fn latest(
        &self,
        commodity: &Commodity,
        as_of: &str,
        matches: impl Fn(&PriceDirective) -> bool,
    ) -> Option<&Amount> {
        self.by_commodity
            .get(commodity)?
            .iter()
            .rev()
            .find(|directive| directive.date.as_str() <= as_of && matches(directive))
            .map(|directive| &directive.price)
    }

    /// Latest `P` directive for `commodity` dated ≤ `as_of`, regardless of the
    /// target it is priced in.
    #[must_use]
    pub fn lookup(&self, commodity: &Commodity, as_of: &str) -> Option<&Amount> {
        self.latest(commodity, as_of, |_| true)
    }

    /// Latest `P` directive dated ≤ `as_of` pricing `commodity` directly in
    /// `target`.
    #[must_use]
    pub fn lookup_in(
        &self,
        commodity: &Commodity,
        target: &Commodity,
        as_of: &str,
    ) -> Option<&Amount> {
        self.latest(commodity, as_of, |directive| {
            &directive.price.commodity == target
        })
    }

    /// Default valuation target (most frequent price commodity; `None` when
    /// there are no directives).
    #[must_use]
    pub fn base_commodity(&self) -> Option<&Commodity> {
        self.base.as_ref()
    }
}

/// Out-param for [`value_at`]: commodities that had to be skipped (deduped, in
/// encounter order — which, over a `BTreeMap`, is lexical).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValuationMeta {
    /// Commodities with no direct price to the target at `as_of`.
    pub unpriced: Vec<Commodity>,
}

/// Non-normalizing exact multiply, mirroring `money.ts`'s `mul` (`m·m`, `p+p`,
/// NO trailing-zero stripping). The engine's canonical `Dec::mul` normalizes to
/// match hledger's parser; valuation must keep the TS representation so ported
/// expectations line up bit-for-bit.
///
/// `pub(crate)` so the holdings engine (`crate::holdings`) reuses the exact same
/// non-normalizing multiply for its basis/market-value math instead of
/// duplicating it.
pub(crate) fn mul_raw(a: Dec, b: Dec) -> Result<Dec, DecError> {
    let mantissa = a
        .mantissa
        .checked_mul(b.mantissa)
        .ok_or(DecError::Overflow)?;
    let places = a.places.checked_add(b.places).ok_or(DecError::Overflow)?;
    Ok(Dec::new(mantissa, places))
}

/// Value a [`MixedAmount`] in `target` at `as_of`: identity for `target` itself,
/// exact `mul_raw` via the latest direct price otherwise. Commodities without a
/// direct price are SKIPPED and, when `meta` is given, recorded there (deduped).
///
/// # Errors
/// Returns [`DecError`] on decimal overflow.
pub fn value_at(
    ma: &MixedAmount,
    target: &Commodity,
    db: &PriceDb,
    as_of: &str,
    mut meta: Option<&mut ValuationMeta>,
) -> Result<Dec, DecError> {
    let mut total = Dec::zero();
    for (commodity, qty) in ma.iter() {
        if commodity == target {
            total = total.add(*qty)?;
            continue;
        }
        match db.lookup_in(commodity, target, as_of) {
            Some(price) => {
                total = total.add(mul_raw(*qty, price.quantity)?)?;
            }
            None => {
                if let Some(sink) = meta.as_deref_mut()
                    && !sink.unpriced.contains(commodity)
                {
                    sink.unpriced.push(commodity.clone());
                }
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{amount, price};
    use super::*;

    fn directives() -> Vec<PriceDirective> {
        vec![
            price("2024-09-30", "EUR", amount("$", 111, 2)),
            price("2024-09-30", "AAPL", amount("$", 22800, 2)),
            price("2025-12-31", "EUR", amount("$", 110, 2)),
            price("2025-12-31", "AAPL", amount("$", 25500, 2)),
            price("2026-06-30", "EUR", amount("$", 116, 2)),
            // later same-commodity directive in a different target
            price("2026-06-30", "EUR", amount("GBP", 85, 2)),
        ]
    }

    fn c(s: &str) -> Commodity {
        Commodity(s.into())
    }

    #[test]
    fn lookup_returns_latest_le_asof_inclusive() {
        let db = PriceDb::build(&directives());
        assert_eq!(
            db.lookup(&c("AAPL"), "2025-12-30").unwrap().quantity,
            Dec::new(22800, 2)
        );
        assert_eq!(
            db.lookup(&c("AAPL"), "2025-12-31").unwrap().quantity,
            Dec::new(25500, 2)
        );
        assert_eq!(
            db.lookup(&c("AAPL"), "2026-07-08").unwrap().quantity,
            Dec::new(25500, 2)
        );
    }

    #[test]
    fn lookup_returns_none_before_first_or_unknown() {
        let db = PriceDb::build(&directives());
        assert!(db.lookup(&c("AAPL"), "2024-09-29").is_none());
        assert!(db.lookup(&c("DOGE"), "2026-07-08").is_none());
    }

    #[test]
    fn lookup_same_date_last_declared_wins() {
        let db = PriceDb::build(&directives());
        assert_eq!(
            db.lookup(&c("EUR"), "2026-06-30").unwrap().commodity,
            c("GBP")
        );
    }

    #[test]
    fn lookup_in_skips_other_targets() {
        let db = PriceDb::build(&directives());
        assert_eq!(
            db.lookup_in(&c("EUR"), &c("$"), "2026-06-30")
                .unwrap()
                .quantity,
            Dec::new(116, 2)
        );
        assert_eq!(
            db.lookup_in(&c("EUR"), &c("GBP"), "2026-06-30")
                .unwrap()
                .quantity,
            Dec::new(85, 2)
        );
        assert!(db.lookup_in(&c("EUR"), &c("GBP"), "2026-06-29").is_none());
        assert!(db.lookup_in(&c("AAPL"), &c("GBP"), "2026-07-08").is_none());
    }

    #[test]
    fn base_commodity_most_frequent_then_lexical() {
        assert_eq!(
            PriceDb::build(&directives()).base_commodity(),
            Some(&c("$"))
        );
        let tie = vec![
            price("2026-01-01", "EUR", amount("GBP", 85, 2)),
            price("2026-01-02", "AAPL", amount("$", 25500, 2)),
        ];
        assert_eq!(PriceDb::build(&tie).base_commodity(), Some(&c("$")));
        assert_eq!(PriceDb::build(&[]).base_commodity(), None);
    }

    #[test]
    fn value_at_converts_and_passes_target_through() {
        let db = PriceDb::build(&directives());
        let mut ma = MixedAmount::new();
        ma.accumulate(&c("$"), Dec::new(1000, 2)).unwrap();
        ma.accumulate(&c("EUR"), Dec::new(20000, 2)).unwrap();
        // 10.00 + 200 EUR × $1.10 = 10.00 + 220.0000 = $230, kept at scale 4.
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2026-01-15", None).unwrap(),
            Dec::new(2300000, 4)
        );
    }

    #[test]
    fn value_at_skips_unpriced_and_dedupes_meta() {
        let db = PriceDb::build(&directives());
        let mut ma = MixedAmount::new();
        ma.accumulate(&c("DOGE"), Dec::new(5, 0)).unwrap();
        ma.accumulate(&c("EUR"), Dec::new(10000, 2)).unwrap();
        ma.accumulate(&c("AAPL"), Dec::new(10, 0)).unwrap(); // priced in $ but asOf predates all directives
        let mut meta = ValuationMeta::default();
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2024-01-01", Some(&mut meta)).unwrap(),
            Dec::new(0, 0)
        );
        // Encounter order is lexical (BTreeMap), unlike the TS insertion-ordered
        // Map; the set is identical and the report layer sorts anyway.
        assert_eq!(meta.unpriced, vec![c("AAPL"), c("DOGE"), c("EUR")]);
        // Second pass does not duplicate.
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2024-01-01", Some(&mut meta)).unwrap(),
            Dec::new(0, 0)
        );
        assert_eq!(meta.unpriced, vec![c("AAPL"), c("DOGE"), c("EUR")]);
    }

    #[test]
    fn value_at_without_meta() {
        let db = PriceDb::build(&directives());
        let mut ma = MixedAmount::new();
        ma.accumulate(&c("DOGE"), Dec::new(5, 0)).unwrap();
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2026-07-08", None).unwrap(),
            Dec::new(0, 0)
        );
    }
}
