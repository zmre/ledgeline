// Stock-holdings engine (WP-10). Pure TS: no Svelte/DOM imports — ports to
// Rust later, and mapping computeHoldings over a date series gives the
// post-MVP holdings-over-time chart for free.
//
// One average-cost pool per symbol across the whole scope (NOT per account):
// an in-scope→in-scope transfer nets to zero shares and zero basis impact.
// Basis is kept in the valuation base commodity; a cost-less acquisition lot
// taints the pool (basis: null) — we never guess a basis from price directives.

import {accountMatches} from "../domain/accounts";
import {add, cmp, dec, isZero, mul, sub, toNumber, type Dec} from "../domain/money";
import type {ISODate, PriceDirective, Transaction} from "../domain/types";
import {buildPriceDb, type PriceDb} from "../reports/prices";
import {isCurrency} from "./commodities";
import type {Holding, HoldingsReport, HoldingsScope, HoldingsWarning} from "./types";

/** A dated per-unit price in the base commodity. */
export interface DatedPrice {
    qty: Dec;
    date: ISODate;
}

/**
 * Rounded bigint division, half-even (banker's rounding). domain/money has no
 * Dec division on purpose — this is the one place holdings math needs it, and
 * every use documents its target precision.
 */
function divRoundHalfEven(numerator: bigint, denominator: bigint): bigint {
    if (denominator === 0n) throw new RangeError("divRoundHalfEven: division by zero");
    const negative = numerator < 0n !== denominator < 0n;
    const n = numerator < 0n ? -numerator : numerator;
    const d = denominator < 0n ? -denominator : denominator;
    let q = n / d;
    const r = n % d;
    const twice = r * 2n;
    if (twice > d || (twice === d && q % 2n === 1n)) q += 1n;
    return negative ? -q : q;
}

/** Rescale both operands to a common precision and return the mantissa pair. */
function commonMantissas(a: Dec, b: Dec): [bigint, bigint] {
    const p = Math.max(a.p, b.p);
    const scale = (x: Dec): bigint => x.m * 10n ** BigInt(p - x.p);
    return [scale(a), scale(b)];
}

/**
 * Average-cost basis left after a sell: basis × sharesAfter / sharesBefore,
 * computed exactly on mantissas and rounded HALF-EVEN to the basis's own
 * precision (the only rounding in the pool math; format-time display rounding
 * is separate and unchanged).
 */
function reduceBasis(basis: Dec, sharesAfter: Dec, sharesBefore: Dec): Dec {
    const [afterM, beforeM] = commonMantissas(sharesAfter, sharesBefore);
    return {m: divRoundHalfEven(basis.m * afterM, beforeM), p: basis.p};
}

/**
 * Per-unit price from a `@@` total: total / |qty|, rounded half-even to
 * total.p + qty.p decimal places (enough that shares × price round-trips the
 * total to within half an ulp).
 */
function perUnitFromTotal(total: Dec, qty: Dec): Dec {
    const p = total.p + qty.p;
    const scaledTotal = total.m * 10n ** BigInt(2 * qty.p);
    const absQty = qty.m < 0n ? -qty.m : qty.m;
    return {m: divRoundHalfEven(scaledTotal, absQty), p};
}

/** Average-cost pool for one stock symbol — the shared substrate for computeHoldings and the WP-10 check rules. */
export interface SymbolPool {
    symbol: string;
    /** Net shares over processed postings (may be zero or negative). */
    shares: Dec;
    /** Running basis in the base commodity; meaningful only when `tainted` is false. */
    basis: Dec;
    /** True once any acquisition lot lacked a usable cost. */
    tainted: boolean;
    /** Txn indices with ≥1 cost-less acquisition lot, journal order, deduped. */
    costlessBuyTxns: number[];
    /** Most recent txn that took the running share total negative, if any. */
    negativeCrossTxn: number | null;
    /** Accounts whose own net shares are > 0, sorted. */
    accounts: string[];
    /** Latest `name:` tag seen (posting tags first, then txn tags), else the symbol. */
    name: string;
    /** Latest txn touching the symbol. */
    lastTxnIndex: number;
}

