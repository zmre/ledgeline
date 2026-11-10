// Find Matches' pure decisions (plan 35 Phase 3): what a card shows of each
// half, its choices and header, how a pair is remembered across re-reads, and
// what a write names.

import {describe, expect, it} from "vitest";
import {decodeMatches} from "#lib/api/matchDecode.js";
import type {Transaction} from "#lib/domain/types.js";
import {matchesBody} from "#lib/testing/fakeEngine.js";
import {reviewTxnWire} from "#lib/testing/reviewFixture.js";
import {choicesOf, endLabels, gapLabel, halfView, headerOf, importSince, otherHalf, pairKey, postingsTagged, rejectionsOf, wireHalf} from "./model";

const STYLE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null} as const;
const usd = (cents: number) => ({commodity: "$", qty: {m: BigInt(cents), p: 2}, style: STYLE});

/** A transfer half: the real account, then the clearing posting. */
function half(index: number, date: string, description: string, real: string, cents: number, tags: [string, string][] = []): Transaction {
    return {
        index,
        date,
        status: "cleared",
        description,
        code: "",
        comment: "",
        tags: [],
        postings: [
            {account: real, amounts: [usd(cents)], status: "unmarked", comment: "", tags: []},
            {account: "transfers:cc", amounts: [usd(-cents)], status: "unmarked", comment: "", tags},
        ],
    };
}

const OUT = half(1, "2026-12-31", "ACH AMEX EPAYMENT", "assets:checking", -50000);
const IN = half(2, "2027-01-02", "PAYMENT THANK YOU", "liabilities:amex", 50000);
const LATE = half(3, "2027-01-05", "PAYMENT AGAIN", "liabilities:visa", 50000);

const wire = (txn: Transaction) => ({...reviewTxnWire(txn, {unknown: false}), posting: 1, file: "main.journal"});

function page(alternatives: unknown[] = []) {
    return decodeMatches(
        matchesBody({
            counts: {clear: alternatives.length === 0 ? 1 : 0, check: alternatives.length === 0 ? 0 : 1},
            proposals: [
                {
                    a: wire(OUT),
                    b: wire(IN),
                    gapDays: 2,
                    score: 2,
                    clear: alternatives.length === 0,
                    ambiguous: alternatives.length > 0,
                    from: {index: 1, account: "assets:checking", label: "checking"},
                    to: {index: 2, account: "liabilities:amex", label: "amex"},
                    alternatives,
                    group: [],
                },
            ],
        })
    );
}

describe("UNIT matching model", () => {
    it("shows a half's real account and amount, and its clearing posting", () => {
        const [proposal] = page().proposals;
        const view = halfView(proposal!.a);
        expect(view.real?.account).toBe("assets:checking");
        expect(view.realAmount?.qty.m).toBe(-50000n);
        expect(view.clearing?.account).toBe("transfers:cc");
    });

    it("reads the header from the engine's ends, and works an alternative's out by sign", () => {
        const proposal = page([{a: wire(OUT), b: wire(LATE), gapDays: 5, score: 2, taken: false}]).proposals[0]!;
        const choices = choicesOf(proposal);
        expect(choices).toHaveLength(2);
        expect(headerOf(choices[0]!, proposal)).toMatchObject({from: "checking", to: "amex", amount: "$500.00"});
        expect(headerOf(choices[1]!, proposal)).toMatchObject({from: "checking", to: "visa", toAccount: "liabilities:visa", amount: "$500.00"});
        expect(otherHalf(choices[1]!, proposal).txn.index).toBe(3);
    });

    it("labels ends by last segment, or in full when those collide", () => {
        expect(endLabels("assets:checking", "liabilities:amex")).toEqual(["checking", "amex"]);
        expect(endLabels("assets:bank1:checking", "assets:bank2:checking")).toEqual(["assets:bank1:checking", "assets:bank2:checking"]);
    });

    it("keys a pair by its halves' text, either way round", () => {
        const proposal = page().proposals[0]!;
        expect(pairKey(proposal.a, proposal.b)).toBe(pairKey(proposal.b, proposal.a));
        expect(pairKey(proposal.a, proposal.b)).toBe("fp-1-0/1|fp-2-0/1");
    });

    it("names a half for a write, with its description only when it changed", () => {
        const proposal = page().proposals[0]!;
        expect(wireHalf(proposal.a)).toEqual({index: 1, posting: 1, fingerprint: "fp-1-0"});
        expect(wireHalf(proposal.a, "ACH AMEX EPAYMENT ")).toEqual({index: 1, posting: 1, fingerprint: "fp-1-0"});
        expect(wireHalf(proposal.a, "Amex payment")).toEqual({index: 1, posting: 1, fingerprint: "fp-1-0", description: "Amex payment"});
    });

    it("finds the postings carrying a tag, and a transaction's rejections once each", () => {
        const rejected = half(4, "2027-01-01", "TRANSFER", "assets:savings", 50000, [
            ["nomatch", "r4x9"],
            ["nomatch", "b2c1"],
            ["match", "k7f3a2"],
        ]);
        expect(rejectionsOf(rejected)).toEqual(["r4x9", "b2c1"]);
        expect(postingsTagged(rejected, "match", "k7f3a2")).toEqual([1]);
        expect(postingsTagged(rejected, "match", "other")).toEqual([]);
        expect(rejectionsOf(OUT)).toEqual([]);
    });

    it("says how far apart", () => {
        expect(gapLabel(0)).toBe("same day");
        expect(gapLabel(1)).toBe("1 day apart");
        expect(gapLabel(2)).toBe("2 days apart");
    });

    it("scopes an import's Find Matches to the transactions it appended, until a re-sort moves them", () => {
        expect(importSince("imports/amex.journal", 12, false)).toBe("imports/amex.journal:12");
        expect(importSince("amex.journal", 12, true)).toBeNull();
        expect(importSince(null, 12, false)).toBeNull();
        expect(importSince("amex.journal", 0, false)).toBeNull();
    });
});
