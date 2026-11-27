// Entries' views and the week grid's writes, over rows shaped like the time
// fixture's.
import {describe, expect, it} from "vitest";
import {entry, lockedEntry, qty} from "../testing.js";
import {billingText, flatten, groupByAccount, groupByDay, overDayWarnings, peopleOf, planCellEdit, weekGrid} from "./group.js";

const a = entry({id: "f@1", date: "2026-11-25", account: "clients:acme:tax-return", quantity: qty(130)});
const b = entry({id: "f@2", date: "2026-11-26", account: "clients:acme:advisory", quantity: qty(60)});
const c = entry({id: "f@3", date: "2026-11-26", account: "internal:admin", quantity: qty(15), billed: "not-billable", billable: false, customer: null});

describe("UNIT time entries grouping", () => {
    it("by day: newest first, file order within, with subtotals", () => {
        const days = groupByDay([a, b, c]);
        expect(days.map((d) => [d.key, d.seconds])).toEqual([
            ["2026-11-26", 75 * 60],
            ["2026-11-25", 130 * 60],
        ]);
        expect(flatten(days).map((e) => e.id)).toEqual(["f@2", "f@3", "f@1"]);
    });

    it("by account: A–Z, newest first within", () => {
        const older = entry({id: "f@0", date: "2026-11-02", account: "clients:acme:advisory", quantity: qty(2)});
        const accounts = groupByAccount([older, a, b, c]);
        expect(accounts.map((g) => g.key)).toEqual(["clients:acme:advisory", "clients:acme:tax-return", "internal:admin"]);
        expect(accounts[0].entries.map((e) => e.id)).toEqual(["f@2", "f@0"]);
        expect(accounts[0].seconds).toBe(62 * 60);
    });

    it("the week grid: accounts × days with every total", () => {
        const outside = entry({id: "f@9", date: "2026-11-30"});
        const grid = weekGrid([a, b, c, outside], "2026-11-23", ["internal:old"]);
        expect(grid.days).toEqual(["2026-11-23", "2026-11-24", "2026-11-25", "2026-11-26", "2026-11-27", "2026-11-28", "2026-11-29"]);
        expect(grid.rows.map((r) => r.account)).toEqual(["clients:acme:advisory", "clients:acme:tax-return", "internal:admin", "internal:old"]);
        expect(grid.rows[1].cells[2].seconds).toBe(130 * 60);
        expect(grid.rows[3].seconds).toBe(0);
        expect(grid.dayTotals).toEqual([0, 0, 130 * 60, 75 * 60, 0, 0, 0]);
        expect(grid.seconds).toBe(205 * 60);
    });

    it("lists people and says how a line stands", () => {
        expect(peopleOf([a, entry({person: "sam"}), entry({person: null})])).toEqual(["pat", "sam"]);
        expect(billingText(b)).toBe("Unbilled");
        expect(billingText(c)).toBe("Not billable");
        expect(billingText(lockedEntry())).toBe("Invoice 101");
        expect(billingText(entry({nobill: "", billed: "nobill"}))).toBe("Not billed");
        expect(billingText(entry({nobill: "courtesy", billed: "nobill"}))).toBe("Not billed: courtesy");
        expect(billingText(entry({invoice: "108", bill: "LEE-7", billed: "invoiced"}))).toBe("Invoice 108 · Bill LEE-7");
    });

    it("flags a person's day over 24 hours, per person", () => {
        const long = (person: string | null, minutes: number) => entry({person, quantity: qty(minutes)});
        expect(overDayWarnings([long("pat", 800), long("pat", 700), long("sam", 1400)])).toEqual([{date: "2026-11-26", person: "pat", seconds: 1500 * 60}]);
        expect(overDayWarnings([long("pat", 1440)])).toEqual([]);
    });
});

describe("UNIT time week grid cell writes", () => {
    const cellOf = (entries: ReturnType<typeof entry>[]) => ({date: "2026-11-26", entries, seconds: entries.reduce((t, e) => t + e.quantity.seconds, 0)});
    const account = "clients:acme:advisory";

    it("an empty cell adds a line", () => {
        expect(planCellEdit(cellOf([]), account, 5400)).toEqual({kind: "add", date: "2026-11-26", account, duration: "90m"});
    });

    it("no change writes nothing (to the minute)", () => {
        expect(planCellEdit(cellOf([b]), account, 3620)).toEqual({kind: "none"});
    });

    it("more time goes on the day's last open line", () => {
        const first = entry({id: "f@1", quantity: qty(30)});
        expect(planCellEdit(cellOf([first, b]), account, 120 * 60)).toEqual({kind: "edit", id: "f@2", duration: "90m"});
    });

    it("more time on a cell with only billed time gets a new line", () => {
        const billed = lockedEntry({date: "2026-11-26"});
        expect(planCellEdit(cellOf([billed]), account, 240 * 60)).toEqual({kind: "add", date: "2026-11-26", account, duration: "60m"});
    });

    it("less time comes off the open line, and removes it at zero", () => {
        expect(planCellEdit(cellOf([b]), account, 1800)).toEqual({kind: "edit", id: "f@2", duration: "30m"});
        expect(planCellEdit(cellOf([b]), account, 0)).toEqual({kind: "delete", id: "f@2"});
    });

    it("refuses a cut into billed, locked or clocked time", () => {
        const billed = lockedEntry({date: "2026-11-26"});
        expect(planCellEdit(cellOf([billed, b]), account, 30 * 60).kind).toBe("refuse");
        const clocked = entry({id: "s@s1", format: "timeclock"});
        expect(planCellEdit(cellOf([clocked]), account, 1800).kind).toBe("refuse");
    });

    it("refuses a cell holding two people's time", () => {
        const plan = planCellEdit(cellOf([b, entry({id: "g@1", person: "sam"})]), account, 0);
        expect(plan.kind).toBe("refuse");
    });
});
