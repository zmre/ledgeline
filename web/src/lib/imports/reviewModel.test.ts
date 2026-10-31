// How a card's saves combine into one override, and what replay (D25) does
// with the held overrides when the import is regenerated.

import {describe, expect, it} from "vitest";
import {decodeCsvStagedTxn, decodeQbStagedTxn} from "#lib/api/importReviewDecode.js";
import type {WireEditOp} from "#lib/api/native.js";
import type {Transaction} from "#lib/domain/types.js";
import {qbStagedTxnWire, stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {mergeOps, replayOverrides} from "./reviewModel";
import type {StagedOverride, StagedTxn} from "./reviewTypes";

/** A generated staged transaction at `ordinal`, with fingerprint `fp`. */
function staged(ordinal: number, fp: string, edit: (txn: Transaction) => void = () => {}): StagedTxn {
    const txn = txnFixture(ordinal + 1, "HARDWARE STORE 4417");
    edit(txn);
    return decodeCsvStagedTxn(stagedTxnWire(txn, {ordinal, fingerprint: fp, key: "hardware store"}), "staged");
}

function override(item: StagedTxn, ops: WireEditOp[], flagged = false): StagedOverride {
    return {ref: item.ref, fingerprint: item.fingerprint as string, ops, original: item, flagged};
}

const RECATEGORISE: WireEditOp = {op: "setPostingAccount", posting: 0, account: "expenses:home:supplies"};
const REVIEWED: WireEditOp = {op: "setTxnTag", name: "reviewed", value: "2026-10-30"};
const RENAME: WireEditOp = {op: "setDescription", description: "Hardware Store"};

describe("UNIT reviewModel — mergeOps", () => {
    it("keeps one op per thing written, the last one winning, in the order the winners were written", () => {
        const merged = mergeOps(
            [RENAME, {op: "setTxnTag", name: "reviewed", value: "2026-10-01"}, {op: "setPostingAccount", posting: 0, account: "a"}],
            [REVIEWED, {op: "setPostingAccount", posting: 0, account: "b"}, {op: "setPostingAccount", posting: 1, account: "c"}]
        );
        expect(merged).toEqual([RENAME, REVIEWED, {op: "setPostingAccount", posting: 0, account: "b"}, {op: "setPostingAccount", posting: 1, account: "c"}]);
    });

    it("lets a later save's replace stand alone: it re-renders the whole form", () => {
        const replace: WireEditOp = {op: "replace", transaction: {} as never};
        expect(mergeOps([RENAME, REVIEWED], [replace, REVIEWED])).toEqual([replace, REVIEWED]);
    });

    it("keeps an earlier replace first and folds later surgical ops after it", () => {
        const replace: WireEditOp = {op: "replace", transaction: {} as never};
        expect(mergeOps([replace, RENAME], [{op: "setDescription", description: "Again"}])).toEqual([replace, {op: "setDescription", description: "Again"}]);
    });

    it("a tag removed after it was set is removed", () => {
        expect(mergeOps([REVIEWED], [{op: "setTxnTag", name: "reviewed", value: null}])).toEqual([{op: "setTxnTag", name: "reviewed", value: null}]);
    });
});

describe("UNIT reviewModel — replayOverrides (D25)", () => {
    it("keeps an override whose text is unchanged", () => {
        const item = staged(2, "fp-a");
        const report = replayOverrides([override(item, [RECATEGORISE, REVIEWED])], [staged(0, "x"), staged(1, "y"), staged(2, "fp-a")]);
        expect(report.kept).toHaveLength(1);
        expect(report.kept[0]?.ops).toEqual([RECATEGORISE, REVIEWED]);
        expect(report.dropped).toEqual([]);
        expect(report.flagged).toEqual([]);
    });

    it("re-keys one whose unchanged text moved to another ordinal", () => {
        const item = staged(2, "fp-a");
        const report = replayOverrides([override(item, [REVIEWED])], [staged(0, "new-row"), staged(1, "y"), staged(2, "z"), staged(3, "fp-a")]);
        expect(report.kept[0]?.ref).toEqual({kind: "csv", ordinal: 3});
        expect(report.flagged).toEqual([]);
    });

    it("drops one a rule now covers: every op is a no-op on the regenerated text", () => {
        const item = staged(0, "fp-a");
        const ruled = staged(0, "fp-b", (txn) => {
            txn.postings[0]!.account = "expenses:home:supplies";
        });
        const report = replayOverrides([override(item, [RECATEGORISE])], [ruled]);
        expect(report.kept).toEqual([]);
        expect(report.dropped).toEqual([{override: override(item, [RECATEGORISE]), reason: "covered"}]);
    });

    it("keeps the rest of an edit the user's own rule half-covered, unflagged and pruned", () => {
        // The common case: recategorised AND marked reviewed, then saved as a
        // rule. The rule now writes the account; the reviewed stamp carries on.
        const item = staged(0, "fp-a");
        const ruled = staged(0, "fp-b", (txn) => {
            txn.postings[0]!.account = "expenses:home:supplies";
        });
        const report = replayOverrides([override(item, [RECATEGORISE, REVIEWED])], [ruled]);
        expect(report.kept).toHaveLength(1);
        expect(report.kept[0]).toMatchObject({fingerprint: "fp-b", ops: [REVIEWED], flagged: false});
        expect(report.kept[0]?.original).toBe(ruled);
        expect(report.flagged).toEqual([]);
    });

    it("keeps and flags one whose text changed in a way the edit did not account for", () => {
        // Another rule renamed it; the user only recategorised a version they
        // never saw with this description.
        const item = staged(0, "fp-a");
        const renamed = staged(0, "fp-b", (txn) => {
            txn.description = "Hardware Store Inc";
        });
        const report = replayOverrides([override(item, [RECATEGORISE])], [renamed]);
        expect(report.kept[0]).toMatchObject({fingerprint: "fp-b", ops: [RECATEGORISE], flagged: true});
        expect(report.flagged).toEqual([report.kept[0]]);
    });

    it("flags a conflict: the rule set a DIFFERENT account than the override", () => {
        const item = staged(0, "fp-a");
        const elsewhere = staged(0, "fp-b", (txn) => {
            txn.postings[0]!.account = "expenses:shopping";
        });
        const report = replayOverrides([override(item, [RECATEGORISE])], [elsewhere]);
        expect(report.kept[0]).toMatchObject({ops: [RECATEGORISE], flagged: true});
    });

    it("keeps a replace whole and flags it when its text changed", () => {
        const replace: WireEditOp = {op: "replace", transaction: {} as never};
        const item = staged(0, "fp-a");
        const report = replayOverrides([override(item, [replace, REVIEWED])], [staged(0, "fp-b")]);
        expect(report.kept[0]).toMatchObject({ops: [replace, REVIEWED], flagged: true});
    });

    it("drops one whose transaction is gone", () => {
        const item = staged(5, "fp-a");
        const report = replayOverrides([override(item, [REVIEWED])], [staged(0, "x")]);
        expect(report.dropped.map((drop) => drop.reason)).toEqual(["gone"]);
    });

    it("lets each fresh transaction be claimed once, unchanged texts first", () => {
        // Override A's text moved to where B's was; B's text changed. A must
        // take its own moved text, and B the one at its place.
        const a = staged(0, "fp-a");
        const b = staged(1, "fp-b");
        const fresh = [staged(0, "fp-new"), staged(1, "fp-a")];
        const report = replayOverrides([override(b, [REVIEWED]), override(a, [REVIEWED])], fresh);
        const byFingerprint = new Map(report.kept.map((kept) => [kept.original.fingerprint, kept.ref]));
        expect(byFingerprint.get("fp-a")).toEqual({kind: "csv", ordinal: 1});
        expect(report.dropped.map((drop) => drop.reason)).toEqual(["gone"]);
    });

    it("replays QuickBooks overrides by Trans #", () => {
        const item = decodeQbStagedTxn(qbStagedTxnWire(txnFixture(1, "Office"), "1042"), "qb");
        const moved = decodeQbStagedTxn(qbStagedTxnWire(txnFixture(2, "Office"), "1042", {fingerprint: "qb-new"}), "qb");
        const report = replayOverrides([override(item, [REVIEWED])], [moved]);
        expect(report.kept[0]).toMatchObject({ref: {kind: "qb", id: "1042"}, fingerprint: "qb-new", flagged: false});
    });
});
