// The format branch (WP-17 Phase C): `POST /api/import/stage` already decides,
// server-side, whether an upload is a QuickBooks Journal export — the ONLY
// thing this screen may do with that fact is read `staged.format` and swap
// panels. No new heuristics, no confidence UI, no confirmation step (the
// plan's Phase C contract, restated in `qbJournalModel.isQuickbooksJournalStage`'s
// own doc comment).
//
// Its own file, for the same reason `NewTransactionsPanel.staged.svelte.test.ts`
// is: `importStore` is a module singleton, and vitest gives each test FILE its
// own module registry — the cheapest honest way to stage a file without
// leaking it into every other New Transactions test.

import {importStore} from "$lib/imports/importStore.svelte";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {CAPABILITIES, STAGE, upload} from "$lib/testing/importFixtures";
import {render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import NewTransactionsPanel from "./NewTransactionsPanel.svelte";

/** `POST /api/import/stage` on a `Journal.xlsx` drop — `import_api::stage_qb_journal`'s shape. */
const QB_STAGE = {
    stageId: "qb-stage-1",
    format: "quickbooks-journal",
    preview: {header: null, rows: [], rowCount: 0, truncated: false},
    statement: null,
    notes: [],
    candidates: [],
    defaults: {csvPath: "", journalId: "2026/2026.journal"},
};

/** `GET /api/import/qb-journal/{stageId}` — `QbJournalPanel`'s own mount fetch. */
const QB_PREVIEW = {
    stageId: "qb-stage-1",
    transactionCount: 2,
    postingCount: 4,
    dateFormat: {format: "%m/%d/%Y", ambiguous: false},
    unmappedAccounts: ["3000 Member Equity"],
    sample: [],
    idMatches: null,
};

async function setup(capabilities: unknown, table: Record<string, unknown>): Promise<void> {
    await connectFakeEngine({"/api/import/capabilities": capabilities, ...table});
    await importStore.reloadCapabilities(FAKE_ENGINE);
}

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT NewTransactionsPanel — the QuickBooks Journal format branch", () => {
    it("shows the QuickBooks panel, not the ordinary rules-candidate flow, when the stage says so", async () => {
        // A real engine's `formats` list includes `xlsx` (QuickBooks' own
        // export extension) — the fixture's default list is trimmed to
        // `csv`/`ofx` for the ordinary tests, so it is widened here rather
        // than having the client-side extension guard (`importModel.refuseFile`)
        // refuse the upload before it ever reaches the server's own
        // `qb_journal::detect`.
        await setup(
            {...CAPABILITIES, formats: [...CAPABILITIES.formats, "xlsx"]},
            {
                "/api/import/stage": QB_STAGE,
                "/api/import/qb-journal/qb-stage-1": QB_PREVIEW,
            }
        );
        await importStore.offerFile(upload("Journal.xlsx"));

        render(NewTransactionsPanel);

        await vi.waitFor(() => expect(screen.getByTestId("qb-journal-panel")).toBeTruthy());
        // The ordinary CSV flow's own sections never mounted for this upload.
        expect(screen.queryByTestId("imports-destinations")).toBeNull();
        expect(screen.queryByTestId("imports-candidates")).toBeNull();
        expect(screen.queryByTestId("imports-balance")).toBeNull();
    });

    it("leaves an ordinary CSV upload on the unchanged rules-candidate flow", async () => {
        await setup(CAPABILITIES, {"/api/import/stage": STAGE});
        await importStore.offerFile(upload("bank.csv"));

        render(NewTransactionsPanel);

        expect(screen.getByTestId("imports-destinations")).toBeDefined();
        expect(screen.getByTestId("imports-candidates")).toBeDefined();
        expect(screen.queryByTestId("qb-journal-panel")).toBeNull();
    });
});
