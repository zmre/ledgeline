import {describe, expect, it} from "vitest";
import {stackEmptyReason} from "./periodStackEmpty";

const series = (...values: number[][]) => values.map((v) => ({values: v}));

describe("UNIT PeriodStackChart empty rule", () => {
    it("says there is nothing when there are no buckets", () => {
        expect(stackEmptyReason([], [], [])).toBe("no-data");
    });

    it("says there is nothing when every series value and every net is zero", () => {
        expect(stackEmptyReason(["a", "b"], series([0, 0], [0, 0]), [0, 0])).toBe("no-data");
        expect(stackEmptyReason(["a", "b"], [], [0, 0], 2)).toBe("no-data");
    });

    it("draws a net alone, with no series under it", () => {
        expect(stackEmptyReason(["a", "b"], [], [0, 5], 2)).toBeNull();
    });

    it("refuses fewer buckets than the caller's minimum — a number, not a trend", () => {
        expect(stackEmptyReason(["2026"], series([5]), [5], 2)).toBe("too-few");
        expect(stackEmptyReason(["2026"], series([5]), [5])).toBeNull();
    });

    it("draws two buckets", () => {
        expect(stackEmptyReason(["a", "b"], series([5, 6]), [5, 6], 2)).toBeNull();
    });

    it("reads a short values array as zero past its end", () => {
        expect(stackEmptyReason(["a", "b"], series([0]), [0, 0])).toBe("no-data");
        expect(stackEmptyReason(["a", "b"], series([0]), [])).toBe("no-data");
    });
});
