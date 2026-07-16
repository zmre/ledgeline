// Holdings scope store (WP-10): Svelte 5 runes state for the /holdings route.
// The scope's asOf defaults to localToday() and is NEVER persisted to
// localStorage — every fresh visit opens at today (plans/10). The report is a
// pure $derived over journal.txns/journal.prices via computeHoldings, so scope
// changes recompute without refetching.
/* eslint-disable svelte/prefer-svelte-reactivity -- the account Sets are
   immutable snapshots: every change replaces `value` wholesale, so plain Set
   is correct (and the contract exposes ReadonlySet). */
import type {HoldingsReport, HoldingsScope} from "$lib/holdings/types";
import {computeHoldings} from "$lib/holdings/engine";
import {holdingsSeries, type HoldingsSeries} from "$lib/holdings/series";
import {toggleSubtreeRoot} from "$lib/filters/treeSelect";
import type {ISODate} from "$lib/domain/types";
import {localToday} from "./filters.svelte";
import {journal} from "./journal.svelte";

/** Fresh-visit default: everything included, as of today (recomputed per call, never remembered). */
export function defaultScope(): HoldingsScope {
    return {accounts: new Set<string>(), mode: "include", asOf: localToday()};
}

let value = $state<HoldingsScope>(defaultScope());

/**
 * Observe scope changes outside component context (used by the holdings URL
 * sync, which is plain TS and cannot declare rune effects itself). The
 * callback fires once immediately, then after every change. Returns an
 * unsubscribe — same contract as subscribeFilters.
 */
export function subscribeHoldingsScope(cb: (s: HoldingsScope) => void): () => void {
    return $effect.root(() => {
        $effect(() => {
            cb(value);
        });
    });
}

export const holdingsScope = {
    get value(): HoldingsScope {
        return value;
    },
    /** Toggle an account's selection, keeping the subtree-root invariant (same rules as the journal filters). */
    toggleAccount(name: string): void {
        value = {...value, accounts: toggleSubtreeRoot(value.accounts, name)};
    },
    /** Switch include/exclude; the selection is kept (plans/10 UI behavior). */
    setMode(mode: HoldingsScope["mode"]): void {
        value = {...value, mode};
    },
    setAsOf(asOf: ISODate): void {
        value = {...value, asOf};
    },
    /** Clear the account selection (back to "everything"); mode and asOf are kept. */
    clear(): void {
        value = {...value, accounts: new Set<string>()};
    },
    /** Replace the whole scope at once (URL-sync startup restore). */
    replace(s: HoldingsScope): void {
        value = {accounts: new Set(s.accounts), mode: s.mode, asOf: s.asOf};
    },
};

const report = $derived.by(() => computeHoldings(journal.txns, journal.prices, value));

/** The holdings report for the current journal + scope (pure derivation, no refetch). */
export function getHoldingsReport(): HoldingsReport {
    return report;
}

/** Trailing-12-month monthly market-value/basis series for the current scope, ending at scope.asOf. */
const trend = $derived.by(() => holdingsSeries(journal.txns, journal.prices, value, {interval: "monthly", count: 12}));

export function getHoldingsTrend(): HoldingsSeries {
    return trend;
}
