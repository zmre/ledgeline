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

import {importStore} from "#lib/imports/importStore.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {CAPABILITIES, STAGE, upload} from "#lib/testing/importFixtures.js";
import {fireEvent, render, screen} from "@testing-library/svelte";
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

afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

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
    it("reveals the QuickBooks import's result after Import is pressed", async () => {
        // `#lib/ui/reveal`, armed by QbJournalPanel's own handler: the stage
        // here is offered straight to the store, so no drop armed anything.
        await setup(
            {...CAPABILITIES, formats: [...CAPABILITIES.formats, "xlsx"]},
            {
                "/api/import/stage": {...QB_STAGE, stageId: "qb-stage-2"},
                "/api/import/qb-journal/qb-stage-2": {...QB_PREVIEW, stageId: "qb-stage-2", unmappedAccounts: []},
                "/api/import/qb-journal/commit": new Response(JSON.stringify({error: "the journal moved"}), {status: 500}),
            }
        );
        await importStore.offerFile(upload("Journal.xlsx"));
        // jsdom has no layout: the result's section reports itself below the 768px fold.
        vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
            const top = this.matches('[data-testid="qb-result-reveal"]') ? 2000 : 0;
            return {top, height: 400, bottom: top + 400, left: 0, right: 0, width: 0, x: 0, y: top, toJSON: () => ({})} as DOMRect;
        });
        const scrollBy = vi.fn();
        vi.stubGlobal("scrollBy", scrollBy);

        render(NewTransactionsPanel);
        await vi.waitFor(() => expect((screen.getByTestId("qb-commit") as HTMLButtonElement).disabled).toBe(false));
        await new Promise((resolve) => setTimeout(resolve, 30));
        expect(scrollBy).not.toHaveBeenCalled();

        await fireEvent.click(screen.getByTestId("qb-commit"));
        await vi.waitFor(() => expect(screen.getByTestId("qb-commit-error")).toBeDefined());
        await vi.waitFor(() => expect(scrollBy).toHaveBeenCalled());
    });
});
