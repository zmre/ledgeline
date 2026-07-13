import {describe, expect, it} from "vitest";
import {cmp, dec, toNumber} from "../domain/money";
import {computeHoldings} from "./engine";
import {amt, pd, scope, txn, usd, withCost, type PostingSpec} from "./test-helpers";
import type {Holding, HoldingsReport} from "./types";

const buy = (account: string, symbol: string, qty: number, costCents: number, per = true): PostingSpec => ({
    account,
    amounts: [withCost(amt(symbol, qty, 0), costCents, per)],
});
const sell = (account: string, symbol: string, qty: number): PostingSpec => ({account, amounts: [amt(symbol, -qty, 0)]});
const buyNoCost = (account: string, symbol: string, qty: number): PostingSpec => ({account, amounts: [amt(symbol, qty, 0)]});

const only = (report: HoldingsReport, symbol: string): Holding => {
    const holding = report.holdings.find((h) => h.symbol === symbol);
    expect(holding).toBeDefined();
    return holding!;
};

describe("UNIT holdings/engine average-cost basis", () => {
    it("accumulates @ per-unit buys and reduces a partial sell at average cost", () => {
        const txns = [
            // deliberately out of journal order: the engine must sort by date, then index
            txn(3, "2025-03-10", [sell("assets:broker:vti", "VTI", 5), {account: "assets:broker:cash", amounts: [usd(115000)]}]),
            txn(1, "2025-01-10", [buy("assets:broker:vti", "VTI", 10, 20000), {account: "assets:broker:cash", amounts: [usd(-200000)]}]),
            txn(2, "2025-02-10", [buy("assets:broker:vti", "VTI", 10, 22000), {account: "assets:broker:cash", amounts: [usd(-220000)]}]),
        ];
        const report = computeHoldings(txns, [pd("2025-04-01", "VTI", 25000)], scope("2025-04-30"));

        expect(report.base).toBe("$");
        const vti = only(report, "VTI");
        expect(toNumber(vti.shares)).toBe(15);
        expect(cmp(vti.basis!, dec(315000, 2))).toBe(0); // (2000 + 2200) × 15/20, exact
        expect(vti.price).toMatchObject({date: "2025-04-01", source: "directive"});
        expect(toNumber(vti.price!.qty)).toBe(250);
        expect(toNumber(vti.marketValue!)).toBe(3750);
        expect(toNumber(vti.gain!)).toBe(600);
        expect(vti.gainPct).toBeCloseTo((600 / 3150) * 100, 10);
        expect(vti.accounts).toEqual(["assets:broker:vti"]);
        expect(toNumber(report.totals.marketValue)).toBe(3750);
        expect(toNumber(report.totals.basis!)).toBe(3150);
        expect(toNumber(report.totals.gain!)).toBe(600);
        expect(report.warnings).toEqual([]);
    });

    it("handles @@ total-cost buys", () => {
        const txns = [txn(1, "2025-01-10", [buy("assets:broker", "VTI", 4, 85000, false)])]; // 4 VTI @@ $850.00
        const report = computeHoldings(txns, [pd("2025-02-01", "VTI", 25000)], scope("2025-03-01"));
        const vti = only(report, "VTI");
        expect(cmp(vti.basis!, dec(85000, 2))).toBe(0);
        expect(toNumber(vti.marketValue!)).toBe(1000);
        expect(toNumber(vti.gain!)).toBe(150);
    });

    it("rounds sell reductions half-even to the basis precision", () => {
        // 2 shares @@ $1.01 → sell 1 → 0.505 rounds to 0.50 (even); @@ $1.03 → 0.515 rounds to 0.52
        const txns = [
            txn(1, "2025-01-10", [buy("a", "EEE", 2, 101, false)]),
            txn(2, "2025-01-10", [buy("a", "OOO", 2, 103, false)]),
            txn(3, "2025-02-10", [sell("a", "EEE", 1), sell("a", "OOO", 1)]),
        ];
        const report = computeHoldings(txns, [], scope("2025-03-01"));
        expect(cmp(only(report, "EEE").basis!, dec(50, 2))).toBe(0);
        expect(cmp(only(report, "OOO").basis!, dec(52, 2))).toBe(0);
    });
});

