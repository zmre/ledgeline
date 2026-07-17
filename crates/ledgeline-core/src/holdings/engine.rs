//! Average-cost stock-holdings engine — port of
//! `web/src/lib/holdings/engine.ts`.
//!
//! One average-cost pool per symbol across the WHOLE scope (not per account): an
//! in-scope→in-scope transfer nets to zero shares and zero basis impact. Basis
//! is kept in the valuation base commodity; a cost-less acquisition lot taints
//! the pool (`basis = None`) — we never guess a basis from price directives.
//!
//! All money math is exact [`Dec`], reusing the same non-normalizing multiply
//! (`reports::prices::mul_raw`) as the valuation engine so the ported numbers
//! line up with the TS `domain/money` representation bit-for-bit. The only
//! rounding is the half-even sell reduction (`div_round_half_even`), matching the
//! TS `divRoundHalfEven`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use crate::decimal::{Dec, DecError};
use crate::model::{AccountDeclaration, Commodity, Cost, CostKind, PriceDirective, Transaction};
use crate::reports::prices::mul_raw;
use crate::reports::{PriceDb, ReportError, account_matches};
use crate::wire::{account_tag_map, inherited_account_tags};

use super::commodities::is_currency;
use super::types::{
    Holding, HoldingPrice, HoldingsReport, HoldingsScope, HoldingsTotals, HoldingsWarning,
    PriceSource, ScopeMode, WarningKind,
};

/// `10^exp` as an `i128`, checked for overflow (mirrors `decimal::pow10`, which
/// is private to that module).
fn pow10(exp: u32) -> Result<i128, DecError> {
    10i128.checked_pow(exp).ok_or(DecError::Overflow)
}

/// Rounded division, half-even (banker's rounding) — port of the TS
/// `divRoundHalfEven`. `domain/money` has no `Dec` division on purpose; this is
/// the one place holdings math needs it.
///
/// The denominator is always positive at every call site (a share count or a
/// `|qty|`); a zero denominator is unreachable and is surfaced as the same
/// never-unwrapped overflow arm rather than panicking.
fn div_round_half_even(numerator: i128, denominator: i128) -> Result<i128, ReportError> {
    if denominator == 0 {
        return Err(ReportError::Decimal(DecError::Overflow));
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

/// Rescale both operands to a common precision and return the mantissa pair
/// (port of the TS `commonMantissas`).
fn common_mantissas(a: Dec, b: Dec) -> Result<(i128, i128), ReportError> {
    let places = a.places.max(b.places);
    let scale = |x: Dec| -> Result<i128, ReportError> {
        let factor = pow10(places - x.places)?;
        Ok(x.mantissa.checked_mul(factor).ok_or(DecError::Overflow)?)
    };
    Ok((scale(a)?, scale(b)?))
}

/// Average-cost basis left after a sell: `basis × sharesAfter / sharesBefore`,
/// computed exactly on mantissas and rounded HALF-EVEN to the basis's own
/// precision (port of the TS `reduceBasis`).
fn reduce_basis(basis: Dec, shares_after: Dec, shares_before: Dec) -> Result<Dec, ReportError> {
    let (after_m, before_m) = common_mantissas(shares_after, shares_before)?;
    let numerator = basis
        .mantissa
        .checked_mul(after_m)
        .ok_or(DecError::Overflow)?;
    Ok(Dec::new(
        div_round_half_even(numerator, before_m)?,
        basis.places,
    ))
}

/// Per-unit price from a `@@` total: `total / |qty|`, rounded half-even to
/// `total.p + qty.p` decimal places (port of the TS `perUnitFromTotal`).
fn per_unit_from_total(total: Dec, qty: Dec) -> Result<Dec, ReportError> {
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

/// A dated per-unit price in the base commodity (port of the TS `DatedPrice`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct DatedPrice {
    qty: Dec,
    date: String,
}

/// Average-cost pool for one stock symbol. Only the fields consumed by
/// [`compute_holdings`] are tracked (the TS `costlessBuyTxns`/`negativeCrossTxn`/
/// `lastTxnIndex` feed the separate WP-10 check rules, which are out of scope
/// here).
struct SymbolPool {
    /// Net shares over processed postings (may be zero or negative).
    shares: Dec,
    /// Running basis in the base commodity; meaningful only when not `tainted`.
    basis: Dec,
    /// True once any acquisition lot lacked a usable cost.
    tainted: bool,
    /// Date the current position was opened (reset on each re-open); `None`
    /// until a buy is seen.
    first_basis_date: Option<String>,
    /// Accounts whose own net shares are `> 0`, sorted.
    accounts: Vec<String>,
    /// Latest `name:` tag seen — posting-comment tag first, then the account's
    /// own + ancestors' declared tag, then the txn tag — else the symbol.
    name: String,
}

impl SymbolPool {
    fn new(symbol: &str) -> Self {
        Self {
            shares: Dec::zero(),
            basis: Dec::zero(),
            tainted: false,
            first_basis_date: None,
            accounts: Vec::new(),
            name: symbol.to_string(),
        }
    }
}

/// One in-scope stock leg gathered from a posting.
struct LotEntry {
    qty: Dec,
    cost: Option<Cost>,
}

/// The value of the first `name` tag in `tags`, if present.
fn tag_value<'a>(tags: &'a [(String, String)], name: &str) -> Option<&'a str> {
    tags.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Journal order: date asc, then txn index asc (input order is never assumed).
fn journal_order(txns: &[Transaction]) -> Vec<&Transaction> {
    let mut ordered: Vec<&Transaction> = txns.iter().collect();
    ordered.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.index.0.cmp(&b.index.0)));
    ordered
}

