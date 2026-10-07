// Benchmark overlay contracts: the catalog of comparisons the Stocks chart can
// draw, and the decoded shape of GET /api/holdings/benchmarks. Pure TS.
//
// The catalog mirrors `BENCHMARKS` in crates/ledgeline-server/src/benchmarks_api.rs
// symbol for symbol (the server refuses anything else). It lives here too
// because its ORDER is the colour contract: benchmark k always draws in palette
// slot k, so ticking or unticking one box never repaints another line. The
// portfolio they are compared with takes no slot at all — it is drawn in the
// foreground ink (`HoldingsTrend`) — which is what lets all eight catalog
// entries have a categorical hue of their own.

import type {ISODate} from "../domain/types";

/** One benchmark: a dividend-adjusted ETF proxy for a standard measure. */
export interface BenchmarkDef {
    symbol: string;
    /** The measure, then the proxy: "S&P 500 (SPY)". */
    label: string;
}

export const BENCHMARKS: readonly BenchmarkDef[] = Object.freeze([
    {symbol: "SPY", label: "S&P 500 (SPY)"},
    {symbol: "DIA", label: "Dow Jones (DIA)"},
    {symbol: "QQQ", label: "Nasdaq-100 (QQQ)"},
    {symbol: "VTI", label: "US total market (VTI)"},
    {symbol: "BND", label: "US bonds (BND)"},
    {symbol: "VXUS", label: "International (VXUS)"},
    {symbol: "IWM", label: "Small cap (IWM)"},
    {symbol: "GLD", label: "Gold (GLD)"},
]);

/** True for a symbol in the catalog. */
export function isBenchmarkSymbol(value: unknown): value is string {
    return typeof value === "string" && BENCHMARKS.some((b) => b.symbol === value);
}

/** A catalog benchmark's label ("S&P 500 (SPY)"); the symbol itself for any other. */
export function benchmarkLabel(symbol: string): string {
    return BENCHMARKS.find((b) => b.symbol === symbol)?.label ?? symbol;
}

/**
 * The palette slot a benchmark's line is drawn in: its catalog position, so it
 * never depends on which others are shown. Only ever called with a catalog
 * symbol (`isBenchmarkSymbol` gates what reaches the picker).
 */
export function benchmarkSlot(symbol: string): number {
    return BENCHMARKS.findIndex((b) => b.symbol === symbol);
}

/** One simulated line, index-aligned to the value-over-time series it was computed for. */
export interface BenchmarkLine {
    symbol: string;
    points: readonly {date: ISODate; value: number | null}[];
    /** A refresh failed and an older cache drew the line. */
    stale: boolean;
    /** Why there is no line, when there is not. */
    error: string | null;
}

export interface BenchmarksResponse {
    benchmarks: readonly BenchmarkLine[];
}
