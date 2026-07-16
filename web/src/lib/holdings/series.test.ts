import {describe, expect, it} from "vitest";
import {toNumber} from "../domain/money";
import {holdingsSeries} from "./series";
import {amt, pd, scope, txn, usd, withCost, type PostingSpec} from "./test-helpers";

const buy = (account: string, symbol: string, qty: number, costCents: number, per = true): PostingSpec => ({
    account,
    amounts: [withCost(amt(symbol, qty, 0), costCents, per)],
});

// VTI: 10 shares @ $200 on 2025-02-10, +10 @ $220 on 2025-04-10; priced $250 from 2025-01.
const txns = [
    txn(1, "2025-02-10", [buy("assets:broker:vti", "VTI", 10, 20000), {account: "assets:broker:cash", amounts: [usd(-200000)]}]),
    txn(2, "2025-04-10", [buy("assets:broker:vti", "VTI", 10, 22000), {account: "assets:broker:cash", amounts: [usd(-220000)]}]),
];
const prices = [pd("2025-01-01", "VTI", 25000)];

describe("UNIT holdings/series", () => {
    it("snapshots market value at each month-end, ending at asOf", () => {
        const series = holdingsSeries(txns, prices, scope("2025-05-15"), {interval: "monthly", count: 5});
        expect(series.base).toBe("$");
        expect(series.points.map((p) => p.bucket)).toEqual(["2025-01", "2025-02", "2025-03", "2025-04", "2025-05"]);
        // Final point clamps to asOf, not the month's last day.
        expect(series.points.at(-1)!.date).toBe("2025-05-15");
        expect(series.points[0].date).toBe("2025-01-31");

        const value = series.points.map((p) => toNumber(p.marketValue));
        expect(value[0]).toBe(0); // Jan: nothing held yet
        expect(value[1]).toBe(2500); // Feb: 10 × $250
        expect(value[2]).toBe(2500); // Mar: still 10
        expect(value[3]).toBe(5000); // Apr: 20 × $250
        expect(value[4]).toBe(5000); // May
    });

    it("tracks cost basis alongside value and flags basis availability", () => {
        const series = holdingsSeries(txns, prices, scope("2025-05-15"), {interval: "monthly", count: 5});
        expect(series.hasBasis).toBe(true);
        const basis = series.points.map((p) => (p.basis === null ? null : toNumber(p.basis)));
        expect(basis).toEqual([0, 2000, 2000, 4200, 4200]); // $2000 then +$2200
    });

    it("respects exclude scoping (zero value at every point when the only holding account is excluded)", () => {
        const series = holdingsSeries(txns, prices, scope("2025-05-15", "exclude", ["assets:broker:vti"]), {interval: "monthly", count: 3});
        expect(series.points.every((p) => toNumber(p.marketValue) === 0)).toBe(true);
        // No holdings ⇒ the empty-portfolio basis total is a (non-null) zero at every point.
        expect(series.points.every((p) => p.basis !== null && toNumber(p.basis) === 0)).toBe(true);
    });

    it("time-travels: an earlier asOf never sees later buys", () => {
        const series = holdingsSeries(txns, prices, scope("2025-03-31"), {interval: "monthly", count: 2});
        expect(series.points.map((p) => p.bucket)).toEqual(["2025-02", "2025-03"]);
        expect(series.points.map((p) => toNumber(p.marketValue))).toEqual([2500, 2500]); // second buy (Apr) is in the future
    });
});
