import {describe, expect, it} from "vitest";
import {paddedDomain, trendOf} from "./yAxis";

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