/// A buy lot's cost in the base commodity, or `None` when it has none (or an
/// unconvertible one). Port of the TS `costInBase`.
fn cost_in_base(
    qty: Dec,
    cost: Option<&Cost>,
    db: &PriceDb,
    base: &Commodity,
    date: &str,
) -> Result<Option<Dec>, ReportError> {
    let Some(cost) = cost else {
        return Ok(None);
    };
    let own = if cost.kind == CostKind::Unit {
        mul_raw(qty, cost.amount.quantity)?
    } else {
        cost.amount.quantity
    };
    if cost.amount.commodity == *base {
        return Ok(Some(own));
    }
    match db.lookup_in(&cost.amount.commodity, base, date) {
        Some(rate) => Ok(Some(mul_raw(own, rate.quantity)?)),
        None => Ok(None),
    }
}

/// Build one average-cost pool per stock symbol from postings dated ≤ `as_of`
/// whose account passes `in_scope`. Port of the TS `buildPools`; see that
/// function's doc for the netting/taint/reduction rules.
fn build_pools(
    txns: &[Transaction],
    db: &PriceDb,
    base: &Commodity,
    as_of: &str,
    in_scope: &dyn Fn(&str) -> bool,
    account_tags: &HashMap<&str, &[(String, String)]>,
) -> Result<BTreeMap<String, SymbolPool>, ReportError> {
    let mut pools: BTreeMap<String, SymbolPool> = BTreeMap::new();
    // symbol -> account -> net shares.
    let mut per_account: BTreeMap<String, BTreeMap<String, Dec>> = BTreeMap::new();

    for txn in journal_order(txns) {
        if txn.date.as_str() > as_of {
            continue;
        }

        // Gather this txn's in-scope stock legs per symbol (posting order
        // preserved within each symbol's Vec; symbols are independent pools).
        let mut by_symbol: BTreeMap<String, Vec<LotEntry>> = BTreeMap::new();
        for posting in &txn.postings {
            if !in_scope(&posting.account.0) {
                continue;
            }
            for amount in &posting.amounts {
                if is_currency(&amount.commodity.0) {
                    continue;
                }
                let symbol = amount.commodity.0.clone();
                by_symbol.entry(symbol.clone()).or_default().push(LotEntry {
                    qty: amount.quantity,
                    cost: amount.cost.as_deref().cloned(),
                });

                // Ensure the pool exists; update its name and per-account tally.
                let pool = pools
                    .entry(symbol.clone())
                    .or_insert_with(|| SymbolPool::new(&symbol));
                // Precedence mirrors the wire `ptags`: the posting's own
                // `name:` comment tag, then the account's own + ancestors'
                // declared `name:` (most-specific first), then the txn `name:`.
                let name = tag_value(&posting.tags, "name")
                    .map(str::to_string)
                    .or_else(|| {
                        inherited_account_tags(&posting.account, account_tags)
                            .into_iter()
                            .find(|(key, _)| key == "name")
                            .map(|(_, value)| value)
                    })
                    .or_else(|| tag_value(&txn.tags, "name").map(str::to_string));
                if let Some(name) = name {
                    pool.name = name;
                }
                let accounts = per_account.entry(symbol).or_default();
                let updated = match accounts.get(&posting.account.0) {
                    Some(prev) => prev.add(amount.quantity)?,
                    None => amount.quantity,
                };
                accounts.insert(posting.account.0.clone(), updated);
            }
        }

        for (symbol, entries) in &by_symbol {
            let mut net = Dec::zero();
            for entry in entries {
                net = net.add(entry.qty)?;
            }
            if net.is_zero() {
                continue; // pure transfer within scope: zero shares, zero basis impact
            }
            let Some(pool) = pools.get_mut(symbol) else {
                continue; // unreachable: gathered above
            };
            for entry in entries {
                let leg_before = pool.shares;
                let leg_after = leg_before.add(entry.qty)?;
                if entry.qty.mantissa > 0 {
                    if leg_before.mantissa <= 0 {
                        // (re)opening the position resets its basis date
                        pool.first_basis_date = Some(txn.date.clone());
                    }
                    match cost_in_base(entry.qty, entry.cost.as_ref(), db, base, &txn.date)? {
                        None => pool.tainted = true,
                        Some(lot_cost) => pool.basis = pool.basis.add(lot_cost)?,
                    }
                } else if entry.qty.mantissa < 0 && leg_before.mantissa > 0 {
                    pool.basis = if leg_after.mantissa >= 0 {
                        reduce_basis(pool.basis, leg_after, leg_before)?
                    } else {
                        Dec::new(0, pool.basis.places)
                    };
                }
                pool.shares = leg_after;
            }
        }
    }

    for (symbol, accounts) in &per_account {
        if let Some(pool) = pools.get_mut(symbol) {
            // BTreeMap key order is lexical, matching the TS explicit `.sort()`.
            pool.accounts = accounts
                .iter()
                .filter(|(_, shares)| shares.mantissa > 0)
                .map(|(account, _)| account.clone())
                .collect();
        }
    }
    Ok(pools)
}

