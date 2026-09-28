import {describe, expect, it} from "vitest";
import {dec} from "$lib/domain/money";
import type {BenchmarkLine} from "$lib/holdings/benchmarks";
import type {HoldingsPoint, HoldingsSeries} from "$lib/holdings/types";
import {benchmarkOverlays, trendLabels} from "./trendView";

const point = (bucket: string, date: string, label = bucket): HoldingsPoint => ({date, bucket, label, marketValue: dec(100, 0), basis: null});

const series = (points: HoldingsPoint[]): HoldingsSeries => ({base: "$", points, hasBasis: false});

describe("UNIT holdings trendView", () => {
    describe("trendLabels", () => {
        it("labels daily buckets by month and day", () => {
            expect(trendLabels([point("2026-09-27", "2026-09-27"), point("2026-09-28", "2026-09-28")])).toEqual(["Sep 27", "Sep 28"]);
        });

        it("labels a weekly bucket by the date its value is taken at, not its ISO week number", () => {
            const weeks = [point("2026-W38", "2026-09-20", "W38 2026"), point("2026-W40", "2026-09-28", "W40 2026")];
            expect(trendLabels(weeks)).toEqual(["Sep 20", "Sep 28"]);
        });

        it("adds the year when the chart spans two", () => {
            expect(trendLabels([point("2025-12-31", "2025-12-31"), point("2026-01-02", "2026-01-02")])).toEqual(["Dec 31 '25", "Jan 2 '26"]);
        });

        it("keeps the engine's label for months, quarters and years", () => {
            expect(trendLabels([point("2026-08", "2026-08-31", "Aug 2026"), point("2026-Q3", "2026-09-28", "Q3 2026")])).toEqual(["Aug 2026", "Q3 2026"]);
        });
    });

    describe("benchmarkOverlays", () => {
        const trend = series([point("2026-08", "2026-08-31"), point("2026-09", "2026-09-28")]);
        const line = (symbol: string, dates: string[], values: (number | null)[]): BenchmarkLine => ({
            symbol,
            points: dates.map((date, i) => ({date, value: values[i] ?? null})),
            stale: false,
            error: null,
        });
        const lines: Record<string, BenchmarkLine> = {
            SPY: line("SPY", ["2026-08-31", "2026-09-28"], [100, 110]),
            GLD: line("GLD", ["2026-08-31", "2026-09-28"], [null, 90]),
            QQQ: line("QQQ", ["2026-07-31", "2026-08-31"], [1, 2]),
        };
        const lookup = (symbol: string): BenchmarkLine | null => lines[symbol] ?? null;

        it("draws each ticked line dashed, under its catalog label and in its catalog slot, keeping null gaps", () => {
            expect(benchmarkOverlays(trend, ["SPY", "GLD"], lookup)).toEqual([
                {name: "S&P 500 (SPY)", values: [100, 110], dashed: true, slot: 1},
                {name: "Gold (GLD)", values: [null, 90], dashed: true, slot: 8},
            ]);
        });

        it("keeps a benchmark's slot whatever else is ticked", () => {
            expect(benchmarkOverlays(trend, ["GLD"], lookup).map((s) => s.slot)).toEqual([8]);
        });

        it("drops a line computed for other dates rather than stretching it onto this chart", () => {
            expect(benchmarkOverlays(trend, ["QQQ"], lookup)).toEqual([]);
        });

        it("skips a ticked benchmark that has no line yet", () => {
            expect(benchmarkOverlays(trend, ["DIA", "SPY"], lookup).map((s) => s.name)).toEqual(["S&P 500 (SPY)"]);
        });
    });
});
