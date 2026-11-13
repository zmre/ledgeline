// The span summary itself (leaves, totals, pace, health, labels) is the
// engine's now and is locked to the TypeScript it replaced by
// `crates/ledgeline-core/tests/budget_summary_golden.rs`. What is tested here is
// the part that stayed: sizing a bar.
import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "../domain/money";
import {barGeometry, barGradient, fillClip, primaryValue} from "./budgetSummary";

/** A single-commodity `$` amount at places 0. */
const usd = (n: number): MixedAmount => new Map([["$", dec(n, 0)]]);

describe("UNIT budgetSummary — primaryValue", () => {
    it("returns the single-commodity magnitude, 0 for empty, null for multi-commodity", () => {
        expect(primaryValue(usd(352))).toBe(352);
        expect(primaryValue(new Map())).toBe(0);
        expect(
            primaryValue(
                new Map([
                    ["$", dec(100, 0)],
                    ["EUR", dec(50, 0)],
                ])
            )
        ).toBeNull();
    });
});

describe("UNIT budgetSummary — barGeometry", () => {
    it("leaves headroom past the marker when under budget", () => {
        const g = barGeometry(352, 400, 400); // scaleMax = 500
        expect(g.markerPct).toBeCloseTo(80, 5); // 400/500
        expect(g.fillPct).toBeCloseTo(70.4, 5); // 352/500
        expect(g.ratio).toBeCloseTo(0.88, 5);
    });

    it("saturates the fill and slides the marker left when over budget", () => {
        const g = barGeometry(210, 150, 150); // scaleMax = 210
        expect(g.markerPct).toBeCloseTo(71.4286, 3); // 150/210
        expect(g.fillPct).toBe(100); // one fill, no split at the marker
        expect(g.ratio).toBeCloseTo(1.4, 5);
    });

    it("places the pace mark on the same scale as the marker", () => {
        const g = barGeometry(352, 400, 200); // half way through the span
        expect(g.pacePct).toBeCloseTo(40, 5); // 200/500
        expect(g.markerPct).toBeCloseTo(80, 5);
    });

    it("drops the pace mark when it would be drawn on top of the goal marker", () => {
        expect(barGeometry(352, 400, 400).pacePct).toBeNull();
        // Within half a percentage point of the marker: one tick, not two.
        expect(barGeometry(352, 400, 399).pacePct).toBeNull();
    });

    it("handles a zero budget (spent with no goal amount)", () => {
        expect(barGeometry(375, 0, 0)).toEqual({fillPct: 100, markerPct: 100, pacePct: null, ratio: null});
        expect(barGeometry(0, 0, 0)).toEqual({fillPct: 0, markerPct: 100, pacePct: null, ratio: null});
    });
});

describe("UNIT budgetSummary — barGradient (plan 41: a gradient fixed to the track)", () => {
    it("ramps an expense green → amber → orange up to the goal marker, and red past it", () => {
        const g = barGeometry(352, 400, 400); // marker at 80 %
        expect(barGradient("expense", g)).toBe(
            "linear-gradient(to right, var(--color-success) 0%, var(--color-warning) 60%, var(--color-orange) 80%, var(--color-error) 80%, var(--color-error) 100%)"
        );
    });

    it("keys the stops to the marker, not the fill, so the same goal draws the same ramp at any spend", () => {
        expect(barGradient("expense", barGeometry(100, 400, 400))).toBe(barGradient("expense", barGeometry(390, 400, 400)));
    });

    it("never paints income red: a calm info → success ramp, success past the target", () => {
        const css = barGradient("revenue", barGeometry(13_000, 10_000, 10_000));
        expect(css).not.toContain("error");
        expect(css).not.toContain("orange");
        expect(css).toContain("var(--color-info) 0%");
    });

    it("reads any spending against a zero goal as overspend, and any income against one as a win", () => {
        const zero = barGeometry(50, 0, 0);
        expect(barGradient("expense", zero)).toContain("--color-error");
        expect(barGradient("revenue", zero)).not.toContain("--color-error");
    });
});

describe("UNIT budgetSummary — fillClip", () => {
    it("reveals exactly the fill's share of the full-width layer", () => {
        expect(fillClip(70.4)).toBe("inset(0 29.6% 0 0)");
        expect(fillClip(0)).toBe("inset(0 100% 0 0)");
        expect(fillClip(100)).toBe("inset(0 0% 0 0)");
    });
});
