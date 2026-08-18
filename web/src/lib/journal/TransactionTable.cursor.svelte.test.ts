// The journal's keyboard cursor, at the level jsdom can actually see: which row
// carries `aria-current`, and what the edit keys dispatch.
//
// It CANNOT see scrolling — jsdom has no layout engine, so `scrollTop` is always
// 0. That half is covered by `rowModel.test.ts` (the arithmetic) and by
// Playwright's `toBeInViewport` (the reveal).
//
// A separate file from `TransactionTable.svelte.test.ts` because module-level
// runes state is shared per FILE, and this one drives the keymap and the
// row-action store.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {dec} from "$lib/domain/money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction} from "$lib/domain/types";
import {keymap} from "$lib/keys/keymap.svelte";
import TransactionTable from "./TransactionTable.svelte";
import {rowActions} from "./rowAction.svelte";

const style: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style});

function txn(index: number, date: ISODate, description: string): Transaction {
    const postings: Posting[] = [
        {account: "expenses:misc", amounts: [usd(1000)], status: "unmarked", comment: "", tags: []},
        {account: "assets:bank:checking", amounts: [usd(-1000)], status: "unmarked", comment: "", tags: []},
    ];
    return {index, date, status: "unmarked", code: "", description, comment: "", tags: [], postings, haystack: description.toLowerCase()};
}

const TXNS: Transaction[] = [txn(1, "2026-07-03", "Plumber"), txn(2, "2026-07-02", "Coffee"), txn(3, "2026-07-01", "Rent")];

/** jsdom has no ResizeObserver, and the accounts column header registers one. */
class FakeResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
}

beforeEach(() => {
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    // Without a width the component picks the narrow card layout and there are
    // no <tr>s to assert on. A stand-in for a viewport, not a layout claim.
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => 1024});
});

afterEach(() => {
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
    keymap.reset();
    rowActions.reset();
});

/** Press a key the way the window listener would see it. */
async function press(key: string, init: KeyboardEventInit = {}): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true, ...init}));
    await tick();
}

const cursored = (): string | null => document.querySelector("[aria-current='true']")?.querySelector("td:nth-child(3)")?.textContent?.trim() ?? null;

describe("COMPONENT TransactionTable cursor", () => {
    it("has no cursor until a key moves it", () => {
        render(TransactionTable, {txns: TXNS});

        expect(document.querySelector("[aria-current='true']")).toBeNull();
    });

    it("lands on the first row and then advances with j", async () => {
        render(TransactionTable, {txns: TXNS});

        await press("j");
        expect(cursored()).toBe("Plumber");

        await press("j");
        expect(cursored()).toBe("Coffee");
    });

    it("moves back with k and stops at the top", async () => {
        render(TransactionTable, {txns: TXNS});
        await press("j");
        await press("j");

        await press("k");
        expect(cursored()).toBe("Plumber");

        await press("k");
        expect(cursored()).toBe("Plumber");
    });

    it("answers the arrow keys as well as j and k", async () => {
        render(TransactionTable, {txns: TXNS});

        await press("ArrowDown");
        await press("ArrowDown");

        expect(cursored()).toBe("Coffee");
    });

    it("jumps to the ends with gg and G", async () => {
        render(TransactionTable, {txns: TXNS});

        await press("G");
        expect(cursored()).toBe("Rent");

        await press("g");
        await press("g");
        expect(cursored()).toBe("Plumber");
    });

    it("moves by a half page on ctrl-d and back on ctrl-u", async () => {
        render(TransactionTable, {txns: TXNS});

        await press("d", {ctrlKey: true});
        const afterDown = cursored();
        await press("u", {ctrlKey: true});

        expect(afterDown).not.toBeNull();
        expect(cursored()).toBe("Plumber");
    });

    it("clears on Escape", async () => {
        render(TransactionTable, {txns: TXNS});
        await press("j");

        await press("Escape");

        expect(document.querySelector("[aria-current='true']")).toBeNull();
    });

    it("marks the cursored row with aria-current and nothing else", async () => {
        render(TransactionTable, {txns: TXNS});

        await press("j");

        expect(document.querySelectorAll("[aria-current='true']")).toHaveLength(1);
    });

    it("announces the cursored row, since there is no focus to carry it", async () => {
        render(TransactionTable, {txns: TXNS});

        await press("j");

        expect(screen.getByText("Row 1 of 3. 2026-07-03, Plumber.")).toBeDefined();
    });

    it("unregisters its keys when it goes away", async () => {
        const view = render(TransactionTable, {txns: TXNS});

        view.unmount();
        await tick();

        expect(keymap.active.filter((binding) => binding.keys === "j")).toHaveLength(0);
    });
});

describe("COMPONENT TransactionTable edit keys", () => {
    // `editing.canEdit` is false with no server configured, so the edit keys are
    // registered-but-disabled — which is itself the thing to assert.
    it("does not offer edit keys on a read-only engine", () => {
        render(TransactionTable, {txns: TXNS});

        expect(keymap.active.filter((binding) => ["e", "c", "s", "x"].includes(binding.keys))).toHaveLength(0);
    });

    it("keeps the movement keys available regardless", () => {
        render(TransactionTable, {txns: TXNS});

        expect(keymap.active.map((binding) => binding.keys)).toContain("j");
    });

    it("leaves ctrl-u unclaimed when it has nothing to do", async () => {
        // Ctrl-U is view-source in Chrome. Claiming it when the table is not
        // even mounted would break the browser for nothing.
        const view = render(TransactionTable, {txns: TXNS});
        view.unmount();
        await tick();

        const event = new KeyboardEvent("keydown", {key: "u", ctrlKey: true, cancelable: true});
        keymap.handle(event);

        expect(event.defaultPrevented).toBe(false);
    });
});
