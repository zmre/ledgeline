// The Time area's shared filters: parse and serialize round-trip, old links
// still land, the periods resolve on today and step with `[`/`]`.
import {describe, expect, it} from "vitest";
import {addMonths, rangeText, workingDays} from "../dates.js";
import {
    clearFilters,
    DEFAULT_FILTERS,
    filtersOf,
    filtersQueryString,
    isFiltered,
    parseFilters,
    presetRange,
    previousRange,
    resolveRange,
    stepPeriod,
    withQuery,
    type TimeFilters,
} from "./query.js";

const TODAY = "2026-11-26"; // a Thursday
const parse = (qs: string): TimeFilters => parseFilters(new URLSearchParams(qs));

describe("UNIT time filters URL state", () => {
    it("defaults to this month, and writes nothing for it", () => {
        expect(parse("")).toEqual(DEFAULT_FILTERS);
        expect(filtersQueryString(DEFAULT_FILTERS)).toBe("");
    });

    it("round-trips every filter in a fixed order and drops what it can't read", () => {
        const qs = "period=last-12-months&person=pat&customer=Acme+Corp&billable=yes&billed=unbilled&account=clients%3Aacme&q=K-1";
        expect(filtersQueryString(parse(qs))).toBe(qs);
        expect(parse("period=forever&billable=maybe&billed=paid&from=2026-02-30")).toEqual(DEFAULT_FILTERS);
    });

    it("reads an old Year to date link as This year", () => {
        expect(parse("period=ytd").period).toBe("this-year");
    });

    it("a custom range wins over the preset and drops it from the URL", () => {
        const f = {...DEFAULT_FILTERS, period: "this-year" as const, from: "2026-10-01", to: "2026-10-15"};
        expect(filtersQueryString(f)).toBe("from=2026-10-01&to=2026-10-15");
        expect(resolveRange(f, TODAY)).toMatchObject({from: "2026-10-01", to: "2026-10-15", end: "2026-10-15", step: {unit: "day", n: 15}});
    });

    it("keeps only the filters of a tab's query, for the other tab's link", () => {
        const query = {...DEFAULT_FILTERS, person: "sam", group: "week", depth: 3};
        expect(filtersOf(query)).toEqual({...DEFAULT_FILTERS, person: "sam"});
        expect(withQuery("/app/time", "")).toBe("/app/time");
        expect(withQuery("/app/time", "person=sam")).toBe("/app/time?person=sam");
    });

    it("knows a filter from a period, and clears only filters", () => {
        expect(isFiltered({...DEFAULT_FILTERS, period: "last-week"})).toBe(false);
        const f = {...DEFAULT_FILTERS, period: "last-week" as const, q: "fieldwork", billed: "unbilled" as const, depth: 3};
        expect(isFiltered(f)).toBe(true);
        expect(clearFilters(f)).toEqual({...DEFAULT_FILTERS, period: "last-week", depth: 3});
    });
});

