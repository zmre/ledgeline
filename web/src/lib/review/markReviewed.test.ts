import {describe, expect, it} from "vitest";
import type {BatchTransactionsBody, LedgelineApi} from "#lib/api/native.js";
import {queueItemWire, reviewTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {markReviewedWith, reviewedOps} from "./markReviewed";

describe("reviewedOps", () => {
    it("is the review card's own reviewed:<date> tag", () => {
        expect(reviewedOps("2026-11-13")).toEqual([{op: "setTxnTag", name: "reviewed", value: "2026-11-13"}]);
    });
});

describe("markReviewedWith", () => {
    it("asks for the fingerprint, then tags with it in one batch", async () => {
        const calls: string[] = [];
        let sent: BatchTransactionsBody | null = null;
        const api = {
            reviewItem: async (index: number, today: string) => {
                calls.push(`item ${index} ${today}`);
                return {generation: "g1", item: queueItemWire(txnFixture(index, "Coffee"), {fingerprint: "fp-7"})};
            },
            batchTransactions: async (body: BatchTransactionsBody) => {
                calls.push("batch");
                sent = body;
                return {generation: "g2", edited: [reviewTxnWire(txnFixture(7, "Coffee"), {reviewed: true})]};
            },
        } as unknown as LedgelineApi;
        await markReviewedWith(api, 7, "2026-11-13");
        expect(calls).toEqual(["item 7 2026-11-13", "batch"]);
        expect(sent).toEqual({edits: [{index: 7, fingerprint: "fp-7", ops: [{op: "setTxnTag", name: "reviewed", value: "2026-11-13"}]}]});
    });
});
