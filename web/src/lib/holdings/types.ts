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
    /** ≤ 5 by gainPct desc, gainPct != null only. */
    topGainers: Holding[];
    /** ≤ 5 by gainPct asc, gainPct != null only. */
    topLosers: Holding[];
    /** Scope-local, rendered inline on the page. */
    warnings: HoldingsWarning[];
}
