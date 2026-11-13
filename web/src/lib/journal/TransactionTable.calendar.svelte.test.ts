// The journal's calendar view (plan 40), mounted: the toggle beside the column
// menu, transactions on their days with rounded amounts and trade pills, the
// "N more" overflow and its day list, the editor on a click, the month keys,
// and the list's own keys standing down. A separate file: the keymap, the
// filter store and the journal view are module singletons.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, ISODate, Transaction} from "#lib/domain/types.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {filters} from "#lib/stores/filters.svelte.js";
import {journalView, resetJournalView} from "#lib/stores/journalView.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {summaryBody, VIEW_GENERATION, wireRow} from "#lib/testing/journalViewFixture.js";
import {txnModal} from "./edit/modalState.svelte";
import TransactionTable from "./TransactionTable.svelte";

const USD: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: USD});

function txn(index: number, date: ISODate, description: string, postings: [string, Amount][]): Transaction {
    return {
        index,
        date,
        status: "unmarked",
        description,
        code: "",
        comment: "",
        tags: [],
        postings: postings.map(([account, amount]) => ({account, amounts: [amount], status: "unmarked", comment: "", tags: []})),
    };
}

const sale: Amount = {
    commodity: "AAPL",
    qty: dec(-120, 0),
    style: {...USD, side: "R", spaced: true, precision: 0},
    cost: {commodity: "$", qty: dec(150, 0), per: true},
};

// Display order: newest first, as the engine serves the view.
const TXNS: Transaction[] = [
    txn(9, "2026-04-20", "Sell AAPL", [
        ["assets:broker:aapl", sale],
        ["assets:broker:cash", usd(1800000)],
    ]),
    ...[8, 7, 6, 5, 4].map((index) =>
        txn(index, "2026-04-10", `Coffee ${index}`, [
            ["expenses:coffee", usd(450)],
            ["assets:cash", usd(-450)],
        ])
    ),
    txn(3, "2026-04-03", "Paycheck with a very long description that cannot fit", [
        ["assets:bank:checking", usd(320049)],
        ["income:salary", usd(-320049)],
    ]),
];

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

/** Every day cell's list area reports `px` tall: jsdom lays nothing out. */
function cellHeight(px: number): void {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {configurable: true, get: () => px});
}

const dayCell = (day: ISODate): HTMLElement => document.querySelector(`[data-day="${day}"]`) as HTMLElement;

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

beforeEach(async () => {
    filters.reset();
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => 1024});
    cellHeight(200);
    await show();
});

afterEach(() => {
    filters.setCalendar(false);
    txnModal.close();
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
    Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
});

/** Mount in the calendar on April 2026. */
async function mountApril(): Promise<void> {
    render(TransactionTable);
    await fireEvent.click(screen.getByTestId("view-calendar"));
    filters.showMonth("2026-04");
    await tick();
}

describe("COMPONENT TransactionTable calendar view", () => {
    it("switches from the list with the toggle beside the column menu, hiding the menu", async () => {
        filters.pin("assets:bank:checking");
        render(TransactionTable);
        expect(screen.getByTitle("Configure columns")).toBeTruthy();

        await fireEvent.click(screen.getByTestId("view-calendar"));

        expect(screen.getByTestId("journal-calendar")).toBeTruthy();
        expect(screen.queryByTitle("Configure columns")).toBeNull();
        expect(filters.value.calendar).toBe(true);
        expect(filters.value.pin).toBeNull();
        expect(screen.getByTestId("view-calendar").getAttribute("aria-pressed")).toBe("true");

        await fireEvent.click(screen.getByTestId("view-list"));
        expect(screen.queryByTestId("journal-calendar")).toBeNull();
    });

    it("puts each transaction on its day: text left, whole-dollar amount right", async () => {
        await mountApril();
        expect(screen.getByTestId("calendar-month").textContent).toBe("April 2026");

        const payday = within(dayCell("2026-04-03")).getByTestId("calendar-item");
        expect(payday.textContent).toContain("Paycheck with a very long description");
        expect(within(payday).getByTestId("calendar-amount").textContent).toBe("$3,200");
        expect(within(payday).getByTestId("calendar-amount").className).toContain("text-success");
        // The description is the part that gives way.
        expect(within(payday).getByText(/Paycheck/).className).toContain("truncate");
    });

    it("pills a sale with the symbol, the shares and the cash", async () => {
        await mountApril();
        const item = within(dayCell("2026-04-20")).getByTestId("calendar-item");
        expect(within(item).getByTestId("calendar-trade").textContent).toBe("SELL");
        expect(item.textContent).toContain("AAPL -120");
        expect(within(item).getByTestId("calendar-amount").textContent).toBe("$18,000");
    });

    it("says how many more do not fit, and lists the whole day on a click", async () => {
        // 16px root, 1.25rem (20px) lines: 60px holds three lines — two items and "3 more".
        cellHeight(60);
        await mountApril();
        const cell = dayCell("2026-04-10");
        expect(within(cell).getAllByTestId("calendar-item")).toHaveLength(2);
        const more = within(cell).getByTestId("calendar-more");
        expect(more.textContent).toBe("3 more");

        await fireEvent.click(more);

        const list = within(cell).getByTestId("calendar-day-list");
        expect(within(list).getAllByTestId("calendar-item")).toHaveLength(5);
        // Oldest first within the day, as the journal reads.
        expect(within(list).getAllByTestId("calendar-item")[0].textContent).toContain("Coffee 4");
    });

    it("opens the editor on the transaction clicked", async () => {
        await mountApril();
        await fireEvent.click(within(dayCell("2026-04-03")).getByTestId("calendar-item"));
        expect(txnModal.open).toBe(true);
        expect(txnModal.mode).toBe("edit");
        expect(txnModal.targetIndex).toBe(3);
    });

    it("moves the filter's month with h, l and t", async () => {
        await mountApril();
        await press("l");
        expect(filters.value).toMatchObject({from: "2026-05-01", to: "2026-05-31"});
        await press("h");
        await press("h");
        expect(filters.value).toMatchObject({from: "2026-03-01", to: "2026-03-31"});
        await press("t");
        expect(filters.value.from?.slice(0, 7)).toBe(new Date().toISOString().slice(0, 7));
    });

    it("stands the list's own keys down, keeping add and the view switch", async () => {
        await mountApril();
        const keys = keymap.active.map((binding) => binding.keys);
        expect(keys).not.toContain("j");
        expect(keys).not.toContain("Enter");
        expect(keys).toContain("n");
        expect(keys).toContain("v");

        await press("v");
        expect(filters.value.calendar).toBe(false);
        expect(keymap.active.map((binding) => binding.keys)).toContain("j");
    });
});
