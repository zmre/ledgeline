import {describe, expect, it} from "vitest";
import type {StackBarRow} from "./stackBars";
import {seriesAt, topToBottom} from "./stackHit";

/** Series 0 and 2 stack up, series 1 down; the same bands `PeriodStackChart.bandsAt` builds. */
const NEGATIVE = [false, true, false];
const ROWS: StackBarRow[] = [
    {
        v: [10, -5, 20],
        band: [
            [0, 10],
            [0, -5],
            [10, 30],
        ],
    },
    {
        v: [30, 0, 10],
        band: [
            [0, 30],
            [0, 0],
            [30, 40],
        ],
    },
];

describe("UNIT stackHit", () => {
    describe("seriesAt", () => {
        it("finds the band a value falls in, on either side of zero", () => {
            expect(seriesAt(ROWS, 0, 5)).toBe(0);
            expect(seriesAt(ROWS, 0, 25)).toBe(2);
            expect(seriesAt(ROWS, 0, -3)).toBe(1);
        });

        it("finds nothing above or below the stacks", () => {
            expect(seriesAt(ROWS, 0, 31)).toBeNull();
            expect(seriesAt(ROWS, 0, -6)).toBeNull();
        });

        it("never lands on a zero-thickness band", () => {
            expect(seriesAt(ROWS, 1, 0)).toBe(0);
            expect(seriesAt(ROWS, 1, -0.1)).toBeNull();
        });

        it("interpolates between buckets, as a linear area is drawn", () => {
            // Halfway: series 0 spans [0, 20], series 2 spans [20, 35].
            expect(seriesAt(ROWS, 0.5, 19)).toBe(0);
            expect(seriesAt(ROWS, 0.5, 21)).toBe(2);
            // Series 1 has shrunk to [0, -2.5].
            expect(seriesAt(ROWS, 0.5, -2)).toBe(1);
            expect(seriesAt(ROWS, 0.5, -3)).toBeNull();
        });

        it("finds nothing outside the buckets", () => {
            expect(seriesAt(ROWS, -0.1, 5)).toBeNull();
            expect(seriesAt(ROWS, 1.1, 5)).toBeNull();
            expect(seriesAt([], 0, 5)).toBeNull();
            expect(seriesAt(ROWS, Number.NaN, 5)).toBeNull();
        });
    });

    describe("topToBottom", () => {
        it("lists the up stack outermost first, then the down stack from the rule outward", () => {
            expect(topToBottom([10, -5, 20, -8], [false, true, false, true])).toEqual([2, 0, 1, 3]);
        });

        it("leaves out a zero", () => {
            expect(topToBottom([30, 0, 10], NEGATIVE)).toEqual([2, 0]);
        });
    });
});