describe("UNIT time filters periods", () => {
    it("resolves each preset on today, its end capped at today", () => {
        expect(presetRange("this-week", TODAY)).toMatchObject({from: "2026-11-23", to: "2026-11-29", end: TODAY});
        expect(presetRange("last-week", TODAY)).toMatchObject({from: "2026-11-16", to: "2026-11-22", end: "2026-11-22"});
        expect(presetRange("this-month", TODAY)).toMatchObject({from: "2026-11-01", to: "2026-11-30", end: TODAY});
        expect(presetRange("last-month", TODAY)).toMatchObject({from: "2026-10-01", to: "2026-10-31"});
        expect(presetRange("this-quarter", TODAY)).toMatchObject({from: "2026-10-01", to: "2026-12-31", end: TODAY});
        expect(presetRange("this-year", TODAY)).toMatchObject({from: "2026-01-01", to: "2026-12-31", end: TODAY});
        expect(presetRange("last-year", TODAY)).toMatchObject({from: "2025-01-01", to: "2025-12-31", end: "2025-12-31"});
        expect(presetRange("last-12-months", TODAY)).toMatchObject({from: "2025-12-01", to: "2026-11-30", end: TODAY});
    });

    it("runs All time from the first day with time to today", () => {
        expect(presetRange("all", TODAY, "2025-03-04")).toMatchObject({from: "2025-03-04", to: TODAY, end: TODAY});
        expect(presetRange("all", TODAY)).toMatchObject({from: TODAY, to: TODAY});
    });

    it("compares this month to date with last month to the same day", () => {
        expect(previousRange(presetRange("this-month", TODAY))).toMatchObject({from: "2026-10-01", to: "2026-10-26", end: "2026-10-26"});
        expect(previousRange(presetRange("last-month", TODAY))).toMatchObject({from: "2026-09-01", to: "2026-09-30"});
        expect(previousRange(presetRange("this-week", TODAY))).toMatchObject({from: "2026-11-16", to: "2026-11-19"});
        expect(previousRange(presetRange("this-year", TODAY))).toMatchObject({from: "2025-01-01", to: "2025-11-26"});
        // A custom span of days: the same number of days just before it.
        expect(previousRange(resolveRange({...DEFAULT_FILTERS, from: "2026-11-10", to: "2026-11-19"}, TODAY))).toMatchObject({
            from: "2026-10-31",
            to: "2026-11-09",
        });
    });

    it("keeps a month's last day a last day when shifting", () => {
        expect(addMonths("2026-03-31", -1)).toBe("2026-02-28");
        expect(addMonths("2026-02-28", 1)).toBe("2026-03-31");
        expect(addMonths("2026-01-15", -2)).toBe("2025-11-15");
    });

    it("steps back and forth with [ and ], landing on a preset when it is one", () => {
        const back = stepPeriod(DEFAULT_FILTERS, TODAY, -1);
        expect(back).toMatchObject({period: "last-month", from: "", to: ""});
        const twoBack = stepPeriod(back, TODAY, -1);
        expect(twoBack).toMatchObject({from: "2026-09-01", to: "2026-09-30"});
        expect(stepPeriod(twoBack, TODAY, 1)).toMatchObject({period: "last-month", from: ""});
        expect(stepPeriod({...DEFAULT_FILTERS, period: "this-week"}, TODAY, -1)).toMatchObject({period: "last-week"});
        expect(stepPeriod({...DEFAULT_FILTERS, period: "this-year"}, TODAY, -1)).toMatchObject({period: "last-year"});
        // A custom whole-month span steps by months, not days.
        expect(stepPeriod({...DEFAULT_FILTERS, from: "2026-01-01", to: "2026-02-28"}, TODAY, 1)).toMatchObject({from: "2026-03-01", to: "2026-04-30"});
        expect(stepPeriod({...DEFAULT_FILTERS, from: "2026-11-10", to: "2026-11-12"}, TODAY, -1)).toMatchObject({from: "2026-11-07", to: "2026-11-09"});
        // All time has nothing before or after it; a view option rides along.
        const all = {...DEFAULT_FILTERS, period: "all" as const, group: "account"};
        expect(stepPeriod(all, TODAY, -1)).toBe(all);
        expect(stepPeriod({...DEFAULT_FILTERS, group: "account"}, TODAY, -1)).toMatchObject({period: "last-month", group: "account"});
    });

    it("counts Monday to Friday", () => {
        expect(workingDays("2026-11-01", "2026-11-26")).toBe(19);
        expect(workingDays("2026-11-23", "2026-11-29")).toBe(5);
        expect(workingDays("2026-11-28", "2026-11-29")).toBe(0);
        expect(workingDays("2026-11-02", "2026-11-01")).toBe(0);
        expect(workingDays("2026-01-01", "2026-12-31")).toBe(261);
    });

    it("writes ranges for people", () => {
        expect(rangeText("2026-11-01", "2026-11-26")).toBe("Nov 1 – 26, 2026");
        expect(rangeText("2026-10-27", "2026-11-02")).toBe("Oct 27 – Nov 2, 2026");
        expect(rangeText("2025-12-01", "2026-11-30")).toBe("Dec 1, 2025 – Nov 30, 2026");
    });
});