function tagValue(tags: [string, string][], name: string): string | null {
    for (const [key, value] of tags) {
        if (key === name) return value;
    }
    return null;
}

/** Journal order: date asc, then txn index asc — sorted explicitly, input order is never assumed. */
function journalOrder(txns: Transaction[]): Transaction[] {
    return [...txns].sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : a.index - b.index));
}

interface LotEntry {
    account: string;
    qty: Dec;
    cost?: {commodity: string; qty: Dec; per: boolean};
}

/**
 * Build one average-cost pool per stock symbol from postings dated ≤ asOf
 * whose account passes `inScope`. Within a transaction the in-scope legs of a
 * symbol are netted FIRST: a net of zero is a transfer between own accounts —
 * no share or basis impact, so cost-less transfer legs never taint the pool.
 * Otherwise legs apply in posting order: buys with a cost annotation add cost
 * in the base commodity (`@` per-unit multiplies, `@@` is the total; non-base
 * costs convert via the latest direct P directive at the txn date, else the
 * lot is treated as cost-less and taints the pool); sells reduce basis
 * proportionally at average cost. Overselling clamps basis to zero, and a
 * sell from a non-positive position leaves basis untouched (there is no
 * average cost to apply).
 */
export function buildPools(txns: Transaction[], db: PriceDb, base: string, asOf: ISODate, inScope: (account: string) => boolean): Map<string, SymbolPool> {
    const pools = new Map<string, SymbolPool>();
    const perAccount = new Map<string, Map<string, Dec>>();

    const poolFor = (symbol: string, txnIndex: number): SymbolPool => {
        let pool = pools.get(symbol);
        if (pool === undefined) {
            pool = {
                symbol,
                shares: dec(0n, 0),
                basis: dec(0n, 0),
                tainted: false,
                costlessBuyTxns: [],
                negativeCrossTxn: null,
                accounts: [],
                name: symbol,
                lastTxnIndex: txnIndex,
            };
            pools.set(symbol, pool);
            perAccount.set(symbol, new Map());
        }
        return pool;
    };

    for (const txn of journalOrder(txns)) {
        if (txn.date > asOf) continue;

        // Gather this txn's in-scope stock legs per symbol (posting order preserved).
        const bySymbol = new Map<string, LotEntry[]>();
        for (const posting of txn.postings) {
            if (!inScope(posting.account)) continue;
            for (const amount of posting.amounts) {
                if (isCurrency(amount.commodity)) continue;
                const entries = bySymbol.get(amount.commodity);
                const entry: LotEntry = {account: posting.account, qty: amount.qty, cost: amount.cost};
                if (entries === undefined) bySymbol.set(amount.commodity, [entry]);
                else entries.push(entry);

                const pool = poolFor(amount.commodity, txn.index);
                pool.lastTxnIndex = txn.index;
                const name = tagValue(posting.tags, "name") ?? tagValue(txn.tags, "name");
                if (name !== null) pool.name = name;
                const accounts = perAccount.get(amount.commodity)!;
                const prev = accounts.get(posting.account);
                accounts.set(posting.account, prev === undefined ? amount.qty : add(prev, amount.qty));
            }
        }

        for (const [symbol, entries] of bySymbol) {
            const pool = pools.get(symbol)!;
            const net = entries.reduce((acc, entry) => add(acc, entry.qty), dec(0n, 0));
            if (net.m === 0n) continue; // pure transfer within scope: zero shares, zero basis impact
            const before = pool.shares;
            for (const entry of entries) {
                const legBefore = pool.shares;
                const legAfter = add(legBefore, entry.qty);
                if (entry.qty.m > 0n) {
                    const lotCost = costInBase(entry.qty, entry.cost, db, base, txn.date);
                    if (lotCost === null) {
                        pool.tainted = true;
                        if (pool.costlessBuyTxns[pool.costlessBuyTxns.length - 1] !== txn.index) pool.costlessBuyTxns.push(txn.index);
                    } else {
                        pool.basis = add(pool.basis, lotCost);
                    }
                } else if (entry.qty.m < 0n && legBefore.m > 0n) {
                    pool.basis = legAfter.m >= 0n ? reduceBasis(pool.basis, legAfter, legBefore) : dec(0n, pool.basis.p);
                }
                pool.shares = legAfter;
            }
            if (before.m >= 0n && pool.shares.m < 0n) pool.negativeCrossTxn = txn.index;
        }
    }

    for (const [symbol, accounts] of perAccount) {
        const pool = pools.get(symbol)!;
        pool.accounts = [...accounts.entries()]
            .filter(([, shares]) => shares.m > 0n)
            .map(([account]) => account)
            .sort();
    }
    return pools;
}

