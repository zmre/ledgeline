// The confirmation before a step that writes the books: focus on the safe
// choice, Tab kept inside (past the fields it holds), Escape from anywhere,
// and focus handed back on close (TW-9: Tab used to reach the bill grid
// under the backdrop, and closing left focus on <body>).
import {render, screen} from "@testing-library/svelte";
import {createRawSnippet} from "svelte";
import {describe, expect, it, vi} from "vitest";
import ConfirmDialog from "./ConfirmDialog.svelte";

const fields = createRawSnippet(() => ({render: () => `<label>Via <select aria-label="Via"><option>Email</option></select></label>`}));

function mount() {
    const onCancel = vi.fn();
    const view = render(ConfirmDialog, {title: "Send invoice?", confirmLabel: "Send", busy: false, onConfirm: vi.fn(), onCancel, children: fields});
    return {view, onCancel};
}

function tab(shift = false): void {
    document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", {key: "Tab", shiftKey: shift, bubbles: true, cancelable: true}));
}

describe("COMPONENT ConfirmDialog", () => {
    it("opens on Cancel, the safe choice, even after a field", () => {
        mount();
        expect(document.activeElement).toBe(screen.getByRole("button", {name: "Cancel"}));
    });

    it("keeps Tab and Shift+Tab inside, fields included", () => {
        mount();
        const select = screen.getByLabelText("Via", {selector: "select"});
        const confirm = screen.getByTestId("confirm-dialog-confirm");
        confirm.focus();
        tab();
        expect(document.activeElement).toBe(select);
        tab(true);
        expect(document.activeElement).toBe(confirm);
    });

    it("keeps the confirm button off while confirmDisabled, Cancel on", () => {
        render(ConfirmDialog, {title: "Apply?", confirmLabel: "Apply", confirmDisabled: true, onConfirm: vi.fn(), onCancel: vi.fn()});
        expect((screen.getByTestId("confirm-dialog-confirm") as HTMLButtonElement).disabled).toBe(true);
        expect((screen.getByRole("button", {name: "Cancel"}) as HTMLButtonElement).disabled).toBe(false);
    });

    it("cancels on Escape with focus anywhere", () => {
        const {onCancel} = mount();
        (document.activeElement as HTMLElement | null)?.blur();
        document.body.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true}));
        expect(onCancel).toHaveBeenCalledOnce();
    });

    it("hands focus back to what opened it", () => {
        const opener = document.createElement("button");
        document.body.append(opener);
        opener.focus();
        const {view} = mount();
        expect(document.activeElement).not.toBe(opener);
        view.unmount();
        expect(document.activeElement).toBe(opener);
        opener.remove();
    });

    it("keeps focus in the dialog when going busy disables the focused button", async () => {
        const {view} = mount();
        expect(document.activeElement).toBe(screen.getByRole("button", {name: "Cancel"}));
        await view.rerender({busy: true});
        expect(document.activeElement).toBe(screen.getByTestId("confirm-dialog"));
    });

    it("lands on the safe choice when it opened busy and then stops being busy", async () => {
        // A question with no fields, as the Settings page's "Create a settings file?".
        const text = createRawSnippet(() => ({render: () => `<p>These books have no settings file yet. Create it?</p>`}));
        const view = render(ConfirmDialog, {
            title: "Create a settings file?",
            confirmLabel: "Create",
            busy: true,
            onConfirm: vi.fn(),
            onCancel: vi.fn(),
            children: text,
        });
        expect(document.activeElement).toBe(screen.getByTestId("confirm-dialog"));
        await view.rerender({busy: false});
        expect(document.activeElement).toBe(screen.getByRole("button", {name: "Cancel"}));
    });
});
