// Amendment A's pure decisions (plan 35): which postings Find match is
// offered for, a transaction's links, the focused view's running total in
// exact decimals, the body a selection links with, the flags on an exact
// partner, and the lookback presets.

import {describe, expect, it} from "vitest";
import {decodeTransactionMatches} from "#lib/api/matchDecode.js";
import type {Transaction} from "#lib/domain/types.js";
import {transactionMatchesBody} from "#lib/testing/fakeEngine.js";
import {clearingHalf, matchHalfWire} from "#lib/testing/matchFixture.js";
import {reviewTxnWire} from "#lib/testing/reviewFixture.js";
import {findableClearing, flowAmount, focusedHalf, focusLinkBody, linkIdsOf, LOOKBACK_PRESETS, olderLabel, partnerFlags, selectionTotal} from "./model";
import type {MatchPartner} from "./types";

/** Bill.com: one $2,247.50 bank debit, paid out as a $2,000 bill and a $247.50 bill. */
const DEBIT = clearingHalf(1, "2019-02-19", "BILL.COM PAYABLES", "assets:checking", -224750);
const BILL_A = clearingHalf(2, "2019-02-20", "ACME INV 12", "expenses:services", 200000);
const BILL_B = clearingHalf(3, "2019-02-21", "BOLT INV 7", "expenses:software", 24750);
const EXACT = clearingHalf(4, "2019-01-02", "OLD REFUND", "expenses:services", 224750);
/** Cents that a float sum would get wrong: 0.10 + 0.20 must be exactly 0.30. */
const DIME = clearingHalf(5, "2019-02-19", "DIME", "assets:checking", -30);
const TEN = clearingHalf(6, "2019-02-19", "TEN", "expenses:a", 10);
const TWENTY = clearingHalf(7, "2019-02-20", "TWENTY", "expenses:b", 20);

/** Decode `/for` for `txn` offering `nearby` (each with its gap) as one posting's nearby list. */
function forView(txn: Transaction, nearby: Transaction[]) {
    const partner = (other: Transaction, gapDays = 1) => ({...matchHalfWire(other), gapDays});
    return decodeTransactionMatches(
        transactionMatchesBody(reviewTxnWire(txn, {unknown: false}), {
            postings: [{posting: 1, exact: [], combinations: [], nearby: nearby.map((other) => partner(other))}],
        })
    );
}

describe("UNIT findableClearing / linkIdsOf (A2 entry points)", () => {
    it("offers an unlinked clearing posting, by its written position", () => {
        expect(findableClearing(DEBIT)).toEqual([1]);
    });

    it("does not offer a posting that is linked, ignored, or not on a clearing account", () => {
        expect(findableClearing(clearingHalf(1, "2019-02-19", "X", "assets:checking", -100, [["match", "k7f3a2"]]))).toEqual([]);
        expect(findableClearing(clearingHalf(1, "2019-02-19", "X", "assets:checking", -100, [["match", "ignore"]]))).toEqual([]);
        const plain: Transaction = {...DEBIT, postings: DEBIT.postings.map((posting) => ({...posting, tags: []}))};
        expect(findableClearing(plain)).toEqual([]);
    });

    it("does not offer a virtual posting, one with a cost, or one with an assertion (D3)", () => {
        const withClearing = (change: object): Transaction => ({
            ...DEBIT,
            postings: DEBIT.postings.map((posting, at) => (at === 1 ? {...posting, ...change} : posting)),
        });
        expect(findableClearing(withClearing({type: "virtual"}))).toEqual([]);
        const amount = DEBIT.postings[1]?.amounts[0];
        if (amount === undefined) throw new Error("fixture");
        expect(findableClearing(withClearing({amounts: [{...amount, cost: {commodity: "EUR", qty: {m: 1n, p: 0}, per: true}}]}))).toEqual([]);
        expect(findableClearing(withClearing({balanceAssertion: {amount, inclusive: false, total: false}}))).toEqual([]);
    });

    it("lists a transaction's link ids once each, never `ignore`", () => {
        const linked = clearingHalf(1, "2019-02-19", "X", "assets:checking", -100, [["match", "k7f3a2"]]);
        expect(linkIdsOf(linked)).toEqual(["k7f3a2"]);
        expect(linkIdsOf(clearingHalf(1, "2019-02-19", "X", "assets:checking", -100, [["match", "ignore"]]))).toEqual([]);
        expect(linkIdsOf(DEBIT)).toEqual([]);
    });
});

