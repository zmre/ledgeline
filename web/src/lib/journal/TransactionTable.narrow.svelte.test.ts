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

const TAGGED: Transaction = {
    ...txn(3, "2026-04-12", "Train"),
    comment: "trip: munich",
    tags: [["trip", "munich"]],
    postings: [
        {account: "expenses:travel", amounts: [usd(2000)], status: "unmarked", comment: "window seat, receipt: r-1", tags: [["receipt", "r-1"]]},
        {account: "assets:cash", amounts: [usd(-2000)], status: "unmarked", comment: "seat: 12A", tags: [["seat", "12A"]]},
    ],
};

const TXNS = [TAGGED, txn(2, "2026-04-10", "Coffee"), txn(1, "2026-04-03", "Bagel")];

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

    it("renders each transaction as a card: date · description · amount, then the accounts in full", async () => {
        width(390);
        render(TransactionTable);

        const cards = await screen.findAllByTestId("journal-card");
        expect(cards).toHaveLength(3);
        expect(screen.queryByRole("table")).toBeNull();
        const first = cards[1];
        expect(first.querySelector("[data-testid=card-date]")?.getAttribute("title")).toBe("2026-04-10");
        expect(first.textContent).toContain("Coffee");
        expect(first.textContent).toContain("$4.50");
        // Whole names, not last segments: a phone wraps them onto a second line.
        expect(first.querySelector("[data-testid=card-accounts]")?.textContent).toBe("assets:cash → expenses:coffee");
    });

    it("opens a read-only detail sheet when a card is tapped, and Edit there opens the editor", async () => {
        width(390);
        render(TransactionTable);

        const [, first] = await screen.findAllByTestId("journal-card");
        await fireEvent.click(first);
        expect(txnModal.open).toBe(false);
        const sheet = screen.getByTestId("txn-detail");
        expect(screen.getByTestId("detail-description").textContent).toBe("Coffee");
        const postings = screen.getByTestId("detail-postings");
        expect(postings.textContent).toContain("expenses:coffee");
        expect(postings.textContent).toContain("assets:cash");
        expect(postings.textContent).toContain("$-4.50");
        // Two buttons only; the backdrop closes it.
        expect([...sheet.querySelectorAll("[data-testid=detail-actions] button")].map((button) => button.textContent?.trim())).toEqual([
            "Edit",
            "Mark reviewed",
        ]);

        await fireEvent.click(screen.getByTestId("detail-edit"));
        expect(screen.queryByTestId("txn-detail")).toBeNull();
        expect(txnModal.open).toBe(true);
        expect(txnModal.targetIndex).toBe(2);
        txnModal.close();
    });

    it("shows comments without the tags the Tags line already lists", async () => {
        width(390);
        render(TransactionTable);

        const [tagged] = await screen.findAllByTestId("journal-card");
        await fireEvent.click(tagged);
        expect(screen.getByTestId("detail-tags").textContent).toBe("Tags: trip:munich");
        // "; trip: munich" is all tag: no comment line at all.
        expect(screen.queryByTestId("detail-comment")).toBeNull();
        expect(screen.getAllByTestId("detail-posting-comment").map((line) => line.textContent)).toEqual(["window seat"]);
        await fireEvent.click(screen.getByTestId("detail-backdrop"));
    });

    it("opens a card's sheet from its status badge rather than cycling the status", async () => {
        width(390);
        render(TransactionTable);

        const [, first] = await screen.findAllByTestId("journal-card");
        expect(first.querySelector("[aria-label^='Cycle status']")).toBeNull();
        const badge = first.querySelector("[title=unmarked]");
        if (badge === null) throw new Error("no status badge on the card");
        await fireEvent.click(badge);
        expect(screen.getByTestId("txn-detail")).toBeTruthy();
        await fireEvent.click(screen.getByTestId("detail-backdrop"));
    });

    it("opens the same sheet from a card's description button, and a tap outside closes it", async () => {
        width(390);
        render(TransactionTable);

        const [, first] = await screen.findAllByTestId("row-description");
        await fireEvent.click(first);
        expect(screen.getByTestId("txn-detail")).toBeTruthy();
        await fireEvent.click(screen.getByTestId("detail-backdrop"));
        expect(screen.queryByTestId("txn-detail")).toBeNull();
        expect(txnModal.open).toBe(false);
    });

    it("keeps the table at an iPad's 768pt-wide list", async () => {
        width(768);
        render(TransactionTable);

        await vi.waitFor(() => expect(screen.getByRole("table")).toBeTruthy());
        expect(screen.queryByTestId("journal-card")).toBeNull();
    });
});
