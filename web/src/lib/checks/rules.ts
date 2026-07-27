// MVP check rules (WP-08). Pure functions over domain Transactions — relative
// imports only (same convention as lib/reports/). `today()` comes from
// lib/reports/periods.ts, the codebase's single sanctioned local-Date read.

import {declaredTypes, resolveAccountType, type AccountType} from "../domain/accountTypes";
import {add, isZero, mul, neg, type Dec} from "../domain/money";
import type {Amount, Transaction} from "../domain/types";
import {buildPools, latestCostPrices, latestDirectivePrice, type SymbolPool} from "../holdings/engine";
import {today} from "../reports/periods";
import {buildPriceDb, type PriceDb} from "../reports/prices";
import type {CheckContext, CheckRule, Problem} from "./engine";

/** Plain decimal rendering for problem messages (no locale styling — messages are diagnostics). */
function decToString(d: Dec): string {
    const negative = d.m < 0n;
    const digits = (negative ? -d.m : d.m).toString().padStart(d.p + 1, "0");
    const whole = d.p === 0 ? digits : digits.slice(0, digits.length - d.p);
    const frac = d.p === 0 ? "" : `.${digits.slice(digits.length - d.p)}`;
    return `${negative ? "-" : ""}${whole}${frac}`;
}

/**
 * The value an amount contributes to transaction balancing: amounts carrying a
 * cost annotation balance in the COST commodity (hledger semantics — otherwise
 * every `10 AAPL @ $220.00` purchase would look unbalanced). `@` per-unit costs
 * multiply; `@@` total costs take the posting amount's sign.
 */
function balanceValue(amount: Amount): {commodity: string; qty: Dec} {
    const cost = amount.cost;
    if (cost === undefined) return {commodity: amount.commodity, qty: amount.qty};
    if (cost.per) return {commodity: cost.commodity, qty: mul(amount.qty, cost.qty)};
    return {commodity: cost.commodity, qty: amount.qty.m < 0n ? neg(cost.qty) : cost.qty};
}

const unbalanced: CheckRule = {
    id: "unbalanced",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        // The engine already ran this check, more accurately — see
        // CheckContext.engineChecked. Deferring avoids both a duplicate finding
        // and this rule's false positives on journals hledger accepts.
        if (ctx.engineChecked === true) return [];
        const problems: Problem[] = [];
        for (const txn of txns) {
            const elided = txn.postings.filter((posting) => posting.amounts.length === 0).length;
            if (elided >= 2) {
                problems.push({
                    txnIndex: txn.index,
                    rule: "unbalanced",
                    severity: "error",
                    message: `${elided} postings have no amount — at most one may be elided`,
                });
                continue;
            }
            if (elided === 1) continue; // the amountless posting absorbs the remainder
            const residue = new Map<string, Dec>();
            for (const posting of txn.postings) {
                for (const amount of posting.amounts) {
                    const {commodity, qty} = balanceValue(amount);
                    const prev = residue.get(commodity);
                    residue.set(commodity, prev === undefined ? qty : add(prev, qty));
                }
            }
            const nonzero = [...residue.entries()].filter(([, qty]) => !isZero(qty));
            if (nonzero.length > 0) {
                const detail = nonzero.map(([commodity, qty]) => `${commodity} ${decToString(qty)}`).join(", ");
                problems.push({txnIndex: txn.index, rule: "unbalanced", severity: "error", message: `postings do not sum to zero: ${detail} remaining`});
            }
        }
        return problems;
    },
};

const pending: CheckRule = {
    id: "pending",
    run(txns: Transaction[]): Problem[] {
        return txns
            .filter((txn) => txn.status === "pending")
            .map((txn) => ({txnIndex: txn.index, rule: "pending", severity: "warning" as const, message: "transaction is marked pending (!)"}));
    },
};

const UNCATEGORIZED_SEGMENTS = new Set(["unknown", "uncategorized"]);
/**
 * `*:unknown`, `*:uncategorized` (any depth, incl. bare), or a bare top-level
 * income/expense root with no subaccount.
 *
 * The bare-root case is decided by TYPE, not by the literal names
 * "expenses"/"income": the whole point of the rule is to catch a posting that
 * never got a category, and a chart of accounts rooted at `cogs:` or `gastos:`
 * has exactly the same mistake to catch.
 */
