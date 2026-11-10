import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import {pctChange as reexported} from "./incomeStatementRows";
import {pctChange} from "./percent";

const usd = (m: number, p = 2): MixedAmount => new Map([["$", dec(m, p)]]);

describe("UNIT reports/ui/percent — pctChange", () => {
    it("is the function the income statement re-exports, unchanged", () => {
        expect(reexported).toBe(pctChange);
        expect(pctChange(usd(2862648), usd(2851671), "$")).toBe(0.4);
        expect(pctChange(usd(538352), usd(1088079), "$")).toBe(-50.5);
        expect(pctChange(usd(-5000), usd(-10000), "$")).toBe(50);
        expect(pctChange(usd(10000), usd(0), "$")).toBeNull();
        expect(pctChange(usd(10000), undefined, "$")).toBeNull();
        expect(pctChange(usd(10000), usd(5000), null)).toBeNull();
    });

    it("the precision check is opt-in: a prior that rounds to zero is refused only when asked", () => {
        const tiny = usd(4, 3); // $0.004
        expect(pctChange(usd(100), tiny, "$")).not.toBeNull();
        expect(pctChange(usd(100), tiny, "$", 2)).toBeNull();
        expect(pctChange(usd(100), usd(5, 3), "$", 2)).not.toBeNull(); // $0.005 rounds to $0.01
    });
});
