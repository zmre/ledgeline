// The dashboard's pure helpers, and its shape against the server's own golden
// (crates/ledgeline-invoicer/tests/snapshots/accrual-api-dashboard.json), so
// the SPA and `GET /api/dashboard` can't drift apart unnoticed.
import {describe, expect, it} from "vitest";
import {barBlocks, bucketName, count, days, isDashboard, listHref, share} from "./dashboard.js";
import {golden} from "./dashboardGolden.js";

describe("UNIT dashboard", () => {
    it("reads the server's golden as a dashboard", () => {
        const accrual = golden("accrual");
        expect(isDashboard(accrual)).toBe(true);
        expect(isDashboard(golden("cash"))).toBe(true);
        expect(isDashboard({basis: "cash"})).toBe(false);
        expect(isDashboard(null)).toBe(false);
        const usd = accrual.commodities[0];
        expect(usd.buckets.map((b) => b.key)).toEqual(["current", "1-30", "31-60", "61-90", "91-"]);
    });

    it("links to the invoice list with the documented parameters, in a fixed order", () => {
        const base = "/app/invoices";
        expect(listHref(base, {})).toBe("/app/invoices");
        expect(listHref(base, {status: "overdue"})).toBe("/app/invoices?status=overdue");
        expect(listHref(base, {aged: "31-60"})).toBe("/app/invoices?aged=31-60");
        expect(listHref(base, {aged: "91-"})).toBe("/app/invoices?aged=91-");
        expect(listHref(base, {customer: "Customer X", status: "overdue"})).toBe("/app/invoices?status=overdue&customer=Customer+X");
        expect(listHref(base, {customer: ""})).toBe("/app/invoices");
    });

    it("draws the bar's blocks as shares of its scale, a zero block not at all", () => {
        const owed = (overdue: string, notDue: string, scale: string) => ({
            overdue: {amount: overdue, text: "", count: 1},
            not_due: {amount: notDue, text: "", count: 1},
            outstanding: {amount: "0", text: "", count: 2},
            scale: {amount: scale, text: ""},
        });
        expect(barBlocks(owed("2500.00", "2500.00", "10000"))).toEqual([
            {id: "overdue", width: 25},
            {id: "not-due", width: 25},
            {id: "rest", width: 50},
        ]);
        // Above the minimum the scale is the total: no grey rest.
        expect(barBlocks(owed("12700.00", "10150.00", "22850.00")).map((b) => b.id)).toEqual(["overdue", "not-due"]);
        const small = barBlocks(owed("0", "9700.00", "10000"));
        expect(small.map((b) => b.id)).toEqual(["not-due", "rest"]);
        expect(small[0].width).toBeCloseTo(97);
        expect(small[1].width).toBeCloseTo(3);
        expect(barBlocks(owed("0", "0", "10000"))).toEqual([{id: "rest", width: 100}]);
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
        expect(bucketName({key: "91-", label: "91+"})).toBe("91+ days past due");
    });
});
