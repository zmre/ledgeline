// Benchmark overlay store: one request per ticked benchmark, fetched only
// AFTER the value-over-time chart it overlays has drawn (the caller syncs once
// the base trend is on screen), and cached per window so unticking and
// re-ticking a box costs nothing.
//
// Per benchmark rather than one batched request so each checkbox owns its own
// state: a slow or failed Yahoo fetch for one index shows its hint beside that
// box and never holds up or blanks the others.
//
// Everything is keyed by the WINDOW — the exact series query (`trendQuery`) —
// so a scope, date or gain-period change drops every held line at once: a line
// computed for last month's window drawn over this month's chart would be a
// confident wrong comparison.
/* eslint-disable svelte/prefer-svelte-reactivity -- `entries` is replaced
   wholesale on every change (an immutable snapshot), so a plain Map is correct. */
import {LedgelineApi} from "$lib/api/native";
import {decodeBenchmarks} from "$lib/api/benchmarksDecode";
import type {BenchmarkLine} from "$lib/holdings/benchmarks";
import type {HoldingsScope} from "$lib/holdings/types";
import {trendQuery} from "./holdings.svelte";

/** One benchmark's overlay state. */
export type BenchmarkState = {status: "loading"} | {status: "ready"; line: BenchmarkLine} | {status: "error"; message: string};

let windowKey = $state<string | null>(null);
let entries = $state.raw<ReadonlyMap<string, BenchmarkState>>(new Map());

function keyOf(serverUrl: string, scope: HoldingsScope): string {
    return JSON.stringify([serverUrl, trendQuery(scope)]);
}

function set(symbol: string, next: BenchmarkState): void {
    const copy = new Map(entries);
    copy.set(symbol, next);
    entries = copy;
}

async function fetchOne(serverUrl: string, scope: HoldingsScope, symbol: string, key: string): Promise<void> {
    set(symbol, {status: "loading"});
    try {
        const response = decodeBenchmarks(await new LedgelineApi(serverUrl).holdingsBenchmarks({...trendQuery(scope), symbols: symbol}));
        if (windowKey !== key) return; // superseded by a newer window
        const line = response.benchmarks.find((b) => b.symbol === symbol);
        if (line === undefined) set(symbol, {status: "error", message: `The engine returned no ${symbol} line.`});
        else if (line.error !== null) set(symbol, {status: "error", message: line.error});
        else set(symbol, {status: "ready", line});
    } catch (cause) {
        if (windowKey !== key) return;
        set(symbol, {status: "error", message: cause instanceof Error ? cause.message : String(cause)});
    }
}

export const benchmarkLines = {
    /** Each benchmark's state for the current window; absent = never requested. */
    get entries(): ReadonlyMap<string, BenchmarkState> {
        return entries;
    },
    /**
     * Make sure every symbol in `symbols` is fetched (or being fetched) for the
     * window `scope` names. A new window forgets everything held for the old
     * one; symbols already requested for this window are left alone, so this
     * is safe to call from an effect on every change.
     */
    sync(serverUrl: string, scope: HoldingsScope, symbols: readonly string[]): void {
        const key = keyOf(serverUrl, scope);
        if (key !== windowKey) {
            windowKey = key;
            entries = new Map();
        }
        for (const symbol of symbols) {
            if (!entries.has(symbol)) void fetchOne(serverUrl, scope, symbol, key);
        }
    },
    /** Forget one symbol's failure and ask again (the hint's retry). */
    retry(serverUrl: string, scope: HoldingsScope, symbol: string): void {
        const key = keyOf(serverUrl, scope);
        if (key !== windowKey) return;
        void fetchOne(serverUrl, scope, symbol, key);
    },
    /** Drop everything (tests). */
    reset(): void {
        windowKey = null;
        entries = new Map();
    },
};
