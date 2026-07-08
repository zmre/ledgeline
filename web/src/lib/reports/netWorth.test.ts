import {describe, expect, it} from "vitest";
import {dec} from "../domain/money";
import type {PriceDirective} from "../domain/types";
import {netWorth} from "./netWorth";
import {buildPriceDb} from "./prices";
import {amt, txn, usd} from "./test-helpers";

const P = (date: string, commodity: string, price: ReturnType<typeof amt>): PriceDirective => ({date, commodity, price});

const prices = buildPriceDb([P("2026-01-31", "EUR", amt("$", 110, 2)), P("2026-02-28", "EUR", amt("$", 120, 2))]);

const txns = [
    txn("2026-01-10", [
        ["assets:bank:checking", usd(10000)],
        ["equity:opening", usd(-10000)],
    ]),
    txn("2026-01-20", [
        ["assets:wise", amt("EUR", 5000, 2)], // 50.00 EUR
        ["equity:opening", usd(-5500)],
    ]),
    txn("2026-02-15", [
        ["liabilities:visa", usd(-2000)],
        ["expenses:food", usd(2000)],
    ]),
];

describe("UNIT reports/netWorth", () => {
    it("values cumulative balances at each bucket end using the price in effect", () => {
        const report = netWorth(txns, prices, {end: "2026-02-28", interval: "monthly", count: 2});
        expect(report.buckets).toEqual(["2026-01", "2026-02"]);
        expect(report.rows.map((r) => [r.account, r.depth])).toEqual([
            ["assets", 1],
            ["liabilities", 1],
        ]);

        const [assets, liabilities] = report.rows;
        // Jan 31: $100 + 50 EUR × $1.10 = $155; Feb 28: $100 + 50 EUR × $1.20 = $160.
        expect(assets.values).toEqual([new Map([["$", dec(1550000, 4)]]), new Map([["$", dec(1600000, 4)]])]);
        // No liabilities until Feb; natural (negative) sign.
        expect(liabilities.values).toEqual([new Map(), new Map([["$", dec(-2000, 2)]])]);
        // Net worth per bucket.
        expect(report.totals).toEqual([new Map([["$", dec(1550000, 4)]]), new Map([["$", dec(1400000, 4)]])]);
        expect(report.meta).toBeUndefined(); // everything priced
    });

    it("skips unpriced commodities and reports them in meta", () => {
        const report = netWorth(txns, prices, {end: "2026-01-25", interval: "monthly", count: 1});
        expect(report.rows[0].values).toEqual([new Map([["$", dec(10000, 2)]])]); // EUR held but skipped: first price is 01-31, after asOf 01-25
        expect(report.meta).toEqual({unpriced: ["EUR"]});
    });

    it("honors an explicit valueIn target", () => {
        const report = netWorth(txns, prices, {end: "2026-01-31", interval: "monthly", count: 1, valueIn: "EUR"});
        expect(report.rows[0].values).toEqual([new Map([["EUR", dec(5000, 2)]])]); // $ has no price in EUR → skipped
        expect(report.meta).toEqual({unpriced: ["$"]});
    });

    it("reports raw mixed amounts when no valuation target exists", () => {
        const report = netWorth(txns, buildPriceDb([]), {end: "2026-02-28", interval: "monthly", count: 1});
        expect(report.rows[0].values).toEqual([
            new Map([
                ["$", dec(10000, 2)],
                ["EUR", dec(5000, 2)],
            ]),
        ]);
        expect(report.meta).toBeUndefined();
    });
});
