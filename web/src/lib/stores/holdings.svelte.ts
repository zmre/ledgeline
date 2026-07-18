// Holdings scope + data store (WP-10, now native). The scope (accounts/mode/
// asOf) is Svelte 5 runes state driving the scope bar + URL sync; the report
// and trend are fetched from the native /api/holdings[/series] endpoints for
// the current scope and decoded into the existing domain types, so the holdings
// UI renders unchanged. The scope's asOf defaults to localToday() and is NEVER
// persisted (every fresh visit opens at today). A monotonic request token drops
// stale responses when the scope changes faster than the network answers.
/* eslint-disable svelte/prefer-svelte-reactivity -- the account Sets are
   immutable snapshots: every change replaces `value` wholesale, so plain Set
   is correct (and the contract exposes ReadonlySet). */
import {LedgelineApi} from "$lib/api/native";
import {decodeHoldingsReport, decodeHoldingsSeries} from "$lib/api/nativeDecode";
import type {ISODate} from "$lib/domain/types";
import {toggleSubtreeRoot} from "$lib/filters/treeSelect";
import type {GainPeriod, HoldingsReport, HoldingsScope, HoldingsSeries} from "$lib/holdings/types";
import {gainSinceFor} from "$lib/holdings/ui/gainPeriod";
import {localToday} from "./filters.svelte";

/** Trailing series window shown under the details table (matches the former client-side default). */
const TREND_INTERVAL = "monthly";
const TREND_COUNT = 12;

/** Fresh-visit default: everything included, as of today, all-time gain (recomputed per call, never remembered). */
export function defaultScope(): HoldingsScope {
    return {accounts: new Set<string>(), mode: "include", asOf: localToday(), gainPeriod: "all"};
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
    /** Switch the gain window (all-time vs YTD vs trailing 12mo); everything else is kept. */
    setGainPeriod(gainPeriod: GainPeriod): void {
        value = {...value, gainPeriod};
    },
    /** Clear the account selection (back to "everything"); mode, asOf and gain window are kept. */
    clear(): void {
        value = {...value, accounts: new Set<string>()};
    },
    /** Replace the whole scope at once (URL-sync startup restore). */
    replace(s: HoldingsScope): void {
        value = {accounts: new Set(s.accounts), mode: s.mode, asOf: s.asOf, gainPeriod: s.gainPeriod};
    },
};

export type HoldingsStatus = "idle" | "loading" | "ready" | "error";

let report = $state<HoldingsReport | null>(null);
let trend = $state<HoldingsSeries | null>(null);
let status = $state<HoldingsStatus>("idle");
let error = $state<Error | null>(null);
let seq = 0;

export const holdingsData = {
    /** The decoded holdings report for the last loaded scope, or null before the first load. */
    get report(): HoldingsReport | null {
        return report;
    },
    /** The value-over-time series for the last loaded scope, or null before the first load. */
    get trend(): HoldingsSeries | null {
        return trend;
    },
    get status(): HoldingsStatus {
        return status;
    },
    get error(): Error | null {
        return error;
    },
    /** Fetch + decode the report and trend for `scope`; stale responses (superseded by a newer load) are discarded. */
    async load(serverUrl: string, scope: HoldingsScope): Promise<void> {
        const token = ++seq;
        status = "loading";
        try {
            const api = new LedgelineApi(serverUrl);
            const accounts = [...scope.accounts].join(",");
            // gainSince narrows only the report's gain; the value-over-time series is always all-time.
            const gainSince = gainSinceFor(scope.gainPeriod, scope.asOf);
            const [rawReport, rawSeries] = await Promise.all([
                api.holdings({asOf: scope.asOf, accounts, mode: scope.mode, gainSince}),
                api.holdingsSeries({asOf: scope.asOf, accounts, mode: scope.mode, interval: TREND_INTERVAL, count: TREND_COUNT}),
            ]);
            if (token !== seq) return;
            report = decodeHoldingsReport(rawReport);
            trend = decodeHoldingsSeries(rawSeries);
            status = "ready";
            error = null;
        } catch (cause) {
            if (token !== seq) return;
            status = "error";
            error = cause instanceof Error ? cause : new Error(String(cause));
        }
    },
};
