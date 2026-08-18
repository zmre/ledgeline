// Mounting the transaction table.
//
// This file exists because of a specific failure: the account chips were given a
// measured width, every pure test passed, and in a real browser the column menu
// stopped opening. Nothing in the suite mounted this component — `branchOrder.
// test.ts` greps its SOURCE for "<TransactionTable" — so a component that threw
// on mount, or an action that blew up when `ResizeObserver` actually existed,
// had nothing to fail against.
//
// jsdom cannot lay anything out, so nothing here asserts geometry. What it CAN
// do is run the code that only runs in a browser: jsdom ships no
// `ResizeObserver` at all, which means the production path — construct, observe,
// receive a callback, disconnect — was dead code under test while being the only
// path that runs for the user. A fake supplies it, so the sequence is exercised.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {resetChipMeasurer} from "$lib/components/textWidth";
import {dec} from "$lib/domain/money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction} from "$lib/domain/types";
import {settings} from "$lib/stores/settings.svelte";
import {accountColumn} from "./accountColumn.svelte";
import TransactionTable from "./TransactionTable.svelte";

/** Give jsdom the 2D canvas it lacks, in a monospaced 6px font. Not a layout claim. */
function withMeasurableFont(): void {
    resetChipMeasurer();
    const context = {font: "12px monospace", measureText: (text: string) => ({width: text.length * 6})};
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(context as never);
}

const usdStyle: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: usdStyle});

function txn(index: number, date: ISODate, accounts: [string, string], description: string): Transaction {
    const postings: Posting[] = [
        {account: accounts[0], amounts: [usd(5624)], status: "unmarked", comment: "", tags: []},
        {account: accounts[1], amounts: [usd(-5624)], status: "unmarked", comment: "", tags: []},
    ];
    return {
        index,
        date,
        status: "unmarked",
        description,
        code: "",
        comment: "",
        tags: [],
        postings,
        haystack: [description, ...accounts].join("\n").toLowerCase(),
    };
}

const TXNS = [
    txn(1, "2026-07-03", ["expenses:household:repairs:plumbing", "assets:bank:checking"], "Plumber"),
    txn(2, "2026-07-02", ["expenses:investment:advisory-fees", "assets:morganstanley:pw-roth-ira:cash"], "Advisor"),
];

/** A `ResizeObserver` jsdom does not have, so the browser-only path can run. */
class FakeResizeObserver {
    static instances: FakeResizeObserver[] = [];
    targets: Element[] = [];
    disconnects = 0;
    constructor(private readonly callback: ResizeObserverCallback) {
        FakeResizeObserver.instances.push(this);
    }
    observe(target: Element): void {
        this.targets.push(target);
    }
    unobserve(target: Element): void {
        this.targets = this.targets.filter((element) => element !== target);
    }
    disconnect(): void {
        this.disconnects += 1;
        this.targets = [];
    }
    emit(width: number): void {
        const entry = {target: this.targets[0], contentRect: {width, height: 20}} as unknown as ResizeObserverEntry;
        this.callback([entry], this as unknown as ResizeObserver);
    }
}

/** The observer watching the Accounts header, as opposed to Svelte's own `bind:clientWidth` ones. */
function accountsObserver(): FakeResizeObserver | undefined {
    return FakeResizeObserver.instances.find((observer) => observer.targets.some((target) => target.tagName === "TH" && target.textContent === "Accounts"));
}

let columnsBefore: typeof settings.columns;

beforeEach(() => {
    FakeResizeObserver.instances = [];
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    // jsdom reports every element as 0px wide, and the component reads
    // `bind:clientWidth` to choose between the desktop table and the narrow
    // card layout — so without this the table under test never renders at all
    // and there is no `<th>` to observe. This is a stand-in for a viewport, not
    // a layout claim: no assertion here depends on the number being right.
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => 1024});
    columnsBefore = settings.columns;
    accountColumn.width = 0;
});

afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    resetChipMeasurer();
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
    settings.columns = columnsBefore;
    accountColumn.width = 0;
});

describe("COMPONENT TransactionTable", () => {
    it("mounts and renders its rows without throwing", () => {
        // The blunt one. A component that throws while rendering takes its
        // SIBLINGS down with it — which is how a change to the table's `<th>`
        // stopped the column menu beside it from opening.
        expect(() => render(TransactionTable, {txns: TXNS})).not.toThrow();

        expect(screen.getByText("Plumber")).toBeDefined();
        expect(screen.getByRole("columnheader", {name: "Accounts"})).toBeDefined();
    });

    it("keeps the column menu mounted and operable alongside the table", () => {
        const {container} = render(TransactionTable, {txns: TXNS});

        // The menu's own behaviour is ColumnMenu.svelte.test.ts's job; what is
        // checked HERE is that mounting it beside the table leaves it intact and
        // still driving the store — a sibling that throws takes it down.
        expect(screen.getByTitle("Configure columns")).toBeDefined();
        expect(container.querySelector<HTMLDetailsElement>("details.dropdown")?.open).toBe(false);

        const amount = screen.getByRole("checkbox", {name: "Amount"});
        amount.click();
        expect(settings.columns.amount).toBe(false);
    });

    it("observes the accounts header exactly once, however many rows are alive", () => {
        render(TransactionTable, {txns: TXNS});
        const observer = accountsObserver();

        expect(observer).toBeDefined();
        expect(observer?.targets).toHaveLength(1);
    });

    it("survives a resize callback and publishes the width to the chips", () => {
        render(TransactionTable, {txns: TXNS});
        const observer = accountsObserver();

        expect(() => observer?.emit(447)).not.toThrow();
        expect(accountColumn.width).toBe(447);
    });

    it("disconnects and retracts the width when the table goes away", () => {
        const view = render(TransactionTable, {txns: TXNS});
        const observer = accountsObserver();
        observer?.emit(447);

        view.unmount();

        expect(observer?.disconnects).toBe(1);
        // A stale desktop width would have the card layout's chips fitting
        // themselves to a cell that no longer exists.
        expect(accountColumn.width).toBe(0);
    });

    it("REGRESSION: the measured width reaches the label and lengthens it", async () => {
        // End to end, and the one that proves the feature is alive rather than
        // merely not crashing: observer → shared width → chip rooms → the string
        // in the DOM. Every earlier test of this change stopped at one of the
        // arrows, which is how a build that silently fell back to the character
        // budget looked completely healthy.
        withMeasurableFont();
        render(TransactionTable, {txns: TXNS});

        // Before any width arrives there is nothing to measure against, so the
        // coarse 30-character budget applies and eats two whole segments.
        expect(screen.getByText("ass:mor:pw-roth-ira:cash")).toBeDefined();

        accountsObserver()?.emit(447);
        await tick();

        // With a real cell width the same chip buys `morganstanley` back.
        expect(screen.getByText("ass:morganstanley:pw-roth-ira:cash")).toBeDefined();
        expect(screen.queryByText("ass:mor:pw-roth-ira:cash")).toBeNull();
    });

    it("degrades to the character budget when the engine cannot measure text", async () => {
        // Plain jsdom: `ResizeObserver` is faked but there is still no 2D
        // canvas, which is the same shape as a browser that refuses the font.
        // The proven-live fallback, not a theoretical one.
        render(TransactionTable, {txns: TXNS});

        accountsObserver()?.emit(447);
        await tick();

        expect(screen.getByText("ass:mor:pw-roth-ira:cash")).toBeDefined();
    });

    it("renders when the accounts column is switched off and there is no header to watch", () => {
        settings.columns = {...settings.columns, accounts: false};

        expect(() => render(TransactionTable, {txns: TXNS})).not.toThrow();
        expect(screen.queryByRole("columnheader", {name: "Accounts"})).toBeNull();
    });
});
