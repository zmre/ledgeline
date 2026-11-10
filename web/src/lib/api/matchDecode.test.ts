// The transfer-matching decoders (plan 35) and the journal row's transfer
// fields: every field checked, nothing defaulted to a quiet zero. The live
// round trip is `matching/matchingLive.integration.test.ts`; the writes are
// pinned in Rust (crates/ledgeline-server/tests/match_endpoints.rs).

import {describe, expect, it} from "vitest";
import {reviewTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {wireRow} from "#lib/testing/journalViewFixture.js";
import {combinationWire, matchesPageWire, transferHalf} from "#lib/testing/matchFixture.js";
import {matchesBody, transactionMatchesBody} from "#lib/testing/fakeEngine.js";
import {summaryBody} from "#lib/testing/journalViewFixture.js";
import {ApiShapeError} from "./client";
import {decodeJournalSummary, decodeRow} from "./journalViewDecode";
import {decodeLinkAll, decodeMatches, decodeMatchWrite, decodeTransactionMatches} from "./matchDecode";

const OUT = txnFixture(1, "ACH AMEX EPAYMENT", "transfers:cc", "2026-12-01");
const IN = txnFixture(2, "PAYMENT THANK YOU", "transfers:cc", "2026-12-03");
const OTHER = txnFixture(5, "ANOTHER PAYMENT", "transfers:cc", "2026-12-04");

const half = (txn: typeof OUT, posting = 0, file: unknown = "checking.journal"): Record<string, unknown> => ({
    ...reviewTxnWire(txn, {unknown: false}),
    posting,
    file,
});
const end = (index: number, account: string | null, label: string | null): Record<string, unknown> => ({index, account, label});

function proposalWire(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        a: half(OUT),
        b: half(IN),
        gapDays: 2,
        score: 2,
        clear: true,
        ambiguous: false,
        from: end(1, "assets:checking", "checking"),
        to: end(2, "liabilities:amex", "amex"),
        alternatives: [],
        group: [],
        ...overrides,
    };
}

describe("UNIT decodeMatches", () => {
    it("decodes proposals: halves as review rows with their posting, ends, alternatives", () => {
        const alternative = {a: half(OUT), b: half(OTHER), gapDays: 3, score: 1, taken: false};
        const page = decodeMatches(
            matchesBody({counts: {clear: 0, check: 1}, proposals: [proposalWire({clear: false, ambiguous: true, alternatives: [alternative]})]})
        );
        expect(page).toMatchObject({generation: "fake-1", clearing: true, counts: {clear: 0, check: 1}});
        const proposal = page.proposals[0]!;
        expect(proposal.a).toMatchObject({posting: 0, file: "checking.journal", fingerprint: "fp-1-0", unknown: false});
        expect(proposal.a.txn.description).toBe("ACH AMEX EPAYMENT");
        expect(proposal.from).toEqual({index: 1, account: "assets:checking", label: "checking"});
        expect(proposal.alternatives[0]).toMatchObject({gapDays: 3, score: 1, taken: false});
        expect(proposal.alternatives[0]?.b.txn.index).toBe(5);
        expect(Object.isFrozen(proposal)).toBe(true);
    });

    it("decodes a journal with no clearing account", () => {
        expect(decodeMatches(matchesBody({clearing: false}))).toMatchObject({clearing: false, proposals: []});
    });

    it.each<[string, Record<string, unknown>]>([
        ["counts that disagree", {counts: {clear: 0, check: 0}, proposals: [proposalWire()]}],
        ["a posting past the transaction's", {counts: {clear: 1, check: 0}, proposals: [proposalWire({a: half(OUT, 2)})]}],
        ["a half whose file is null", {counts: {clear: 1, check: 0}, proposals: [proposalWire({a: half(OUT, 0, null)})]}],
        ["a half whose file is not a string", {counts: {clear: 1, check: 0}, proposals: [proposalWire({b: half(IN, 0, 3)})]}],
        ["both halves one transaction", {counts: {clear: 1, check: 0}, proposals: [proposalWire({b: half(OUT)})]}],
        ["ends naming another transaction", {counts: {clear: 1, check: 0}, proposals: [proposalWire({to: end(9, null, null)})]}],
        ["a missing clearing flag", {clearing: undefined}],
    ])("refuses %s", (_, overrides) => {
        expect(() => decodeMatches(matchesBody(overrides))).toThrow(ApiShapeError);
    });
});

