// No calendar below the `md` breakpoint (plan 42): a day cell at phone width
// holds a few characters, so the toggle and the `v` key go, and a `cal=1`
// filter (a shared link, a restored URL) shows its month as the list. A
// separate file: the keymap, the filter store and the journal view are module
// singletons, and the measured width is stubbed on the prototype.
//
// jsdom has no layout, so the width is whatever `clientWidth` is stubbed to;
// the section reads it once on mount (the ResizeObserver stub never fires), so
// each test picks its width before rendering.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, ISODate, Transaction} from "#lib/domain/types.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {filters} from "#lib/stores/filters.svelte.js";
import {journalView, resetJournalView} from "#lib/stores/journalView.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {summaryBody, VIEW_GENERATION, wireRow} from "#lib/testing/journalViewFixture.js";
import TransactionTable from "./TransactionTable.svelte";
import {txnModal} from "./edit/modalState.svelte.js";

const USD: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: USD});

function txn(index: number, date: ISODate, description: string): Transaction {
    return {
        index,
        date,
        status: "unmarked",
        description,
        code: "",
        comment: "",
        tags: [],
        postings: [
            {account: "expenses:coffee", amounts: [usd(450)], status: "unmarked", comment: "", tags: []},
            {account: "assets:cash", amounts: [usd(-450)], status: "unmarked", comment: "", tags: []},
        ],
    };
}

const TXNS = [txn(2, "2026-04-10", "Coffee"), txn(1, "2026-04-03", "Bagel")];

async function show(): Promise<void> {
    await connectFakeEngine();
    const rows = TXNS.map((t) => wireRow(t));
    vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
        const url = new URL(String(input));
        const json = (body: unknown) => Promise.resolve(new Response(JSON.stringify(body), {status: 200}));
        if (url.pathname.endsWith("/api/transactions")) return Promise.resolve(new Response("", {status: 405}));
        if (url.pathname.endsWith("/api/journal/rows")) return json({generation: VIEW_GENERATION, total: rows.length, offset: 0, rows});
        if (url.pathname.endsWith("/api/journal/summary")) return json(summaryBody(rows.length));
        return Promise.resolve(new Response("no route", {status: 404}));
    });
    await editing.probe();
    resetJournalView();
    journalView.sync(FAKE_ENGINE, VIEW_GENERATION, {from: null, to: null, accounts: new Set(), query: ""});
    await vi.waitFor(() => {
        if (journalView.total === null) throw new Error("not loaded");
    });
}

function width(px: number): void {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => px});
}

beforeEach(async () => {
    filters.reset();
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {configurable: true, get: () => 600});
    await show();
});

afterEach(() => {
    filters.setCalendar(false);
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
    Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
});

describe("COMPONENT TransactionTable below the md breakpoint", () => {
    it("has no list/calendar toggle and no `v` on a phone-width column", () => {
        // 390px: an iPhone, 24rem at the default 16px root.
        width(390);
        render(TransactionTable);

        expect(screen.queryByTestId("view-calendar")).toBeNull();
        expect(screen.queryByTestId("view-list")).toBeNull();
        expect(keymap.active.map((binding) => binding.keys)).not.toContain("v");
    });

    it("shows a cal=1 filter's month as the list", () => {
        filters.setCalendar(true);
        width(390);
        render(TransactionTable);

        expect(screen.queryByTestId("journal-calendar")).toBeNull();
        // The filter is untouched: the same link opens the calendar on a wide window.
        expect(filters.value.calendar).toBe(true);
        // The list's own keys are live, since the list is what shows.
        expect(keymap.active.map((binding) => binding.keys)).toContain("j");
    });

    it("keeps the toggle, `v` and the calendar at the breakpoint and above", () => {
        filters.setCalendar(true);
        // 48rem exactly: the `md` breakpoint.
        width(768);
        render(TransactionTable);

        expect(screen.getByTestId("journal-calendar")).toBeTruthy();
        expect(screen.getByTestId("view-calendar")).toBeTruthy();
        expect(keymap.active.map((binding) => binding.keys)).toContain("v");
    });

    it("renders each transaction as a two-line card: date · description · amount, then the accounts", async () => {
        width(390);
        render(TransactionTable);

        const cards = await screen.findAllByTestId("journal-card");
        expect(cards).toHaveLength(2);
        expect(screen.queryByRole("table")).toBeNull();
        const first = cards[0];
        expect(first.querySelector("[data-testid=card-date]")?.getAttribute("title")).toBe("2026-04-10");
        expect(first.textContent).toContain("Coffee");
        expect(first.textContent).toContain("$4.50");
    });

    it("opens the whole transaction when a card's description is tapped", async () => {
        width(390);
        render(TransactionTable);

        const [first] = await screen.findAllByTestId("row-description");
        await fireEvent.click(first);
        expect(txnModal.open).toBe(true);
        expect(txnModal.targetIndex).toBe(2);
        txnModal.close();
    });

    it("keeps the table at an iPad's 768pt-wide list", async () => {
        width(768);
        render(TransactionTable);

        await vi.waitFor(() => expect(screen.getByRole("table")).toBeTruthy());
        expect(screen.queryByTestId("journal-card")).toBeNull();
    });
});
