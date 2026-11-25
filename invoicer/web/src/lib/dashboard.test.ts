// The dashboard's pure helpers, and its shape against the server's own golden
// (crates/ledgeline-invoicer/tests/snapshots/accrual-api-dashboard.json), so
// the SPA and `GET /api/dashboard` can't drift apart unnoticed.
import {describe, expect, it} from "vitest";
import {
    axisTicks,
    barSegments,
    bucketName,
    compactNumber,
    count,
    days,
    isDashboard,
    listHref,
    paidDaysParam,
    paidDaysQuery,
    paidWindowLabel,
    paidWindows,
    percent,
    recentEventText,
    recentStatus,
    share,
} from "./dashboard.js";
import {golden} from "./dashboardGolden.js";

describe("UNIT dashboard", () => {
    it("reads the server's golden as a dashboard", () => {
        const accrual = golden("accrual");
        expect(isDashboard(accrual)).toBe(true);
        expect(isDashboard(golden("cash"))).toBe(true);
        expect(isDashboard({basis: "cash"})).toBe(false);
        expect(isDashboard(null)).toBe(false);
        const usd = accrual.commodities[0];
        expect(usd.buckets.map((b) => b.key)).toEqual(["current", "1-30", "31-60", "61-90", "91+"]);
        expect(accrual.paid.days).toBe(accrual.paid_window_days);
        expect(accrual.recent.length).toBeGreaterThan(0);
    });

    it("links to the invoice list with the documented parameters, in a fixed order", () => {
        const base = "/app/invoices";
        expect(listHref(base, {})).toBe("/app/invoices");
        expect(listHref(base, {status: "overdue"})).toBe("/app/invoices?status=overdue");
        expect(listHref(base, {aged: "31-60"})).toBe("/app/invoices?aged=31-60");
        expect(listHref(base, {aged: "91+"})).toBe("/app/invoices?aged=91%2B");
        expect(listHref(base, {customer: "Customer X", status: "overdue"})).toBe("/app/invoices?status=overdue&customer=Customer+X");
        expect(listHref(base, {customer: ""})).toBe("/app/invoices");
    });

    it("puts the axis on round steps past the scale, at most about six of them", () => {
        // The $10,000 minimum: 0 to 10k by 2k.
        expect(axisTicks("10000")).toEqual([0, 2000, 4000, 6000, 8000, 10000]);
        // Above it the total: $22,850 runs to 25k by 5k.
        expect(axisTicks("22850.00")).toEqual([0, 5000, 10000, 15000, 20000, 25000]);
        expect(axisTicks(12_600)).toEqual([0, 2500, 5000, 7500, 10000, 12500, 15000]);
        expect(axisTicks(1_234_567).at(-1)).toBe(1_250_000);
        expect(axisTicks("0")).toEqual([0]);
        expect(axisTicks("not a number")).toEqual([0]);
        // Every step is the same, and the last tick reaches the scale.
        for (const top of [10_000, 13_001, 99_999, 450, 7]) {
            const ticks = axisTicks(top);
            expect(ticks.at(-1)!).toBeGreaterThanOrEqual(top);
            expect(ticks.length).toBeLessThanOrEqual(7);
        }
    });

    it("labels the axis compactly", () => {
        expect([0, 500, 2000, 2500, 12_500, 25_000, 1_500_000].map(compactNumber)).toEqual(["0", "500", "2k", "2.5k", "12.5k", "25k", "1.5M"]);
    });

    it("lays the segments end to end on the axis, a zero part not at all", () => {
        const owed = (overdue: string, notDue: string) => ({
            overdue: {amount: overdue, text: "", count: 1},
            not_due: {amount: notDue, text: "", count: 1},
            outstanding: {amount: String(Number(overdue) + Number(notDue)), text: "", count: 2},
        });
        expect(barSegments(owed("2500.00", "2500.00"), 10_000)).toEqual([
            {id: "overdue", start: 0, width: 25, share: 50},
            {id: "not-due", start: 25, width: 25, share: 50},
        ]);
        const accrual = barSegments(owed("12700.00", "10150.00"), 25_000);
        expect(accrual.map((b) => b.id)).toEqual(["overdue", "not-due"]);
        expect(accrual[0].width).toBeCloseTo(50.8);
        expect(accrual[1].start).toBeCloseTo(50.8);
        expect(accrual[1].width).toBeCloseTo(40.6);
        expect(accrual[0].share).toBeCloseTo(55.58, 1);
        const cash = barSegments(owed("0", "9700.00"), 10_000);
        expect(cash).toEqual([{id: "not-due", start: 0, width: 97, share: 100}]);
        expect(barSegments(owed("0", "0"), 10_000)).toEqual([]);
    });

    it("words shares", () => {
        expect(percent(55.58)).toBe("56%");
        expect(percent(0.3)).toBe("under 1%");
        expect(percent(0)).toBe("0%");
        expect(percent(100)).toBe("100%");
    });

    it("offers the paid windows, the setting's own among them, and keeps the default out of the URL", () => {
        expect(paidWindows(30)).toEqual([7, 30, 90, 365]);
        expect(paidWindows(14)).toEqual([7, 14, 30, 90, 365]);
        expect(paidWindowLabel(30)).toBe("Last 30 days");
        expect(paidWindowLabel(1)).toBe("Last day");
        const param = (qs: string) => paidDaysParam(new URLSearchParams(qs));
        expect(param("paid_days=90")).toBe(90);
        expect(param("paid_days= 7 ")).toBe(7);
        for (const bad of ["", "paid_days=0", "paid_days=-7", "paid_days=abc", "paid_days=4000", "paid_days=7.5"]) expect(param(bad), bad).toBeNull();
        expect(paidDaysQuery(30, 30)).toBe("");
        expect(paidDaysQuery(90, 30)).toBe("paid_days=90");
    });

    it("links the paid figure by its payment dates", () => {
        expect(listHref("/app/invoices", {paid_from: "2026-10-24", paid_to: "2026-11-22"})).toBe("/app/invoices?paid_from=2026-10-24&paid_to=2026-11-22");
    });

    it("says a recent invoice's status and last event in words", () => {
        expect(recentStatus("overdue")).toEqual({label: "Overdue", tone: "error"});
        expect(recentStatus("paid")).toEqual({label: "Paid", tone: "success"});
        expect(recentStatus("open").tone).toBe("warning");
        expect(recentStatus("draft").tone).toBe("neutral");
        expect(recentStatus("void").label).toBe("Void");
        expect(recentEventText({event: "paid", updated: "2026-11-05"})).toBe("Paid 2026-11-05");
        expect(recentEventText({event: "edited", updated: "2026-11-21"})).toBe("Edited 2026-11-21");
    });

    it("computes shares safely", () => {
        expect(share("50", "200")).toBe(25);
        expect(share("1", "0")).toBe(0);
        expect(share("-5", "10")).toBe(0);
        expect(share("not a number", "10")).toBe(0);
        expect(share("30", "10")).toBe(100);
    });

    it("words counts, days and buckets", () => {
        expect(days(1)).toBe("1 day");
        expect(days(125)).toBe("125 days");
        expect(days("25.3")).toBe("25.3 days");
        expect(count(1, "invoice")).toBe("1 invoice");
        expect(count(3, "entry", "entries")).toBe("3 entries");
        expect(bucketName({key: "current", label: "Current"})).toBe("Not yet due");
        expect(bucketName({key: "91+", label: "91+"})).toBe("91+ days past due");
    });
});
