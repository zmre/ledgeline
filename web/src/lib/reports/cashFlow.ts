// Cash flow (WP-06): changes in cash-like asset accounts per bucket.
// Pure TS: no Svelte/DOM imports.

import {accountTotals, atDepth, rollUp} from "../domain/aggregate";
import {categorize} from "../domain/accounts";
import {maAdd, type MixedAmount} from "../domain/money";
import type {ISODate, Transaction} from "../domain/types";
import {bucketEnd, bucketStart, compareISO, lastNBuckets} from "./periods";
import type {PeriodReport} from "./types";

// hledger's Cash-type inference regex (used when accounts carry no explicit
// `type:` declaration). Matching a segment anywhere under an asset root also
// covers descendants (assets:bank:wise:eur is cash-like via "bank").
// TODO(post-MVP): prefer declared account types from the /accounts endpoint.
const CASH_RE = /^assets?(:.+)?:(cash|bank|che(ck|que)ing|savings?|current)(:|$)/i;

/** Name-based "cash-like asset" heuristic (contract addition, see plans/06-reports-engine.md). */
export function isCashLike(account: string): boolean {
    return categorize(account) === "asset" && CASH_RE.test(account);
}

/**
 * Per-bucket changes (natural signs: inflow positive) in cash-like asset
 * accounts, for the last `count` buckets ending with the bucket containing
 * `end`. The final bucket is truncated at `end` (matching hledger `-e`
 * semantics where `end` = day before the exclusive end date). `totals[i]` is
 * the net cash flow of bucket i (sum of direct changes, not of the rolled-up
 * rows, so ancestors are not double-counted).
 */
export function cashFlow(txns: Transaction[], opts: {end: ISODate; interval: "monthly" | "quarterly" | "yearly"; count: number; depth: number}): PeriodReport {
    const buckets = lastNBuckets(opts.end, opts.interval, opts.count);
    const totals: MixedAmount[] = [];
    const perBucket: Map<string, MixedAmount>[] = [];
    for (const key of buckets) {
        const to = compareISO(opts.end, bucketEnd(key)) < 0 ? opts.end : bucketEnd(key);
        const direct = accountTotals(txns, {from: bucketStart(key), to});
        for (const account of [...direct.keys()]) {
            if (!isCashLike(account)) direct.delete(account);
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
