// WP-10 stock check rules: journal-wide at today(), unscoped, over the
// average-cost pools shared with lib/holdings/engine.

import {describe, expect, it} from "vitest";
import type {Transaction} from "../domain/types";
import {amt, pd, txn, withCost, type PostingSpec} from "../holdings/test-helpers";
import {runChecks, type CheckContext, type Problem} from "./engine";

const buy = (account: string, symbol: string, qty: number, costCents: number): PostingSpec => ({
    account,
    amounts: [withCost(amt(symbol, qty, 0), costCents, true)],
});
const buyNoCost = (account: string, symbol: string, qty: number): PostingSpec => ({account, amounts: [amt(symbol, qty, 0)]});
const sell = (account: string, symbol: string, qty: number): PostingSpec => ({account, amounts: [amt(symbol, -qty, 0)]});

const run = (txns: Transaction[], rule: string, ctx: CheckContext = {prices: []}): Problem[] => runChecks(txns, ctx).filter((p) => p.rule === rule);

describe("UNIT checks/rules stock-missing-basis", () => {
    it("flags each cost-less acquisition lot of a currently-held stock, anchored to the offending buy", () => {
        const txns = [
            txn(1, "2020-01-10", [buyNoCost("assets:broker", "GLD", 10)]),
            txn(2, "2020-02-10", [buy("assets:broker", "VTI", 10, 20000)]),
            txn(3, "2020-03-10", [buyNoCost("assets:broker", "GLD", 5)]),
        ];
        const problems = run(txns, "stock-missing-basis");
        expect(problems.map((p) => p.txnIndex)).toEqual([1, 3]);
        expect(problems.every((p) => p.severity === "warning" && p.message.includes("GLD"))).toBe(true);
    });

    it("ignores stocks that are no longer held", () => {
        const txns = [txn(1, "2020-01-10", [buyNoCost("assets:broker", "ZZZ", 10)]), txn(2, "2020-02-10", [sell("assets:broker", "ZZZ", 10)])];
        expect(run(txns, "stock-missing-basis")).toEqual([]);
    });
});

describe("UNIT checks/rules stock-negative", () => {
    it("flags net-negative shares, anchored to the txn that took the running total negative", () => {
        const txns = [
            txn(1, "2020-01-10", [buy("assets:broker", "SHT", 5, 1000)]),
            txn(2, "2020-02-10", [sell("assets:broker", "SHT", 10)]), // 5 → -5: the crossing
            txn(3, "2020-03-10", [sell("assets:broker", "SHT", 2)]),
        ];
        const problems = run(txns, "stock-negative");
        expect(problems).toHaveLength(1);
        expect(problems[0]).toMatchObject({txnIndex: 2, severity: "warning"});
        expect(problems[0].message).toContain("opening position was likely never entered");
    });

    it("flags a sell of a never-bought symbol at that sell", () => {
        const problems = run([txn(7, "2020-01-10", [sell("assets:broker", "NVR", 3)])], "stock-negative");
        expect(problems).toHaveLength(1);
        expect(problems[0].txnIndex).toBe(7);
    });

    it("stays quiet when the position recovers to non-negative", () => {
        const txns = [txn(1, "2020-01-10", [sell("a", "SHT", 3)]), txn(2, "2020-02-10", [buy("a", "SHT", 3, 1000)])];
        expect(run(txns, "stock-negative")).toEqual([]);
    });
});

describe("UNIT checks/rules stock-unpriced", () => {
    it("flags a held stock with no P directive and no usable cost annotation, anchored to its latest txn", () => {
        const txns = [
            txn(1, "2020-01-10", [buyNoCost("assets:broker", "GLD", 10)]),
            txn(2, "2020-02-10", [sell("assets:broker", "GLD", 2)]), // latest touch, still held (8)
        ];
        const problems = run(txns, "stock-unpriced");
        expect(problems).toHaveLength(1);
        expect(problems[0]).toMatchObject({txnIndex: 2, severity: "warning"});
        expect(problems[0].message).toContain("GLD");
    });

    it("accepts a P directive or a cost annotation as a price source", () => {
        const txns = [
            txn(1, "2020-01-10", [buyNoCost("assets:broker", "AAA", 10)]), // P directive below
            txn(2, "2020-01-10", [buy("assets:broker", "VTI", 10, 20000)]), // priced via its own cost
        ];
        expect(run(txns, "stock-unpriced", {prices: [pd("2020-01-01", "AAA", 1000)]})).toEqual([]);
    });
});
