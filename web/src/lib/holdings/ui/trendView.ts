// Pure helpers behind the value-over-time chart: readable x labels for fine
// buckets, and the benchmark overlay lines aligned to the portfolio's points.
// No Svelte runtime — node-testable.

import type {PeriodSeries} from "$lib/components/periodSeries";
import {benchmarkLabel, benchmarkSlot, type BenchmarkLine} from "$lib/holdings/benchmarks";
import type {HoldingsPoint, HoldingsSeries} from "$lib/holdings/types";
import {MONTH_NAMES} from "$lib/reports/periods";

const DAY_KEY = /^\d{4}-\d{2}-\d{2}$/;
const WEEK_KEY = /^\d{4}-W\d{2}$/;

/** "2026-09-28" → "Sep 28", or "Sep 28 '26" when the chart spans more than one year. */
function dayLabel(date: string, withYear: boolean): string {
    const month = MONTH_NAMES[Number(date.slice(5, 7)) - 1] ?? date.slice(5, 7);
    const day = Number(date.slice(8, 10));
    return withYear ? `${month} ${day} '${date.slice(2, 4)}` : `${month} ${day}`;
}

/**
 * The chart's x labels. Daily and weekly buckets are labelled by their DATE
 * ("Sep 28") — the engine's own labels for them are a raw ISO date and an ISO
 * week number ("W39 2026"), both fine in a table and unreadable on an axis. A
 * weekly point is the week's last day (clamped to asOf), which is the date its
 * value is taken at. Coarser buckets keep the engine's label ("Sep 2026").
 */
export function trendLabels(points: readonly HoldingsPoint[]): string[] {
    const first = points[0]?.date ?? "";
    const last = points.at(-1)?.date ?? "";
    const withYear = first.slice(0, 4) !== last.slice(0, 4);
    return points.map((p) => (DAY_KEY.test(p.bucket) || WEEK_KEY.test(p.bucket) ? dayLabel(p.date, withYear) : p.label));
}

/**
 * The overlay series for the ticked benchmarks that have a line for THIS
 * chart: dashed, in each benchmark's fixed palette slot, with `null` gaps where
 * the benchmark has no price.
 *
 * A line whose dates do not match the chart's point for point is left out
 * rather than stretched onto it — it was computed for another window (a scope
 * change raced its response), and index-aligning it would draw a comparison
 * against the wrong dates.
 */
export function benchmarkOverlays(trend: HoldingsSeries, picked: readonly string[], lineOf: (symbol: string) => BenchmarkLine | null): PeriodSeries[] {
    const dates = trend.points.map((p) => p.date);
    return picked.flatMap((symbol) => {
        const line = lineOf(symbol);
        if (line === null || line.points.length !== dates.length || line.points.some((p, i) => p.date !== dates[i])) return [];
        return [{name: benchmarkLabel(symbol), values: line.points.map((p) => p.value), dashed: true, slot: benchmarkSlot(symbol)}];
    });
}
