import {describe, expect, it} from "vitest";
import {colorAt, DEFAULT_PALETTE} from "../format/palette";

/**
 * How many benchmarks the engine's catalog holds (`BENCHMARKS` in
 * benchmarks_api.rs, pinned entry for entry by benchmarks_endpoints.rs
 * `the_catalog_lists_every_benchmark_in_slot_order`). Each line arrives with its
 * catalog position as `slot`.
 */
const CATALOG_SIZE = 8;

describe("UNIT holdings benchmarks colour contract", () => {
    // The portfolio is drawn in the foreground ink, so every catalog entry gets
    // a categorical slot of its own. When the portfolio held slot 0 of the old
    // eight-slot palette, the eighth benchmark (GLD) fell past the end into the
    // muted tail grey.
    it("gives every catalog slot its own categorical colour, none of them the tail grey or the ink", () => {
        const colors = Array.from({length: CATALOG_SIZE}, (_, slot) => colorAt(DEFAULT_PALETTE, slot));

        expect(new Set(colors).size).toBe(CATALOG_SIZE);
        for (const color of colors) {
            expect(DEFAULT_PALETTE.categorical).toContain(color);
            expect(color).not.toBe(DEFAULT_PALETTE.other);
            expect(color).not.toBe(DEFAULT_PALETTE.flowNet);
        }
    });
});
