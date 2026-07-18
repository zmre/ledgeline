import {describe, expect, it} from "vitest";
import {localToday} from "$lib/stores/filters.svelte";
import {gainSinceFor, gainWindowSuffix} from "./gainPeriod";

describe("UNIT holdings gainPeriod", () => {
    describe("gainSinceFor", () => {
        it("all-time sends no param (undefined)", () => {
            expect(gainSinceFor("all", "2026-07-16")).toBeUndefined();
        });

        it("YTD is Jan 1 of the asOf's year", () => {
            expect(gainSinceFor("ytd", "2026-07-16")).toBe("2026-01-01");
            expect(gainSinceFor("ytd", "2024-02-29")).toBe("2024-01-01");
        });

        it("YTD off today lands on Jan 1 of the current year (no hardcoded year)", () => {
            const today = localToday();
            expect(gainSinceFor("ytd", today)).toBe(`${today.slice(0, 4)}-01-01`);
        });

        it("trailing 12 months is asOf minus one year", () => {
            expect(gainSinceFor("12mo", "2026-07-16")).toBe("2025-07-16");
            expect(gainSinceFor("12mo", "2025-01-01")).toBe("2024-01-01");
        });

        it("12mo off today is exactly one year before today", () => {
            const today = localToday();
            const y = Number(today.slice(0, 4));
            expect(gainSinceFor("12mo", today)).toBe(`${y - 1}${today.slice(4)}`);
        });

        it("12mo normalizes a Feb-29 asOf forward into the non-leap prior year", () => {
            // 2023 is not a leap year, so 2024-02-29 minus a year rolls to 2023-03-01 (no invalid date emitted).
            expect(gainSinceFor("12mo", "2024-02-29")).toBe("2023-03-01");
        });
    });

    describe("gainWindowSuffix", () => {
        it("is empty for all-time and tagged for windowed periods", () => {
            expect(gainWindowSuffix("all")).toBe("");
            expect(gainWindowSuffix("ytd")).toBe(" (YTD)");
            expect(gainWindowSuffix("12mo")).toBe(" (12mo)");
        });
    });
});
