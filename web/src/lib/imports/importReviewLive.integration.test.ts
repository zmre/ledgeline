// INTEGRATION: reviewing an import before it is written (plan 32 Phase 4)
// against a live engine. Skipped unless LEDGELINE_API_URL is set — `just
// test-integration` sets it, serving fixtures/sample.journal, whose directory
// is the rules discovery root, so `fixtures/rules/simple/checking.csv.rules`
// is the rules file `rules/simple/checking.csv.rules`.
//
// READ-ONLY by construction: a stage lives in the engine's temp space, a dry
// run and a rule preview write nothing, and this file never commits — a commit
// would write a CSV and append to a committed fixture. The writes are pinned in
// Rust (crates/ledgeline-server/tests/import_endpoints.rs) over temp copies.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {LedgelineApi, type ImportRunBody} from "#lib/api/native.js";
import {decodeRulePreview} from "#lib/api/importReviewDecode.js";
import {decodeDryRun, decodeStagedFile} from "#lib/api/nativeDecode.js";
import {decodeKeySimilarSet} from "#lib/api/reviewDecode.js";

const apiUrl = process.env.LEDGELINE_API_URL;
const CSV = new URL("../../../../fixtures/rules/simple/checking.csv", import.meta.url);
const RULES_ID = "rules/simple/checking.csv.rules";

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live import review", () => {
    const api = (): LedgelineApi => new LedgelineApi(apiUrl ?? "");

    async function body(): Promise<ImportRunBody> {
        const staged = decodeStagedFile(await api().stageImport("checking.csv", readFileSync(CSV)));
        return {
            stageId: staged.stageId,
            rulesId: RULES_ID,
            csvPath: "rules/simple/checking.csv",
            journalId: "sample.journal",
            balance: null,
            balanceAccount: null,
        };
    }

    it("returns every staged transaction as a card, with its CSV record lined up", async () => {
        const run = decodeDryRun(await api().importDryRun(await body()));
        if (!run.ok) throw new Error(run.stderr);
        expect(run.txns).toHaveLength(5);
        expect(run.records.aligned).toBe(true);
        expect(run.records.rows).toHaveLength(5);
        expect(run.records.fields).toEqual(["date", "description", "amount"]);
        const hardware = run.txns.find((txn) => txn.txn.description === "HARDWARE STORE");
        expect(hardware?.unknown).toBe(true);
        expect(hardware?.key).toBe("hardware store");
        const at = hardware?.ref.kind === "csv" ? hardware.ref.ordinal : -1;
        expect(run.records.rows[at]).toContain("HARDWARE STORE");
        expect(run.overrides).toBe(0);
        expect(run.cliNote).toBeNull();
    });

    it("applies an override to what the preview shows", async () => {
        const request = await body();
        const first = decodeDryRun(await api().importDryRun(request));
        if (!first.ok) throw new Error(first.stderr);
        const hardware = first.txns.find((txn) => txn.txn.description === "HARDWARE STORE");
        if (hardware === undefined || hardware.ref.kind !== "csv" || hardware.fingerprint === null) throw new Error("no HARDWARE STORE card");
        const posting = hardware.txn.postings.filter((p) => p.generated === undefined).findIndex((p) => p.account === "expenses:unknown");
        const edited = decodeDryRun(
            await api().importDryRun({
                ...request,
                overrides: [
                    {
                        ordinal: hardware.ref.ordinal,
                        fingerprint: hardware.fingerprint,
                        ops: [{op: "setPostingAccount", posting, account: "expenses:home:supplies"}],
                    },
                ],
            })
        );
        if (!edited.ok) throw new Error(edited.stderr);
        expect(edited.overrides).toBe(1);
        expect(edited.entries).not.toBe(first.entries);
        expect(edited.entries).toContain("expenses:home:supplies");
        expect(edited.cliNote).not.toBeNull();
        const after = edited.txns[hardware.ref.ordinal];
        expect(after?.overridden).toBe(true);
        expect(after?.unknown).toBe(false);
        // Still keyed by the generated text it was made on.
        expect(after?.fingerprint).toBe(hardware.fingerprint);
    });

    it("previews a rule against this import", async () => {
        const run = await body();
        const preview = decodeRulePreview(
            await api().importRulePreview({
                stageId: run.stageId,
                rulesId: run.rulesId,
                csvPath: run.csvPath,
                journalId: run.journalId,
                rule: {matchers: [{field: null, pattern: "HARDWARE"}], assignments: [{field: "account2", value: "expenses:home:supplies"}]},
            })
        );
        if (!preview.ok) throw new Error(preview.stderr);
        expect(preview.total).toBe(5);
        expect(preview.matched).toBe(1);
        expect(preview.alreadyCategorised).toBe(0);
        expect(preview.changedTotal).toBe(1);
        expect(preview.changed[0]?.after).toContain("expenses:home:supplies");
    });

    it("finds journal transactions similar to a staged one by its key", async () => {
        const set = decodeKeySimilarSet(await api().reviewSimilarKey("hardware store", "2026-07-08"));
        expect(set.index).toBe(0);
        expect(set.key).toBe("hardware store");
    });
});
