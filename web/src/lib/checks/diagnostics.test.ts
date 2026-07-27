// Engine-computed diagnostics joining the checks pipeline: they enter through
// CheckContext.diagnostics (precomputed — not a CheckRule) and must reach every
// consumer the local rule findings already reach: the badge (maxSeverity), the
// drawer (runChecks order) and per-row flags (groupByTxn).
//
// Fixtures are hand-written to the wire contract; the Rust side that emits them
// does not exist yet, so nothing here talks to a server.

import {describe, expect, it} from "vitest";
import {normalizeDiagnostics} from "../api/normalize";
import {dec} from "../domain/money";
import type {Amount, AmountStyle, Posting, Transaction} from "../domain/types";
import {groupByTxn, maxSeverity, runChecks, type CheckRule, type Problem} from "./engine";

const usdStyle: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: usdStyle});

/** A balanced transaction, so ALL_RULES stays silent and only diagnostics show up. */
function balancedTxn(index: number): Transaction {
    const postings: Posting[] = [
        {account: "expenses:food", amounts: [usd(1000)], status: "unmarked", comment: "", tags: []},
        {account: "assets:bank", amounts: [usd(-1000)], status: "unmarked", comment: "", tags: []},
    ];
    return {
        index,
        date: "2026-07-01",
        status: "unmarked",
        description: `txn ${index}`,
        code: "",
        comment: "",
        tags: [],
        postings,
        haystack: `txn ${index}`,
    };
}

// tindex is 1-based, so position 0 → index 1, position 2 → index 3.
const txns = [balancedTxn(1), balancedTxn(2), balancedTxn(3)];

const ASSERTION_MESSAGE = ["balance assertion failed in assets:bank", "  expected: $10.00", "  actual:   $12.00"].join("\n");

const payload = {
    transactions: [],
    diagnostics: [
        {txnIndex: 0, rule: "unbalanced", severity: "error", message: "This transaction is unbalanced. The real postings' sum should be 0 but is: $-1.00"},
        {txnIndex: 2, rule: "assertion", severity: "error", message: ASSERTION_MESSAGE},
    ],
};

const diagnostics = normalizeDiagnostics(payload, txns);
const ctx = {prices: [], diagnostics};

describe("UNIT engine diagnostics in the checks pipeline", () => {
    it("an error diagnostic drives maxSeverity to error even when every rule is silent", () => {
        expect(maxSeverity(runChecks(txns, {prices: []}))).toBeNull(); // clean without them
        expect(maxSeverity(runChecks(txns, ctx))).toBe("error");
    });

    it("groups under the transaction's own index, not the 0-based wire position", () => {
        const byTxn = groupByTxn(runChecks(txns, ctx));
        expect(byTxn.get(1)?.map((p) => p.rule)).toEqual(["unbalanced"]);
        expect(byTxn.get(3)?.map((p) => p.rule)).toEqual(["assertion"]);
        expect(byTxn.get(0)).toBeUndefined();
        expect(byTxn.get(2)).toBeUndefined();
    });

    it("carries the multi-line assertion message through to the row flags intact", () => {
        const flags = groupByTxn(runChecks(txns, ctx)).get(3) ?? [];
        expect(flags[0].message).toBe(ASSERTION_MESSAGE);
        expect(flags[0].message).toContain("\n");
    });

    it("leads the list, ahead of rule findings, and is stable across runs", () => {
        const noisy: CheckRule = {
            id: "noisy",
            run: (list: Transaction[]): Problem[] => list.map((t) => ({txnIndex: t.index, rule: "noisy", severity: "info", message: "noise"})),
        };
        const out = runChecks(txns, ctx, [noisy]);
        expect(out.map((p) => p.rule)).toEqual(["unbalanced", "assertion", "noisy", "noisy", "noisy"]);
        expect(runChecks(txns, ctx, [noisy])).toEqual(out);
    });

    it("leaves the local rule findings untouched — this is purely additive", () => {
        const withoutDiags = runChecks(txns, {prices: []});
        const withDiags = runChecks(txns, ctx);
        expect(withDiags.slice(diagnostics.length)).toEqual(withoutDiags);
    });

    it("an absent diagnostics field changes nothing", () => {
        expect(runChecks(txns, {prices: []})).toEqual(runChecks(txns, {prices: [], diagnostics: normalizeDiagnostics({}, txns)}));
    });

    it("both engine and local unbalanced findings land in the one `unbalanced` drawer group", () => {
        const unbalancedTxn: Transaction = {
            ...balancedTxn(1),
            postings: [{account: "expenses:food", amounts: [usd(1000)], status: "unmarked", comment: "", tags: []}],
        };
        const out = runChecks([unbalancedTxn], ctx);
        const rules = new Set(out.filter((p) => p.txnIndex === 1).map((p) => p.rule));
        expect(rules).toEqual(new Set(["unbalanced"]));
    });
});
