// The combobox's behaviour, minus anything positional — jsdom has no layout
// engine, so where the popup lands is `anchoredPopup.test.ts` (pure) plus
// Playwright. What is testable here is the state machine and the key table.
//
// This component had no test file at all before.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {describe, expect, it} from "vitest";
import AccountInput from "./AccountInput.svelte";

const ACCOUNTS = ["expenses:groceries:costco", "expenses:groceries:whole-foods", "expenses:gas", "assets:bank:checking"];

function mount(props: Record<string, unknown> = {}) {
    const view = render(AccountInput, {props: {accountNames: ACCOUNTS, ...props}});
    return {view, field: screen.getByLabelText("Account") as HTMLInputElement};
}

async function type(field: HTMLInputElement, text: string): Promise<void> {
    await fireEvent.input(field, {target: {value: text}});
    await tick();
}

/** Dispatch a keydown on the field and report whether the component claimed it. */
function key(field: HTMLElement, k: string, init: KeyboardEventInit = {}): boolean {
    const event = new KeyboardEvent("keydown", {key: k, bubbles: true, cancelable: true, ...init});
    field.dispatchEvent(event);
    return event.defaultPrevented;
}

const options = (): string[] => screen.queryAllByRole("option").map((o) => o.textContent?.trim() ?? "");

describe("COMPONENT AccountInput popup", () => {
    it("does not open merely because the field exists", () => {
        // Opening on focus would spray popups down the transaction form as you tab.
        mount();

        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("opens on typing and ranks matches", async () => {
        const {field} = mount();

        await type(field, "ex:gr");

        expect(options()).toEqual(["expenses:groceries:costco", "expenses:groceries:whole-foods"]);
    });

    it("opens on ArrowDown without typing", async () => {
        const {field} = mount();

        key(field, "ArrowDown");
        await tick();

        expect(screen.queryByRole("listbox")).not.toBeNull();
    });

    it("moves the highlight with arrows and wraps", async () => {
        const {field} = mount();
        await type(field, "ex:gr");

        key(field, "ArrowDown");
        await tick();
        expect(screen.getAllByRole("option")[1].getAttribute("aria-selected")).toBe("true");

        key(field, "ArrowDown");
        await tick();
        expect(screen.getAllByRole("option")[0].getAttribute("aria-selected")).toBe("true");
    });

    it("accepts the highlighted option on Enter", async () => {
        const {field} = mount();
        await type(field, "ex:gr");

        key(field, "Enter");
        await tick();

        expect(field.value).toBe("expenses:groceries:costco");
        expect(screen.queryByRole("listbox")).toBeNull();
    });
});

describe("COMPONENT AccountInput Tab completion", () => {
    it("completes to the longest common prefix", async () => {
        const {field} = mount();
        await type(field, "ex:g");

        expect(key(field, "Tab")).toBe(true);
        await tick();

        expect(field.value).toBe("expenses:g");
    });

    it("cycles once there is nothing left to complete, starting at the first candidate", async () => {
        const {field} = mount();
        await type(field, "expenses:groceries:");

        key(field, "Tab");
        await tick();
        expect(field.value).toBe("expenses:groceries:costco");

        key(field, "Tab");
        await tick();
        expect(field.value).toBe("expenses:groceries:whole-foods");
    });

    it("REGRESSION: Tab falls through when there is nothing to complete", async () => {
        // The anti-trap rule. If Tab is claimed unconditionally there is no way
        // out of this field and the transaction popup becomes a keyboard trap.
        const {field} = mount();
        await type(field, "zzzz");

        expect(key(field, "Tab")).toBe(false);
    });

    it("REGRESSION: Shift+Tab is always ordinary focus traversal", async () => {
        // Never a backward cycle — this is the guaranteed way back out.
        const {field} = mount();
        await type(field, "ex:g");

        expect(key(field, "Tab", {shiftKey: true})).toBe(false);
    });
});

describe("COMPONENT AccountInput Escape", () => {
    it("REGRESSION: the first Escape closes only the popup, and does not reach the parent", async () => {
        // THE bug this component had: in the transaction popup it was passed no
        // `onCancel`, so Escape did nothing locally and bubbled to the modal
        // wrapper, which closed and discarded the half-typed transaction.
        let cancelled = 0;
        const {field} = mount({onCancel: () => (cancelled += 1)});
        await type(field, "ex:gr");

        key(field, "Escape");
        await tick();

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(cancelled).toBe(0);
    });

    it("reaches the parent on a second Escape, once the popup is closed", async () => {
        let cancelled = 0;
        const {field} = mount({onCancel: () => (cancelled += 1)});
        await type(field, "ex:gr");

        key(field, "Escape");
        await tick();
        key(field, "Escape");

        expect(cancelled).toBe(1);
    });

    it("claims Escape either way, so an enclosing overlay sees it was handled", async () => {
        // `defaultPrevented` is the cooperation protocol: no `stopPropagation`
        // anywhere in this codebase.
        const {field} = mount();
        await type(field, "ex:gr");

        expect(key(field, "Escape")).toBe(true);
    });
});

describe("COMPONENT AccountInput commit", () => {
    it("commits on Enter when no popup is open", async () => {
        let committed = 0;
        const {field} = mount({onCommit: () => (committed += 1)});
        await type(field, "zzzz");

        key(field, "Enter");

        expect(committed).toBe(1);
    });

    it("commits on blur", async () => {
        let committed = 0;
        const {field} = mount({onCommit: () => (committed += 1)});

        await fireEvent.blur(field);

        expect(committed).toBe(1);
    });

    it("ignores keys while an IME is composing", async () => {
        let committed = 0;
        const {field} = mount({onCommit: () => (committed += 1)});

        key(field, "Enter", {isComposing: true});

        expect(committed).toBe(0);
    });
});
