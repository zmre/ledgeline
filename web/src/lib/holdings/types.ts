// Holdings report contracts (WP-10). Pure TS: no Svelte/DOM imports — ports
// to Rust later. Computed by ./engine.ts over the normalized journal.

import type {Dec} from "../domain/money";
import type {ISODate} from "../domain/types";

export interface HoldingsScope {
    /** Subtree roots, same invariant as JournalFilter. */
    accounts: ReadonlySet<string>;
    /** include + empty set = everything. */
    mode: "include" | "exclude";
    asOf: ISODate;
}

export interface Holding {
    symbol: string;
    /** `name:` tag, else symbol. */
    name: string;
    /** In-scope accounts currently holding shares. */
    accounts: string[];
    /** > 0 by construction. */
    shares: Dec;
    /** null = tainted (some lot lacks a cost). */
    basis: Dec | null;
    /** Date the current position was opened (first lot since shares were last ≤ 0); null only if never bought in scope. */
    firstBasisDate: ISODate | null;
    price: {qty: Dec; date: ISODate; source: "directive" | "cost"} | null;
    /** shares × price, null when unpriced. */
    marketValue: Dec | null;
    /** marketValue − basis, null when either is missing. */
    gain: Dec | null;
    /** Display-boundary number; null when basis missing/zero. */
    gainPct: number | null;
}

export interface HoldingsWarning {
    symbol: string;
    kind: "missing-basis" | "negative-shares" | "unpriced";
    message: string;
}

export interface HoldingsReport {
    asOf: ISODate;
    base: string;
    /** shares > 0, sorted market value desc (unpriced last, by symbol). */
    holdings: Holding[];
    totals: {marketValue: Dec; basis: Dec | null; gain: Dec | null; gainPct: number | null};
    /** gainPct > 0 only, sorted desc, ≤ 5. */
    topGainers: Holding[];
    /** gainPct < 0 only, sorted asc, ≤ 5 (zero-gain holdings in neither list). */
    topLosers: Holding[];
    /** Scope-local, rendered inline on the page. */
    warnings: HoldingsWarning[];
}

// Holdings-over-time series (served by GET /api/holdings/series). Kept here — the
// former client-side series.ts engine was dropped when /holdings went native.
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
    /** Oldest → newest, length = requested count. */
    points: HoldingsPoint[];
    /** True when at least one point has a non-null basis (so the UI knows whether to draw the basis line). */
    hasBasis: boolean;
}
