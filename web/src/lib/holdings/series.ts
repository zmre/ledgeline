// Holdings-over-time series (post-MVP). Pure TS: no Svelte/DOM imports — ports
// to Rust later.
//
// The WP-10 engine was designed for exactly this: a portfolio snapshot at each
// of the last `count` period boundaries is `computeHoldings` mapped over a date
// series (see engine.ts header). We reuse computeHoldings unchanged so the
// totals math has a single source of truth; that costs one full recompute per
// point (buildPriceDb + a pool sweep), which is fine at count≈12 but is the
// obvious thing to make incremental if a large journal ever needs it.

import type {Dec} from "../domain/money";
import type {ISODate, PriceDirective, Transaction} from "../domain/types";
import {bucketEnd, bucketLabel, compareISO, lastNBuckets, type Interval} from "../reports/periods";
import {computeHoldings} from "./engine";
import type {HoldingsScope} from "./types";

export interface HoldingsPoint {
    /** Snapshot date: the bucket's last day, clamped so the final point never overshoots scope.asOf. */
    date: ISODate;
    /** Bucket key (e.g. "2026-07"), for axis labels. */
    bucket: string;
    /** Human bucket label (e.g. "Jul 2026"). */
    label: string;
    /** Total priced market value at `date`, in the base commodity (unpriced holdings excluded, per the honest-totals rule). */
    marketValue: Dec;
    /** Total cost basis at `date`, null when any held lot is tainted or unpriced (same refusal as HoldingsReport.totals.basis). */
    basis: Dec | null;
}

export interface HoldingsSeries {
    base: string;
    /** Oldest → newest, length = opts.count. */
    points: HoldingsPoint[];
    /** True when at least one point has a non-null basis (so the UI knows whether to draw the basis line). */
    hasBasis: boolean;
}

/**
 * Portfolio market value (and cost basis) at each of the last `count` period
 * boundaries ending at scope.asOf, oldest first. Same scope (accounts/mode) as
 * the live report; only asOf time-travels. Points before any in-scope holding
 * existed are naturally zero.
 */
export function holdingsSeries(txns: Transaction[], prices: PriceDirective[], scope: HoldingsScope, opts: {interval: Interval; count: number}): HoldingsSeries {
    const keys = lastNBuckets(scope.asOf, opts.interval, opts.count);
    let base = "$";
    let hasBasis = false;
    const points = keys.map((key): HoldingsPoint => {
        const end = bucketEnd(key);
        const date = compareISO(end, scope.asOf) > 0 ? scope.asOf : end;
        const report = computeHoldings(txns, prices, {...scope, asOf: date});
        base = report.base;
        if (report.totals.basis !== null) hasBasis = true;
        return {date, bucket: key, label: bucketLabel(key), marketValue: report.totals.marketValue, basis: report.totals.basis};
    });
    return {base, points, hasBasis};
}
