// Entries' URL contract: parse and serialize round-trip, defaults stay out
// of the URL, periods resolve against today, and the API gets what it reads.
import {describe, expect, it} from "vitest";
import {addDays, addMonths, isIsoDate, mondayOf, monthRange, weekDates} from "./dates.js";
import {
    clearEntriesFilters,
    entriesApiQuery,
    entriesFiltered,
    entriesQueryString,
    filterPerson,
    parseEntriesQuery,
    periodRange,
    shownRange,
    UNATTRIBUTED,
} from "./query.js";

const TODAY = "2026-11-26"; // a Thursday
const parse = (qs: string) => parseEntriesQuery(new URLSearchParams(qs));

describe("UNIT time entries query", () => {
    it("defaults to this month by day, and writes nothing for it", () => {
        const query = parse("");
        expect(query).toMatchObject({period: "this-month", group: "day", from: "", to: "", person: "", billed: ""});
        expect(entriesQueryString(query)).toBe("");
        expect(entriesApiQuery(query, TODAY)).toBe("from=2026-11-01&to=2026-11-30");
    });

    it("round-trips every filter in a fixed order", () => {
        const qs = "from=2026-11-01&to=2026-11-15&person=sam&customer=Acme+Corp&billable=yes&billed=unbilled&account=clients%3Aacme&q=K-1&group=account";
        expect(entriesQueryString(parse(qs))).toBe(qs);
    });

    it("drops what it can't read", () => {
        const query = parse("period=fortnight&from=2026-13-01&billable=maybe&billed=paid&group=month");
        expect(query).toMatchObject({period: "this-month", from: "", billable: "", billed: "", group: "day"});
    });

    it("a custom range wins over a preset, and the preset leaves the URL", () => {
        const query = parse("period=last-week&from=2026-10-01");
        expect(shownRange(query, TODAY)).toEqual({from: "2026-10-01", to: null});
        expect(entriesQueryString(query)).toBe("from=2026-10-01");
    });

    it("resolves the presets on today", () => {
        expect(periodRange("this-week", TODAY)).toEqual({from: "2026-11-23", to: "2026-11-29"});
        expect(periodRange("last-week", TODAY)).toEqual({from: "2026-11-16", to: "2026-11-22"});
        expect(periodRange("last-month", TODAY)).toEqual({from: "2026-10-01", to: "2026-10-31"});
        expect(periodRange("this-year", TODAY)).toEqual({from: "2026-01-01", to: "2026-12-31"});
        expect(periodRange("last-year", TODAY)).toEqual({from: "2025-01-01", to: "2025-12-31"});
        expect(periodRange("all", TODAY)).toEqual({from: null, to: null});
        expect(entriesApiQuery(parse("period=all"), TODAY)).toBe("");
    });

    it("the week grid shows its own week, Monday to Sunday", () => {
        expect(entriesApiQuery(parse("group=week"), TODAY)).toBe("from=2026-11-23&to=2026-11-29");
        // Any day of a week names its Monday.
        const query = parse("group=week&week=2026-11-04");
        expect(query.week).toBe("2026-11-02");
        expect(entriesQueryString(query)).toBe("group=week&week=2026-11-02");
        expect(entriesApiQuery(query, TODAY)).toBe("from=2026-11-02&to=2026-11-08");
    });

    it("sends the API its own spelling: billable true/false, unattributed as empty", () => {
        const api = new URLSearchParams(entriesApiQuery(parse(`billable=no&person=${UNATTRIBUTED}`), TODAY));
        expect(api.get("billable")).toBe("false");
        expect(api.get("person")).toBe("");
        expect(filterPerson(parse(`person=${UNATTRIBUTED}`))).toBeUndefined();
        expect(filterPerson(parse("person=sam"))).toBe("sam");
    });

    it("knows a filter from a period, and clears only filters", () => {
        expect(entriesFiltered(parse("period=last-week&group=week"))).toBe(false);
        const query = parse("period=last-week&q=fieldwork&customer=Globex");
        expect(entriesFiltered(query)).toBe(true);
        expect(entriesQueryString(clearEntriesFilters(query))).toBe("period=last-week");
    });
});

describe("UNIT time dates", () => {
    it("does calendar arithmetic without a time zone", () => {
        expect(isIsoDate("2026-02-29")).toBe(false);
        expect(isIsoDate("2028-02-29")).toBe(true);
        expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
        expect(addDays("2026-03-08", 1)).toBe("2026-03-09"); // US DST starts
        expect(mondayOf("2026-11-29")).toBe("2026-11-23"); // Sunday
        expect(mondayOf("2026-11-23")).toBe("2026-11-23");
        expect(weekDates("2026-11-23").at(-1)).toBe("2026-11-29");
        expect(monthRange("2026-02-10")).toEqual({from: "2026-02-01", to: "2026-02-28"});
        expect(addMonths("2026-01", -1)).toBe("2025-12");
        expect(addMonths("2026-12", 1)).toBe("2027-01");
    });
});
