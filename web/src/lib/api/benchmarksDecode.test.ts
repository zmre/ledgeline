import {describe, expect, it} from "vitest";
import {ApiShapeError} from "./client";
import {decodeBenchmarks} from "./benchmarksDecode";

// The literal shape `WireBenchmarks` serializes to (benchmarks_api.rs), as the
// server's own integration test (`the_line_is_the_same_money_into_the_benchmark`)
// produces it.
const WIRE = {
    base: "$",
    benchmarks: [
        {
            symbol: "SPY",
            label: "S&P 500 (SPY)",
            points: [
                {date: "2025-01-31", value: 1100},
                {date: "2025-02-28", value: 1210},
                {date: "2025-03-31", value: 2112},
            ],
            pricedThrough: "2025-03-31",
            stale: false,
            unvaluedFlows: 0,
            error: null,
        },
        {
            symbol: "GLD",
            label: "Gold (GLD)",
            points: [
                {date: "2025-01-31", value: null},
                {date: "2025-02-28", value: null},
                {date: "2025-03-31", value: null},
            ],
            pricedThrough: null,
            stale: true,
            unvaluedFlows: 2,
            error: "No GLD price history covers this window.",
        },
    ],
};

describe("UNIT decodeBenchmarks", () => {
    it("decodes lines, gaps, staleness and per-line errors", () => {
        const decoded = decodeBenchmarks(WIRE);
        expect(decoded.base).toBe("$");
        expect(decoded.benchmarks.map((b) => b.symbol)).toEqual(["SPY", "GLD"]);
        expect(decoded.benchmarks[0].points.map((p) => p.value)).toEqual([1100, 1210, 2112]);
        expect(decoded.benchmarks[0].error).toBeNull();
        expect(decoded.benchmarks[1].points.map((p) => p.value)).toEqual([null, null, null]);
        expect(decoded.benchmarks[1]).toMatchObject({pricedThrough: null, stale: true, unvaluedFlows: 2, error: "No GLD price history covers this window."});
    });

    it("freezes what it returns", () => {
        const decoded = decodeBenchmarks(WIRE);
        expect(Object.isFrozen(decoded.benchmarks)).toBe(true);
        expect(Object.isFrozen(decoded.benchmarks[0].points)).toBe(true);
    });

    it("refuses a body that is not the benchmarks shape", () => {
        expect(() => decodeBenchmarks(null)).toThrow(ApiShapeError);
        expect(() => decodeBenchmarks({base: "$"})).toThrow(ApiShapeError);
        expect(() => decodeBenchmarks({base: "$", benchmarks: [{symbol: "SPY"}]})).toThrow(ApiShapeError);
    });

    it("refuses a value that is neither a number nor null, rather than drawing it as zero", () => {
        const bad = {base: "$", benchmarks: [{...WIRE.benchmarks[0], points: [{date: "2025-01-31", value: "1100"}]}]};
        expect(() => decodeBenchmarks(bad)).toThrow(ApiShapeError);
        const missingDate = {base: "$", benchmarks: [{...WIRE.benchmarks[0], points: [{value: 1}]}]};
        expect(() => decodeBenchmarks(missingDate)).toThrow(ApiShapeError);
    });
});