describe("UNIT selectionTotal (A2 running total)", () => {
    const view = forView(DEBIT, [BILL_A, BILL_B]);
    const focused = focusedHalf(view.transaction, 1);
    const [billA, billB] = view.postings[0]?.nearby ?? [];
    if (billA === undefined || billB === undefined) throw new Error("fixture");

    it("is balanced only when the picked postings cancel this one exactly", () => {
        expect(selectionTotal(focused, [])).toMatchObject({balanced: false, problem: null});
        expect(selectionTotal(focused, [billA])).toEqual({label: "$2000.00 of $2247.50", balanced: false, problem: null});
        expect(selectionTotal(focused, [billA, billB])).toEqual({label: "$2247.50 of $2247.50", balanced: true, problem: null});
    });

    it("adds in exact decimals: $0.10 + $0.20 is exactly $0.30", () => {
        const dimes = forView(DIME, [TEN, TWENTY]);
        const [ten, twenty] = dimes.postings[0]?.nearby ?? [];
        if (ten === undefined || twenty === undefined) throw new Error("fixture");
        expect(selectionTotal(focusedHalf(dimes.transaction, 1), [ten, twenty])).toEqual({label: "$0.30 of $0.30", balanced: true, problem: null});
    });

    it("refuses two postings of one transaction, and another commodity", () => {
        expect(selectionTotal(focused, [billA, billA]).problem).toContain("one transaction");
        const euro = {
            ...billB,
            txn: {...billB.txn, postings: billB.txn.postings.map((p) => ({...p, amounts: p.amounts.map((a) => ({...a, commodity: "EUR"}))}))},
        };
        expect(selectionTotal(focused, [billA, euro])).toMatchObject({balanced: false, problem: expect.stringContaining("commodity") as string});
    });

    it("links one partner as a pair and several as a group, this posting first", () => {
        expect(focusLinkBody(focused, [billA])).toEqual({a: {index: 1, posting: 1, fingerprint: "fp-1-0"}, b: {index: 2, posting: 1, fingerprint: "fp-2-0"}});
        expect(focusLinkBody(focused, [billA, billB])).toEqual({
            members: [
                {index: 1, posting: 1, fingerprint: "fp-1-0"},
                {index: 2, posting: 1, fingerprint: "fp-2-0"},
                {index: 3, posting: 1, fingerprint: "fp-3-0"},
            ],
        });
    });

    it("shows what moved on the real side, signed", () => {
        expect(flowAmount(billA)?.qty).toEqual({m: 200000n, p: 2});
        expect(flowAmount(focused)?.qty).toEqual({m: -224750n, p: 2});
    });
});

describe("UNIT partnerFlags / presets (A1, A2)", () => {
    const partner = (gapDays: number): MatchPartner => {
        const view = decodeTransactionMatches(
            transactionMatchesBody(reviewTxnWire(DEBIT, {unknown: false}), {
                postings: [{posting: 1, exact: [{...matchHalfWire(EXACT), gapDays}], combinations: [], nearby: []}],
            })
        );
        const found = view.postings[0]?.exact[0];
        if (found === undefined) throw new Error("fixture");
        return found;
    };

    it("flags an exact partner outside the window or before the lookback", () => {
        expect(partnerFlags(partner(48), "2019-03-01", 10, 365)).toEqual(["outside the 10-day window"]);
        expect(partnerFlags(partner(48), "2020-06-01", 60, 365)).toEqual(["before the lookback"]);
        expect(partnerFlags(partner(48), "2020-06-01", 60, 0)).toEqual([]);
        expect(partnerFlags(partner(3), "2019-03-01", 10, 365)).toEqual([]);
    });

    it("offers 90 days, 1 year, 2 years and All time, and words the older count", () => {
        expect(LOOKBACK_PRESETS.map((preset) => [preset.days, preset.label])).toEqual([
            [90, "90 days"],
            [365, "1 year"],
            [730, "2 years"],
            [0, "All time"],
        ]);
        expect(olderLabel(0)).toBeNull();
        expect(olderLabel(12)).toBe("12 more older than the lookback");
    });
});
