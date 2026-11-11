import {describe, expect, test} from "vitest";
import type {Dec, MixedAmount} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {MetricDelta} from "#lib/reports/insightsTypes.js";
import {
    baseMetric,
    BURN_MULTIPLE_BANDS,
    burnBand,
    deltaLine,
    extras,
    fmt,
    fmtBase,
    fmtDays,
    fmtMonths,
    fmtPct,
    fmtPctPoints,
    fmtRatio,
    fmtSignedAmount,
    fmtSignedPct,
    growthLine,
    monthlyAverage,
    pointsLine,
    signClass,
} from "./format";

const USD: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const styles = new Map<string, AmountStyle>([["$", USD]]);

const d = (m: bigint, p: number): Dec => ({m, p});
const usd = (cents: bigint): MixedAmount => new Map([["$", d(cents, 2)]]);

function metric(current: bigint, previous: bigint, pct: number | null): MetricDelta {
    return {current: usd(current), previous: usd(previous), delta: usd(current - previous), pct};
}

describe("insights format helpers", () => {
    test("formats the base commodity, 0 when absent", () => {
        expect(fmtBase(usd(123456n), "$", styles)).toBe("$1,234.56");
        expect(fmtBase(new Map(), "$", styles)).toBe("$0.00");
    });

    test("lists non-base commodities as extras", () => {
        const mixed: MixedAmount = new Map([
            ["$", d(1000n, 2)],
            ["EUR", d(4500n, 2)],
        ]);
        const lines = extras(mixed, "$", styles);
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("EUR");
    });

    test("deltaLine: revenue up is green, expenses up is red", () => {
        const up = metric(150000n, 100000n, 50);
        const rev = deltaLine(up, "$", styles, true);
        expect(rev.arrow).toBe("▲");
        expect(rev.klass).toBe("text-success");
        expect(rev.text).toBe("$500.00 (50.0%)");

        const exp = deltaLine(up, "$", styles, false);
        expect(exp.klass).toBe("text-error");
    });

    test("deltaLine: a decrease flips arrow and sentiment", () => {
        const down = metric(80000n, 100000n, -20);
        const good = deltaLine(down, "$", styles, true);
        expect(good.arrow).toBe("▼");
        expect(good.klass).toBe("text-error");
    });

    test("deltaLine: no change is neutral with no percent", () => {
        const flat = deltaLine(metric(100000n, 100000n, null), "$", styles, true);
        expect(flat.arrow).toBe("▪");
        expect(flat.klass).toBe("text-base-content/50");
        expect(flat.text).toBe("$0.00");
    });

    test("monthlyAverage divides the total across the months", () => {
        // $12,000.00 over 12 months → $1,000.00.
        expect(fmt("$", monthlyAverage(d(1200000n, 2), 12), styles)).toBe("$1,000.00");
        // Guard: zero months yields zero, not a divide-by-zero.
        expect(fmt("$", monthlyAverage(d(1200000n, 2), 0), styles)).toBe("$0.00");
    });

    test("signed amount and percent formatting", () => {
        expect(fmtSignedAmount(d(50000n, 2), "$", styles)).toBe("+$500.00");
        expect(fmtSignedAmount(d(-50000n, 2), "$", styles)).toBe("−$500.00");
        expect(fmtSignedAmount(null, "$", styles)).toBe("—");
        expect(fmtSignedPct(5.2)).toBe("+5.2%");
        expect(fmtSignedPct(-4)).toBe("−4.0%");
        expect(fmtSignedPct(null)).toBe("—");
    });

    test("signClass maps sign to a colour, for numbers and bigints", () => {
        expect(signClass(5)).toBe("text-success");
        expect(signClass(-1)).toBe("text-error");
        expect(signClass(0n)).toBe("text-base-content/50");
        expect(signClass(3n)).toBe("text-success");
        expect(signClass(null)).toBe("text-base-content/50");
    });
});

describe("startup-metrics format helpers (plans/37)", () => {
    test("fmtMonths: whole months from 10, one decimal below, singular for exactly one", () => {
        expect(fmtMonths(66.04584819154356)).toBe("66 months");
        expect(fmtMonths(186.05)).toBe("186 months");
        expect(fmtMonths(1234.4)).toBe("1,234 months");
        expect(fmtMonths(2.46)).toBe("2.5 months");
        expect(fmtMonths(1)).toBe("1 month");
        expect(fmtMonths(0.96)).toBe("1 month");
        expect(fmtMonths(0)).toBe("0 months");
    });

    test("fmtPctPoints: signed, one decimal, pp", () => {
        expect(fmtPctPoints(1.8632)).toBe("+1.9 pp");
        expect(fmtPctPoints(-0.44)).toBe("-0.4 pp");
        expect(fmtPctPoints(0.04)).toBe("0.0 pp");
    });

    test("fmtRatio: one decimal and a multiplication sign", () => {
        expect(fmtRatio(1.6192592592592592)).toBe("1.6×");
        expect(fmtRatio(3.368)).toBe("3.4×");
        expect(fmtRatio(0)).toBe("0.0×");
    });

    test("fmtPct and fmtDays: one decimal, grouped", () => {
        expect(fmtPct(120.83333333333333)).toBe("120.8%");
        expect(fmtPct(1234.56)).toBe("1,234.6%");
        expect(fmtPct(null)).toBe("—");
        expect(fmtDays(25.50338091660406)).toBe("25.5 days");
    });

    test("pointsLine: the change in points, coloured by sentiment, null without a comparison", () => {
        expect(pointsLine(81.8632607062359, 80, true)).toEqual({arrow: "▲", text: "1.9 pp", klass: "text-success"});
        expect(pointsLine(78, 80, true)).toEqual({arrow: "▼", text: "2.0 pp", klass: "text-error"});
        expect(pointsLine(80.01, 80, true)?.arrow).toBe("▪");
        expect(pointsLine(80, null, true)).toBeNull();
    });

    test("growthLine: a percentage that is itself the change", () => {
        expect(growthLine(0, true)).toEqual({arrow: "▪", text: "0.0% growth", klass: "text-base-content/50"});
        expect(growthLine(12.5, true)?.klass).toBe("text-success");
        expect(growthLine(-12.5, true)).toEqual({arrow: "▼", text: "12.5% growth", klass: "text-error"});
        expect(growthLine(null, true)).toBeNull();
    });

    test("burnBand: Sacks's bands, with the boundaries falling the documented way", () => {
        expect(BURN_MULTIPLE_BANDS.map((band) => band.label)).toEqual(["Very efficient", "Good", "Suspect", "Bad"]);
        expect(burnBand(0.56).label).toBe("Very efficient");
        expect(burnBand(0.56).klass).toBe("text-success");
        expect(burnBand(1).label).toBe("Good");
        expect(burnBand(1.62).label).toBe("Good");
        expect(burnBand(1.62).klass).toBe("");
        expect(burnBand(2).label).toBe("Good");
        expect(burnBand(2.01).label).toBe("Suspect");
        expect(burnBand(2.01).klass).toBe("text-warning");
        expect(burnBand(3).label).toBe("Suspect");
        expect(burnBand(3.5).label).toBe("Bad");
        expect(burnBand(3.5).klass).toBe("text-error");
    });

    test("baseMetric: two Decs as a MetricDelta, null pct from a zero base", () => {
        const metric = baseMetric(d(70200n, 0), d(43200n, 0), "$");
        expect(metric.delta.get("$")).toEqual(d(27000n, 0));
        expect(metric.pct).toBeCloseTo(62.5, 10);
        expect(baseMetric(d(1n, 0), d(0n, 0), "$").pct).toBeNull();
    });
});
