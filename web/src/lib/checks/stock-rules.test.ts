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

// A share leg posted to equity/income/expense is the FUNDING side of the
// movement, not a place shares are held. Counting it nets the acquisition to
// zero, so the shares never enter the pool and a later sale drives the pool
// negative — the check rules then contradict both the holdings page and the
// balance sheet. Rust's build_pools has always skipped these legs
// (`is_holding_account`); this is the TS port catching up (FE-2).
describe("UNIT checks/rules stock rules skip non-holding (equity/income/expense) legs", () => {
    const vest = (symbol: string, qty: number): PostingSpec[] => [
        buyNoCost("assets:brokerage", symbol, qty),
        {account: "income:rsu", amounts: [amt(symbol, -qty, 0)]},
    ];

    it("does not report negative shares for an RSU vest that is later sold in full", () => {
        const txns = [
            txn(1, "2020-01-15", vest("ACME", 10)),
            txn(2, "2020-06-01", [
                {account: "assets:brokerage", amounts: [withCost(amt("ACME", -10, 0), 10000, true)]},
                {account: "assets:cash", amounts: [amt("$", 100000, 2)]},
            ]),
        ];
        expect(run(txns, "stock-negative")).toEqual([]);
    });

    it("reports the cost-less vest lot as missing basis while the shares are still held", () => {
        const problems = run([txn(1, "2020-01-15", vest("ACME", 10))], "stock-missing-basis");
        expect(problems).toHaveLength(1);
        expect(problems[0]).toMatchObject({txnIndex: 1, severity: "warning"});
        expect(problems[0].message).toContain("ACME");
    });

    it("keeps the shares of an explicitly booked opening balance", () => {
        // equity:opening is the funding side: the pool holds 100, not 0, so an
        // unpriced/missing-basis finding can actually fire.
        const txns = [
            txn(1, "2020-01-01", [buyNoCost("assets:brokerage", "OPEN", 100), {account: "equity:opening balances", amounts: [amt("OPEN", -100, 0)]}]),
        ];
        expect(run(txns, "stock-missing-basis").map((p) => p.txnIndex)).toEqual([1]);
        expect(run(txns, "stock-unpriced").map((p) => p.txnIndex)).toEqual([1]);
        expect(run(txns, "stock-negative")).toEqual([]);
    });

    it("classifies the funding leg by declared TYPE, not by its name", () => {
        // `vesting:rsu` looks like nothing in particular; the account directive
        // says it is revenue, so it funds rather than holds.
        const txns = [
            txn(1, "2020-01-15", [buyNoCost("assets:brokerage", "ACME", 10), {account: "vesting:rsu", amounts: [amt("ACME", -10, 0)]}]),
            txn(2, "2020-06-01", [
                {account: "assets:brokerage", amounts: [withCost(amt("ACME", -10, 0), 10000, true)]},
                {account: "assets:cash", amounts: [amt("$", 100000, 2)]},
            ]),
        ];
        const ctx: CheckContext = {prices: [], decls: [{name: "vesting", type: "revenue"}]};
        expect(run(txns, "stock-negative", ctx)).toEqual([]);
        expect(run(txns, "stock-negative")).toHaveLength(1); // undeclared: `vesting:rsu` reads as a holding account
    });

    it("still nets a genuine transfer between two holding accounts to zero", () => {
        const txns = [
            txn(1, "2020-01-10", [buy("assets:broker:a", "VTI", 10, 20000)]),
            txn(2, "2020-02-10", [sell("assets:broker:a", "VTI", 4), buyNoCost("assets:broker:b", "VTI", 4)]),
        ];
        expect(run(txns, "stock-missing-basis")).toEqual([]);
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