describe("UNIT holdings/engine scoping", () => {
    const twoAccounts = [
        txn(1, "2025-01-10", [buy("assets:broker:a", "VTI", 10, 20000)]),
        txn(2, "2025-01-20", [buy("assets:broker:b", "VTI", 5, 21000)]),
        txn(3, "2025-01-25", [buy("assets:other:c", "VTI", 2, 22000)]),
    ];
    const prices = [pd("2025-02-01", "VTI", 25000)];

    it("include mode + empty set means all accounts", () => {
        const vti = only(computeHoldings(twoAccounts, prices, scope("2025-06-30")), "VTI");
        expect(toNumber(vti.shares)).toBe(17);
        expect(vti.accounts).toEqual(["assets:broker:a", "assets:broker:b", "assets:other:c"]);
    });

    it("include mode matches whole subtrees", () => {
        const vti = only(computeHoldings(twoAccounts, prices, scope("2025-06-30", "include", ["assets:broker"])), "VTI");
        expect(toNumber(vti.shares)).toBe(15);
        expect(toNumber(vti.basis!)).toBe(3050);
        expect(vti.accounts).toEqual(["assets:broker:a", "assets:broker:b"]);
    });

    it("exclude mode removes the selected subtrees only", () => {
        const vti = only(computeHoldings(twoAccounts, prices, scope("2025-06-30", "exclude", ["assets:broker:b"])), "VTI");
        expect(toNumber(vti.shares)).toBe(12);
        expect(toNumber(vti.basis!)).toBe(2440);
        expect(vti.accounts).toEqual(["assets:broker:a", "assets:other:c"]);
    });

    it("an in-scope→in-scope transfer nets to zero shares and leaves basis untouched", () => {
        const txns = [
            txn(1, "2025-01-10", [buy("assets:broker:a", "VTI", 10, 20000)]),
            txn(2, "2025-02-10", [sell("assets:broker:a", "VTI", 4), buyNoCost("assets:broker:b", "VTI", 4)]),
        ];
        const vti = only(computeHoldings(txns, prices, scope("2025-06-30")), "VTI");
        expect(toNumber(vti.shares)).toBe(10);
        expect(cmp(vti.basis!, dec(200000, 2))).toBe(0); // the cost-less incoming leg must NOT taint the pool
        expect(vti.accounts).toEqual(["assets:broker:a", "assets:broker:b"]);
    });
});

