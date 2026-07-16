// Cash flow (WP-06): changes in cash-like asset accounts per bucket.
// Pure TS: no Svelte/DOM imports.

import {accountTotals, atDepth, rollUp} from "../domain/aggregate";
import {inferAccountType} from "../domain/accountTypes";
import {maAdd, type MixedAmount} from "../domain/money";
import type {ISODate, Transaction} from "../domain/types";
import {bucketEnd, bucketStart, compareISO, lastNBuckets} from "./periods";
import type {PeriodReport} from "./types";

/**
 * Name-based "cash-like asset" heuristic — the fallback used when a journal
 * declares no account types. Delegates to the single copy of hledger's Cash
 * name regex in domain/accountTypes.ts. When declarations ARE present, callers
 * pass `opts.isCash` from `cashPredicate(accountDecls)` instead, which honors
 * `type:` tags (own → nearest ancestor → this name inference).
 */
export function isCashLike(account: string): boolean {
    return inferAccountType(account) === "cash";
}

/**
 * Per-bucket changes (natural signs: inflow positive) in cash-like asset
 * accounts, for the last `count` buckets ending with the bucket containing
 * `end`. The final bucket is truncated at `end` (matching hledger `-e`
 * semantics where `end` = day before the exclusive end date). `totals[i]` is
 * the net cash flow of bucket i (sum of direct changes, not of the rolled-up
 * rows, so ancestors are not double-counted).
 */
export function cashFlow(
    txns: Transaction[],
    opts: {end: ISODate; interval: "monthly" | "quarterly" | "yearly"; count: number; depth: number; isCash?: (account: string) => boolean}
): PeriodReport {
    const isCash = opts.isCash ?? isCashLike;
    const buckets = lastNBuckets(opts.end, opts.interval, opts.count);
    const totals: MixedAmount[] = [];
    const perBucket: Map<string, MixedAmount>[] = [];
    for (const key of buckets) {
        const to = compareISO(opts.end, bucketEnd(key)) < 0 ? opts.end : bucketEnd(key);
        const direct = accountTotals(txns, {from: bucketStart(key), to});
        for (const account of [...direct.keys()]) {
            if (!isCash(account)) direct.delete(account);
        }
        let total: MixedAmount = new Map();
        for (const ma of direct.values()) total = maAdd(total, ma);
        totals.push(total);
        perBucket.push(atDepth(rollUp(direct), opts.depth));
    }
    const accounts = [...new Set(perBucket.flatMap((clamped) => [...clamped.keys()]))].sort();
    const rows = accounts.map((account) => ({
        account,
        depth: account.split(":").length,
        values: perBucket.map((clamped): MixedAmount => clamped.get(account) ?? new Map()),
    }));
    return {buckets, rows, totals};
}
