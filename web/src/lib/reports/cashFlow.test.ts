import {describe, expect, it} from "vitest";
import {dec} from "../domain/money";
import {cashFlow, isCashLike} from "./cashFlow";
import {amt, txn, usd} from "./test-helpers";

const ma = (cents: number) => new Map([["$", dec(cents, 2)]]);

const txns = [
    txn("2026-01-10", [
        ["assets:bank:checking", usd(10000)],
        ["income:salary", usd(-10000)],
    ]),
    txn("2026-02-05", [
        ["expenses:food", usd(3000)],
        ["assets:bank:checking", usd(-3000)],
    ]),
    txn("2026-02-14", [
        ["assets:broker:taxable:aapl", amt("AAPL", 2, 0)],
        ["assets:broker:taxable:cash", usd(-40000)],
    ]),
    txn("2026-02-20", [
        ["assets:bank:savings", usd(5000)],
        ["assets:bank:checking", usd(-5000)],
    ]),
    txn("2026-03-10", [
        ["assets:bank:checking", usd(7000)],
        ["income:salary", usd(-7000)],
    ]),
    // After `end` (mid-bucket truncation):
    txn("2026-03-20", [
        ["assets:bank:checking", usd(9999)],
        ["income:salary", usd(-9999)],
    ]),
];

describe("UNIT reports/cashFlow", () => {
    describe("isCashLike", () => {
        it("matches hledger's Cash-account name heuristic", () => {
            expect(isCashLike("assets:bank:checking")).toBe(true);
            expect(isCashLike("assets:bank:wise:eur")).toBe(true); // descendant of a cash-like segment
            expect(isCashLike("assets:broker:taxable:cash")).toBe(true);
            expect(isCashLike("asset:savings")).toBe(true); // singular root
            expect(isCashLike("assets:broker:taxable:aapl")).toBe(false);
            expect(isCashLike("assets")).toBe(false);
            expect(isCashLike("expenses:bank")).toBe(false); // not an asset
            expect(isCashLike("liabilities:cc:visa")).toBe(false);
        });
    });

    it("buckets changes in cash-like accounts, truncating the last bucket at end", () => {
        const report = cashFlow(txns, {end: "2026-03-15", interval: "monthly", count: 3, depth: 4});
        expect(report.buckets).toEqual(["2026-01", "2026-02", "2026-03"]);

        // Union of clamped accounts, sorted; AAPL holdings are NOT cash-like.
        expect(report.rows.map((r) => r.account)).toEqual([
            "assets",
            "assets:bank",
            "assets:bank:checking",
            "assets:bank:savings",
            "assets:broker",
            "assets:broker:taxable",
            "assets:broker:taxable:cash",
        ]);

        const byAccount = new Map(report.rows.map((r) => [r.account, r.values]));
        expect(byAccount.get("assets:bank:checking")).toEqual([ma(10000), ma(-8000), ma(7000)]); // 03-20 txn beyond end
        expect(byAccount.get("assets:bank:savings")).toEqual([new Map(), ma(5000), new Map()]);
        expect(byAccount.get("assets:broker:taxable:cash")).toEqual([new Map(), ma(-40000), new Map()]);
        expect(byAccount.get("assets")).toEqual([ma(10000), ma(-43000), ma(7000)]);

        // Net flow per bucket = sum of DIRECT changes (no ancestor double-counting).
        expect(report.totals).toEqual([ma(10000), ma(-43000), ma(7000)]);
    });

    it("clamps rows to depth", () => {
        const report = cashFlow(txns, {end: "2026-03-15", interval: "monthly", count: 3, depth: 2});
        expect(report.rows.map((r) => r.account)).toEqual(["assets", "assets:bank", "assets:broker"]);
        expect(report.totals).toEqual([ma(10000), ma(-43000), ma(7000)]); // totals unaffected by depth
    });

    it("supports quarterly buckets", () => {
        const report = cashFlow(txns, {end: "2026-06-30", interval: "quarterly", count: 2, depth: 1});
        expect(report.buckets).toEqual(["2026-Q1", "2026-Q2"]);
        // Q1 direct changes: 100.00 − 30.00 − 50.00 + 50.00 − 400.00 + 70.00 + 99.99 = −160.01; Q2 empty.
        expect(report.rows).toEqual([{account: "assets", depth: 1, values: [ma(-16001), new Map()]}]);
        expect(report.totals).toEqual([ma(-16001), new Map()]);
    });

    it("honors a custom isCash predicate that diverges from the name heuristic", () => {
        // Treat the AAPL holding account as cash and the name-cash checking account as NOT cash.
        const isCash = (account: string): boolean => account === "assets:broker:taxable:aapl";
        const report = cashFlow(txns, {end: "2026-03-15", interval: "monthly", count: 3, depth: 4, isCash});
        expect(report.rows.map((r) => r.account)).toEqual(["assets", "assets:broker", "assets:broker:taxable", "assets:broker:taxable:aapl"]);
        // Only the AAPL leg (2 shares, Feb) is counted now — no $ flows at all.
        const byAccount = new Map(report.rows.map((r) => [r.account, r.values]));
        expect(byAccount.get("assets:broker:taxable:aapl")).toEqual([new Map(), new Map([["AAPL", dec(2, 0)]]), new Map()]);
        expect(report.totals).toEqual([new Map(), new Map([["AAPL", dec(2, 0)]]), new Map()]);
    });
});
