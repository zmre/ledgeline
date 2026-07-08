import {describe, expect, it} from "vitest";
import type {PriceDirective} from "../domain/types";
import {amt} from "./test-helpers";
import {buildPriceDb, valueAt, type ValuationMeta} from "./prices";

const P = (date: string, commodity: string, price: ReturnType<typeof amt>): PriceDirective => ({date, commodity, price});

const directives: PriceDirective[] = [
    P("2024-09-30", "EUR", amt("$", 111, 2)),
    P("2024-09-30", "AAPL", amt("$", 22800, 2)),
    P("2025-12-31", "EUR", amt("$", 110, 2)),
    P("2025-12-31", "AAPL", amt("$", 25500, 2)),
    P("2026-06-30", "EUR", amt("$", 116, 2)),
    P("2026-06-30", "EUR", amt("GBP", 85, 2)), // later same-commodity directive in a different target
];

describe("UNIT reports/prices", () => {
    describe("buildPriceDb.lookup", () => {
        const db = buildPriceDb(directives);

        it("returns the latest directive dated ≤ asOf (inclusive boundary)", () => {
            expect(db.lookup("AAPL", "2025-12-30")?.qty).toEqual({m: 22800n, p: 2});
            expect(db.lookup("AAPL", "2025-12-31")?.qty).toEqual({m: 25500n, p: 2});
            expect(db.lookup("AAPL", "2026-07-08")?.qty).toEqual({m: 25500n, p: 2});
        });

        it("returns null before the first directive or for unknown commodities", () => {
            expect(db.lookup("AAPL", "2024-09-29")).toBeNull();
            expect(db.lookup("DOGE", "2026-07-08")).toBeNull();
        });

        it("same-date directives: the last one declared wins", () => {
            expect(db.lookup("EUR", "2026-06-30")?.commodity).toBe("GBP");
        });
    });

    describe("buildPriceDb.lookupIn", () => {
        const db = buildPriceDb(directives);

        it("skips directives priced in other targets", () => {
            expect(db.lookupIn("EUR", "$", "2026-06-30")?.qty).toEqual({m: 116n, p: 2});
            expect(db.lookupIn("EUR", "GBP", "2026-06-30")?.qty).toEqual({m: 85n, p: 2});
            expect(db.lookupIn("EUR", "GBP", "2026-06-29")).toBeNull(); // no earlier GBP price
            expect(db.lookupIn("AAPL", "GBP", "2026-07-08")).toBeNull();
        });
    });

    describe("buildPriceDb.baseCommodity", () => {
        it("picks the most frequent target commodity", () => {
            expect(buildPriceDb(directives).baseCommodity()).toBe("$");
        });

        it("breaks frequency ties lexically and returns null when empty", () => {
            expect(buildPriceDb([P("2026-01-01", "EUR", amt("GBP", 85, 2)), P("2026-01-02", "AAPL", amt("$", 25500, 2))]).baseCommodity()).toBe("$");
            expect(buildPriceDb([]).baseCommodity()).toBeNull();
        });
    });

    describe("valueAt", () => {
        const db = buildPriceDb(directives);

        it("converts exactly via mul and passes the target through unchanged", () => {
            const ma = new Map([
                ["$", {m: 1000n, p: 2}], // $10.00 as-is
                ["EUR", {m: 20000n, p: 2}], // 200 EUR × $1.10 = $220.00
            ]);
            expect(valueAt(ma, "$", db, "2026-01-15")).toEqual({m: 2300000n, p: 4}); // 10.00 + 220.0000 = $230
        });

        it("skips unpriced commodities and reports them via the meta out-param, deduped", () => {
            const ma = new Map([
                ["DOGE", {m: 5n, p: 0}],
                ["EUR", {m: 10000n, p: 2}],
                ["AAPL", {m: 10n, p: 0}], // priced in $, but asOf predates every directive
            ]);
            const meta: ValuationMeta = {unpriced: []};
            expect(valueAt(ma, "$", db, "2024-01-01", meta)).toEqual({m: 0n, p: 0});
            expect(meta.unpriced).toEqual(["DOGE", "EUR", "AAPL"]);
            expect(valueAt(ma, "$", db, "2024-01-01", meta)).toEqual({m: 0n, p: 0}); // second pass does not duplicate
            expect(meta.unpriced).toEqual(["DOGE", "EUR", "AAPL"]);
        });

        it("works without the meta out-param (specified signature)", () => {
            expect(valueAt(new Map([["DOGE", {m: 5n, p: 0}]]), "$", db, "2026-07-08")).toEqual({m: 0n, p: 0});
        });
    });
});
