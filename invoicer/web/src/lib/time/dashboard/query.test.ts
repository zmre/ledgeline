import {describe, expect, it} from "vitest";
import {addMonths, rangeText, workingDays} from "../dates.js";
import {
    apiFilter,
    autoGrain,
    dashQueryString,
    DEFAULT_QUERY,
    entriesQuery,
    grainOf,
    parseDashQuery,
    presetRange,
    previousRange,
    resolveRange,
    stepPeriod,
    type DashQuery,
} from "./query.js";

const TODAY = "2026-11-26"; // a Thursday
const parse = (qs: string): DashQuery => parseDashQuery(new URLSearchParams(qs));

describe("UNIT time dashboard URL state", () => {
    it("defaults to this month, depth 2, comparing, split by top-level account", () => {
        expect(parse("")).toEqual(DEFAULT_QUERY);
        expect(dashQueryString(DEFAULT_QUERY)).toBe("");
    });

    it("round-trips every filter with Entries' names and drops what it can't read", () => {
        const qs = "period=ytd&person=pat&customer=Acme+Corp&billable=yes&account=clients%3Aacme&depth=3&compare=off&split=customer&grain=week&target=off";
        const q = parse(qs);
        expect(q).toMatchObject({
            period: "ytd",
            person: "pat",
            customer: "Acme Corp",
            billable: "yes",
            account: "clients:acme",
            depth: 3,
            compare: false,
            split: "customer",
            grain: "week",
            target: false,
        });
        expect(dashQueryString(q)).toBe(qs);
        expect(parse("period=forever&billable=maybe&depth=-1&split=colour&grain=hour&from=2026-02-30")).toEqual(DEFAULT_QUERY);
        expect(parse("depth=40").depth).toBe(9);
    });

    it("a custom range wins over the preset and drops it from the URL", () => {
        const q = {...DEFAULT_QUERY, period: "ytd" as const, from: "2026-10-01", to: "2026-10-15"};
        expect(dashQueryString(q)).toBe("from=2026-10-01&to=2026-10-15");
        expect(resolveRange(q, TODAY)).toMatchObject({from: "2026-10-01", to: "2026-10-15", end: "2026-10-15", step: {unit: "day", n: 15}});
    });
});

describe("UNIT time dashboard periods", () => {
    it("resolves each preset on today, its end capped at today", () => {
        expect(presetRange("this-week", TODAY)).toMatchObject({from: "2026-11-23", to: "2026-11-29", end: TODAY});
        expect(presetRange("last-week", TODAY)).toMatchObject({from: "2026-11-16", to: "2026-11-22", end: "2026-11-22"});
        expect(presetRange("this-month", TODAY)).toMatchObject({from: "2026-11-01", to: "2026-11-30", end: TODAY});
        expect(presetRange("last-month", TODAY)).toMatchObject({from: "2026-10-01", to: "2026-10-31"});
        expect(presetRange("this-quarter", TODAY)).toMatchObject({from: "2026-10-01", to: "2026-12-31", end: TODAY});
        expect(presetRange("ytd", TODAY)).toMatchObject({from: "2026-01-01", to: TODAY});
        expect(presetRange("last-12-months", TODAY)).toMatchObject({from: "2025-12-01", to: "2026-11-30", end: TODAY});
    });

    it("compares this month to date with last month to the same day", () => {
        expect(previousRange(presetRange("this-month", TODAY))).toMatchObject({from: "2026-10-01", to: "2026-10-26", end: "2026-10-26"});
        expect(previousRange(presetRange("last-month", TODAY))).toMatchObject({from: "2026-09-01", to: "2026-09-30"});
        expect(previousRange(presetRange("this-week", TODAY))).toMatchObject({from: "2026-11-16", to: "2026-11-19"});
        expect(previousRange(presetRange("ytd", TODAY))).toMatchObject({from: "2025-01-01", to: "2025-11-26"});
        // A custom span of days: the same number of days just before it.
        expect(previousRange(resolveRange({...DEFAULT_QUERY, from: "2026-11-10", to: "2026-11-19"}, TODAY))).toMatchObject({
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
        const back = stepPeriod(DEFAULT_QUERY, TODAY, -1);
        expect(back).toMatchObject({period: "last-month", from: "", to: ""});
        const twoBack = stepPeriod(back, TODAY, -1);
        expect(twoBack).toMatchObject({from: "2026-09-01", to: "2026-09-30"});
        expect(stepPeriod(twoBack, TODAY, 1)).toMatchObject({period: "last-month", from: ""});
        expect(stepPeriod({...DEFAULT_QUERY, period: "this-week"}, TODAY, -1)).toMatchObject({period: "last-week"});
        // A custom whole-month span steps by months, not days.
        expect(stepPeriod({...DEFAULT_QUERY, from: "2026-01-01", to: "2026-02-28"}, TODAY, 1)).toMatchObject({from: "2026-03-01", to: "2026-04-30"});
        expect(stepPeriod({...DEFAULT_QUERY, from: "2026-11-10", to: "2026-11-12"}, TODAY, -1)).toMatchObject({from: "2026-11-07", to: "2026-11-09"});
    });

    it("chooses the grain from the period's length unless the URL says", () => {
        expect(autoGrain({from: "2026-11-01", to: "2026-11-30"})).toBe("day");
        expect(autoGrain({from: "2026-10-01", to: "2026-12-31"})).toBe("week");
        expect(autoGrain({from: "2025-12-01", to: "2026-11-30"})).toBe("month");
        const r = presetRange("this-month", TODAY);
        expect(grainOf(DEFAULT_QUERY, r)).toBe("day");
        expect(grainOf({...DEFAULT_QUERY, grain: "week"}, r)).toBe("week");
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

describe("UNIT time dashboard requests and drill-downs", () => {
    const r = presetRange("this-month", TODAY);

    it("hands the summary the page's filters, unattributed as the empty person", () => {
        expect(apiFilter(DEFAULT_QUERY, r)).toEqual({from: "2026-11-01", to: "2026-11-30"});
        expect(apiFilter({...DEFAULT_QUERY, person: "-", customer: "Acme Corp", billable: "no", account: "clients"}, r)).toEqual({
            from: "2026-11-01",
            to: "2026-11-30",
            person: "",
            customer: "Acme Corp",
            billable: false,
            account: "clients",
        });
    });

    it("links into Entries with the same filters and explicit dates, narrowed by the figure", () => {
        const q = {...DEFAULT_QUERY, person: "pat", billable: "yes" as const};
        expect(entriesQuery(q, r)).toBe("from=2026-11-01&to=2026-11-30&person=pat&billable=yes");
        expect(entriesQuery(q, r, {account: "clients:acme", from: "2026-11-02", to: "2026-11-02"})).toBe(
            "from=2026-11-02&to=2026-11-02&person=pat&billable=yes&account=clients%3Aacme"
        );
        expect(entriesQuery(DEFAULT_QUERY, r, {person: "", billed: "unbilled", q: "email"})).toBe(
            "from=2026-11-01&to=2026-11-30&person=-&billed=unbilled&q=email"
        );
    });
});