/// Latest `P` directive ≤ `as_of` pricing `symbol` directly in `base` (ties: last
/// declared wins), with its date. Port of the TS `latestDirectivePrice` (scans
/// the raw directive list so it can return the date, unlike `PriceDb::lookup_in`).
fn latest_directive_price(
    prices: &[PriceDirective],
    symbol: &str,
    base: &str,
    as_of: &str,
) -> Option<DatedPrice> {
    let mut best: Option<DatedPrice> = None;
    for directive in prices {
        if directive.commodity.0 != symbol
            || directive.price.commodity.0 != base
            || directive.date.as_str() > as_of
        {
            continue;
        }
        let take = match &best {
            None => true,
            Some(current) => directive.date.as_str() >= current.date.as_str(),
        };
        if take {
            best = Some(DatedPrice {
                qty: directive.price.quantity,
                date: directive.date.clone(),
            });
        }
    }
    best
}

/// Per symbol, the latest cost annotation ≤ `as_of` usable as a base-commodity
/// price — scanned across the WHOLE journal (not just in-scope), buys and sells
/// alike. Port of the TS `latestCostPrices`.
fn latest_cost_prices(
    txns: &[Transaction],
    db: &PriceDb,
    base: &Commodity,
    as_of: &str,
) -> Result<BTreeMap<String, DatedPrice>, ReportError> {
    let mut latest: BTreeMap<String, DatedPrice> = BTreeMap::new();
    for txn in journal_order(txns) {
        if txn.date.as_str() > as_of {
            continue;
        }
        for posting in &txn.postings {
            for amount in &posting.amounts {
                let Some(cost) = amount.cost.as_deref() else {
                    continue;
                };
                if is_currency(&amount.commodity.0) || amount.quantity.is_zero() {
                    continue;
                }
                let per_unit = if cost.kind == CostKind::Unit {
                    cost.amount.quantity
                } else {
                    per_unit_from_total(cost.amount.quantity, amount.quantity)?
                };
                let in_base = if cost.amount.commodity == *base {
                    per_unit
                } else {
                    match db.lookup_in(&cost.amount.commodity, base, &txn.date) {
                        Some(rate) => mul_raw(per_unit, rate.quantity)?,
                        None => continue,
                    }
                };
                latest.insert(
                    amount.commodity.0.clone(),
                    DatedPrice {
                        qty: in_base,
                        date: txn.date.clone(),
                    },
                );
            }
        }
    }
    Ok(latest)
}

/// Account predicate for a scope: `Include` + empty set = everything;
/// `account_matches` subtree semantics. Port of the TS `scopePredicate`.
fn scope_predicate(scope: &HoldingsScope) -> impl Fn(&str) -> bool + '_ {
    let selected: Vec<&str> = scope.accounts.iter().map(String::as_str).collect();
    move |account: &str| {
        let matches = selected.iter().any(|&sel| account_matches(sel, account));
        match scope.mode {
            ScopeMode::Include => selected.is_empty() || matches,
            ScopeMode::Exclude => !matches,
        }
    }
}

/// `gain / basis × 100` as a display-boundary `f64`, or `None` when basis is zero.
fn gain_pct(gain: Dec, basis: Dec) -> Option<f64> {
    if basis.is_zero() {
        None
    } else {
        Some((gain.floating_point() / basis.floating_point()) * 100.0)
    }
}

