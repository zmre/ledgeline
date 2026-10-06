import {describe, expect, it} from "vitest";
import {stackBars, type StackBarLayout, type StackBarRow} from "./stackBars";

/** 1 value = 1 px, zero at y = 100, so a band reads straight off the path. */
const LAYOUT: StackBarLayout = {left: (i) => i * 50, width: 20, y: (v) => 100 - v, gap: 0, radius: 0};

/** The same stacking `PeriodStackChart.bandsAt` does, for building rows. */
function row(v: number[], negative: boolean[]): StackBarRow {
    let up = 0;
    let down = 0;
    const band = v.map((value, i): [number, number] => {
        if (negative[i]) return [down, (down += value)];
        return [up, (up += value)];
    });
    return {v, band};
}

describe("UNIT stackBars", () => {
    it("draws nothing for a zero", () => {
        const negative = [false, false, true];
        const bars = stackBars([row([10, 0, -5], negative), row([0, 0, 0], negative)], negative, LAYOUT);

        expect(bars.map((b) => b.key)).toEqual(["0:0", "0:2"]);
    });

    it("stacks each segment on the one before it, positives up from zero and negatives down", () => {
        const negative = [false, true, false];
        const bars = stackBars([row([10, -5, 20], negative)], negative, LAYOUT);

        expect(bars.map((b) => [b.series, b.d])).toEqual([
            [0, "M0,90H20V100H0Z"],
            [2, "M0,70H20V90H0Z"],
            [1, "M0,100H20V105H0Z"],
        ]);
    });

    it("places each bucket's bar at its left edge", () => {
        const negative = [false];
        const bars = stackBars([row([10], negative), row([10], negative)], negative, LAYOUT);

        expect(bars[1].d).toBe("M50,90H70V100H50Z");
    });

    it("opens the gap at every segment's inner end except the one on the zero rule", () => {
        const negative = [false, false, true, true];
        const bars = stackBars([row([10, 20, -5, -8], negative)], negative, {...LAYOUT, gap: 2});

        expect(bars.map((b) => b.d)).toEqual(["M0,90H20V100H0Z", "M0,70H20V88H0Z", "M0,100H20V105H0Z", "M0,107H20V113H0Z"]);
    });

    it("draws a segment thinner than the gap as nothing, never inverted", () => {
        const negative = [false, false];
        const [, thin] = stackBars([row([10, 1], negative)], negative, {...LAYOUT, gap: 2});

        expect(thin.d).toBe("M0,89H20V89H0Z");
    });

    it("rounds only the outermost segment of each side, at its data end", () => {
        const negative = [false, false, true];
        const bars = stackBars([row([10, 20, -30], negative)], negative, {...LAYOUT, radius: 4});

        expect(bars[0].d).toBe("M0,90H20V100H0Z");
        // Up: the top corners.
        expect(bars[1].d).toBe("M0,90V74A4,4 0 0 1 4,70H16A4,4 0 0 1 20,74V90Z");
        // Down: the bottom corners.
        expect(bars[2].d).toBe("M0,100H20V126A4,4 0 0 1 16,130H4A4,4 0 0 1 0,126Z");
    });

    it("never rounds past half the width or the segment's own height", () => {
        const negative = [false];
        const [bar] = stackBars([row([2], negative)], negative, {...LAYOUT, radius: 4});

        expect(bar.d).toBe("M0,100V100A2,2 0 0 1 2,98H18A2,2 0 0 1 20,100V100Z");
    });

    it("keeps a zero of a negative series on the negative side, so the up stack's top is still rounded", () => {
        // Series 1 is negative somewhere else, so its zero here is not on top of series 0.
        const negative = [false, true];
        const bars = stackBars([row([10, 0], negative)], negative, {...LAYOUT, radius: 4});

        expect(bars).toHaveLength(1);
        expect(bars[0].d).toContain("A4,4");
    });
});