describe("UNIT holdings/engine taint and pricing", () => {
    it("a cost-less buy taints the pool: basis null, warning, totals refuse", () => {
        const txns = [txn(1, "2025-01-10", [buyNoCost("assets:broker", "GLD", 10)]), txn(2, "2025-01-20", [buy("assets:broker", "VTI", 10, 20000)])];
        const prices = [pd("2025-02-01", "GLD", 18000), pd("2025-02-01", "VTI", 22000)];
        const report = computeHoldings(txns, prices, scope("2025-06-30"));

        expect(only(report, "GLD").basis).toBeNull();
        expect(only(report, "GLD").gain).toBeNull();
        expect(only(report, "GLD").gainPct).toBeNull();
        expect(toNumber(only(report, "GLD").marketValue!)).toBe(1800); // priced via directive despite the taint
        expect(report.warnings).toEqual([{symbol: "GLD", kind: "missing-basis", message: expect.stringContaining("GLD")}]);
        expect(toNumber(report.totals.marketValue)).toBe(4000);
        expect(report.totals.basis).toBeNull();
        expect(report.totals.gain).toBeNull();
        expect(report.totals.gainPct).toBeNull();
    });

    it("a cost annotation in a non-base commodity converts via the P directive at the txn date, else taints", () => {
        const txns = [
            txn(1, "2025-01-10", [{account: "a", amounts: [withCost(amt("VTI", 10, 0), 10000, true, "EUR")]}]), // 10 VTI @ €100
            txn(2, "2025-01-10", [{account: "a", amounts: [withCost(amt("XYZ", 10, 0), 10000, true, "GBP")]}]), // no GBP→$ price: taint
        ];
        const prices = [pd("2025-01-01", "EUR", 110), pd("2025-02-01", "VTI", 15000), pd("2025-02-01", "XYZ", 15000)];
        const report = computeHoldings(txns, prices, scope("2025-06-30"));
        expect(cmp(only(report, "VTI").basis!, dec(1100_0000, 4))).toBe(0); // €1000 × 1.10
        expect(only(report, "XYZ").basis).toBeNull();
        expect(report.warnings).toEqual([{symbol: "XYZ", kind: "missing-basis", message: expect.stringContaining("XYZ")}]);
    });

    it("falls back to the latest cost annotation as the price (source: cost), including @@ per-unit division", () => {
        const txns = [
            txn(1, "2025-01-10", [buy("assets:broker", "XXX", 10, 5000)]), // @ $50
            txn(2, "2025-03-01", [buy("assets:broker", "XXX", 4, 26000, false)]), // @@ $260 → $65/share
        ];
        const report = computeHoldings(txns, [], scope("2025-06-30"));
        const xxx = only(report, "XXX");
        expect(xxx.price).toMatchObject({date: "2025-03-01", source: "cost"});
        expect(toNumber(xxx.price!.qty)).toBe(65);
        expect(toNumber(xxx.shares)).toBe(14);
        expect(toNumber(xxx.basis!)).toBe(760);
        expect(toNumber(xxx.marketValue!)).toBe(910);
        expect(report.warnings).toEqual([]);
    });

    it("excludes unpriced holdings from totals.marketValue, warns, and sorts them last", () => {
        const txns = [txn(1, "2025-01-10", [buy("assets:broker", "VTI", 10, 20000)]), txn(2, "2025-01-20", [buyNoCost("assets:broker", "NOP", 3)])];
        const report = computeHoldings(txns, [pd("2025-02-01", "VTI", 22000)], scope("2025-06-30"));

        expect(report.holdings.map((h) => h.symbol)).toEqual(["VTI", "NOP"]);
        const nop = only(report, "NOP");
        expect(nop.price).toBeNull();
        expect(nop.marketValue).toBeNull();
        expect(toNumber(report.totals.marketValue)).toBe(2200);
        expect(report.totals.basis).toBeNull();
        expect(report.warnings.map((w) => [w.symbol, w.kind])).toEqual([
            ["NOP", "unpriced"],
            ["NOP", "missing-basis"],
        ]);
    });
});

describe("UNIT holdings/engine firstBasisDate", () => {
    const prices = [pd("2025-02-01", "VTI", 25000)];

    it("a simple buy sets the buy date", () => {
        const txns = [txn(1, "2025-01-10", [buy("a", "VTI", 10, 20000)])];
        expect(only(computeHoldings(txns, prices, scope("2025-06-30")), "VTI").firstBasisDate).toBe("2025-01-10");
    });

    it("a full sell-out then re-buy resets to the re-buy date (basis semantics)", () => {
        const txns = [
            txn(1, "2025-01-10", [buy("a", "VTI", 10, 20000)]),
            txn(2, "2025-02-10", [sell("a", "VTI", 10)]),
            txn(3, "2025-03-10", [buy("a", "VTI", 4, 21000)]),
        ];
        expect(only(computeHoldings(txns, prices, scope("2025-06-30")), "VTI").firstBasisDate).toBe("2025-03-10");
    });

    it("a partial sell keeps the original buy date", () => {
        const txns = [txn(1, "2025-01-10", [buy("a", "VTI", 10, 20000)]), txn(2, "2025-02-10", [sell("a", "VTI", 4)])];
        expect(only(computeHoldings(txns, prices, scope("2025-06-30")), "VTI").firstBasisDate).toBe("2025-01-10");
    });

    it("buying more on top of an open position keeps the earliest buy date", () => {
        const txns = [txn(1, "2025-01-10", [buy("a", "VTI", 10, 20000)]), txn(2, "2025-02-10", [buy("a", "VTI", 5, 22000)])];
        expect(only(computeHoldings(txns, prices, scope("2025-06-30")), "VTI").firstBasisDate).toBe("2025-01-10");
    });
});

