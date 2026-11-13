import {describe, expect, it} from "vitest";
import {BASE_ROOT_PX, DEFAULT_SCALE, remToPx, SCALE_STEPS, stepScale} from "./scale";

describe("UNIT scale", () => {
    describe("SCALE_STEPS", () => {
        it("is the plan's ladder, ascending, with actual size on it", () => {
            expect(SCALE_STEPS).toEqual([70, 80, 90, 100, 110, 125, 150, 175, 200]);
            expect(SCALE_STEPS).toContain(DEFAULT_SCALE);
        });
    });

    describe("stepScale", () => {
        it("moves one step either way from a step", () => {
            expect(stepScale(100, "in")).toBe(110);
            expect(stepScale(110, "in")).toBe(125);
            expect(stepScale(100, "out")).toBe(90);
            expect(stepScale(150, "out")).toBe(125);
        });

        it("walks the whole ladder in both directions", () => {
            let at = SCALE_STEPS[0];
            const up = [at];
            while (at !== stepScale(at, "in")) up.push((at = stepScale(at, "in")));
            expect(up).toEqual(SCALE_STEPS);
            const down = [at];
            while (at !== stepScale(at, "out")) down.push((at = stepScale(at, "out")));
            expect(down).toEqual([...SCALE_STEPS].reverse());
        });

        it("snaps an off-step value to the next step in the direction asked", () => {
            expect(stepScale(133, "in")).toBe(150);
            expect(stepScale(133, "out")).toBe(125);
            expect(stepScale(101, "out")).toBe(100);
            expect(stepScale(99, "in")).toBe(100);
        });

        it("clamps at both ends, including for hand-edited values past them", () => {
            expect(stepScale(200, "in")).toBe(200);
            expect(stepScale(70, "out")).toBe(70);
            expect(stepScale(300, "in")).toBe(200);
            expect(stepScale(300, "out")).toBe(200);
            expect(stepScale(50, "out")).toBe(70);
            expect(stepScale(50, "in")).toBe(70);
        });

        it("treats a non-finite current value as actual size", () => {
            expect(stepScale(Number.NaN, "in")).toBe(110);
            expect(stepScale(Number.POSITIVE_INFINITY, "out")).toBe(90);
        });
    });

    describe("remToPx", () => {
        it("converts against the root it is given", () => {
            expect(remToPx(2.5, BASE_ROOT_PX)).toBe(40);
            expect(remToPx(2.5, 32)).toBe(80);
            expect(remToPx(0.75, 11.2)).toBeCloseTo(8.4);
        });

        it("defaults to the live root, which is the base with no document", () => {
            expect(remToPx(1)).toBe(BASE_ROOT_PX);
        });
    });
});
