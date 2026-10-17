// Benchmark overlay contracts: the decoded shapes of
// GET /api/holdings/benchmarks/catalog and GET /api/holdings/benchmarks. Pure TS.
//
// The catalog itself lives ONLY in the engine (`BENCHMARKS` in
// crates/ledgeline-server/src/benchmarks_api.rs, plan 26 B1). Its ORDER is the
// colour contract: benchmark k always draws in palette slot k, which each line
// carries as `slot`, so ticking or unticking one box never repaints another
// line. The portfolio they are compared with takes no slot at all — it is drawn
// in the foreground ink (`HoldingsTrend`) — which is what lets every catalog
// entry have a categorical hue of its own.

import type {ISODate} from "../domain/types";

/** One benchmark: a dividend-adjusted ETF proxy for a standard measure. */
export interface BenchmarkDef {
    symbol: string;
    /** The measure, then the proxy: "S&P 500 (SPY)". */
    label: string;
}

/** The engine's catalog, in slot order. */
export type BenchmarkCatalog = readonly BenchmarkDef[];

/** One simulated line, index-aligned to the value-over-time series it was computed for. */
export interface BenchmarkLine {
    symbol: string;
    /** The catalog label, e.g. "S&P 500 (SPY)". */
    label: string;
    /** The palette slot to draw in: the benchmark's catalog position. */
    slot: number;
    points: readonly {date: ISODate; value: number | null}[];
    /** A refresh failed and an older cache drew the line. */
    stale: boolean;
    /** Why there is no line, when there is not. */
    error: string | null;
}

export interface BenchmarksResponse {
    benchmarks: readonly BenchmarkLine[];
}
