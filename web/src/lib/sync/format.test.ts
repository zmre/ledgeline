import {describe, expect, it} from "vitest";
import {FIXTURE_LAST_SYNC, FIXTURE_NEXT_AUTO, FIXTURE_NOW} from "./fixtures";
import {daysBetween, formatMoney, negate, shortDate, when} from "./format";

describe("UNIT sync format", () => {
    it("formats money from text without losing digits", () => {
        expect(formatMoney("4210.55")).toBe("$4,210.55");
        expect(formatMoney("-1032.18")).toBe("−$1,032.18");
        expect(formatMoney("5")).toBe("$5.00");
        expect(formatMoney("-0.00")).toBe("$0.00");
        expect(formatMoney("1234567.891")).toBe("$1,234,567.891");
        expect(formatMoney("12.5", "EUR")).toBe("€12.50");
        expect(formatMoney("12.5", "XYZ")).toBe("12.5 XYZ");
        expect(formatMoney("1,000.00")).toBe("1,000.00 USD");
        expect(formatMoney("-1032.18", "")).toBe("−1,032.18");
    });

    it("negates text", () => {
        expect(negate("5")).toBe("-5");
        expect(negate("-5.10")).toBe("5.10");
        expect(negate("+2")).toBe("-2");
        expect(negate("0.00")).toBe("0.00");
    });

    it("places times relative to now", () => {
        expect(when(FIXTURE_LAST_SYNC, FIXTURE_NOW, "en-US")).toBe("7:12 AM");
        expect(when(FIXTURE_NEXT_AUTO, FIXTURE_NOW, "en-US")).toBe("1:15 PM");
        expect(when(FIXTURE_LAST_SYNC - 86_400, FIXTURE_NOW, "en-US")).toBe("yesterday 7:12 AM");
        expect(when(FIXTURE_LAST_SYNC + 86_400, FIXTURE_NOW, "en-US")).toBe("tomorrow 7:12 AM");
        expect(when(FIXTURE_LAST_SYNC - 5 * 86_400, FIXTURE_NOW, "en-US")).toBe("Nov 9");
    });

    it("counts local days and writes short dates", () => {
        expect(daysBetween("2026-10-02", "2026-11-14")).toBe(43);
        expect(daysBetween("2026-11-14", "2026-11-14")).toBe(0);
        expect(daysBetween("nope", "2026-11-14")).toBeNull();
        expect(shortDate("2026-11-11", "2026-11-14", "en-US")).toBe("Nov 11");
        expect(shortDate("2025-12-30", "2026-11-14", "en-US")).toBe("Dec 30, 2025");
    });
});
