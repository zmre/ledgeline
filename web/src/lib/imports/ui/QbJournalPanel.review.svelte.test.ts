// QuickBooks imports get review too (plan 32 D30): once every account is
// mapped, Review Unknowns and Review All open a staged run over the built
// transactions, with `u`/`r`; its saves are held for the Import and counted
// beside it. No "Remember as a rule": an alias has no payee to write a rule for.
//
// Every test uses its own stage id, as `QbJournalPanel.svelte.test.ts`
// explains: the store fetches once per stage.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {reviewLaunch, resetReviewLaunch} from "#lib/review/ui/launch.svelte.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {qbStagedTxnWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {qbJournalStore} from "../qbJournalStore.svelte";
import QbJournalPanel from "./QbJournalPanel.svelte";

function preview(stageId: string, options: {unmapped?: string[]; txns?: Record<string, unknown>[]} = {}): Record<string, unknown> {
    return {
        stageId,
        transactionCount: 3,
        postingCount: 6,
        dateFormat: {format: "%m/%d/%Y", ambiguous: false},
        unmappedAccounts: options.unmapped ?? [],
        sample: [],
        idMatches: options.unmapped?.length ? null : {new: 3, unchanged: 0, conflicting: [], conflictingTotal: 0},
        txns: options.txns ?? [
            qbStagedTxnWire(txnFixture(1, "OFFICE DEPOT"), "441"),
            qbStagedTxnWire(txnFixture(2, "PAYROLL", "income:salary"), "442", {unknown: false}),
            qbStagedTxnWire(txnFixture(3, "STAPLES"), "443"),
        ],
    };
}

async function mount(stageId: string, body: Record<string, unknown>): Promise<void> {
    await connectFakeEngine({[`/api/import/qb-journal/${stageId}`]: body});
    render(QbJournalPanel, {props: {stageId, accountNames: []}});
    // The store is a singleton: wait for THIS stage's preview, not the last test's.
    await vi.waitFor(() => expect(qbJournalStore.previewView === "data" && qbJournalStore.preview?.stageId === stageId).toBe(true));
    await vi.waitFor(() => expect(screen.getByTestId("qb-journal-panel")).toBeTruthy());
}

const press = (key: string): void => keymap.handle(new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true}));

beforeEach(() => resetReviewLaunch());
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
});

describe("COMPONENT QbJournalPanel — review (D30)", () => {
    it("offers Review Unknowns and Review All with their counts once every account is mapped", async () => {
        await mount("qb-review-1", preview("qb-review-1"));
        expect(screen.getByTestId("imports-review-unknowns-count").textContent).toBe("2");
        expect(screen.getByTestId("imports-review-all-count").textContent).toBe("3");
        expect(screen.getByTestId("imports-review").textContent).toContain("Import");
    });

    it("offers nothing while an account is unmapped", async () => {
        await mount("qb-review-2", preview("qb-review-2", {unmapped: ["3000 Member Equity"], txns: []}));
        expect(screen.queryByTestId("imports-review")).toBeNull();
        press("u");
        expect(reviewLaunch.staged).toBeNull();
    });

    it("opens a staged run over the built transactions, with no rule builder", async () => {
        await mount("qb-review-3", preview("qb-review-3"));
        await fireEvent.click(screen.getByTestId("imports-review-unknowns"));
        const launch = reviewLaunch.staged;
        expect(launch?.kind).toBe("unknowns");
        expect(launch?.items.map((item) => item.ref)).toEqual([
            {kind: "qb", id: "441"},
            {kind: "qb", id: "443"},
        ]);
        expect(launch?.rules).toBeNull();
        expect(launch?.writeLabel).toBe("Import");
        expect(launch?.sink).toBe(qbJournalStore.reviewSink);
    });

    it("binds u and r", async () => {
        await mount("qb-review-4", preview("qb-review-4"));
        press("r");
        expect(reviewLaunch.staged?.kind).toBe("all");
        expect(reviewLaunch.staged?.items).toHaveLength(3);
    });

    it("counts the held edits beside Import, and stops offering an unknown it already recategorised", async () => {
        await mount("qb-review-5", preview("qb-review-5"));
        const [first] = qbJournalStore.stagedTxns;
        if (first === undefined) throw new Error("expected a transaction");
        qbJournalStore.reviewSink.stage({
            overrides: [{item: first, ops: [{op: "setPostingAccount", posting: 0, account: "expenses:office"}]}],
            journalEdits: [{index: 90, fingerprint: "fp-90", ops: [{op: "setPostingAccount", posting: 0, account: "expenses:office"}]}],
        });
        await vi.waitFor(() => expect(screen.getByTestId("imports-review-edits-count").textContent).toBe("1 reviewed edit · 1 existing transaction edited"));
        expect(screen.getByTestId("imports-review-unknowns-count").textContent).toBe("1");
    });
});