describe("UNIT holdings/engine row filtering", () => {
    it("drops a fully sold (zero-share) symbol silently", () => {
        const txns = [txn(1, "2025-01-10", [buy("a", "VTI", 10, 20000)]), txn(2, "2025-02-10", [sell("a", "VTI", 10)])];
        const report = computeHoldings(txns, [pd("2025-02-01", "VTI", 22000)], scope("2025-06-30"));
        expect(report.holdings).toEqual([]);
        expect(report.warnings).toEqual([]);
    });

    it("drops a negative pool with a negative-shares warning", () => {
        const txns = [txn(1, "2025-01-10", [sell("a", "SHT", 5)])];
        const report = computeHoldings(txns, [], scope("2025-06-30"));
        expect(report.holdings).toEqual([]);
        expect(report.warnings).toEqual([{symbol: "SHT", kind: "negative-shares", message: expect.stringContaining("never entered")}]);
    });
});

describe("UNIT holdings/engine asOf time travel", () => {
    const txns = [
        txn(1, "2025-01-05", [{account: "assets:broker", amounts: [withCost(amt("AAPL", 10, 0), 10000, true)], tags: [["name", "Apple Inc."]]}]),
        txn(2, "2025-06-05", [{account: "assets:broker", amounts: [withCost(amt("AAPL", 10, 0), 12000, true)]}], [["name", "Apple Computer"]]),
    ];
    const prices = [pd("2025-01-15", "AAPL", 11000), pd("2025-07-01", "AAPL", 15000)];

    it("early asOf sees only the first lot, first price, and first name tag", () => {
        const aapl = only(computeHoldings(txns, prices, scope("2025-03-01")), "AAPL");
        expect(toNumber(aapl.shares)).toBe(10);
        expect(toNumber(aapl.basis!)).toBe(1000);
        expect(aapl.price).toMatchObject({date: "2025-01-15"});
        expect(toNumber(aapl.price!.qty)).toBe(110);
        expect(aapl.name).toBe("Apple Inc.");
    });

    it("late asOf sees both lots, the newer price, and the txn-level name tag", () => {
        const aapl = only(computeHoldings(txns, prices, scope("2025-12-31")), "AAPL");
        expect(toNumber(aapl.shares)).toBe(20);
        expect(toNumber(aapl.basis!)).toBe(2200);
        expect(aapl.price).toMatchObject({date: "2025-07-01"});
        expect(toNumber(aapl.price!.qty)).toBe(150);
        expect(aapl.name).toBe("Apple Computer");
    });
});

describe("UNIT holdings/engine gainers and losers", () => {
    it("splits by gain sign (gainers desc, losers asc), caps at 5, and skips zero/null gainPct", () => {
        // All bought at $100/share: G1 +60% … G6 +10%, L1 -30% L2 -20% L3 -10%, Z0 flat, T0 tainted (gainPct null).
        const priced: [string, number][] = [
            ["G1", 16000],
            ["G2", 15000],
            ["G3", 14000],
            ["G4", 13000],
            ["G5", 12000],
            ["G6", 11000],
            ["L1", 7000],
            ["L2", 8000],
            ["L3", 9000],
            ["Z0", 10000],
        ];
        const txns = priced.map(([symbol], i) => txn(i + 1, "2025-01-10", [buy("a", symbol, 1, 10000)]));
        txns.push(txn(priced.length + 1, "2025-01-10", [buyNoCost("a", "T0", 1)]));
        const prices = priced.map(([symbol, cents]) => pd("2025-02-01", symbol, cents));
        prices.push(pd("2025-02-01", "T0", 99900));
        const report = computeHoldings(txns, prices, scope("2025-06-30"));

        expect(report.topGainers.map((h) => h.symbol)).toEqual(["G1", "G2", "G3", "G4", "G5"]); // > 0 only, desc, G6 capped off
        expect(report.topLosers.map((h) => h.symbol)).toEqual(["L1", "L2", "L3"]); // < 0 only, asc — Z0 and T0 in neither
    });

    it("returns an empty losers list when every priced holding gained", () => {
        const txns = [txn(1, "2025-01-10", [buy("a", "AAA", 1, 10000)]), txn(2, "2025-01-10", [buy("a", "BBB", 1, 10000)])];
        const prices = [pd("2025-02-01", "AAA", 12000), pd("2025-02-01", "BBB", 11000)];
        const report = computeHoldings(txns, prices, scope("2025-06-30"));

        expect(report.topGainers.map((h) => h.symbol)).toEqual(["AAA", "BBB"]);
        expect(report.topLosers).toEqual([]);
    });
});
