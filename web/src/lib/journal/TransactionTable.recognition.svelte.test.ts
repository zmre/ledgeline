// The journal's recognition group rows (plan 45 D8): a month of posted
// recognition entries is ONE row ("Recognition · Nov 2026 · 2 entries ·
// $1,833.33"); ▸, `l` and Enter expand it, which asks the engine for the month
// again (`expand=`) and shows its entries under it, indented; `l` on one of
// them collapses the month and hands the cursor back to the group row. A
// separate file: the keymap, the journal view and the expansion are module
// singletons.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {Transaction} from "#lib/domain/types.js";
import {keymap} from "#ui/keys/keymap.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {journalView, resetJournalView} from "#lib/stores/journalView.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, FAKE_JOURNAL_INFO} from "#lib/testing/fakeEngine.js";
import {summaryBody, VIEW_GENERATION, wireRow} from "#lib/testing/journalViewFixture.js";
import {txnModal} from "./edit/modalState.svelte";
import {recognitionExpand, resetRecognitionExpand} from "./recognitionExpand.svelte";
import TransactionTable from "./TransactionTable.svelte";

const STYLE = {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]] as [string, number[]]};
const usd = (cents: number) => ({commodity: "$", qty: {m: BigInt(cents), p: 2}, style: STYLE});

function entry(index: number, date: string, cents: number): Transaction {
    return {
        index,
        date,
        status: "unmarked",
        description: "Customer X Recognized",
        code: "",
        comment: `recognition: cx1217, recognized: ${date}`,
        tags: [
            ["recognition", "cx1217"],
            ["recognized", date],
        ],
        postings: [
            {account: "liabilities:deferred-revenue", amounts: [usd(cents)], status: "unmarked", comment: "", tags: []},
            {account: "revenue:sales", amounts: [usd(-cents)], status: "unmarked", comment: "", tags: []},
        ],
    };
}

const LATE = entry(20, "2026-11-17", 100000);
const EARLY = entry(19, "2026-11-03", 83333);
const COFFEE: Transaction = {
    index: 5,
    date: "2026-10-30",
    status: "cleared",
    description: "Coffee",
    code: "",
    comment: "",
    tags: [],
    postings: [
        {account: "expenses:coffee", amounts: [usd(450)], status: "unmarked", comment: "", tags: []},
        {account: "assets:checking", amounts: [usd(-450)], status: "unmarked", comment: "", tags: []},
    ],
};

const group = (expanded: boolean) => ({month: "2026-11", count: 2, totals: [{commodity: "$", quantity: {m: "183333", p: 2}}], expanded});
const COLLAPSED = [{...wireRow(LATE), recognitionGroup: group(false)}, wireRow(COFFEE)];
const EXPANDED = [{...wireRow(LATE), recognitionGroup: group(true)}, wireRow(LATE), wireRow(EARLY), wireRow(COFFEE)];

let rowsRequests: URL[] = [];

async function show(): Promise<void> {
    await connectFakeEngine();
    rowsRequests = [];
    vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
        const url = new URL(String(input));
        const json = (body: unknown, status = 200) => Promise.resolve(new Response(JSON.stringify(body), {status}));
        const path = url.pathname;
        if (path.endsWith("/api/transactions")) return Promise.resolve(new Response("", {status: 405}));
        if (path.endsWith("/api/journal")) return json(FAKE_JOURNAL_INFO);
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        const rows = url.searchParams.get("expand") === "2026-11" ? EXPANDED : COLLAPSED;
        if (path.endsWith("/api/journal/rows")) {
            rowsRequests.push(url);
            return json({generation: VIEW_GENERATION, total: rows.length, offset: 0, rows});
        }
        if (path.endsWith("/api/journal/summary")) return json(summaryBody(rows.length));
        if (path.endsWith("/api/matches")) return json({generation: VIEW_GENERATION, clearing: true, counts: {clear: 0, check: 0}, older: 0, proposals: []});
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    await editing.probe();
    await journal.refresh({force: true});
    resetJournalView();
    sync();
    await vi.waitFor(() => {
        if (journalView.total === null || journalView.rowAt(0) === null) throw new Error("not loaded");
    });
}

