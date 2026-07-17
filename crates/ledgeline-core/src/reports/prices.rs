//! Market-price database + valuation — port of
//! `web/src/lib/reports/prices.ts`.
//!
//! Direct conversions only: a commodity is valued via the latest `P` directive
//! dated ≤ `as_of` that prices it directly in the target commodity. Commodities
//! without such a price are SKIPPED (never guessed) and reported via the
//! optional [`ValuationMeta`] out-param.

use super::mixed_amount::MixedAmount;
use crate::decimal::{Dec, DecError};
use crate::model::{Amount, Commodity, CostKind, PriceDirective, Transaction};
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

/// `10^exp` as an `i128`, checked for overflow (mirrors `decimal::pow10`, which
/// is private to that module).
pub(crate) fn pow10(exp: u32) -> Result<i128, DecError> {
    10i128.checked_pow(exp).ok_or(DecError::Overflow)
}

/// Rounded division, half-even (banker's rounding) — port of the TS
/// `divRoundHalfEven`. `domain/money` has no `Dec` division on purpose; this is
/// the one place price/holdings math needs it.
///
/// The denominator is always positive at every call site (a share count or a
/// `|qty|`); a zero denominator is unreachable and is surfaced as the same
/// never-unwrapped overflow arm rather than panicking.
pub(crate) fn div_round_half_even(numerator: i128, denominator: i128) -> Result<i128, DecError> {
    if denominator == 0 {
        return Err(DecError::Overflow);
    }
    let negative = (numerator < 0) != (denominator < 0);
    let n = numerator.checked_abs().ok_or(DecError::Overflow)?;
    let d = denominator.checked_abs().ok_or(DecError::Overflow)?;
    let mut q = n / d;
    let r = n % d;
    let twice = r.checked_mul(2).ok_or(DecError::Overflow)?;
    if twice > d || (twice == d && q % 2 == 1) {
        q = q.checked_add(1).ok_or(DecError::Overflow)?;
    }
    Ok(if negative { -q } else { q })
}

/// Per-unit price from a `@@` total: `total / |qty|`, rounded half-even to
/// `total.p + qty.p` decimal places (port of the TS `perUnitFromTotal`). Shared
/// by the holdings engine and net-worth cost inference.
pub(crate) fn per_unit_from_total(total: Dec, qty: Dec) -> Result<Dec, DecError> {
    let places = total
        .places
        .checked_add(qty.places)
        .ok_or(DecError::Overflow)?;
    let factor = pow10(qty.places.checked_mul(2).ok_or(DecError::Overflow)?)?;
    let scaled_total = total
        .mantissa
        .checked_mul(factor)
        .ok_or(DecError::Overflow)?;
    let abs_qty = qty.mantissa.checked_abs().ok_or(DecError::Overflow)?;
    Ok(Dec::new(
        div_round_half_even(scaled_total, abs_qty)?,
        places,
    ))
}

/// The exact multiplicative reciprocal of `unit` as a terminating `Dec`, or
/// `None` when `1/unit` does not terminate in base 10 (its reduced denominator
/// has a prime factor other than 2 or 5) or when `unit` is zero.
///
/// Used to mirror hledger's price-graph reversal: from an inferred `P` directive
/// the reverse rate is derived so a commodity that appears only as a cost
/// DENOMINATOR (e.g. the GLD gift's `… @ 0.005 GLD` leg) is still valued.
fn exact_reciprocal(unit: Dec) -> Option<Dec> {
    if unit.mantissa == 0 {
        return None;
    }
    let sign = unit.mantissa.signum();
    let magnitude = unit.mantissa.checked_abs()?;
    // 1/unit = 10^p / |m|; grow the decimal places `k` (from 0) until |m| divides
    // 10^(p+k) — i.e. the reciprocal terminates. `checked_pow` returning `None`
    // (10^exp overflowing i128) means it never will → non-terminating.
    let mut exp = unit.places;
    loop {
        let numerator = 10i128.checked_pow(exp)?;
        if numerator % magnitude == 0 {
            return Some(Dec::new(sign * (numerator / magnitude), exp - unit.places));
        }
        exp = exp.checked_add(1)?;
    }
}