describe("UNIT match write decoders", () => {
    it("decodes a write and a link-all", () => {
        const write = decodeMatchWrite({generation: "g2", id: "k7f3a2", edited: [reviewTxnWire(OUT), reviewTxnWire(IN)]});
        expect(write.id).toBe("k7f3a2");
        expect(write.edited.map((txn) => txn.txn.index)).toEqual([1, 2]);
        expect(decodeLinkAll({generation: "g3", linked: 2, matchIds: ["a", "b"]})).toEqual({generation: "g3", linked: 2, matchIds: ["a", "b"]});
        expect(() => decodeLinkAll({generation: "g3", linked: 3, matchIds: ["a"]})).toThrow(ApiShapeError);
        expect(() => decodeMatchWrite({generation: "g2", edited: []})).toThrow(ApiShapeError);
    });
});

describe("UNIT transfer rows (plan 35 D11)", () => {
    const transferRow = (overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
        ...wireRow(OUT),
        linked: true,
        transfer: {
            matchId: "k7f3a2",
            halves: [wireRow(OUT), wireRow(IN)],
            from: end(1, "assets:checking", "checking"),
            to: end(2, "liabilities:amex", "amex"),
            ...overrides,
        },
    });

    it("decodes a collapsed pair with both halves, and leaves an ordinary row without the fields", () => {
        const row = decodeRow(transferRow());
        expect(row.linked).toBe(true);
        expect(row.transfer?.matchId).toBe("k7f3a2");
        expect(row.transfer?.halves.map((h) => h.txn.index)).toEqual([1, 2]);
        expect(row.transfer?.to.label).toBe("amex");
        const plain = decodeRow(wireRow(IN));
        expect(plain.linked).toBeUndefined();
        expect(plain.transfer).toBeUndefined();
        expect(decodeRow({...wireRow(IN), linked: true}).linked).toBe(true);
    });

    it.each<[string, Record<string, unknown>]>([
        ["one half", {halves: [wireRow(OUT)]}],
        ["halves that are not the row", {halves: [wireRow(IN), wireRow(OTHER)]}],
        ["ends naming one half twice", {to: end(1, null, null)}],
        ["a missing match id", {matchId: undefined}],
    ])("refuses a transfer with %s", (_, overrides) => {
        expect(() => decodeRow(transferRow(overrides))).toThrow(ApiShapeError);
    });

    it("reads the summary's transfer count, 0 from an engine that predates it", () => {
        expect(decodeJournalSummary(summaryBody(6, {transfers: 1})).transfers).toBe(1);
        expect(decodeJournalSummary(summaryBody(6)).transfers).toBe(0);
        expect(() => decodeJournalSummary(summaryBody(6, {transfers: -1}))).toThrow(ApiShapeError);
    });
});

describe("UNIT decodeMatches: Amendment A", () => {
    const BILL_A = txnFixture(6, "Rent bill", "transfers:cc", "2026-12-01");
    const BILL_B = txnFixture(7, "Hosting bill", "transfers:cc", "2026-12-02");
    const combination = (overrides: Record<string, unknown> = {}) =>
        proposalWire({b: half(BILL_A), group: [half(BILL_A), half(BILL_B)], to: end(6, "expenses:rent", "rent"), clear: false, score: 0, ...overrides});

    it("decodes older and a combination's group, b first", () => {
        const page = decodeMatches(matchesBody({counts: {clear: 0, check: 1}, older: 4, proposals: [combination()]}));
        expect(page.older).toBe(4);
        const proposal = page.proposals[0]!;
        expect(proposal.group.map((member) => member.txn.index)).toEqual([6, 7]);
        expect(proposal.b.txn.index).toBe(6);
        expect(Object.isFrozen(proposal.group)).toBe(true);
    });

    it("refuses a missing older, a missing group, and a group that does not start with b or repeats a transaction", () => {
        const body = (proposal: Record<string, unknown>, extra: Record<string, unknown> = {}) =>
            matchesBody({counts: {clear: 0, check: 1}, proposals: [proposal], ...extra});
        expect(() => decodeMatches({...body(combination()), older: undefined})).toThrow(ApiShapeError);
        expect(() => decodeMatches({...body(combination()), older: -1})).toThrow(ApiShapeError);
        expect(() => decodeMatches(body({...proposalWire({clear: false}), group: undefined}))).toThrow(ApiShapeError);
        expect(() => decodeMatches(body(combination({group: [half(BILL_B), half(BILL_A)]})))).toThrow(ApiShapeError);
        expect(() => decodeMatches(body(combination({group: [half(BILL_A)]})))).toThrow(ApiShapeError);
        expect(() => decodeMatches(body(combination({group: [half(BILL_A), half(BILL_A)]})))).toThrow(ApiShapeError);
    });
});

