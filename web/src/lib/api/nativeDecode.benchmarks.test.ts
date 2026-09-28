// `decodeBenchmarks` — `GET /api/holdings/benchmarks`. Its own file, like the
// profiles decoder's tests, only to keep `nativeDecode.test.ts`'s churn down.

import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {decodeBenchmarks} from "./nativeDecode";

// The literal shape `WireBenchmarks` serializes to (benchmarks_api.rs), as the
// server's own integration test (`the_line_is_the_same_money_into_the_benchmark`)
// produces it.
const WIRE = {
    benchmarks: [
        {
            symbol: "SPY",
            points: [
                {date: "2025-01-31", value: 1100},
                {date: "2025-02-28", value: 1210},
                {date: "2025-03-31", value: 2112},
            ],
            stale: false,
            error: null,
        },
        {
            symbol: "GLD",
            points: [
                {date: "2025-01-31", value: null},
                {date: "2025-02-28", value: null},
                {date: "2025-03-31", value: null},
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
        const missingDate = {benchmarks: [{...WIRE.benchmarks[0], points: [{value: 1}]}]};
        expect(() => decodeBenchmarks(missingDate)).toThrow(ApiShapeError);
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
