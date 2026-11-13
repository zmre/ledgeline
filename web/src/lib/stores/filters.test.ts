import {beforeEach, describe, expect, it} from "vitest";
import {defaultFilter, filters, holdMonth, localToday, presetRange, type JournalFilter} from "./filters.svelte";

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

        it("defaults to the last 90 days with no accounts and empty query", () => {
            const expected = presetRange("last90", localToday());
            expect(filters.value.preset).toBe("last90");
            expect(filters.value.from).toBe(expected.from);
            expect(filters.value.to).toBe(expected.to);
            expect(filters.value.accounts.size).toBe(0);
            expect(filters.value.query).toBe("");
        });

        it("the default filter spans a 90-day window ending today (inclusive)", () => {
            const dflt = defaultFilter();
            const today = localToday();
            expect(dflt.preset).toBe("last90");
            expect(dflt.to).toBe(today);
            const {from, to} = dflt;
            expect(from).not.toBeNull();
            expect(to).not.toBeNull();
            if (from === null || to === null) return;
            const toUtc = (d: string): number => Date.UTC(Number(d.slice(0, 4)), Number(d.slice(5, 7)) - 1, Number(d.slice(8, 10)));
            expect((toUtc(to) - toUtc(from)) / 86_400_000).toBe(89); // 90 calendar days, both ends inclusive
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

    describe("running-balance pin (plan 36)", () => {
        beforeEach(() => {
            filters.reset();
        });

        it("starts unpinned", () => {
            expect(defaultFilter().pin).toBeNull();
            expect(filters.value.pin).toBeNull();
        });

        it("pins without touching the account filter", () => {
            filters.toggleAccount("expenses:food");
            filters.pin("assets:bank");
            expect(filters.value.pin).toEqual({account: "assets:bank", commodity: null});
            expect([...filters.value.accounts]).toEqual(["expenses:food"]);
        });

        it("survives every other filter change", () => {
            filters.pin("assets:bank", "EUR");
            filters.toggleAccount("income:dividends");
            filters.setAccounts(new Set(["expenses"]));
            filters.clearAccounts();
            filters.setQuery("coffee");
            filters.setRange("2026-01-01", "2026-01-31");
            filters.applyPreset("ytd");
            expect(filters.value.pin).toEqual({account: "assets:bank", commodity: "EUR"});
        });

        it("focusAccount filters to the account and pins it in one change, keeping the dates", () => {
            filters.setRange("2026-02-01", "2026-02-28");
            const before = filters.value;
            filters.focusAccount("assets:bank:checking");
            expect(filters.value).not.toBe(before);
            expect([...filters.value.accounts]).toEqual(["assets:bank:checking"]);
            expect(filters.value.pin).toEqual({account: "assets:bank:checking", commodity: null});
            expect({from: filters.value.from, to: filters.value.to}).toEqual({from: "2026-02-01", to: "2026-02-28"});
        });

        it("switches the commodity, and does nothing without a pin", () => {
            filters.setPinCommodity("EUR");
            expect(filters.value.pin).toBeNull();
            filters.pin("assets:bank");
            filters.setPinCommodity("EUR");
            expect(filters.value.pin).toEqual({account: "assets:bank", commodity: "EUR"});
        });

        it("unpin removes the pin and leaves the account filter alone", () => {
            filters.focusAccount("assets:bank");
            filters.unpin();
            expect(filters.value.pin).toBeNull();
            expect([...filters.value.accounts]).toEqual(["assets:bank"]);
        });

        it("reset clears it", () => {
            filters.pin("assets:bank", "EUR");
            filters.reset();
            expect(filters.value.pin).toBeNull();
        });

        it("replace copies it, so the restored filter does not share the caller's object", () => {
            const pin = {account: "assets:bank", commodity: "EUR"};
            filters.replace({...defaultFilter(), pin});
            expect(filters.value.pin).toEqual(pin);
            expect(filters.value.pin).not.toBe(pin);
            filters.replace({from: null, to: null, accounts: new Set(), query: ""});
            expect(filters.value.pin).toBeNull();
        });
    });

    describe("calendar view (plan 40)", () => {
        const today = "2026-11-13";
        const base: JournalFilter = {from: "2026-08-16", to: "2026-11-13", accounts: new Set(), query: "", preset: "last90", pin: null};

        beforeEach(() => filters.reset());

        it("holdMonth leaves the list alone", () => {
            expect(holdMonth(base, today)).toBe(base);
        });

        it("holdMonth snaps the range to the month it means and drops the preset", () => {
            expect(holdMonth({...base, calendar: true}, today)).toMatchObject({from: "2026-11-01", to: "2026-11-30", preset: null});
        });

        it("holdMonth keeps a whole month as it is, a live preset included", () => {
            const thisMonth: JournalFilter = {...base, from: "2026-11-01", to: "2026-11-30", preset: "thisMonth", calendar: true};
            expect(holdMonth(thisMonth, today)).toBe(thisMonth);
        });

        it("holdMonth never keeps a pin in the calendar", () => {
            const pinned: JournalFilter = {...base, from: "2026-11-01", to: "2026-11-30", pin: {account: "assets:bank", commodity: null}, calendar: true};
            expect(holdMonth(pinned, today).pin).toBeNull();
        });

        it("entering the calendar clears the running balance and holds a month", () => {
            filters.pin("assets:bank");
            filters.setCalendar(true);
            expect(filters.value.calendar).toBe(true);
            expect(filters.value.pin).toBeNull();
            const {from, to} = filters.value;
            expect(from?.slice(8)).toBe("01");
            expect(from?.slice(0, 7)).toBe(to?.slice(0, 7));
        });

        it("a range picked in the calendar becomes a month", () => {
            filters.setCalendar(true);
            filters.setRange("2025-03-10", "2025-06-02");
            expect(filters.value).toMatchObject({from: "2025-06-01", to: "2025-06-30"});
            filters.applyPreset("lastYear");
            expect(filters.value.to?.slice(5)).toBe("12-31");
            expect(filters.value.from?.slice(5)).toBe("12-01");
        });

        it("showMonth moves the month", () => {
            filters.setCalendar(true);
            filters.showMonth("2024-02");
            expect(filters.value).toMatchObject({from: "2024-02-01", to: "2024-02-29", calendar: true});
        });

        it("pinning leaves the calendar, keeping its month", () => {
            filters.setCalendar(true);
            filters.showMonth("2024-02");
            filters.focusAccount("assets:bank");
            expect(filters.value).toMatchObject({calendar: false, from: "2024-02-01", to: "2024-02-29", pin: {account: "assets:bank", commodity: null}});
        });

        it("leaving the calendar keeps the month it showed", () => {
            filters.setCalendar(true);
            filters.showMonth("2024-02");
            filters.setCalendar(false);
            expect(filters.value).toMatchObject({calendar: false, from: "2024-02-01", to: "2024-02-29"});
        });

        it("reset in the calendar keeps the view and the month, clearing the rest", () => {
            filters.setCalendar(true);
            filters.showMonth("2024-02");
            filters.setQuery("coffee");
            filters.toggleAccount("expenses");
            filters.reset();
            expect(filters.value).toMatchObject({calendar: true, from: "2024-02-01", to: "2024-02-29", query: ""});
            expect(filters.value.accounts.size).toBe(0);
        });

        it("a restored link with both a calendar and a pin keeps the calendar", () => {
            filters.replace({
                from: "2026-01-05",
                to: "2026-03-01",
                accounts: new Set(),
                query: "",
                pin: {account: "assets:bank", commodity: null},
                calendar: true,
            });
            expect(filters.value).toMatchObject({calendar: true, pin: null, from: "2026-03-01", to: "2026-03-31"});
        });
    });
});
