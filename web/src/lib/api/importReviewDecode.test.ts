// The import-review wire (plan 32 Phase 4): staged transactions, the raw
// records behind them, the QuickBooks preview's staged transactions and the
// rule builder's live check — decoded from literal engine JSON.

import {describe, expect, it} from "vitest";
import {qbStagedTxnWire, stagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {decodeCsvStagedTxn, decodeQbStagedTxn, decodeRulePreview, decodeStagedRecords} from "./importReviewDecode";
import {decodeCommitResult, decodeDryRun, decodeQbPreview} from "./nativeDecode";
import {decodeKeySimilarSet, decodeSimilarSet} from "./reviewDecode";

const DRY_RUN = {
    ok: true,
    entries: "2024-01-19 HARDWARE STORE\n    assets:bank:checking  $-62.13\n    expenses:unknown  $62.13\n\n",
    count: 1,
    status: "would import 1 new transaction",
    skipped: null,
    balance: null,
    aliases: null,
    cliCommand: "ledgeline import",
    idMatches: null,
    txns: [stagedTxnWire(txnFixture(1, "HARDWARE STORE"), {key: "hardware store"})],
    records: {
        aligned: true,
        header: ["Date", "Description", "Amount"],
        fields: ["date", "description", "amount"],
        rows: [["01/19/2024", "HARDWARE STORE", "-62.13"]],
    },
    overrides: 0,
    cliNote: null,
};

describe("UNIT importReviewDecode — the dry run's staged transactions", () => {
    it("decodes each as a review card keyed by its ordinal, with the records lined up", () => {
        const run = decodeDryRun(DRY_RUN);
        if (!run.ok) throw new Error("expected ok");
        expect(run.txns).toHaveLength(1);
        const [txn] = run.txns;
        expect(txn?.ref).toEqual({kind: "csv", ordinal: 0});
        expect(txn?.txn.index).toBe(1);
        expect(txn?.fingerprint).toBe("gen-1");
        expect(txn?.key).toBe("hardware store");
        expect(txn?.unknown).toBe(true);
        expect(txn?.overridden).toBe(false);
        expect(txn?.classification).toBeNull();
        expect(run.records.aligned).toBe(true);
        expect(run.records.rows[0]).toEqual(["01/19/2024", "HARDWARE STORE", "-62.13"]);
        expect(run.records.fields).toEqual(["date", "description", "amount"]);
        expect(run.overrides).toBe(0);
        expect(run.cliNote).toBeNull();
        expect(Object.isFrozen(txn)).toBe(true);
    });

    it("reads an engine that predates review as no cards and nothing aligned", () => {
        const older = Object.fromEntries(Object.entries(DRY_RUN).filter(([field]) => !["txns", "records", "overrides", "cliNote"].includes(field)));
        const run = decodeDryRun(older);
        if (!run.ok) throw new Error("expected ok");
        expect(run.txns).toEqual([]);
        expect(run.records).toEqual({aligned: false, header: null, fields: null, rows: []});
        expect(run.overrides).toBe(0);
    });

    it("carries the override count and the command-line note", () => {
        const run = decodeDryRun({...DRY_RUN, overrides: 2, cliNote: "plus 2 reviewed edits"});
        expect(run.ok && run.overrides).toBe(2);
        expect(run.ok && run.cliNote).toBe("plus 2 reviewed edits");
    });

    it("refuses a staged transaction without a fingerprint or an ordinal", () => {
        expect(() => decodeCsvStagedTxn({...stagedTxnWire(txnFixture(1, "X")), fingerprint: null}, "t")).toThrow(/fingerprint/);
        const noOrdinal = {...stagedTxnWire(txnFixture(1, "X")), ordinal: undefined};
        expect(() => decodeCsvStagedTxn(noOrdinal, "t")).toThrow(/ordinal/);
    });

    it("refuses aligned records that do not line up one per transaction", () => {
        expect(() => decodeStagedRecords({aligned: true, header: null, fields: null, rows: []}, "r", 1)).toThrow(/one per transaction/);
        expect(() => decodeStagedRecords({aligned: false, header: null, fields: null, rows: [["a"]]}, "r", 1)).toThrow(/no rows/);
        expect(decodeStagedRecords({aligned: false, header: ["A"], fields: null, rows: []}, "r", 3).header).toEqual(["A"]);
    });

    it("reads the commit's review counts, absent as zero", () => {
        const base = {csvWritten: "import/bank.csv", journalWritten: "main.journal", imported: 1, ordering: {inOrder: true, moves: []}};
        expect(decodeCommitResult(base)).toMatchObject({overrides: 0, journalEdited: 0});
        expect(decodeCommitResult({...base, overrides: 2, journalEdited: 3})).toMatchObject({overrides: 2, journalEdited: 3});
    });
});

describe("UNIT importReviewDecode — QuickBooks staged transactions", () => {
    it("keys them by Trans # and keeps the classification", () => {
        const txn = decodeQbStagedTxn(qbStagedTxnWire(txnFixture(1, "Office"), "1042", {classification: "changed"}), "t");
        expect(txn.ref).toEqual({kind: "qb", id: "1042"});
        expect(txn.classification).toBe("changed");
        expect(() => decodeQbStagedTxn(qbStagedTxnWire(txnFixture(1, "Office"), "1", {classification: "maybe"}), "t")).toThrow(/classification/);
    });

    it("rides on the preview", () => {
        const preview = decodeQbPreview({
            stageId: "s1",
            transactionCount: 1,
            postingCount: 2,
            dateFormat: {format: "%m/%d/%Y", ambiguous: false},
            unmappedAccounts: [],
            sample: [],
            idMatches: null,
            txns: [qbStagedTxnWire(txnFixture(1, "Office"), "1042")],
        });
        expect(preview.txns.map((txn) => txn.ref)).toEqual([{kind: "qb", id: "1042"}]);
    });
});

describe("UNIT importReviewDecode — the rule builder's live check", () => {
    it("decodes the counts and the changed rows", () => {
        const preview = decodeRulePreview({
            ok: true,
            matched: 7,
            total: 112,
            alreadyCategorised: 3,
            changed: [{ordinal: 4, before: "a", after: "b"}],
            changedTotal: 4,
        });
        expect(preview).toEqual({ok: true, matched: 7, total: 112, alreadyCategorised: 3, changed: [{ordinal: 4, before: "a", after: "b"}], changedTotal: 4});
    });

    it("carries hledger's refusal verbatim", () => {
        expect(decodeRulePreview({ok: false, stderr: "bad regex"})).toEqual({ok: false, stderr: "bad regex"});
    });

    it("refuses a body missing its counts", () => {
        expect(() => decodeRulePreview({ok: true, matched: 1})).toThrow();
    });
});

describe("UNIT reviewDecode — similar by key", () => {
    const body = {generation: "g1", index: 0, key: "hardware store", total: 0, candidates: [], inQueue: [], precedent: null};

    it("accepts the keyed route's index 0 — and only there", () => {
        expect(decodeKeySimilarSet(body).key).toBe("hardware store");
        expect(() => decodeSimilarSet(body)).toThrow(/index/);
        expect(() => decodeKeySimilarSet({...body, index: 3})).toThrow(/expected 0/);
    });
});