/// Market-price directives INFERRED from `@`/`@@` cost annotations, mirroring
/// hledger's `--infer-market-prices`. For each posting amount carrying a cost,
/// infer `P <txn.date> <amount.commodity> <unit cost>` (a `@@` total is divided
/// by `|qty|` to a per-unit price). When the unit cost's reciprocal terminates,
/// the reverse directive `P <txn.date> <cost.commodity> <1/unit cost>` is
/// inferred too — matching hledger's valuation-time price-graph reversal, so a
/// commodity seen only as a cost DENOMINATOR (the GLD gift's
/// `equity … @ 0.005 GLD`) is still valued.
///
/// The result is in journal order (date asc, then txn index). Callers append the
/// explicit `P` directives AFTER these so an explicit price wins a same-date tie
/// (hledger's precedence).
///
/// # Errors
/// Returns [`DecError`] on decimal overflow (never for realistic journals).
pub fn infer_market_prices(txns: &[Transaction]) -> Result<Vec<PriceDirective>, DecError> {
    let mut ordered: Vec<&Transaction> = txns.iter().collect();
    ordered.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.index.0.cmp(&b.index.0)));

    let mut inferred: Vec<PriceDirective> = Vec::new();
    for txn in ordered {
        for posting in &txn.postings {
            for amount in &posting.amounts {
                let Some(cost) = amount.cost.as_deref() else {
                    continue;
                };
                if amount.quantity.is_zero() {
                    continue;
                }
                let unit = match cost.kind {
                    CostKind::Unit => cost.amount.quantity,
                    CostKind::Total => per_unit_from_total(cost.amount.quantity, amount.quantity)?,
                };
                // Forward: the posting's commodity priced in the cost commodity.
                inferred.push(PriceDirective {
                    date: txn.date.clone(),
                    commodity: amount.commodity.clone(),
                    price: Amount {
                        commodity: cost.amount.commodity.clone(),
                        quantity: unit,
                        style: cost.amount.style.clone(),
                        cost: None,
                    },
                });
                // Reverse (only when 1/unit terminates): lets a commodity that
                // appears solely as a cost denominator still be valued.
                if let Some(reciprocal) = exact_reciprocal(unit) {
                    inferred.push(PriceDirective {
                        date: txn.date.clone(),
                        commodity: cost.amount.commodity.clone(),
                        price: Amount {
                            commodity: amount.commodity.clone(),
                            quantity: reciprocal,
                            style: amount.style.clone(),
                            cost: None,
                        },
                    });
                }
            }
        }
    }
    Ok(inferred)
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
    use super::super::test_support::{amount, price, txn, usd};
    use super::*;
    use crate::model::Cost;

    /// An amount carrying a per-unit (`@`) cost, for cost-inference tests.
    fn unit_cost(
        commodity: &str,
        mantissa: i128,
        places: u32,
        cost_commodity: &str,
        cost_mantissa: i128,
        cost_places: u32,
    ) -> Amount {
        let mut a = amount(commodity, mantissa, places);
        a.cost = Some(Box::new(Cost {
            kind: CostKind::Unit,
            amount: amount(cost_commodity, cost_mantissa, cost_places),
        }));
        a
    }

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

    #[test]
    fn exact_reciprocal_terminating_and_not() {
        assert_eq!(exact_reciprocal(Dec::new(5, 3)), Some(Dec::new(200, 0))); // 1/0.005
        assert_eq!(exact_reciprocal(Dec::new(2, 0)), Some(Dec::new(5, 1))); // 1/2 = 0.5
        assert_eq!(exact_reciprocal(Dec::new(4, 0)), Some(Dec::new(25, 2))); // 1/4 = 0.25
        assert_eq!(exact_reciprocal(Dec::new(8, 0)), Some(Dec::new(125, 3))); // 1/8 = 0.125
        assert_eq!(exact_reciprocal(Dec::new(3, 0)), None); // 1/3 never terminates
        assert_eq!(exact_reciprocal(Dec::new(22000, 2)), None); // 1/220 (factor 11)
        assert_eq!(exact_reciprocal(Dec::zero()), None);
    }

    fn gld_gift() -> Vec<crate::model::Transaction> {
        // The fixture's GLD gift: the GLD lot has no cost, the equity leg prices
        // $ in GLD (`$-1,000.00 @ 0.005 GLD`).
        vec![txn(
            2,
            "2025-08-20",
            vec![
                ("assets:broker:gld", vec![amount("GLD", 5, 0)]),
                (
                    "equity:transfers",
                    vec![unit_cost("$", -100_000, 2, "GLD", 5, 3)],
                ),
            ],
        )]
    }

    #[test]
    fn infers_forward_and_reverse_from_costs() {
        let mut txns = vec![txn(
            1,
            "2024-09-16",
            vec![
                (
                    "assets:broker",
                    vec![unit_cost("AAPL", 10, 0, "$", 22000, 2)],
                ),
                ("assets:cash", vec![usd(-220_000)]),
            ],
        )];
        txns.extend(gld_gift());
        let inferred = infer_market_prices(&txns).unwrap();

        // AAPL forward (1/220 does not terminate → no reverse), then the GLD
        // gift's $→GLD forward and its GLD→$ reverse. Journal order.
        assert_eq!(inferred.len(), 3);
        assert_eq!(inferred[0].commodity, c("AAPL"));
        assert_eq!(inferred[0].date, "2024-09-16");
        assert_eq!(inferred[0].price.commodity, c("$"));
        assert_eq!(inferred[0].price.quantity, Dec::new(22000, 2));
        assert_eq!(inferred[1].commodity, c("$"));
        assert_eq!(inferred[1].price.commodity, c("GLD"));
        assert_eq!(inferred[1].price.quantity, Dec::new(5, 3));
        assert_eq!(inferred[2].commodity, c("GLD"));
        assert_eq!(inferred[2].price.commodity, c("$"));
        assert_eq!(inferred[2].price.quantity, Dec::new(200, 0));
    }

    #[test]
    fn inferred_reverse_values_a_cost_denominator_commodity() {
        let db = PriceDb::build(&infer_market_prices(&gld_gift()).unwrap());
        let ma = MixedAmount::single(c("GLD"), Dec::new(5, 0));
        // 5 GLD × $200 (= 1/0.005) = $1000, exact.
        assert_eq!(
            value_at(&ma, &c("$"), &db, "2026-01-01", None).unwrap(),
            Dec::new(1000, 0)
        );
    }
}
