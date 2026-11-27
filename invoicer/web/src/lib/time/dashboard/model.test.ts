import {describe, expect, it} from "vitest";
import {NOVEMBER, NOVEMBER_NOTES, OCTOBER, OCTOBER_NOTES} from "./fixtures.js";
import {
    accountsAt,
    bucketsOf,
    byCustomer,
    chartOf,
    deltaOf,
    duration,
    factsOf,
    hourTicks,
    kpisOf,
    notesOf,
    OTHER_KEY,
    slotsFor,
    topOf,
    treeOf,
    visibleRows,
} from "./model.js";
import {presetRange, previousRange} from "./query.js";

const TODAY = "2026-11-26";
const nov = factsOf(NOVEMBER);
const oct = factsOf(OCTOBER);
const range = presetRange("this-month", TODAY);
const prev = previousRange(range);

describe("UNIT time dashboard durations and deltas", () => {
    it("writes durations as the server does", () => {
        expect(duration(0)).toBe("0m");
        expect(duration(29)).toBe("0m");
        expect(duration(30)).toBe("1m");
        expect(duration(45 * 60)).toBe("45m");
        expect(duration(2 * 3600)).toBe("2h");
        expect(duration(3900)).toBe("1h 05m");
        expect(duration(-900)).toBe("−15m");
    });

    it("shows a change as a duration with its percentage, points, or days", () => {
        expect(deltaOf(1374 * 60, 540 * 60, "duration")).toEqual({direction: "up", text: "+13h 54m (+154%)", label: "13h 54m more than the previous period"});
        expect(deltaOf(0, 3600, "duration")).toMatchObject({direction: "down", text: "−1h (−100%)"});
        expect(deltaOf(3600, 0, "duration").text).toBe("+1h");
        expect(deltaOf(20, 20, "duration")).toMatchObject({direction: "flat", text: "No change"});
        expect(deltaOf(0.905, 0.917, "points")).toMatchObject({direction: "down", text: "−1 pts"});
        expect(deltaOf(7, 2, "days")).toMatchObject({direction: "up", text: "+5", label: "5 days more than the previous period"});
    });
});

describe("UNIT time dashboard KPIs", () => {
    it("totals, billable share, the working-day average and days tracked, this month to date", () => {
        const k = kpisOf(nov, range);
        expect(k.total).toBe(1374 * 60);
        expect(duration(k.total)).toBe("22h 54m");
        expect(k.billable).toBe(1244 * 60);
        expect(k.billableShare).toBeCloseTo(1244 / 1374);
        expect(k.workingDays).toBe(19);
        expect(duration(k.perWorkingDay ?? 0)).toBe("1h 12m");
        expect(k.trackedWorkingDays).toBe(7);
        expect(k.weekendDays).toBe(0);
        expect(k.unbilled).toBe(1244 * 60);
    });

    it("the previous period to the same day, billable time with no customer not unbilled", () => {
        const k = kpisOf(oct, prev);
        expect(k.total).toBe(540 * 60);
        expect(k.workingDays).toBe(18);
        expect(k.trackedWorkingDays).toBe(2);
        expect(k.billable).toBe(495 * 60);
        expect(k.unbilled).toBe(180 * 60);
    });

    it("counts weekend days apart, and has no average without working days", () => {
        const weekend = kpisOf([{...nov[0], date: "2026-11-28"}], {from: "2026-11-28", end: "2026-11-29"});
        expect(weekend).toMatchObject({workingDays: 0, perWorkingDay: null, trackedWorkingDays: 0, weekendDays: 1});
        expect(kpisOf([], range).billableShare).toBeNull();
    });
});

