import {describe, expect, it} from "vitest";
import {colorAt, DEFAULT_PALETTE} from "../format/palette";
import {benchmarkSlot, BENCHMARKS} from "./benchmarks";

describe("UNIT holdings benchmarks colour contract", () => {
    // The portfolio is drawn in the foreground ink, so every catalog entry gets
    // a categorical slot of its own. When the portfolio held slot 0 of the old
    // eight-slot palette, the eighth benchmark (GLD) fell past the end into the
    // muted tail grey.
    it("gives every catalog benchmark its own categorical colour, none of them the tail grey or the ink", () => {
        const colors = BENCHMARKS.map((b) => colorAt(DEFAULT_PALETTE, benchmarkSlot(b.symbol)));

        expect(new Set(colors).size).toBe(BENCHMARKS.length);
        for (const color of colors) {
            expect(DEFAULT_PALETTE.categorical).toContain(color);
            expect(color).not.toBe(DEFAULT_PALETTE.other);
            expect(color).not.toBe(DEFAULT_PALETTE.flowNet);
        }
    });

    it("fixes each benchmark's slot by catalog position", () => {
        expect(BENCHMARKS.map((b) => benchmarkSlot(b.symbol))).toEqual(BENCHMARKS.map((_, i) => i));
    });
});