/** What the journal page does on every tick: the view follows the expanded months. */
function sync(): void {
    journalView.sync(FAKE_ENGINE, VIEW_GENERATION, {from: null, to: null, accounts: new Set(), query: ""}, null, {
        mode: "group",
        expand: recognitionExpand.months,
    });
}

/** Re-sync after a toggle and wait for the rows that answer it. */
async function follow(total: number): Promise<void> {
    sync();
    await vi.waitFor(() => expect(journalView.total).toBe(total));
    journalView.ensure(0, total);
    await vi.waitFor(() => expect(journalView.rowAt(total - 1)).not.toBeNull());
    await tick();
}

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

const rowTexts = (): string[] => [...document.querySelectorAll("tbody tr[data-txn]")].map((row) => (row.textContent ?? "").replace(/\s+/g, " ").trim());

beforeEach(() => {
    resetRecognitionExpand();
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => 1024});
});
afterEach(() => {
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
    keymap.reset();
    resetRecognitionExpand();
    txnModal.close();
});

describe("COMPONENT TransactionTable recognition groups (plan 45 D8)", () => {
    it("shows a month of entries as one row with its count and total", async () => {
        await show();
        render(TransactionTable);
        await vi.waitFor(() => expect(screen.getByTestId("recognition-group-row")).toBeTruthy());
        const row = screen.getByTestId("recognition-group-row");
        expect(row.textContent).toContain("Recognition · Nov 2026 · 2 entries");
        expect(screen.getByTestId("recognition-group-total").textContent).toContain("$1,833.33");
        expect(rowTexts()).toHaveLength(2);
        // Grouped is the engine's default: nothing extra on the wire.
        expect(rowsRequests[0]?.searchParams.has("expand")).toBe(false);
    });

    it("expands from ▸: the month is asked for again and its entries show under the row, indented", async () => {
        await show();
        render(TransactionTable);
        await vi.waitFor(() => expect(screen.getByTestId("recognition-toggle")).toBeTruthy());
        await fireEvent.click(screen.getByTestId("recognition-toggle"));
        expect(recognitionExpand.months).toEqual(["2026-11"]);
        await follow(EXPANDED.length);
        expect(rowsRequests.at(-1)?.searchParams.get("expand")).toBe("2026-11");
        const rows = [...document.querySelectorAll("tbody tr[data-txn]")];
        expect(rows).toHaveLength(4);
        expect(screen.getByTestId("recognition-toggle").getAttribute("aria-expanded")).toBe("true");
        // The two entries are tinted like a transfer's halves; the coffee is not.
        expect(rows[1]?.className).toContain("bg-base-200/40");
        expect(rows[2]?.className).toContain("bg-base-200/40");
        expect(rows[3]?.className).not.toContain("bg-base-200/40");
    });

    it("toggles from the keyboard: Enter and l on the group row, l on an entry collapses to the row", async () => {
        await show();
        render(TransactionTable);
        await vi.waitFor(() => expect(screen.getByTestId("recognition-group-row")).toBeTruthy());
        await press("j");
        expect(screen.getByTestId("recognition-group-row").getAttribute("aria-current")).toBe("true");
        // Enter opens the month, not an editor on its latest entry.
        await press("Enter");
        expect(txnModal.open).toBe(false);
        expect(recognitionExpand.months).toEqual(["2026-11"]);
        await follow(EXPANDED.length);
        // Down onto the first entry; `l` there collapses the month and the cursor goes back to the group row.
        await press("j");
        await press("l");
        expect(recognitionExpand.months).toEqual([]);
        expect(screen.getByTestId("recognition-group-row").getAttribute("aria-current")).toBe("true");
        await follow(COLLAPSED.length);
        await press("l");
        expect(recognitionExpand.months).toEqual(["2026-11"]);
    });
});
