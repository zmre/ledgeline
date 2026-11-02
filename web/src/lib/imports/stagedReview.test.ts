// What the import screens' review buttons count and hand the overlay (plan 32
// D26, D30), and how a held override reads in the panel.

import {describe, expect, it} from "vitest";
import {decodeCsvStagedTxn, decodeQbStagedTxn} from "#lib/api/importReviewDecode.js";
import {qbStagedTxnWire, stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import type {StagedOverride, StagedTxn} from "./reviewTypes";
import {describeOps, editCountLine, stagedRunCounts, stagedRunItems} from "./stagedReview";

const csv = (ordinal: number, unknown: boolean): StagedTxn =>
    decodeCsvStagedTxn(stagedTxnWire(txnFixture(ordinal + 1, `ROW ${ordinal}`, unknown ? "expenses:unknown" : "expenses:home"), {ordinal, unknown}), "t");
const qb = (id: string, unknown: boolean): StagedTxn =>
    decodeQbStagedTxn(qbStagedTxnWire(txnFixture(Number(id), `QB ${id}`, unknown ? "expenses:unknown" : "expenses:home"), id, {unknown}), "t");
const override = (item: StagedTxn, ops: StagedOverride["ops"]): StagedOverride => ({
    ref: item.ref,
    fingerprint: item.fingerprint ?? "",
    ops,
    original: item,
    flagged: false,
});

const SUPPLIES = {op: "setPostingAccount", posting: 0, account: "expenses:home:supplies"} as const;
const REVIEWED = {op: "setTxnTag", name: "reviewed", value: "2026-10-30"} as const;

describe("UNIT stagedReview — what a run covers", () => {
    const txns = [csv(0, true), csv(1, false), csv(2, true)];

    it("covers every transaction for Review All", () => {
        expect(stagedRunItems("all", txns, []).map((txn) => txn.txn.index)).toEqual([1, 2, 3]);
    });

    it("covers the unknowns for Review Unknowns", () => {
        expect(stagedRunItems("unknowns", txns, []).map((txn) => txn.txn.index)).toEqual([1, 3]);
    });

    it("leaves out an unknown a held override already recategorised (the QuickBooks preview shows the export's text)", () => {
        const items = [qb("10", true), qb("11", true)];
        const held = [override(items[0] as StagedTxn, [SUPPLIES]), override(items[1] as StagedTxn, [REVIEWED])];
        expect(stagedRunItems("unknowns", items, held).map((txn) => txn.ref)).toEqual([{kind: "qb", id: "11"}]);
        expect(stagedRunCounts(items, held)).toEqual({unknowns: 1, all: 2});
    });
});

describe("UNIT stagedReview — the edits line", () => {
    it("says what the write will carry, and nothing when it carries nothing", () => {
        expect(editCountLine(0, 0)).toBeNull();
        expect(editCountLine(1, 0)).toBe("1 reviewed edit");
        expect(editCountLine(3, 2)).toBe("3 reviewed edits · 2 existing transactions edited");
        expect(editCountLine(0, 1)).toBe("1 existing transaction edited");
    });

    it("describes an override's ops in a few words each", () => {
        expect(
            describeOps([
                SUPPLIES,
                {op: "setDescription", description: "Hardware"},
                REVIEWED,
                {op: "setTxnTag", name: "reviewed", value: "ignore"},
                {op: "setTxnTag", name: "trip", value: null},
                {op: "setTxnComment", comment: ""},
            ])
        ).toEqual(["posting 1 → expenses:home:supplies", "renamed “Hardware”", "reviewed", "ignored", "tag trip removed", "comment cleared"]);
        expect(
            describeOps([
                {op: "setDate", date: "2026-10-02"},
                {op: "setDate2", date2: null},
                {op: "setStatus", status: "pending"},
                {op: "setCode", code: "7"},
                {op: "setPostingComment", posting: 1, comment: "x"},
                {op: "removePostingAssertion", posting: 0},
            ])
        ).toEqual(["dated 2026-10-02", "secondary date removed", "marked pending", "code 7", "posting 2 comment edited", "posting 1 assertion removed"]);
    });
});