/** A buy lot's cost in the base commodity, or null when it has none (or an unconvertible one). */
function costInBase(qty: Dec, cost: {commodity: string; qty: Dec; per: boolean} | undefined, db: PriceDb, base: string, date: ISODate): Dec | null {
    if (cost === undefined) return null;
    const own = cost.per ? mul(qty, cost.qty) : cost.qty;
    if (cost.commodity === base) return own;
    const rate = db.lookupIn(cost.commodity, base, date);
    return rate === null ? null : mul(own, rate.qty);
}

/** Latest P directive ≤ asOf pricing `symbol` directly in `base` (ties: last declared wins), with its date. */
export function latestDirectivePrice(prices: PriceDirective[], symbol: string, base: string, asOf: ISODate): DatedPrice | null {
    let best: DatedPrice | null = null;
    for (const directive of prices) {
        if (directive.commodity !== symbol || directive.price.commodity !== base || directive.date > asOf) continue;
        if (best === null || directive.date >= best.date) best = {qty: directive.price.qty, date: directive.date};
    }
    return best;
}

/**
 * Per symbol, the latest cost annotation ≤ asOf usable as a base-commodity
 * price — scanned across the WHOLE journal (not just in-scope), buys and
 * sells alike. `@` gives the per-unit price directly; `@@` divides the total
 * by |qty| (see perUnitFromTotal); non-base costs convert via the latest
 * direct P directive at the txn date, else the annotation is unusable.
 */
export function latestCostPrices(txns: Transaction[], db: PriceDb, base: string, asOf: ISODate): Map<string, DatedPrice> {
    const latest = new Map<string, DatedPrice>();
    for (const txn of journalOrder(txns)) {
        if (txn.date > asOf) continue;
        for (const posting of txn.postings) {
            for (const amount of posting.amounts) {
                if (isCurrency(amount.commodity) || amount.cost === undefined || amount.qty.m === 0n) continue;
                const perUnit = amount.cost.per ? amount.cost.qty : perUnitFromTotal(amount.cost.qty, amount.qty);
                let inBase = perUnit;
                if (amount.cost.commodity !== base) {
                    const rate = db.lookupIn(amount.cost.commodity, base, txn.date);
                    if (rate === null) continue;
                    inBase = mul(perUnit, rate.qty);
                }
                latest.set(amount.commodity, {qty: inBase, date: txn.date});
            }
        }
    }
    return latest;
}

/** Account predicate for a scope: include + empty set (or exclude + empty set) = everything; `accountMatches` subtree semantics. */
function scopePredicate(scope: HoldingsScope): (account: string) => boolean {
    const selected = [...scope.accounts];
    const matches = (account: string): boolean => selected.some((sel) => accountMatches(sel, account));
    return scope.mode === "include" ? (account) => selected.length === 0 || matches(account) : (account) => !matches(account);
}