describe("UNIT decodeTransactionMatches", () => {
    const partner = (txn: typeof OUT, gapDays: number) => ({...half(txn), gapDays});
    const transaction = reviewTxnWire(OUT, {unknown: false});

    it("decodes each posting's exact partners, combinations and nearby, and the links", () => {
        const decoded = decodeTransactionMatches(
            transactionMatchesBody(transaction, {
                postings: [
                    {
                        posting: 0,
                        exact: [partner(IN, 2), partner(OTHER, 548)],
                        combinations: [proposalWire({b: half(IN), group: [half(IN), half(OTHER)], clear: false})],
                        nearby: [partner(IN, 2)],
                    },
                ],
                links: [{matchId: "grp001", broken: false, members: [half(OUT), half(IN), half(OTHER)]}],
            })
        );
        expect(decoded.transaction.txn.index).toBe(1);
        const [posting] = decoded.postings;
        expect(posting?.exact.map((p) => [p.txn.index, p.gapDays])).toEqual([
            [2, 2],
            [5, 548],
        ]);
        expect(posting?.combinations[0]?.group).toHaveLength(2);
        expect(posting?.nearby[0]).toMatchObject({posting: 0, file: "checking.journal", gapDays: 2});
        expect(decoded.links[0]).toMatchObject({matchId: "grp001", broken: false});
        expect(decoded.links[0]?.members).toHaveLength(3);
        expect(Object.isFrozen(decoded.postings)).toBe(true);
    });

    it("accepts a transaction with nothing to offer, and refuses bad shapes", () => {
        expect(decodeTransactionMatches(transactionMatchesBody(transaction)).postings).toEqual([]);
        const bad = (posting: Record<string, unknown>) =>
            transactionMatchesBody(transaction, {postings: [{posting: 0, exact: [], combinations: [], nearby: [], ...posting}]});
        expect(() => decodeTransactionMatches(bad({posting: 9}))).toThrow(ApiShapeError);
        expect(() => decodeTransactionMatches(bad({exact: [half(IN)]}))).toThrow(ApiShapeError);
        expect(() => decodeTransactionMatches(bad({combinations: [proposalWire()]}))).toThrow(ApiShapeError);
        expect(() => decodeTransactionMatches(transactionMatchesBody(transaction, {links: [{matchId: "x", members: []}]}))).toThrow(ApiShapeError);
    });
});

describe("UNIT matchFixture combinations", () => {
    it("builds a combination the decoder accepts", () => {
        const debit = transferHalf(1, "2027-01-12", "BILL.COM PAYABLES", "assets:checking", -224750);
        const rent = transferHalf(2, "2027-01-12", "Rent", "expenses:rent", 200000);
        const hosting = transferHalf(3, "2027-01-13", "Hosting", "expenses:hosting", 24750);
        const page = decodeMatches(matchesPageWire([combinationWire(debit, [rent, hosting])]));
        expect(page.counts).toEqual({clear: 0, check: 1});
        const [proposal] = page.proposals;
        expect(proposal?.a.txn.index).toBe(1);
        expect(proposal?.group.map((member) => member.txn.index)).toEqual([2, 3]);
        expect(proposal?.gapDays).toBe(1);
        expect(proposal?.from.index).toBe(1);
    });
});
