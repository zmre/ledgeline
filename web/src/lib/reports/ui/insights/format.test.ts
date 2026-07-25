import {describe, expect, test} from "vitest";
import type {Dec, MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import type {MetricDelta} from "$lib/reports/insightsTypes";
import {deltaLine, extras, fmt, fmtBase, fmtSignedAmount, fmtSignedPct, monthlyAverage, signClass} from "./format";

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
