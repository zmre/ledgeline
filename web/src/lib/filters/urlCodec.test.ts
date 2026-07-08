import {describe, expect, it} from "vitest";
import type {JournalFilter} from "$lib/stores/filters.svelte";
import {filterToSearch, searchToFilter} from "./urlCodec";

const dflt: JournalFilter = {from: "2026-07-01", to: "2026-07-31", accounts: new Set<string>(), query: ""};

const filter = (over: Partial<JournalFilter>): JournalFilter => ({...dflt, accounts: new Set<string>(), ...over});

function roundTrip(f: JournalFilter): JournalFilter {
    return searchToFilter(filterToSearch(f, dflt), dflt);
}

describe("UNIT urlCodec", () => {
    it("serializes the default filter to an empty string and parses it back", () => {
        expect(filterToSearch(dflt, dflt)).toBe("");
        const parsed = searchToFilter("", dflt);
        expect(parsed.from).toBe(dflt.from);
        expect(parsed.to).toBe(dflt.to);
        expect(parsed.accounts.size).toBe(0);
        expect(parsed.query).toBe("");
    });

    it("writes from and to as a pair when either differs from the default", () => {
        const search = filterToSearch(filter({from: "2026-07-05"}), dflt);
        const params = new URLSearchParams(search);
        expect(params.get("from")).toBe("2026-07-05");
        expect(params.get("to")).toBe("2026-07-31");
    });

    it("round-trips an open-ended (all time) range via explicitly empty params", () => {
        const search = filterToSearch(filter({from: null, to: null}), dflt);
        const params = new URLSearchParams(search);
        expect(params.get("from")).toBe("");
        expect(params.get("to")).toBe("");
        const parsed = roundTrip(filter({from: null, to: null}));
        expect(parsed.from).toBeNull();
        expect(parsed.to).toBeNull();
    });

    it("round-trips accounts including names with commas, colons, and spaces", () => {
        const names = ["assets:bank:checking", "expenses:food, drink & fun", "liabilities:credit card"];
        const parsed = roundTrip(filter({accounts: new Set(names)}));
        expect([...parsed.accounts].sort()).toEqual([...names].sort());
    });

    it("round-trips a query with reserved characters", () => {
        const parsed = roundTrip(filter({query: "café & 50% = a+b?"}));
        expect(parsed.query).toBe("café & 50% = a+b?");
    });

    it("round-trips a fully customized filter", () => {
        const f = filter({from: "2025-01-01", to: "2025-12-31", accounts: new Set(["expenses", "income:salary"]), query: "rent"});
        const parsed = roundTrip(f);
        expect(parsed.from).toBe("2025-01-01");
        expect(parsed.to).toBe("2025-12-31");
        expect([...parsed.accounts].sort()).toEqual(["expenses", "income:salary"]);
        expect(parsed.query).toBe("rent");
    });

    it("accepts a leading question mark", () => {
        const parsed = searchToFilter("?q=coffee", dflt);
        expect(parsed.query).toBe("coffee");
    });

    it("ignores malformed dates and unknown params, falling back to defaults", () => {
        const parsed = searchToFilter("from=not-a-date&to=2026-13&bogus=1", dflt);
        expect(parsed.from).toBe(dflt.from);
        expect(parsed.to).toBe(dflt.to);
    });

    it("treats an empty acct param as no selection", () => {
        const parsed = searchToFilter("acct=", dflt);
        expect(parsed.accounts.size).toBe(0);
    });

    it("stores preset names instead of frozen dates and recomputes on parse (live YTD)", () => {
        const dfltP = filter({preset: "thisMonth"});
        const f = filter({from: "2026-01-01", to: "2026-07-08", preset: "ytd"});
        expect(filterToSearch(f, dfltP)).toBe("preset=ytd");
        const parsed = searchToFilter("preset=ytd", dfltP, "2026-09-15"); // restored on a later day
        expect(parsed.preset).toBe("ytd");
        expect(parsed.from).toBe("2026-01-01");
        expect(parsed.to).toBe("2026-09-15"); // recomputed against the current day — never pinned
    });

    it("the default preset serializes to an empty string", () => {
        const dfltP = filter({preset: "thisMonth"});
        expect(filterToSearch(filter({preset: "thisMonth"}), dfltP)).toBe("");
    });

    it("hand-picked ranges (preset null) still write explicit date pairs", () => {
        const f = filter({from: "2025-01-01", to: "2025-06-30", preset: null});
        const params = new URLSearchParams(filterToSearch(f, dflt));
        expect(params.get("from")).toBe("2025-01-01");
        expect(params.get("to")).toBe("2025-06-30");
        expect(params.get("preset")).toBeNull();
    });

    it("ignores unknown preset names, falling back to explicit dates/defaults", () => {
        const parsed = searchToFilter("preset=nonsense", dflt);
        expect(parsed.from).toBe(dflt.from);
        expect(parsed.to).toBe(dflt.to);
    });
});