function isUncategorized(account: string, declared: ReadonlyMap<string, AccountType>): boolean {
    const segments = account.toLowerCase().split(":");
    if (UNCATEGORIZED_SEGMENTS.has(segments[segments.length - 1])) return true;
    if (segments.length !== 1) return false;
    const type = resolveAccountType(account, declared);
    return type === "expense" || type === "revenue";
}

const uncategorized: CheckRule = {
    id: "uncategorized",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        const problems: Problem[] = [];
        const declared = declaredTypes(ctx.decls ?? []);
        for (const txn of txns) {
            const seen = new Set<string>();
            for (const posting of txn.postings) {
                if (!isUncategorized(posting.account, declared) || seen.has(posting.account)) continue;
                seen.add(posting.account);
                problems.push({
                    txnIndex: txn.index,
                    rule: "uncategorized",
                    severity: "warning",
                    message: `posting to uncategorized account "${posting.account}"`,
                });
            }
        }
        return problems;
    },
};

const missingDescription: CheckRule = {
    id: "missing-description",
    run(txns: Transaction[]): Problem[] {
        return txns
            .filter((txn) => txn.description.trim() === "")
            .map((txn) => ({txnIndex: txn.index, rule: "missing-description", severity: "info" as const, message: "transaction has no description"}));
    },
};

const futureDate: CheckRule = {
    id: "future-date",
    run(txns: Transaction[]): Problem[] {
        const cutoff = today();
        return txns
            .filter((txn) => txn.date > cutoff)
            .map((txn) => ({txnIndex: txn.index, rule: "future-date", severity: "info" as const, message: `transaction is dated in the future (${txn.date})`}));
    },
};

/** Journal-wide (unscoped) average-cost pools at today, sorted by symbol for deterministic report order (WP-10 stock rules). */
function stockPools(txns: Transaction[], ctx: CheckContext): {db: PriceDb; base: string; asOf: string; pools: SymbolPool[]} {
    const db = buildPriceDb(ctx.prices);
    const base = db.baseCommodity() ?? "$";
    const asOf = today();
    const pools = [...buildPools(txns, db, base, asOf, () => true).values()].sort((a, b) => (a.symbol < b.symbol ? -1 : 1));
    return {db, base, asOf, pools};
}

const stockMissingBasis: CheckRule = {
    id: "stock-missing-basis",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        return stockPools(txns, ctx)
            .pools.filter((pool) => pool.shares.m > 0n)
            .flatMap((pool) =>
                pool.costlessBuyTxns.map((txnIndex) => ({
                    txnIndex,
                    rule: "stock-missing-basis",
                    severity: "warning" as const,
                    message: `${pool.symbol} lot acquired without a cost annotation — cost basis is unknown`,
                }))
            );
    },
};

const stockNegative: CheckRule = {
    id: "stock-negative",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        return stockPools(txns, ctx)
            .pools.filter((pool) => pool.shares.m < 0n)
            .map((pool) => ({
                txnIndex: pool.negativeCrossTxn ?? pool.lastTxnIndex,
                rule: "stock-negative",
                severity: "warning" as const,
                message: `${pool.symbol} net shares are negative (${decToString(pool.shares)}) — the opening position was likely never entered`,
            }));
    },
};

const stockUnpriced: CheckRule = {
    id: "stock-unpriced",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        const {db, base, asOf, pools} = stockPools(txns, ctx);
        const costPrices = latestCostPrices(txns, db, base, asOf);
        return pools
            .filter((pool) => pool.shares.m > 0n && latestDirectivePrice(ctx.prices, pool.symbol, base, asOf) === null && !costPrices.has(pool.symbol))
            .map((pool) => ({
                txnIndex: pool.lastTxnIndex,
                rule: "stock-unpriced",
                severity: "warning" as const,
                message: `${pool.symbol} has no P price directive and no usable cost annotation — market value is unknown`,
            }));
    },
};

/** All rules, in report order. Adding a rule = one object here. */
export const ALL_RULES: CheckRule[] = [unbalanced, pending, uncategorized, missingDescription, futureDate, stockMissingBasis, stockNegative, stockUnpriced];
