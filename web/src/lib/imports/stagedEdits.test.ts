// The review edits an import carries: merged per transaction, in their wire
// forms, and replayed. The decisions are `reviewModel.test.ts`'s; this pins the
// holding.

import {describe, expect, it} from "vitest";
import {decodeCsvStagedTxn, decodeQbStagedTxn} from "#lib/api/importReviewDecode.js";
import {qbStagedTxnWire, stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {createStagedEdits} from "./stagedEdits.svelte";

const card = (ordinal: number, fingerprint = `gen-${ordinal}`) =>
    decodeCsvStagedTxn(stagedTxnWire(txnFixture(ordinal + 1, "HARDWARE STORE"), {ordinal, fingerprint}), "card");
const REVIEWED = {op: "setTxnTag", name: "reviewed", value: "2026-10-30"} as const;
const SUPPLIES = {op: "setPostingAccount", posting: 0, account: "expenses:home:supplies"} as const;

describe("UNIT stagedEdits", () => {
    it("holds one override per transaction, its ops merged and its first fingerprint kept", () => {
        const edits = createStagedEdits();
        expect(edits.stage({overrides: [{item: card(0), ops: [SUPPLIES]}], journalEdits: []})).toEqual({ok: true});
        // Saved again from the card as it now reads (an overridden row, same generated fingerprint).
        edits.stage({overrides: [{item: card(0), ops: [REVIEWED]}], journalEdits: []});
        expect(edits.overrideCount).toBe(1);
        expect(edits.csvOverrides()).toEqual([{ordinal: 0, fingerprint: "gen-0", ops: [SUPPLIES, REVIEWED]}]);
        expect(edits.qbOverrides()).toEqual([]);
    });

    it("skips entries with nothing to apply, and refuses a card with no fingerprint", () => {
        const edits = createStagedEdits();
        edits.stage({overrides: [{item: card(1), ops: []}], journalEdits: [{index: 9, fingerprint: "j", ops: []}]});
        expect(edits.overrideCount).toBe(0);
        expect(edits.journalEditCount).toBe(0);
        const blind = {...card(2), fingerprint: null};
        expect(edits.stage({overrides: [{item: blind, ops: [REVIEWED]}], journalEdits: []}).ok).toBe(false);
    });

    it("queues journal edits by index, merging their ops", () => {
        const edits = createStagedEdits();
        edits.stage({overrides: [], journalEdits: [{index: 12, fingerprint: "j1", ops: [SUPPLIES]}]});
        edits.stage({overrides: [], journalEdits: [{index: 12, fingerprint: "j2", ops: [{op: "setDescription", description: "Hardware"}]}]});
        expect(edits.journalEdits).toEqual([{index: 12, fingerprint: "j1", ops: [SUPPLIES, {op: "setDescription", description: "Hardware"}]}]);
    });

    it("replays against a fresh import, dropping what a rule covers", () => {
        const edits = createStagedEdits();
        edits.stage({
            overrides: [
                {item: card(0), ops: [SUPPLIES]},
                {item: card(1), ops: [REVIEWED]},
            ],
            journalEdits: [],
        });
        const covered = card(0, "gen-0b");
        const fresh = [
            {...covered, txn: {...covered.txn, postings: covered.txn.postings.map((p, at) => (at === 0 ? {...p, account: "expenses:home:supplies"} : p))}},
            card(1),
        ];
        const report = edits.replay(fresh);
        expect(report.dropped.map((drop) => drop.reason)).toEqual(["covered"]);
        expect(edits.csvOverrides()).toEqual([{ordinal: 1, fingerprint: "gen-1", ops: [REVIEWED]}]);
    });

    it("speaks QuickBooks by Trans #, and forgets everything on clear", () => {
        const edits = createStagedEdits();
        const qb = decodeQbStagedTxn(qbStagedTxnWire(txnFixture(1, "Office"), "1042"), "qb");
        edits.stage({overrides: [{item: qb, ops: [REVIEWED]}], journalEdits: []});
        expect(edits.qbOverrides()).toEqual([{id: "1042", fingerprint: "qb-1042", ops: [REVIEWED]}]);
        expect(edits.overrideFor(qb.ref)?.ops).toEqual([REVIEWED]);
        edits.clear();
        expect(edits.overrideCount).toBe(0);
        expect(edits.overrideFor(qb.ref)).toBeNull();
    });

    it("lets go of one override on request, leaving the rest", () => {
        const edits = createStagedEdits();
        edits.stage({
            overrides: [
                {item: card(0), ops: [SUPPLIES]},
                {item: card(1), ops: [REVIEWED]},
            ],
            journalEdits: [],
        });
        edits.discard({kind: "csv", ordinal: 0});
        edits.discard({kind: "csv", ordinal: 7});
        expect(edits.csvOverrides()).toEqual([{ordinal: 1, fingerprint: "gen-1", ops: [REVIEWED]}]);
    });
});
