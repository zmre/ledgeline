import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import {compareBandLabel, compareCells, comparePctTitle, fmtComparePct, windowFigure} from "./compareColumns";

/** Dollars at two places, from cents. */
const usd = (cents: number): MixedAmount => new Map([["$", dec(cents, 2)]]);
const mixed = (entries: [string, number][]): MixedAmount => new Map(entries.map(([c, cents]) => [c, dec(cents, 2)]));
const cents = (ma: MixedAmount, commodity = "$"): number => {
    const d = ma.get(commodity);
    return d === undefined ? 0 : Number(d.m) * 10 ** (2 - d.p);
};

describe("UNIT reports/ui/compareColumns — the window figure", () => {
    const block = [usd(100), usd(200), usd(400)];

    it("a flow sums its block, a balance takes its last column and an opening its first", () => {
        expect(cents(windowFigure(block, "flow"))).toBe(700);
        expect(cents(windowFigure(block, "balance"))).toBe(400);
        expect(cents(windowFigure(block, "opening"))).toBe(100);
        expect(windowFigure([], "balance").size).toBe(0);
    });

    it("splits 2N values at N, actual first", () => {
        const cells = compareCells([usd(300_000), usd(300_000), usd(250_000), usd(250_000)], 2, "flow", "$");
        expect(cents(cells.actual)).toBe(600_000);
        expect(cents(cells.projected)).toBe(500_000);
        expect(cents(cells.delta)).toBe(100_000);
        expect(cells.pct).toBe(20);
    });
});

describe("UNIT reports/ui/compareColumns — Δ and Δ%", () => {
    it("Δ = actual − projected, so coming in below the projection is negative", () => {
        const cells = compareCells([usd(3_185_000), usd(3_370_000)], 1, "balance", "$");
        expect(cents(cells.delta)).toBe(-185_000);
        expect(cells.pct).toBe(-5.5);
    });

    it("the denominator is the projection's MAGNITUDE: projected −$100, actual +$50 is +$150, +150.0%", () => {
        const cells = compareCells([usd(5_000), usd(-10_000)], 1, "flow", "$");
        expect(cents(cells.delta)).toBe(15_000);
        expect(cells.pct).toBe(150);
        expect(fmtComparePct(cells.pct)).toBe("150.0%");
    });

    it("a zero, absent or rounds-to-zero projection has no Δ% (—), but still a Δ", () => {
        expect(compareCells([usd(100), usd(0)], 1, "flow", "$").pct).toBeNull();
        expect(compareCells([usd(100), new Map()], 1, "flow", "$").pct).toBeNull();
        // $0.004 reads $0.00 at two places: a ratio against it is noise.
        const tiny: MixedAmount = new Map([["$", dec(4, 3)]]);
        expect(compareCells([usd(100), tiny], 1, "flow", "$", 2).pct).toBeNull();
        // Without the precision only an exact zero is refused.
        expect(compareCells([usd(100), tiny], 1, "flow", "$").pct).not.toBeNull();
        expect(cents(compareCells([usd(100), usd(0)], 1, "flow", "$").delta)).toBe(100);
        expect(fmtComparePct(null)).toBe("—");
    });

    it("an unpriced second commodity on either side leaves Δ% empty and Δ per commodity", () => {
        const cells = compareCells(
            [
                mixed([
                    ["$", 1000],
                    ["AAPL", 300],
                ]),
                usd(800),
            ],
            1,
            "balance",
            "$"
        );
        expect(cells.pct).toBeNull();
        expect(cents(cells.delta)).toBe(200);
        expect(cents(cells.delta, "AAPL")).toBe(300);
        expect(compareCells([usd(800), mixed([["EUR", 5]])], 1, "balance", "$").pct).toBeNull();
    });

    it("no base commodity means no Δ%", () => {
        expect(compareCells([usd(200), usd(100)], 1, "flow", null).pct).toBeNull();
    });

    it("rounds half away from zero at ±x.x5", () => {
        // +1.05% and −1.05%
        expect(compareCells([usd(10_105), usd(10_000)], 1, "flow", "$").pct).toBe(1.1);
        expect(compareCells([usd(9_895), usd(10_000)], 1, "flow", "$").pct).toBe(-1.1);
    });

    it("caps the display at ±999%, keeping the exact figure for the title", () => {
        const big = compareCells([usd(123_450), usd(1_000)], 1, "flow", "$").pct;
        expect(big).toBe(12245);
        expect(fmtComparePct(big)).toBe(">999%");
        expect(comparePctTitle(big)).toBe("12245.0%");
        expect(fmtComparePct(-1000)).toBe("<-999%");
        expect(fmtComparePct(999.9)).toBe("999.9%");
        expect(comparePctTitle(null)).toBe("");
    });

    it("names the band's middle heading", () => {
        expect(compareBandLabel("1 Jul 2026")).toBe("As projected from 1 Jul 2026");
    });
});
