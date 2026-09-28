import {describe, expect, it} from "vitest";
import {localToday} from "$lib/stores/filters.svelte";
import {GAIN_PERIODS, gainSinceFor, gainWindowSuffix, isGainPeriod, trendWindowFor, trendWindowNote} from "./gainPeriod";

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

    describe("gainSinceFor, the short and long windows", () => {
        it("one week is seven days back, across a month boundary", () => {
            expect(gainSinceFor("1wk", "2026-09-28")).toBe("2026-09-21");
            expect(gainSinceFor("1wk", "2026-03-03")).toBe("2026-02-24");
        });

        it("one and three months step back calendar months, clamping the day into a shorter month", () => {
            expect(gainSinceFor("1mo", "2026-09-28")).toBe("2026-08-28");
            expect(gainSinceFor("1mo", "2026-03-31")).toBe("2026-02-28");
            expect(gainSinceFor("1mo", "2024-03-31")).toBe("2024-02-29");
            expect(gainSinceFor("1mo", "2026-01-15")).toBe("2025-12-15");
            expect(gainSinceFor("3mo", "2026-05-31")).toBe("2026-02-28");
            expect(gainSinceFor("3mo", "2026-02-10")).toBe("2025-11-10");
        });

        it("five years follows the twelve-month rule, Feb 29 included", () => {
            expect(gainSinceFor("5yr", "2026-09-28")).toBe("2021-09-28");
            expect(gainSinceFor("5yr", "2024-02-29")).toBe("2019-03-01");
        });
    });

    describe("gainWindowSuffix", () => {
        it("is empty for all-time and tagged for windowed periods", () => {
            expect(gainWindowSuffix("all")).toBe("");
            expect(gainWindowSuffix("1wk")).toBe(" (1wk)");
            expect(gainWindowSuffix("1mo")).toBe(" (1mo)");
            expect(gainWindowSuffix("3mo")).toBe(" (3mo)");
            expect(gainWindowSuffix("ytd")).toBe(" (YTD)");
            expect(gainWindowSuffix("12mo")).toBe(" (12mo)");
            expect(gainWindowSuffix("5yr")).toBe(" (5yr)");
        });
    });

    describe("GAIN_PERIODS", () => {
        it("lists the windows shortest first, all time last", () => {
            expect(GAIN_PERIODS.map((p) => p.value)).toEqual(["1wk", "1mo", "3mo", "ytd", "12mo", "5yr", "all"]);
        });

        it("isGainPeriod accepts exactly the listed values", () => {
            for (const p of GAIN_PERIODS) expect(isGainPeriod(p.value)).toBe(true);
            expect(isGainPeriod("6mo")).toBe(false);
            expect(isGainPeriod(null)).toBe(false);
            expect(isGainPeriod("")).toBe(false);
        });
    });

    describe("trendWindowFor", () => {
        it("keeps the twelve-month chart as twelve month-ends and five years as sixty", () => {
            expect(trendWindowFor("12mo", "2026-09-28")).toEqual({interval: "monthly", count: 12});
            expect(trendWindowFor("5yr", "2026-09-28")).toEqual({interval: "monthly", count: 60});
        });

        it("draws the short windows from the gain window's own start, at a grain that gives a line", () => {
            // 1wk: daily from seven days back — eight points, both ends included.
            expect(trendWindowFor("1wk", "2026-09-28")).toEqual({interval: "daily", since: "2026-09-21"});
            expect(trendWindowFor("1mo", "2026-09-28")).toEqual({interval: "daily", since: "2026-08-28"});
            expect(trendWindowFor("3mo", "2026-09-28")).toEqual({interval: "weekly", since: "2026-06-28"});
        });

        it("starts year to date at the prior year's last day, so the first point is the YTD reference", () => {
            expect(trendWindowFor("ytd", "2026-09-28")).toEqual({interval: "auto", since: "2025-12-31"});
        });

        it("leaves all time to the engine, which alone knows when the scope began", () => {
            expect(trendWindowFor("all", "2026-09-28")).toEqual({interval: "auto", since: "inception"});
        });

        it("names every window in words for the chart heading", () => {
            expect(GAIN_PERIODS.map((p) => trendWindowNote(p.value))).toEqual([
                "last 7 days",
                "last month",
                "last 3 months",
                "year to date",
                "last 12 months",
                "last 5 years",
                "all time",
            ]);
        });
    });
});
