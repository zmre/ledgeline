// Benchmark overlay store: ONE batched request for every ticked benchmark,
// fetched only AFTER the value-over-time chart it overlays has drawn (the
// caller syncs once the base trend is on screen), and cached per base series so
// unticking and re-ticking a box costs nothing.
//
// Batched because the server's work is per REQUEST, not per symbol: each
// request rebuilds the holdings series and replays the portfolio's flows before
// it can seed a single line. Ticking another box asks for just the new symbol,
// in a request of its own. Each checkbox still owns its own state: the response
// carries a line (or an error) per symbol, so a failed Yahoo fetch for one
// index shows its hint beside that box and never blanks the others.
//
// Everything is keyed by the BASE SERIES the lines overlay: its window (the
// exact series query, `trendQuery`) and its identity. A scope, date or
// gain-period change drops every held line, and so does anything that reloads
// the base series for the same window — Refresh, Update prices, a journal edit
// — because a line is seeded from the portfolio's value, and a line seeded from
// the old values drawn over the new ones would be a confident wrong comparison.
/* eslint-disable svelte/prefer-svelte-reactivity -- `entries` is replaced
   wholesale on every change (an immutable snapshot), so a plain Map is correct. */
import {LedgelineApi} from "$lib/api/native";
import {decodeBenchmarks} from "$lib/api/benchmarksDecode";
import type {BenchmarkLine} from "$lib/holdings/benchmarks";
import type {HoldingsScope} from "$lib/holdings/types";
import {trendQuery} from "./holdings.svelte";

/** One benchmark's overlay state. */
export type BenchmarkState = {status: "loading"} | {status: "ready"; line: BenchmarkLine} | {status: "error"; message: string};

let windowKey: string | null = null;
let baseSeries: object | null = null;
/** Bumped whenever held lines are dropped, so a response for an older base is ignored. */
let generation = 0;
let entries = $state.raw<ReadonlyMap<string, BenchmarkState>>(new Map());

function keyOf(serverUrl: string, scope: HoldingsScope): string {
    return JSON.stringify([serverUrl, trendQuery(scope)]);
}

function setAll(symbols: readonly string[], stateOf: (symbol: string) => BenchmarkState): void {
    const copy = new Map(entries);
    for (const symbol of symbols) copy.set(symbol, stateOf(symbol));
    entries = copy;
}

function stateFor(lines: readonly BenchmarkLine[], symbol: string): BenchmarkState {
    const line = lines.find((b) => b.symbol === symbol);
    if (line === undefined) return {status: "error", message: `The engine returned no ${symbol} line.`};
    if (line.error !== null) return {status: "error", message: line.error};
    return {status: "ready", line};
}

/** One request for `symbols`, each landing in its own entry. */
async function fetchBatch(serverUrl: string, scope: HoldingsScope, symbols: readonly string[]): Promise<void> {
    const mine = generation;
    setAll(symbols, () => ({status: "loading"}));
    try {
        const response = decodeBenchmarks(await new LedgelineApi(serverUrl).holdingsBenchmarks({...trendQuery(scope), symbols: symbols.join(",")}));
        if (generation !== mine) return; // superseded by a newer base series
        setAll(symbols, (symbol) => stateFor(response.benchmarks, symbol));
    } catch (cause) {
        if (generation !== mine) return;
        const message = cause instanceof Error ? cause.message : String(cause);
        setAll(symbols, () => ({status: "error", message}));
    }
}

/** Drop everything held unless it was computed for this window AND this base series. */
function adopt(serverUrl: string, scope: HoldingsScope, base: object): void {
    const key = keyOf(serverUrl, scope);
    if (key === windowKey && base === baseSeries) return;
    windowKey = key;
    baseSeries = base;
    generation += 1;
    entries = new Map();
}

export const benchmarkLines = {
    /** Each benchmark's state for the current base series; absent = never requested. */
    get entries(): ReadonlyMap<string, BenchmarkState> {
        return entries;
    },
    /**
     * Make sure every symbol in `symbols` is fetched (or being fetched) for
     * `base`, the series loaded for `scope`'s window. A new window or a reloaded
     * series forgets everything held for the old one; symbols already requested
     * for this one are left alone, and the rest go out in ONE request, so this
     * is safe to call from an effect on every change.
     */
    sync(serverUrl: string, scope: HoldingsScope, base: object, symbols: readonly string[]): void {
        adopt(serverUrl, scope, base);
        const missing = symbols.filter((symbol) => !entries.has(symbol));
        if (missing.length > 0) void fetchBatch(serverUrl, scope, missing);
    },
    /** Forget one symbol's failure and ask again (the hint's retry). */
    retry(serverUrl: string, scope: HoldingsScope, base: object, symbol: string): void {
        if (keyOf(serverUrl, scope) !== windowKey || base !== baseSeries) return;
        void fetchBatch(serverUrl, scope, [symbol]);
    },
    /** Drop everything (tests). */
    reset(): void {
        windowKey = null;
        baseSeries = null;
        generation += 1;
        entries = new Map();
    },
};
