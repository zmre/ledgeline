import {beforeEach, describe, expect, it} from "vitest";
import {defaultFilter, filters, localToday, presetRange} from "./filters.svelte";

describe("UNIT filters", () => {
    describe("presetRange (pure, fixed today)", () => {
        it("thisMonth spans the full month regardless of today", () => {
            expect(presetRange("thisMonth", "2026-07-08")).toEqual({from: "2026-07-01", to: "2026-07-31"});
            expect(presetRange("thisMonth", "2026-01-31")).toEqual({from: "2026-01-01", to: "2026-01-31"});
        });

        it("thisMonth handles short February and leap day", () => {
            expect(presetRange("thisMonth", "2026-02-10")).toEqual({from: "2026-02-01", to: "2026-02-28"});
            expect(presetRange("thisMonth", "2024-02-29")).toEqual({from: "2024-02-01", to: "2024-02-29"});
        });

        it("lastMonth crosses the year boundary from January", () => {
            expect(presetRange("lastMonth", "2026-01-15")).toEqual({from: "2025-12-01", to: "2025-12-31"});
        });

        it("lastMonth from a month-end lands on the previous month's real end", () => {
            expect(presetRange("lastMonth", "2026-03-31")).toEqual({from: "2026-02-01", to: "2026-02-28"});
            expect(presetRange("lastMonth", "2024-03-31")).toEqual({from: "2024-02-01", to: "2024-02-29"}); // leap February
            expect(presetRange("lastMonth", "2026-07-31")).toEqual({from: "2026-06-01", to: "2026-06-30"});
        });

        it("last90 includes today (90 days total) across year boundary and leap day", () => {
            expect(presetRange("last90", "2026-01-15")).toEqual({from: "2025-10-18", to: "2026-01-15"});
            expect(presetRange("last90", "2024-04-01")).toEqual({from: "2024-01-03", to: "2024-04-01"}); // spans 2024-02-29
        });

        it("ytd runs from Jan 1 through today", () => {
            expect(presetRange("ytd", "2026-07-08")).toEqual({from: "2026-01-01", to: "2026-07-08"});
            expect(presetRange("ytd", "2026-01-01")).toEqual({from: "2026-01-01", to: "2026-01-01"});
        });

        it("thisYear and lastYear are full calendar years", () => {
            expect(presetRange("thisYear", "2026-07-08")).toEqual({from: "2026-01-01", to: "2026-12-31"});
            expect(presetRange("lastYear", "2026-07-08")).toEqual({from: "2025-01-01", to: "2025-12-31"});
        });

        it("all is unbounded", () => {
            expect(presetRange("all", "2026-07-08")).toEqual({from: null, to: null});
        });
    });

    describe("store (module state, reset between tests)", () => {
        beforeEach(() => {
            filters.reset();
        });

        it("defaults to the current month with no accounts and empty query", () => {
            const expected = presetRange("thisMonth", localToday());
            expect(filters.value.from).toBe(expected.from);
            expect(filters.value.to).toBe(expected.to);
            expect(filters.value.accounts.size).toBe(0);
            expect(filters.value.query).toBe("");
        });

        it("setRange and applyPreset update the range", () => {
            filters.setRange("2025-01-01", null);
            expect(filters.value.from).toBe("2025-01-01");
            expect(filters.value.to).toBeNull();
            filters.applyPreset("all");
            expect(filters.value.from).toBeNull();
            expect(filters.value.to).toBeNull();
        });

        it("toggleAccount adds then removes a plain selection", () => {
            filters.toggleAccount("expenses:food");
            expect([...filters.value.accounts]).toEqual(["expenses:food"]);
            filters.toggleAccount("expenses:food");
            expect(filters.value.accounts.size).toBe(0);
        });

        it("selecting a parent prunes selected descendants (stores only the subtree root)", () => {
            filters.toggleAccount("assets:bank:checking");
            filters.toggleAccount("assets:bank:savings");
            filters.toggleAccount("expenses:food");
            filters.toggleAccount("assets:bank");
            expect([...filters.value.accounts].sort()).toEqual(["assets:bank", "expenses:food"]);
        });

        it("does not treat name prefixes as ancestors (assets:bank vs assets:bankx)", () => {
            filters.toggleAccount("assets:bank");
            filters.toggleAccount("assets:bankx");
            expect([...filters.value.accounts].sort()).toEqual(["assets:bank", "assets:bankx"]);
        });

        it("toggling a covered descendant deselects the covering ancestor", () => {
            filters.toggleAccount("assets");
            filters.toggleAccount("assets:bank:checking");
            expect(filters.value.accounts.size).toBe(0);
        });

        it("clearAccounts keeps the rest of the filter", () => {
            filters.setQuery("coffee");
            filters.toggleAccount("expenses");
            filters.clearAccounts();
            expect(filters.value.accounts.size).toBe(0);
            expect(filters.value.query).toBe("coffee");
        });

        it("reset returns to the default filter", () => {
            filters.setRange(null, null);
            filters.toggleAccount("expenses");
            filters.setQuery("rent");
            filters.reset();
            const dflt = defaultFilter();
            expect(filters.value.from).toBe(dflt.from);
            expect(filters.value.to).toBe(dflt.to);
            expect(filters.value.accounts.size).toBe(0);
            expect(filters.value.query).toBe("");
        });

        it("replace swaps the whole filter and copies the account set", () => {
            const accounts = new Set(["assets:bank"]);
            filters.replace({from: null, to: "2025-06-30", accounts, query: "beer"});
            accounts.add("expenses"); // caller mutation must not leak in
            expect(filters.value.from).toBeNull();
            expect(filters.value.to).toBe("2025-06-30");
            expect([...filters.value.accounts]).toEqual(["assets:bank"]);
            expect(filters.value.query).toBe("beer");
        });
    });
});
