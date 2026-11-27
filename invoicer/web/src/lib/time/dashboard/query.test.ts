// Overview's URL state: the shared filters (tested in ../filters/) and its
// own view options, the summary requests and the drill-downs into Entries.
import {describe, expect, it} from "vitest";
import {presetRange} from "../filters/query.js";
import {apiFilter, autoGrain, dashQueryString, DEFAULT_QUERY, entriesQuery, grainOf, parseDashQuery, type DashQuery} from "./query.js";

const TODAY = "2026-11-26"; // a Thursday
const parse = (qs: string): DashQuery => parseDashQuery(new URLSearchParams(qs));

describe("UNIT time dashboard URL state", () => {
    it("defaults to this month, depth 2, comparing, split by top-level account", () => {
        expect(parse("")).toEqual(DEFAULT_QUERY);
        expect(dashQueryString(DEFAULT_QUERY)).toBe("");
    });

    it("round-trips the filters, then the view options, and drops what it can't read", () => {
        const qs =
            "period=this-year&person=pat&customer=Acme+Corp&billable=yes&billed=unbilled&account=clients%3Aacme&q=K-1&depth=3&compare=off&split=customer&grain=week&target=off";
        const q = parse(qs);
        expect(q).toMatchObject({
            period: "this-year",
            person: "pat",
            customer: "Acme Corp",
            billable: "yes",
            billed: "unbilled",
            account: "clients:acme",
            q: "K-1",
            depth: 3,
            compare: false,
            split: "customer",
            grain: "week",
            target: false,
        });
        expect(dashQueryString(q)).toBe(qs);
        expect(parse("depth=-1&split=colour&grain=hour")).toEqual(DEFAULT_QUERY);
        expect(parse("depth=40").depth).toBe(9);
    });

    it("chooses the grain from the period's length unless the URL says", () => {
        expect(autoGrain({from: "2026-11-01", to: "2026-11-30"})).toBe("day");
        expect(autoGrain({from: "2026-10-01", to: "2026-12-31"})).toBe("week");
        expect(autoGrain({from: "2025-12-01", to: "2026-11-30"})).toBe("month");
        const r = presetRange("this-month", TODAY);
        expect(grainOf(DEFAULT_QUERY, r)).toBe("day");
        expect(grainOf({...DEFAULT_QUERY, grain: "week"}, r)).toBe("week");
    });
});

describe("UNIT time dashboard requests and drill-downs", () => {
    const r = presetRange("this-month", TODAY);

    it("hands the summary the page's filters, unattributed as the empty person", () => {
        expect(apiFilter(DEFAULT_QUERY, r)).toEqual({from: "2026-11-01", to: "2026-11-30"});
        expect(
            apiFilter({...DEFAULT_QUERY, person: "-", customer: "Acme Corp", billable: "no", billed: "invoiced", account: "clients", q: " email "}, r)
        ).toEqual({
            from: "2026-11-01",
            to: "2026-11-30",
            person: "",
            customer: "Acme Corp",
            billable: false,
            billed: "invoiced",
            account: "clients",
            q: "email",
        });
    });

    it("links into Entries with the same filters and period, narrowed by the figure", () => {
        const q = {...DEFAULT_QUERY, person: "pat", billable: "yes" as const, depth: 4};
        expect(entriesQuery(q)).toBe("person=pat&billable=yes");
        expect(entriesQuery({...q, period: "last-month"})).toBe("period=last-month&person=pat&billable=yes");
        expect(entriesQuery({...q, from: "2026-10-01", to: "2026-10-31"})).toBe("from=2026-10-01&to=2026-10-31&person=pat&billable=yes");
        expect(entriesQuery(q, {account: "clients:acme", from: "2026-11-02", to: "2026-11-02"})).toBe(
            "from=2026-11-02&to=2026-11-02&person=pat&billable=yes&account=clients%3Aacme"
        );
        expect(entriesQuery(DEFAULT_QUERY, {person: "", billed: "unbilled", q: "email"})).toBe("person=-&billed=unbilled&q=email");
    });
});
