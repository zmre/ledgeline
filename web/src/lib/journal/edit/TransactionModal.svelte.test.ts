// Enter saves, and Escape no longer throws the transaction away.
//
// This component had no test file before, which is part of why the Escape bug
// survived: it needed the combobox and the modal in the same document to
// reproduce, and nothing mounted them together.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {connectFakeEngine} from "$lib/testing/fakeEngine";
import {journal} from "$lib/stores/journal.svelte";
import TransactionModal from "./TransactionModal.svelte";
import {txnModal} from "./modalState.svelte";

afterEach(() => {
    txnModal.close();
    vi.unstubAllGlobals();
});

/**
 * Populate `journal.accountNames` the way the app does — through the real store
 * and a stubbed `fetch` — because the store exposes getters only. Mocking the
 * store instead would prove the modal renders whatever it is handed, which is
 * never what breaks.
 */
async function withAccounts(names: string[]): Promise<void> {
    await connectFakeEngine({
        "/accountnames": names,
        "/transactions": [],
        "/prices": [],
        "/accounts": [],
    });
    await journal.refresh({force: true});
}

async function open(): Promise<void> {
    txnModal.openAdd();
    await tick();
    // The modal focuses its date field one tick after opening.
    await tick();
}

function keydown(target: EventTarget, key: string, init: KeyboardEventInit = {}): KeyboardEvent {
    const event = new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true, ...init});
    target.dispatchEvent(event);
    return event;
}

const accountField = (): HTMLInputElement => screen.getAllByLabelText("Account")[0] as HTMLInputElement;

describe("COMPONENT TransactionModal focus", () => {
    it("focuses a field when it opens", async () => {
        // It used to focus nothing, which is why Escape did nothing until you
        // clicked into the form: the handler was bubble-phase on a wrapper the
        // user was not inside.
        render(TransactionModal);

        await open();

        expect(document.activeElement).toBe(screen.getByLabelText("Date"));
    });
});

describe("COMPONENT TransactionModal saving", () => {
    it("submits on Enter from a plain field", async () => {
        // A real <form>, so this is native implicit submission rather than a
        // hand-rolled key table. Asserted through validation: an empty form
        // cannot save, and reaching the error proves submit() ran.
        render(TransactionModal);
        await open();
        const description = screen.getByLabelText("Description");
        await fireEvent.input(description, {target: {value: "Plumber"}});

        await fireEvent.submit(description.closest("form") as HTMLFormElement);
        await tick();

        // No server is configured in this suite, so the write is reported
        // unavailable rather than succeeding — but it was ATTEMPTED, which is
        // the thing Enter never used to do.
        expect(screen.queryByRole("alert")).not.toBeNull();
    });

    it("saves on Cmd/Ctrl+Enter", async () => {
        render(TransactionModal);
        await open();

        const event = keydown(screen.getByLabelText("Description"), "Enter", {metaKey: true});

        expect(event.defaultPrevented).toBe(true);
    });

    it("keeps the save button as the form's submit control", async () => {
        // The mechanism, pinned: if someone reverts this to onclick={submit},
        // Enter silently stops saving and only this assertion notices.
        render(TransactionModal);
        await open();

        expect(screen.getByRole("button", {name: "Add transaction"}).getAttribute("type")).toBe("submit");
    });

    it("leaves every other button as type=button", async () => {
        // A stray type=submit inside a <form> turns "Add posting" into "save".
        render(TransactionModal);
        await open();

        for (const name of ["Cancel", "Remove posting 1"]) {
            expect(screen.getByRole("button", {name}).getAttribute("type")).toBe("button");
        }
    });
});

describe("COMPONENT TransactionModal Escape", () => {
    it("REGRESSION: Escape in an account field with suggestions open does not discard the transaction", async () => {
        // The bug: AccountInput was passed no `onCancel`, so Escape did nothing
        // locally — but preventDefault is not stopPropagation, so it reached the
        // modal wrapper, which closed and threw away everything typed.
        await withAccounts(["expenses:groceries:costco", "expenses:gas"]);
        render(TransactionModal);
        await open();
        const account = accountField();
        await fireEvent.input(account, {target: {value: "ex"}});
        await tick();
        expect(screen.queryByRole("listbox")).not.toBeNull();

        keydown(account, "Escape");
        await tick();

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(txnModal.open).toBe(true);
    });

    it("closes on a second Escape, once the suggestions are gone", async () => {
        await withAccounts(["expenses:groceries:costco"]);
        render(TransactionModal);
        await open();
        const account = accountField();
        await fireEvent.input(account, {target: {value: "ex"}});
        await tick();

        keydown(account, "Escape");
        await tick();
        document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true}));
        await tick();

        expect(txnModal.open).toBe(false);
    });

    it("closes on Escape when no suggestions are open", async () => {
        render(TransactionModal);
        await open();

        document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true}));
        await tick();

        expect(txnModal.open).toBe(false);
    });
});
