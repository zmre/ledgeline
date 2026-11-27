// The Accounts dialogs: Rename previews every line before it writes (and
// asks again when the name changes), Add starts under its parent and refuses
// a name that's there, and the account dialog saves only what changed.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {accountRow} from "../testing.js";
import AccountDialog from "./AccountDialog.svelte";
import AddAccountDialog from "./AddAccountDialog.svelte";
import RenameDialog from "./RenameDialog.svelte";

const PREVIEW = {
    from: "internal:admin",
    to: "internal:office",
    lines: [
        {file: "time-accounts.journal", line: 16, before: "account internal:admin", after: "account internal:office"},
        {file: "time/pat/2026-11.timedot", line: 7, before: "    internal:admin   15m", after: "    internal:office   15m"},
    ],
    files: ["time-accounts.journal", "time/pat/2026-11.timedot"],
    applied: false,
};

describe("COMPONENT time account dialogs", () => {
    it("Rename previews the lines, then applies", async () => {
        const onPreview = vi.fn().mockResolvedValue(PREVIEW);
        const onApply = vi.fn().mockResolvedValue(null);
        render(RenameDialog, {from: "internal:admin", onPreview, onApply, onCancel: vi.fn()});
        const submit = screen.getByTestId("account-rename-dialog-submit") as HTMLButtonElement;
        expect(submit.disabled).toBe(true);
        await fireEvent.input(screen.getByTestId("account-rename-to"), {target: {value: "internal:office"}});
        expect(submit.textContent?.trim()).toBe("Preview");
        await fireEvent.click(submit);
        expect(onPreview).toHaveBeenCalledWith("internal:office");
        expect((await screen.findByTestId("account-rename-summary")).textContent?.replace(/\s+/g, " ")).toContain("2 lines in 2 files change");
        expect(screen.getAllByTestId("account-rename-line")).toHaveLength(2);
        expect(submit.textContent?.trim()).toBe("Rename 2 lines");
        await fireEvent.click(submit);
        expect(onApply).toHaveBeenCalledWith("internal:office");
    });

    it("Rename asks for a new preview after the name changes, and shows a refusal", async () => {
        const onPreview = vi.fn().mockResolvedValueOnce(PREVIEW).mockResolvedValueOnce("internal:ops is already an account");
        render(RenameDialog, {from: "internal:admin", onPreview, onApply: vi.fn(), onCancel: vi.fn()});
        await fireEvent.input(screen.getByTestId("account-rename-to"), {target: {value: "internal:office"}});
        await fireEvent.click(screen.getByTestId("account-rename-dialog-submit"));
        await screen.findByTestId("account-rename-summary");
        await fireEvent.input(screen.getByTestId("account-rename-to"), {target: {value: "internal:ops"}});
        expect(screen.getByTestId("account-rename-dialog-submit").textContent?.trim()).toBe("Preview");
        await fireEvent.click(screen.getByTestId("account-rename-dialog-submit"));
        expect((await screen.findByTestId("account-rename-dialog-error")).textContent).toContain("already an account");
    });

    it("Add starts under its parent and refuses a name that's there", async () => {
        const onAdd = vi.fn().mockResolvedValue(null);
        render(AddAccountDialog, {parent: "clients", existing: ["clients", "clients:acme"], onAdd, onCancel: vi.fn()});
        const name = screen.getByTestId("account-add-name") as HTMLInputElement;
        expect(name.value).toBe("clients:");
        const submit = screen.getByTestId("account-add-dialog-submit") as HTMLButtonElement;
        expect(submit.disabled).toBe(true);
        await fireEvent.input(name, {target: {value: "clients:acme"}});
        expect(screen.getByTestId("account-add-dialog-error").textContent).toContain("already there");
        await fireEvent.input(name, {target: {value: "clients:contoso"}});
        await fireEvent.click(submit);
        expect(onAdd).toHaveBeenCalledWith("clients:contoso");
    });

    it("the account dialog saves only what changed", async () => {
        const onSave = vi.fn().mockResolvedValue(null);
        render(AccountDialog, {
            row: accountRow({account: "clients:acme:advisory", customer: {value: "Acme Corp", from: "clients:acme", inherited: true}, item: null}),
            customers: [{value: "Acme Corp", label: "Acme Corp"}],
            items: [{value: "ARCH", label: "ARCH", detail: "Architecture"}],
            onSave,
            onCancel: vi.fn(),
        });
        expect((screen.getByTestId("account-dialog-submit") as HTMLButtonElement).disabled).toBe(true);
        await fireEvent.change(screen.getByTestId("account-dialog-billable"), {target: {value: "no"}});
        await fireEvent.click(screen.getByTestId("account-dialog-archived"));
        await fireEvent.click(screen.getByTestId("account-dialog-submit"));
        await waitFor(() =>
            expect(onSave).toHaveBeenCalledWith([
                {tag: "billable", value: "no"},
                {tag: "archived", value: "yes"},
            ])
        );
    });
});