/// Stock holdings, average-cost basis, prices, and gains for the scoped journal
/// as of `scope.as_of`. Port of the TS `computeHoldings`.
///
/// # Errors
/// Returns [`ReportError`] on decimal overflow (unreachable for realistic
/// journals, but never unwrapped).
pub fn compute_holdings(
    txns: &[Transaction],
    prices: &[PriceDirective],
    accounts: &[AccountDeclaration],
    scope: &HoldingsScope,
) -> Result<HoldingsReport, ReportError> {
    let db = PriceDb::build(prices);
    let base_commodity = db
        .base_commodity()
        .cloned()
        .unwrap_or_else(|| Commodity("$".to_string()));
    let base = base_commodity.0.clone();
    let predicate = scope_predicate(scope);
    let account_tags = account_tag_map(accounts);
    let pools = build_pools(
        txns,
        &db,
        &base_commodity,
        &scope.as_of,
        &predicate,
        &account_tags,
    )?;
    let cost_prices = latest_cost_prices(txns, &db, &base_commodity, &scope.as_of)?;

    let mut holdings: Vec<Holding> = Vec::new();
    let mut warnings: Vec<HoldingsWarning> = Vec::new();
    // A BTreeMap iterates in symbol order — matches the TS explicit symbol sort.
    for (symbol, pool) in &pools {
        if pool.shares.is_zero() {
            continue; // fully sold: dropped silently
        }
        if pool.shares.mantissa < 0 {
            warnings.push(HoldingsWarning {
                symbol: symbol.clone(),
                kind: WarningKind::NegativeShares,
                message: format!(
                    "{symbol}: net shares are negative — the opening position was likely never entered; row hidden"
                ),
            });
            continue;
        }

        let price = match latest_directive_price(prices, symbol, &base, &scope.as_of) {
            Some(directive) => Some(HoldingPrice {
                qty: directive.qty,
                date: directive.date,
                source: PriceSource::Directive,
            }),
            None => cost_prices.get(symbol).map(|cost| HoldingPrice {
                qty: cost.qty,
                date: cost.date.clone(),
                source: PriceSource::Cost,
            }),
        };
        if price.is_none() {
            warnings.push(HoldingsWarning {
                symbol: symbol.clone(),
                kind: WarningKind::Unpriced,
                message: format!(
                    "{symbol}: no market price or usable cost annotation — excluded from totals"
                ),
            });
        }
        if pool.tainted {
            warnings.push(HoldingsWarning {
                symbol: symbol.clone(),
                kind: WarningKind::MissingBasis,
                message: format!("{symbol}: acquired without a cost annotation — basis unknown"),
            });
        }

        let basis = if pool.tainted { None } else { Some(pool.basis) };
        let market_value = match &price {
            Some(p) => Some(mul_raw(pool.shares, p.qty)?),
            None => None,
        };
        let gain = match (market_value, basis) {
            (Some(mv), Some(b)) => Some(mv.sub(b)?),
            _ => None,
        };
        let pct = match (gain, basis) {
            (Some(g), Some(b)) => gain_pct(g, b),
            _ => None,
        };
        holdings.push(Holding {
            symbol: symbol.clone(),
            name: pool.name.clone(),
            accounts: pool.accounts.clone(),
            shares: pool.shares,
            basis,
            first_basis_date: pool.first_basis_date.clone(),
            price,
            market_value,
            gain,
            gain_pct: pct,
        });
    }

    // Market value desc; unpriced last; ties (and unpriced) by symbol asc.
    holdings.sort_by(|a, b| match (a.market_value, b.market_value) {
        (None, None) => a.symbol.cmp(&b.symbol),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(av), Some(bv)) => bv.cmp(&av).then_with(|| a.symbol.cmp(&b.symbol)),
    });

    // Totals refuse (None) when any included holding is tainted or unpriced.
    let mut market_value = Dec::zero();
    let mut basis_total: Option<Dec> = Some(Dec::zero());
    for holding in &holdings {
        if let Some(mv) = holding.market_value {
            market_value = market_value.add(mv)?;
        }
        basis_total = match (basis_total, holding.basis, holding.market_value) {
            (Some(bt), Some(b), Some(_)) => Some(bt.add(b)?),
            _ => None,
        };
    }
    let gain_total = match basis_total {
        Some(bt) => Some(market_value.sub(bt)?),
        None => None,
    };
    let gain_pct_total = match (gain_total, basis_total) {
        (Some(g), Some(bt)) => gain_pct(g, bt),
        _ => None,
    };

    // Only real signs: gainers gain_pct > 0 (desc), losers gain_pct < 0 (asc).
    // Filtering the already-MV-sorted list + a stable sort matches the TS
    // tie-ordering.
    let mut top_gainers: Vec<Holding> = holdings
        .iter()
        .filter(|h| h.gain_pct.is_some_and(|p| p > 0.0))
        .cloned()
        .collect();
    top_gainers.sort_by(|a, b| {
        b.gain_pct
            .partial_cmp(&a.gain_pct)
            .unwrap_or(Ordering::Equal)
    });
    top_gainers.truncate(5);

    let mut top_losers: Vec<Holding> = holdings
        .iter()
        .filter(|h| h.gain_pct.is_some_and(|p| p < 0.0))
        .cloned()
        .collect();
    top_losers.sort_by(|a, b| {
        a.gain_pct
            .partial_cmp(&b.gain_pct)
            .unwrap_or(Ordering::Equal)
    });
    top_losers.truncate(5);

    Ok(HoldingsReport {
        as_of: scope.as_of.clone(),
        base,
        holdings,
        totals: HoldingsTotals {
            market_value,
            basis: basis_total,
            gain: gain_total,
            gain_pct: gain_pct_total,
        },
        top_gainers,
        top_losers,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdings::test_helpers::{
        account_decl, amt, buy, buy_no_cost, pd, posting, scope, sell, txn, usd, with_cost,
    };
    use crate::holdings::types::HoldingsReport;

    fn only<'a>(report: &'a HoldingsReport, symbol: &str) -> &'a Holding {
        report
            .holdings
            .iter()
            .find(|h| h.symbol == symbol)
            .unwrap_or_else(|| panic!("holding {symbol} should exist"))
    }

    fn run(txns: &[Transaction], prices: &[PriceDirective], sc: &HoldingsScope) -> HoldingsReport {
        compute_holdings(txns, prices, &[], sc).expect("compute_holdings succeeds")
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // ---- average-cost basis ----

    #[test]
    fn accumulates_per_unit_buys_and_reduces_partial_sell() {
        // Deliberately out of journal order: the engine sorts by date, then index.
        let txns = [
            txn(
                3,
                "2025-03-10",
                vec![
                    sell("assets:broker:vti", "VTI", 5),
                    posting("assets:broker:cash", vec![usd(115_000)], &[]),
                ],
                &[],
            ),
            txn(
                1,
                "2025-01-10",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 20000, true),
                    posting("assets:broker:cash", vec![usd(-200_000)], &[]),
                ],
                &[],
            ),
            txn(
                2,
                "2025-02-10",
                vec![
                    buy("assets:broker:vti", "VTI", 10, 22000, true),
                    posting("assets:broker:cash", vec![usd(-220_000)], &[]),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-04-01", "VTI", 25000, "$")],
            &scope("2025-04-30", ScopeMode::Include, &[]),
        );

        assert_eq!(report.base, "$");
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(15, 0));
        assert_eq!(vti.basis, Some(Dec::new(315_000, 2))); // (2000 + 2200) × 15/20, exact
        let price = vti.price.as_ref().expect("VTI priced");
        assert_eq!(price.date, "2025-04-01");
        assert_eq!(price.source, PriceSource::Directive);
        assert_eq!(price.qty, Dec::new(25000, 2));
        assert_eq!(vti.market_value, Some(Dec::new(3750, 0)));
        assert_eq!(vti.gain, Some(Dec::new(600, 0)));
        assert!(close(vti.gain_pct.unwrap(), (600.0 / 3150.0) * 100.0));
        assert_eq!(vti.accounts, vec!["assets:broker:vti".to_string()]);
        assert_eq!(report.totals.market_value, Dec::new(3750, 0));
        assert_eq!(report.totals.basis, Some(Dec::new(3150, 0)));
        assert_eq!(report.totals.gain, Some(Dec::new(600, 0)));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn handles_total_cost_buys() {
        let txns = [txn(
            1,
            "2025-01-10",
            vec![buy("assets:broker", "VTI", 4, 85000, false)], // 4 VTI @@ $850.00
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-03-01", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.basis, Some(Dec::new(85000, 2)));
        assert_eq!(vti.market_value, Some(Dec::new(1000, 0)));
        assert_eq!(vti.gain, Some(Dec::new(150, 0)));
    }

    #[test]
    fn rounds_sell_reductions_half_even() {
        // 2 @@ $1.01 → sell 1 → 0.505 rounds to 0.50 (even); @@ $1.03 → 0.515 → 0.52.
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "EEE", 2, 101, false)], &[]),
            txn(2, "2025-01-10", vec![buy("a", "OOO", 2, 103, false)], &[]),
            txn(
                3,
                "2025-02-10",
                vec![sell("a", "EEE", 1), sell("a", "OOO", 1)],
                &[],
            ),
        ];
        let report = run(&txns, &[], &scope("2025-03-01", ScopeMode::Include, &[]));
        assert_eq!(only(&report, "EEE").basis, Some(Dec::new(50, 2)));
        assert_eq!(only(&report, "OOO").basis, Some(Dec::new(52, 2)));
    }

    // ---- scoping ----

    fn two_accounts() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker:a", "VTI", 10, 20000, true)],
                &[],
            ),
            txn(
                2,
                "2025-01-20",
                vec![buy("assets:broker:b", "VTI", 5, 21000, true)],
                &[],
            ),
            txn(
                3,
                "2025-01-25",
                vec![buy("assets:other:c", "VTI", 2, 22000, true)],
                &[],
            ),
        ]
    }

    #[test]
    fn include_empty_set_means_all_accounts() {
        let txns = two_accounts();
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(17, 0));
        assert_eq!(
            vti.accounts,
            vec![
                "assets:broker:a".to_string(),
                "assets:broker:b".to_string(),
                "assets:other:c".to_string()
            ]
        );
    }

    #[test]
    fn include_matches_whole_subtrees() {
        let txns = two_accounts();
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &["assets:broker"]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(15, 0));
        assert_eq!(vti.basis, Some(Dec::new(3050, 0)));
        assert_eq!(
            vti.accounts,
            vec!["assets:broker:a".to_string(), "assets:broker:b".to_string()]
        );
    }

    #[test]
    fn exclude_removes_selected_subtrees_only() {
        let txns = two_accounts();
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Exclude, &["assets:broker:b"]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(12, 0));
        assert_eq!(vti.basis, Some(Dec::new(2440, 0)));
        assert_eq!(
            vti.accounts,
            vec!["assets:broker:a".to_string(), "assets:other:c".to_string()]
        );
    }

    #[test]
    fn in_scope_transfer_nets_to_zero_and_leaves_basis_untouched() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker:a", "VTI", 10, 20000, true)],
                &[],
            ),
            txn(
                2,
                "2025-02-10",
                vec![
                    sell("assets:broker:a", "VTI", 4),
                    buy_no_cost("assets:broker:b", "VTI", 4),
                ],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let vti = only(&report, "VTI");
        assert_eq!(vti.shares, Dec::new(10, 0));
        // the cost-less incoming leg must NOT taint the pool
        assert_eq!(vti.basis, Some(Dec::new(200_000, 2)));
        assert_eq!(
            vti.accounts,
            vec!["assets:broker:a".to_string(), "assets:broker:b".to_string()]
        );
    }

    // ---- taint and pricing ----

    #[test]
    fn costless_buy_taints_the_pool() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![buy_no_cost("assets:broker", "GLD", 10)],
                &[],
            ),
            txn(
                2,
                "2025-01-20",
                vec![buy("assets:broker", "VTI", 10, 20000, true)],
                &[],
            ),
        ];
        let prices = [
            pd("2025-02-01", "GLD", 18000, "$"),
            pd("2025-02-01", "VTI", 22000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );

        let gld = only(&report, "GLD");
        assert_eq!(gld.basis, None);
        assert_eq!(gld.gain, None);
        assert_eq!(gld.gain_pct, None);
        assert_eq!(gld.market_value, Some(Dec::new(1800, 0))); // priced via directive despite taint
        assert_eq!(
            report.warnings,
            vec![HoldingsWarning {
                symbol: "GLD".to_string(),
                kind: WarningKind::MissingBasis,
                message: report.warnings[0].message.clone(),
            }]
        );
        assert!(report.warnings[0].message.contains("GLD"));
        assert_eq!(report.totals.market_value, Dec::new(4000, 0));
        assert_eq!(report.totals.basis, None);
        assert_eq!(report.totals.gain, None);
        assert_eq!(report.totals.gain_pct, None);
    }

    #[test]
    fn non_base_cost_converts_via_directive_else_taints() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![posting(
                    "a",
                    vec![with_cost(amt("VTI", 10, 0), 10000, true, "EUR")],
                    &[],
                )],
                &[],
            ), // 10 VTI @ €100
            txn(
                2,
                "2025-01-10",
                vec![posting(
                    "a",
                    vec![with_cost(amt("XYZ", 10, 0), 10000, true, "GBP")],
                    &[],
                )],
                &[],
            ), // no GBP→$ price: taint
        ];
        let prices = [
            pd("2025-01-01", "EUR", 110, "$"),
            pd("2025-02-01", "VTI", 15000, "$"),
            pd("2025-02-01", "XYZ", 15000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(only(&report, "VTI").basis, Some(Dec::new(11_000_000, 4))); // €1000 × 1.10
        assert_eq!(only(&report, "XYZ").basis, None);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].symbol, "XYZ");
        assert_eq!(report.warnings[0].kind, WarningKind::MissingBasis);
    }

    #[test]
    fn falls_back_to_latest_cost_annotation_as_price() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker", "XXX", 10, 5000, true)],
                &[],
            ), // @ $50
            txn(
                2,
                "2025-03-01",
                vec![buy("assets:broker", "XXX", 4, 26000, false)],
                &[],
            ), // @@ $260 → $65/share
        ];
        let report = run(&txns, &[], &scope("2025-06-30", ScopeMode::Include, &[]));
        let xxx = only(&report, "XXX");
        let price = xxx.price.as_ref().expect("XXX priced from cost");
        assert_eq!(price.date, "2025-03-01");
        assert_eq!(price.source, PriceSource::Cost);
        assert_eq!(price.qty, Dec::new(65, 0));
        assert_eq!(xxx.shares, Dec::new(14, 0));
        assert_eq!(xxx.basis, Some(Dec::new(760, 0)));
        assert_eq!(xxx.market_value, Some(Dec::new(910, 0)));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn excludes_unpriced_from_totals_and_sorts_them_last() {
        let txns = [
            txn(
                1,
                "2025-01-10",
                vec![buy("assets:broker", "VTI", 10, 20000, true)],
                &[],
            ),
            txn(
                2,
                "2025-01-20",
                vec![buy_no_cost("assets:broker", "NOP", 3)],
                &[],
            ),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 22000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );

        let symbols: Vec<&str> = report.holdings.iter().map(|h| h.symbol.as_str()).collect();
        assert_eq!(symbols, ["VTI", "NOP"]);
        let nop = only(&report, "NOP");
        assert_eq!(nop.price, None);
        assert_eq!(nop.market_value, None);
        assert_eq!(report.totals.market_value, Dec::new(2200, 0));
        assert_eq!(report.totals.basis, None);
        let kinds: Vec<(&str, WarningKind)> = report
            .warnings
            .iter()
            .map(|w| (w.symbol.as_str(), w.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("NOP", WarningKind::Unpriced),
                ("NOP", WarningKind::MissingBasis)
            ]
        );
    }

    // ---- firstBasisDate ----

    #[test]
    fn first_basis_date_simple_buy() {
        let txns = [txn(
            1,
            "2025-01-10",
            vec![buy("a", "VTI", 10, 20000, true)],
            &[],
        )];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            only(&report, "VTI").first_basis_date.as_deref(),
            Some("2025-01-10")
        );
    }

    #[test]
    fn first_basis_date_resets_on_rebuy() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "VTI", 10, 20000, true)], &[]),
            txn(2, "2025-02-10", vec![sell("a", "VTI", 10)], &[]),
            txn(3, "2025-03-10", vec![buy("a", "VTI", 4, 21000, true)], &[]),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            only(&report, "VTI").first_basis_date.as_deref(),
            Some("2025-03-10")
        );
    }

    #[test]
    fn first_basis_date_partial_sell_keeps_original() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "VTI", 10, 20000, true)], &[]),
            txn(2, "2025-02-10", vec![sell("a", "VTI", 4)], &[]),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            only(&report, "VTI").first_basis_date.as_deref(),
            Some("2025-01-10")
        );
    }

    #[test]
    fn first_basis_date_buy_more_keeps_earliest() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "VTI", 10, 20000, true)], &[]),
            txn(2, "2025-02-10", vec![buy("a", "VTI", 5, 22000, true)], &[]),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 25000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert_eq!(
            only(&report, "VTI").first_basis_date.as_deref(),
            Some("2025-01-10")
        );
    }

    // ---- row filtering ----

    #[test]
    fn drops_fully_sold_symbol_silently() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "VTI", 10, 20000, true)], &[]),
            txn(2, "2025-02-10", vec![sell("a", "VTI", 10)], &[]),
        ];
        let report = run(
            &txns,
            &[pd("2025-02-01", "VTI", 22000, "$")],
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        assert!(report.holdings.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn drops_negative_pool_with_warning() {
        let txns = [txn(1, "2025-01-10", vec![sell("a", "SHT", 5)], &[])];
        let report = run(&txns, &[], &scope("2025-06-30", ScopeMode::Include, &[]));
        assert!(report.holdings.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].symbol, "SHT");
        assert_eq!(report.warnings[0].kind, WarningKind::NegativeShares);
        assert!(report.warnings[0].message.contains("never entered"));
    }

    // ---- asOf time travel ----

    fn aapl_txns() -> Vec<Transaction> {
        vec![
            txn(
                1,
                "2025-01-05",
                vec![posting(
                    "assets:broker",
                    vec![with_cost(amt("AAPL", 10, 0), 10000, true, "$")],
                    &[("name", "Apple Inc.")],
                )],
                &[],
            ),
            txn(
                2,
                "2025-06-05",
                vec![posting(
                    "assets:broker",
                    vec![with_cost(amt("AAPL", 10, 0), 12000, true, "$")],
                    &[],
                )],
                &[("name", "Apple Computer")],
            ),
        ]
    }

    #[test]
    fn early_asof_sees_first_lot_price_and_name() {
        let txns = aapl_txns();
        let prices = [
            pd("2025-01-15", "AAPL", 11000, "$"),
            pd("2025-07-01", "AAPL", 15000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-03-01", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(10, 0));
        assert_eq!(aapl.basis, Some(Dec::new(1000, 0)));
        assert_eq!(aapl.price.as_ref().unwrap().date, "2025-01-15");
        assert_eq!(aapl.price.as_ref().unwrap().qty, Dec::new(11000, 2));
        assert_eq!(aapl.name, "Apple Inc.");
    }

    #[test]
    fn late_asof_sees_both_lots_newer_price_and_txn_name() {
        let txns = aapl_txns();
        let prices = [
            pd("2025-01-15", "AAPL", 11000, "$"),
            pd("2025-07-01", "AAPL", 15000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-12-31", ScopeMode::Include, &[]),
        );
        let aapl = only(&report, "AAPL");
        assert_eq!(aapl.shares, Dec::new(20, 0));
        assert_eq!(aapl.basis, Some(Dec::new(2200, 0)));
        assert_eq!(aapl.price.as_ref().unwrap().date, "2025-07-01");
        assert_eq!(aapl.price.as_ref().unwrap().qty, Dec::new(15000, 2));
        assert_eq!(aapl.name, "Apple Computer");
    }

    // ---- name resolution: inherited account-directive tags ----

    fn aapl_buy() -> Vec<Transaction> {
        vec![txn(
            1,
            "2024-01-01",
            vec![buy("assets:broker:aapl", "AAPL", 10, 22000, true)],
            &[],
        )]
    }

    fn aapl_prices() -> Vec<PriceDirective> {
        vec![pd("2024-02-01", "AAPL", 22500, "$")]
    }

    #[test]
    fn account_directive_name_used_when_no_posting_or_txn_name() {
        // The repro: the leaf account declares the name; nothing else does.
        let decls = [account_decl(
            "assets:broker:aapl",
            &[("name", "Apple Inc.")],
        )];
        let report = compute_holdings(
            &aapl_buy(),
            &aapl_prices(),
            &decls,
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "AAPL").name, "Apple Inc.");
    }

    #[test]
    fn posting_comment_name_wins_over_account_directive_name() {
        let txns = [txn(
            1,
            "2024-01-01",
            vec![posting(
                "assets:broker:aapl",
                vec![with_cost(amt("AAPL", 10, 0), 22000, true, "$")],
                &[("name", "Posting Wins")],
            )],
            &[],
        )];
        let decls = [account_decl(
            "assets:broker:aapl",
            &[("name", "Apple Inc.")],
        )];
        let report = compute_holdings(
            &txns,
            &aapl_prices(),
            &decls,
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "AAPL").name, "Posting Wins");
    }

    #[test]
    fn account_directive_name_wins_over_txn_name() {
        // Precedence check for the middle rung: account beats a txn-level name.
        let txns = [txn(
            1,
            "2024-01-01",
            vec![buy("assets:broker:aapl", "AAPL", 10, 22000, true)],
            &[("name", "Txn Name")],
        )];
        let decls = [account_decl(
            "assets:broker:aapl",
            &[("name", "Apple Inc.")],
        )];
        let report = compute_holdings(
            &txns,
            &aapl_prices(),
            &decls,
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "AAPL").name, "Apple Inc.");
    }

    #[test]
    fn ancestor_account_name_is_inherited_by_child_with_none() {
        // Only the ANCESTOR `assets:broker` declares a name; the posted leaf
        // `assets:broker:aapl` has no declaration of its own.
        let decls = [account_decl(
            "assets:broker",
            &[("name", "Broker Holdings")],
        )];
        let report = compute_holdings(
            &aapl_buy(),
            &aapl_prices(),
            &decls,
            &scope("2024-12-31", ScopeMode::Include, &[]),
        )
        .expect("compute_holdings succeeds");
        assert_eq!(only(&report, "AAPL").name, "Broker Holdings");
    }

    // ---- gainers and losers ----

    #[test]
    fn splits_gainers_and_losers_and_caps_at_five() {
        // All bought at $100/share: G1 +60% … G6 +10%, L1 -30% L2 -20% L3 -10%,
        // Z0 flat, T0 tainted (gain_pct None).
        let priced: [(&str, i128); 10] = [
            ("G1", 16000),
            ("G2", 15000),
            ("G3", 14000),
            ("G4", 13000),
            ("G5", 12000),
            ("G6", 11000),
            ("L1", 7000),
            ("L2", 8000),
            ("L3", 9000),
            ("Z0", 10000),
        ];
        let mut txns: Vec<Transaction> = priced
            .iter()
            .enumerate()
            .map(|(i, (symbol, _))| {
                #[allow(clippy::cast_possible_truncation)]
                let index = (i + 1) as u32;
                txn(
                    index,
                    "2025-01-10",
                    vec![buy("a", symbol, 1, 10000, true)],
                    &[],
                )
            })
            .collect();
        #[allow(clippy::cast_possible_truncation)]
        let last = (priced.len() + 1) as u32;
        txns.push(txn(
            last,
            "2025-01-10",
            vec![buy_no_cost("a", "T0", 1)],
            &[],
        ));
        let mut prices: Vec<PriceDirective> = priced
            .iter()
            .map(|(symbol, cents)| pd("2025-02-01", symbol, *cents, "$"))
            .collect();
        prices.push(pd("2025-02-01", "T0", 99900, "$"));

        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let gainers: Vec<&str> = report
            .top_gainers
            .iter()
            .map(|h| h.symbol.as_str())
            .collect();
        let losers: Vec<&str> = report
            .top_losers
            .iter()
            .map(|h| h.symbol.as_str())
            .collect();
        assert_eq!(gainers, ["G1", "G2", "G3", "G4", "G5"]); // > 0 only, desc, G6 capped off
        assert_eq!(losers, ["L1", "L2", "L3"]); // < 0 only, asc — Z0 and T0 in neither
    }

    #[test]
    fn empty_losers_when_everything_gained() {
        let txns = [
            txn(1, "2025-01-10", vec![buy("a", "AAA", 1, 10000, true)], &[]),
            txn(2, "2025-01-10", vec![buy("a", "BBB", 1, 10000, true)], &[]),
        ];
        let prices = [
            pd("2025-02-01", "AAA", 12000, "$"),
            pd("2025-02-01", "BBB", 11000, "$"),
        ];
        let report = run(
            &txns,
            &prices,
            &scope("2025-06-30", ScopeMode::Include, &[]),
        );
        let gainers: Vec<&str> = report
            .top_gainers
            .iter()
            .map(|h| h.symbol.as_str())
            .collect();
        assert_eq!(gainers, ["AAA", "BBB"]);
        assert!(report.top_losers.is_empty());
    }
}
