// `decodeBenchmarks` — `GET /api/holdings/benchmarks`. Its own file, like the
// profiles decoder's tests, only to keep `nativeDecode.test.ts`'s churn down.

import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {decodeBenchmarkCatalog, decodeBenchmarks} from "./nativeDecode";

// The literal shape `WireBenchmarks` serializes to (benchmarks_api.rs), as the
// server's own integration test (`the_line_is_the_same_money_into_the_benchmark`)
// produces it.
const WIRE = {
    benchmarks: [
        {
            symbol: "SPY",
            label: "S&P 500 (SPY)",
            slot: 0,
            points: [
                {date: "2025-01-31", value: 1100, returnPct: 0},
                {date: "2025-02-28", value: 1210, returnPct: 0.1},
                {date: "2025-03-31", value: 2112, returnPct: 0.155},
            ],
            stale: false,
            error: null,
        },
        {
            symbol: "GLD",
            label: "Gold (GLD)",
            slot: 7,
            points: [
                {date: "2025-01-31", value: null, returnPct: null},
                {date: "2025-02-28", value: null, returnPct: null},
                {date: "2025-03-31", value: null, returnPct: null},
            ],
            stale: true,
            error: "No GLD price history covers this window.",
        },
    ],
};

describe("UNIT decodeBenchmarks", () => {
    it("decodes lines, gaps, staleness and per-line errors", () => {
        const decoded = decodeBenchmarks(WIRE);
        expect(decoded.benchmarks.map((b) => b.symbol)).toEqual(["SPY", "GLD"]);
        expect(decoded.benchmarks[0].points.map((p) => p.value)).toEqual([1100, 1210, 2112]);
        expect(decoded.benchmarks[0].error).toBeNull();
        expect(decoded.benchmarks[1].points.map((p) => p.value)).toEqual([null, null, null]);
        expect(decoded.benchmarks[0].points.map((p) => p.returnPct)).toEqual([0, 0.1, 0.155]);
        expect(decoded.benchmarks[1].points.map((p) => p.returnPct)).toEqual([null, null, null]);
        expect(decoded.benchmarks[1]).toMatchObject({stale: true, error: "No GLD price history covers this window."});
    });

    it("freezes what it returns", () => {
        const decoded = decodeBenchmarks(WIRE);
        expect(Object.isFrozen(decoded.benchmarks)).toBe(true);
        expect(Object.isFrozen(decoded.benchmarks[0])).toBe(true);
        expect(Object.isFrozen(decoded.benchmarks[0].points)).toBe(true);
    });

    it("refuses a body that is not the benchmarks shape", () => {
        expect(() => decodeBenchmarks(null)).toThrow(ApiShapeError);
        expect(() => decodeBenchmarks({})).toThrow(ApiShapeError);
        expect(() => decodeBenchmarks({benchmarks: [{symbol: "SPY"}]})).toThrow(ApiShapeError);
    });

    it("refuses a value that is neither a number nor null, rather than drawing it as zero", () => {
        const bad = {benchmarks: [{...WIRE.benchmarks[0], points: [{date: "2025-01-31", value: "1100"}]}]};
        expect(() => decodeBenchmarks(bad)).toThrow(ApiShapeError);
        const absent = {benchmarks: [{...WIRE.benchmarks[0], points: [{date: "2025-01-31"}]}]};
        expect(() => decodeBenchmarks(absent)).toThrow(ApiShapeError);
        const missingDate = {benchmarks: [{...WIRE.benchmarks[0], points: [{value: 1, returnPct: 0}]}]};
        expect(() => decodeBenchmarks(missingDate)).toThrow(ApiShapeError);
    });

    // Plan 47 A1d: the Performance chart draws `returnPct`. Always on the wire
    // (null where the benchmark has no price), so a missing key is a renamed one.
    it("refuses a point without returnPct, or with one that is not a finite fraction", () => {
        const point = (extra: Record<string, unknown>) => ({benchmarks: [{...WIRE.benchmarks[0], points: [{date: "2025-01-31", value: 1100, ...extra}]}]});
        expect(decodeBenchmarks(point({returnPct: -0.25})).benchmarks[0].points[0].returnPct).toBe(-0.25);
        const absent = {benchmarks: [{...WIRE.benchmarks[0], points: [{date: "2025-01-31", value: 1100}]}]};
        expect(() => decodeBenchmarks(absent)).toThrow(ApiShapeError);
        expect(() => decodeBenchmarks(point({returnPct: "0.1"}))).toThrow(ApiShapeError);
        expect(() => decodeBenchmarks(point({returnPct: Number.POSITIVE_INFINITY}))).toThrow(ApiShapeError);
    });

    it.each([
        ["an absent stale", "stale"],
        ["an absent error", "error"],
        ["an absent symbol", "symbol"],
    ])("never quietly defaults %s", (_name, key) => {
        const line: Record<string, unknown> = {...WIRE.benchmarks[0]};
        delete line[key];
        expect(() => decodeBenchmarks({benchmarks: [line]})).toThrow(ApiShapeError);
    });
});

describe("UNIT decodeBenchmarkCatalog", () => {
    it("decodes the engine's catalog in slot order, frozen", () => {
        const catalog = decodeBenchmarkCatalog({
            benchmarks: [
                {symbol: "SPY", label: "S&P 500 (SPY)"},
                {symbol: "GLD", label: "Gold (GLD)"},
            ],
        });
        expect(catalog).toEqual([
            {symbol: "SPY", label: "S&P 500 (SPY)"},
            {symbol: "GLD", label: "Gold (GLD)"},
        ]);
        expect(Object.isFrozen(catalog)).toBe(true);
    });

    it("throws rather than offering a box with no label", () => {
        expect(() => decodeBenchmarkCatalog({benchmarks: [{symbol: "SPY"}]})).toThrow(ApiShapeError);
        expect(() => decodeBenchmarkCatalog([])).toThrow(ApiShapeError);
    });
});
