import {describe, expect, it} from "vitest";
import {gainTrend, paddedDomain, paddedReturnDomain, returnTrend, trendOf} from "./yAxis";

describe("UNIT holdings yAxis — the anchored, padded domain (plan 47 D1–D3)", () => {
    it("pads the anchor's range 10 % above and 20 % below", () => {
        // A quiet week (101–102) inside a year that ranged 80–120: drawn against the year.
        expect(paddedDomain({min: 80, max: 120}, [[101, 102]])).toEqual([72, 124]);
    });

    it("lets the shown points widen the anchor, so the line is never clipped", () => {
        // A new high today, above the anchor's max (and a benchmark below its min).
        expect(
            paddedDomain({min: 100, max: 110}, [
                [105, 130],
                [90, 95],
            ])
        ).toEqual([82, 134]);
    });

    it("skips leading points ≤ 0 — the jump from $0 when the account opened", () => {
        expect(paddedDomain(null, [[0, 0, 100, 110]])).toEqual([98, 111]);
        expect(paddedDomain(null, [[null, -5, 100, 110]])).toEqual([98, 111]);
    });

    it("keeps a zero AFTER the first positive point: a sold-out position is real", () => {
        expect(paddedDomain(null, [[100, 0, 50]])).toEqual([0, 110]);
    });

    it("floors at zero when nothing is negative", () => {
        // 20 % of a 10–100 range below 10 would be −8.
        expect(paddedDomain({min: 10, max: 100}, [])).toEqual([0, 109]);
    });

    it("does not floor a range that is genuinely negative", () => {
        expect(paddedDomain({min: -50, max: 50}, [])).toEqual([-70, 60]);
    });

    it("pads a flat range by a share of the value instead", () => {
        expect(paddedDomain({min: 200, max: 200}, [[200, 200]])).toEqual([160, 220]);
    });

    it("answers null when there is nothing positive to anchor on", () => {
        expect(paddedDomain(null, [])).toBeNull();
        expect(paddedDomain(null, [[0, 0, null]])).toBeNull();
        expect(paddedDomain({min: 0, max: 0}, [[0, 0]])).toBeNull();
    });

    it("ignores an anchor that is not a pair of finite numbers", () => {
        expect(paddedDomain({min: Number.NaN, max: 120}, [[100, 110]])).toEqual([98, 111]);
    });
});

describe("UNIT holdings yAxis — the period's direction (plan 47 D4)", () => {
    it("compares the first non-null point with the last", () => {
        expect(trendOf([null, 100, 90, 120])).toBe("up");
        expect(trendOf([120, 130, 100, null])).toBe("down");
        expect(trendOf([100, 150, 100])).toBe("flat");
    });

    it("is flat with fewer than two values", () => {
        expect(trendOf([])).toBe("flat");
        expect(trendOf([null, 5, null])).toBe("flat");
    });
});

describe("UNIT holdings yAxis — the Performance domain (plan 47 A1c)", () => {
    const close = (domain: [number, number] | null, expected: [number, number]): void => {
        expect(domain).not.toBeNull();
        expect(domain?.[0]).toBeCloseTo(expected[0], 10);
        expect(domain?.[1]).toBeCloseTo(expected[1], 10);
    };

    it("pads the return anchor's range 10 % above and 20 % below", () => {
        // A year that ran −10 %..+30 % around a quiet +1 % window.
        close(paddedReturnDomain({min: -0.1, max: 0.3}, [[0, 0.01]]), [-0.18, 0.34]);
    });

    it("never floors at zero: returns go negative", () => {
        close(paddedReturnDomain({min: 0.05, max: 0.25}, [[0, 0.1]]), [-0.05, 0.275]);
        close(paddedReturnDomain(null, [[0, -0.2, -0.1]]), [-0.24, 0.02]);
    });

    it("lets the shown line and every benchmark widen the anchor", () => {
        close(
            paddedReturnDomain({min: 0, max: 0.1}, [
                [0, 0.05],
                [0, 0.2],
                [-0.1, 0],
            ]),
            [-0.16, 0.23]
        );
    });

    it("keeps a leading 0 % — it is the first point, not an account opening", () => {
        close(paddedReturnDomain(null, [[0, 0.1]]), [-0.02, 0.11]);
    });

    it("draws a window where nothing moved against a fixed span, so the 0 % line still shows", () => {
        close(paddedReturnDomain(null, [[0, 0, 0]]), [-0.02, 0.01]);
        close(paddedReturnDomain({min: 0.1, max: 0.1}, []), [0.08, 0.11]);
    });

    it("answers null only when there is no return at all", () => {
        expect(paddedReturnDomain(null, [[null, null]])).toBeNull();
        expect(paddedReturnDomain({min: Number.NaN, max: 1}, [])).toBeNull();
    });
});

describe("UNIT holdings yAxis — the Value colour follows the gain (plan 47 A1a)", () => {
    it("is the gain's sign, whatever the value did", () => {
        // $60k sold out of a fund that rose: value down, gain up — green.
        expect(gainTrend(12_000, [100_000, 52_000])).toBe("up");
        expect(gainTrend(-500, [100_000, 160_000])).toBe("down");
        expect(gainTrend(0, [100_000, 160_000])).toBe("flat");
    });

    it("falls back to value first-vs-last when the gain is unknown", () => {
        expect(gainTrend(null, [100_000, 52_000])).toBe("down");
        expect(gainTrend(null, [100_000, 120_000])).toBe("up");
        expect(gainTrend(Number.NaN, [100_000, 120_000])).toBe("up");
    });
});

describe("UNIT holdings yAxis — the Performance colour (plan 47 A1b)", () => {
    it("is the sign of the last return", () => {
        expect(returnTrend([0, -0.1, 0.02])).toBe("up");
        expect(returnTrend([0, 0.3, -0.01, null])).toBe("down");
        expect(returnTrend([0, 0.2, 0])).toBe("flat");
        expect(returnTrend([null, null])).toBe("flat");
    });
});