describe("UNIT time dashboard hours over time", () => {
    it("buckets by day, week (Monday, clipped to the period) and month", () => {
        expect(bucketsOf(range, "day")).toHaveLength(30);
        const weeks = bucketsOf(range, "week");
        expect(weeks.map((w) => [w.from, w.to])[0]).toEqual(["2026-11-01", "2026-11-01"]);
        expect(weeks[1]).toMatchObject({from: "2026-11-02", to: "2026-11-08"});
        expect(weeks.at(-1)).toMatchObject({from: "2026-11-30", to: "2026-11-30"});
        expect(bucketsOf({from: "2025-12-01", to: "2026-11-30"}, "month").map((m) => m.tick)).toEqual([
            "Dec 2025",
            "Jan 2026",
            "Feb",
            "Mar",
            "Apr",
            "May",
            "Jun",
            "Jul",
            "Aug",
            "Sep",
            "Oct",
            "Nov",
        ]);
    });

    it("stacks by top-level account, colours from the stable order, totals per bucket", () => {
        const chart = chartOf(nov, range, "week", "branch", ["clients", "internal"]);
        expect(chart.series.map((s) => [s.key, s.color])).toEqual([
            ["clients", "var(--chart-1)"],
            ["internal", "var(--chart-2)"],
        ]);
        expect(chart.totals.reduce((a, b) => a + b, 0)).toBe(1374 * 60);
        expect(chart.series[0].drill).toEqual({account: "clients"});
    });

    it("keeps a series' colour when a filter removes the others", () => {
        const order = ["Globex", "Acme Corp", "Initech Inc"];
        const all = chartOf(nov, range, "day", "customer", order);
        const acmeOnly = chartOf(
            nov.filter((f) => f.customer === "Acme Corp"),
            range,
            "day",
            "customer",
            order
        );
        const acme = (c: typeof all) => c.series.find((s) => s.key === "Acme Corp")?.color;
        expect(acme(all)).toBe("var(--chart-2)");
        expect(acme(acmeOnly)).toBe("var(--chart-2)");
        // No customer draws grey, under its own name, and can't be drilled into.
        const none = all.series.find((s) => s.key === OTHER_KEY);
        expect(none).toMatchObject({label: "No customer", italic: true, color: "var(--chart-other)", drill: null});
    });

    it("folds keys past the slots into Other", () => {
        const people = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        const facts = people.map((p, i) => ({...nov[0], person: p, seconds: (i + 1) * 600}));
        const chart = chartOf(facts, range, "day", "person", people);
        expect(chart.series).toHaveLength(8);
        expect(chart.series.at(-1)).toMatchObject({label: "Other", total: (8 + 9) * 600});
        expect(slotsFor(people, new Map()).size).toBe(7);
    });

    it("splits billable in two fixed colours", () => {
        const chart = chartOf(nov, range, "month", "billable", []);
        expect(chart.series.map((s) => [s.label, s.color, s.total])).toEqual([
            ["Billable", "var(--chart-1)", 1244 * 60],
            ["Non-billable", "var(--chart-2)", 130 * 60],
        ]);
    });

    it("draws round hour ticks just past the tallest bar", () => {
        expect(hourTicks(9 * 3600).map((t) => t / 3600)).toEqual([0, 4, 8, 12]);
        expect(hourTicks(0).map((t) => t / 3600)).toEqual([0, 1]);
        expect(hourTicks(1.2 * 3600).map((t) => t / 3600)).toEqual([0, 0.5, 1, 1.5]);
    });
});

describe("UNIT time dashboard account tree", () => {
    const tree = treeOf(nov, oct);

    it("rolls hours, billable hours and the previous period up every level", () => {
        const clients = tree[0];
        expect(clients).toMatchObject({account: "clients", seconds: 1304 * 60, billable: 1244 * 60, previous: 495 * 60});
        const acme = clients.children.find((n) => n.account === "clients:acme");
        expect(acme).toMatchObject({seconds: 494 * 60, billable: 434 * 60, previous: 350 * 60});
        // Time only last period stays, at nothing, so its change shows.
        expect(clients.children.find((n) => n.account === "clients:initech")).toMatchObject({seconds: 0, previous: 40 * 60});
        expect(clients.children[0].account).toBe("clients:globex");
    });

    it("shows rows to the depth, each one's toggle flipping it", () => {
        const at2 = visibleRows(tree, 2, new Set());
        expect(at2.map((r) => r.node.account)).toEqual([
            "clients",
            "clients:globex",
            "clients:acme",
            "clients:initech",
            "clients:orphan",
            "internal",
            "internal:admin",
            "internal:old",
            "internal:training",
        ]);
        expect(at2.find((r) => r.node.account === "clients:acme")).toMatchObject({expandable: true, expanded: false});
        // Opened below the depth.
        const open = visibleRows(tree, 2, new Set(["clients:acme"]));
        expect(open.map((r) => r.node.account)).toContain("clients:acme:tax-return");
        // Closed above it.
        const closed = visibleRows(tree, 2, new Set(["clients"]));
        expect(closed.map((r) => r.node.account)).toEqual(["clients", "internal", "internal:admin", "internal:old", "internal:training"]);
        expect(visibleRows(tree, 1, new Set()).map((r) => r.node.account)).toEqual(["clients", "internal"]);
    });
});

describe("UNIT time dashboard top lists", () => {
    it("keeps the top N by hours with the rest as Other, each with its previous figure", () => {
        const list = topOf(accountsAt(nov, 3), accountsAt(oct, 3), {n: 3});
        expect(list.items.map((i) => [i.key, i.seconds / 60, (i.previous ?? 0) / 60])).toEqual([
            ["clients:globex:audit", 810, 90],
            ["clients:acme:tax-return", 220, 300],
            ["clients:acme:advisory", 214, 50],
        ]);
        expect(list.other).toEqual({keys: 3, seconds: 130 * 60, entries: 5, previous: 20 * 60});
        expect(list.total).toBe(1374 * 60);
        expect(list.items[0].share).toBeCloseTo(810 / 1374);
    });

    it("has no Other when everything fits, and no previous with comparison off", () => {
        const list = topOf(byCustomer(nov), null);
        expect(list.other).toBeNull();
        expect(list.items.every((i) => i.previous === null)).toBe(true);
    });

    it("ranks notes by how often they're written, the empty note left out", () => {
        const list = topOf(notesOf(NOVEMBER_NOTES.rows), notesOf(OCTOBER_NOTES.rows), {by: "entries", n: 2});
        expect(list.items.map((i) => [i.key, i.entries])).toEqual([
            ["fieldwork", 2],
            ["email", 2],
        ]);
        expect(list.other?.keys).toBe(6);
        expect(notesOf(NOVEMBER_NOTES.rows).has("")).toBe(false);
    });
});
