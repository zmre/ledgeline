// Net worth over time (WP-06): asset + liability balances valued at market
// prices per bucket end. Pure TS: no Svelte/DOM imports.

import {accountTotals, rollUp} from "../domain/aggregate";
import {categorize} from "../domain/accounts";
import {isZero, maAdd, type MixedAmount} from "../domain/money";
import type {ISODate, Transaction} from "../domain/types";
import {bucketEnd, compareISO, lastNBuckets, type Interval} from "./periods";
import {valueAt, type PriceDb, type ValuationMeta} from "./prices";
import type {PeriodReport} from "./types";

/**
 * One row per top-level asset/liability account (natural signs: liabilities
 * negative), one column per bucket; `totals[i]` = net worth at the end of
 * bucket i (balances of postings ≤ bucket end, truncated at `end`).
 *
 * Valuation (hledger `--value=end,TARGET`): every commodity is converted to
 * `opts.valueIn ?? prices.baseCommodity()` via the latest direct P directive
 * ≤ the bucket end. Commodities with no such price are SKIPPED and reported
 * in `meta.unpriced` (never guessed). When there is no target at all (no
 * price directives, no `valueIn`) balances are reported unvalued, in their
 * original commodities. `valueIn` is a contract extension, see
 * plans/06-reports-engine.md.
 */
export function netWorth(txns: Transaction[], prices: PriceDb, opts: {end: ISODate; interval: Interval; count: number; valueIn?: string}): PeriodReport {
    const buckets = lastNBuckets(opts.end, opts.interval, opts.count);
    const target = opts.valueIn ?? prices.baseCommodity();
    const meta: ValuationMeta = {unpriced: []};

    const perBucket: {asOf: ISODate; roots: Map<string, MixedAmount>}[] = buckets.map((key) => {
        const asOf = compareISO(opts.end, bucketEnd(key)) < 0 ? opts.end : bucketEnd(key);
        const rolled = rollUp(accountTotals(txns, {to: asOf}));
        const roots = new Map<string, MixedAmount>();
        for (const [account, ma] of rolled) {
            if (account.includes(":")) continue;
            const category = categorize(account);
            if (category === "asset" || category === "liability") roots.set(account, ma);
        }
        return {asOf, roots};
    });

    const accounts = [...new Set(perBucket.flatMap(({roots}) => [...roots.keys()]))].sort();
    const value = (ma: MixedAmount, asOf: ISODate): MixedAmount => {
        if (target === null) return ma;
        const valued = valueAt(ma, target, prices, asOf, meta);
        return isZero(valued) ? new Map() : new Map([[target, valued]]);
    };
    const rows = accounts.map((account) => ({
        account,
        depth: 1,
        values: perBucket.map(({asOf, roots}) => value(roots.get(account) ?? new Map(), asOf)),
    }));
    const totals = perBucket.map((_, i) => rows.reduce((acc: MixedAmount, row) => maAdd(acc, row.values[i]), new Map()));

    const report: PeriodReport = {buckets, rows, totals};
    if (meta.unpriced.length > 0) report.meta = {unpriced: [...meta.unpriced].sort()};
    return report;
}