/** Stock holdings, average-cost basis, prices, and gains for the scoped journal as of `scope.asOf`. */
export function computeHoldings(txns: Transaction[], prices: PriceDirective[], scope: HoldingsScope): HoldingsReport {
    const db = buildPriceDb(prices);
    const base = db.baseCommodity() ?? "$";
    const pools = buildPools(txns, db, base, scope.asOf, scopePredicate(scope));
    const costPrices = latestCostPrices(txns, db, base, scope.asOf);

    const holdings: Holding[] = [];
    const warnings: HoldingsWarning[] = [];
    for (const pool of [...pools.values()].sort((a, b) => (a.symbol < b.symbol ? -1 : 1))) {
        if (pool.shares.m === 0n) continue; // fully sold: dropped silently
        if (pool.shares.m < 0n) {
            warnings.push({
                symbol: pool.symbol,
                kind: "negative-shares",
                message: `${pool.symbol}: net shares are negative — the opening position was likely never entered; row hidden`,
            });
            continue;
        }

        const directive = latestDirectivePrice(prices, pool.symbol, base, scope.asOf);
        const fromCost = directive === null ? (costPrices.get(pool.symbol) ?? null) : null;
        const price: Holding["price"] = directive !== null ? {...directive, source: "directive"} : fromCost !== null ? {...fromCost, source: "cost"} : null;
        if (price === null) {
            warnings.push({symbol: pool.symbol, kind: "unpriced", message: `${pool.symbol}: no market price or usable cost annotation — excluded from totals`});
        }
        if (pool.tainted) {
            warnings.push({symbol: pool.symbol, kind: "missing-basis", message: `${pool.symbol}: acquired without a cost annotation — basis unknown`});
        }

        const basis = pool.tainted ? null : pool.basis;
        const marketValue = price === null ? null : mul(pool.shares, price.qty);
        const gain = marketValue !== null && basis !== null ? sub(marketValue, basis) : null;
        const gainPct = gain !== null && basis !== null && !isZero(basis) ? (toNumber(gain) / toNumber(basis)) * 100 : null;
        holdings.push({symbol: pool.symbol, name: pool.name, accounts: pool.accounts, shares: pool.shares, basis, price, marketValue, gain, gainPct});
    }

    // Market value desc; unpriced last; ties (and unpriced) by symbol asc. Pools iterate symbol-sorted, so a stable sort on value alone would also do.
    holdings.sort((a, b) => {
        if (a.marketValue === null && b.marketValue === null) return a.symbol < b.symbol ? -1 : 1;
        if (a.marketValue === null) return 1;
        if (b.marketValue === null) return -1;
        const byValue = cmp(b.marketValue, a.marketValue);
        return byValue !== 0 ? byValue : a.symbol < b.symbol ? -1 : 1;
    });

    // Totals refuse (null) when any included holding is tainted or unpriced — a partial total silently understates.
    let marketValue = dec(0n, 0);
    let basisTotal: Dec | null = dec(0n, 0);
    for (const holding of holdings) {
        if (holding.marketValue !== null) marketValue = add(marketValue, holding.marketValue);
        basisTotal = basisTotal !== null && holding.basis !== null && holding.marketValue !== null ? add(basisTotal, holding.basis) : null;
    }
    const gainTotal = basisTotal === null ? null : sub(marketValue, basisTotal);
    const gainPctTotal = gainTotal !== null && basisTotal !== null && !isZero(basisTotal) ? (toNumber(gainTotal) / toNumber(basisTotal)) * 100 : null;

    const ranked = holdings.filter((h): h is Holding & {gainPct: number} => h.gainPct !== null);
    const topGainers = [...ranked].sort((a, b) => b.gainPct - a.gainPct).slice(0, 5);
    const topLosers = [...ranked].sort((a, b) => a.gainPct - b.gainPct).slice(0, 5);

    return {
        asOf: scope.asOf,
        base,
        holdings,
        totals: {marketValue, basis: basisTotal, gain: gainTotal, gainPct: gainPctTotal},
        topGainers,
        topLosers,
        warnings,
    };
}
