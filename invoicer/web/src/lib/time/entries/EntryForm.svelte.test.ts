// The add/edit row: what it sends, the note warning, the server's rewrite on
// a refused note, a timeclock session's start/end, and Escape.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {entry} from "../testing.js";
import EntryForm from "./EntryForm.svelte";

const ACCOUNTS = ["clients:acme:advisory", "internal:admin"];

function fill(testid: string, value: string): Promise<boolean> {
    return fireEvent.input(screen.getByTestId(testid), {target: {value}});
}

describe("COMPONENT time EntryForm", () => {
    it("adds a line with today's date and the filter's person", async () => {
        const onAdd = vi.fn().mockResolvedValue({ok: true});
        render(EntryForm, {today: "2026-11-26", accounts: ACCOUNTS, person: "sam", onAdd, onCancel: vi.fn(), testid: "add"});
        expect((screen.getByTestId("add-date") as HTMLInputElement).value).toBe("2026-11-26");
        await fireEvent.input(screen.getByRole("combobox", {name: "Account"}), {target: {value: "internal:admin"}});
        await fill("add-duration", "1h30");
        await fill("add-note", "expense reports");
        await fireEvent.click(screen.getByTestId("add-save"));
        expect(onAdd).toHaveBeenCalledWith({date: "2026-11-26", account: "internal:admin", duration: "1h30", note: "expense reports", person: "sam"});
    });

    it("says what is missing before it asks the server", async () => {
        const onAdd = vi.fn();
        render(EntryForm, {today: "2026-11-26", accounts: ACCOUNTS, onAdd, onCancel: vi.fn(), testid: "add"});
        await fireEvent.click(screen.getByTestId("add-save"));
        expect(screen.getByTestId("add-error").textContent).toContain("Pick an account");
        await fireEvent.input(screen.getByRole("combobox", {name: "Account"}), {target: {value: "internal:admin"}});
        await fill("add-duration", "soon");
        await fireEvent.click(screen.getByTestId("add-save"));
        expect(screen.getByTestId("add-error").textContent).toContain("45m, 1h30, 1.5 or 0:45");
        expect(onAdd).not.toHaveBeenCalled();
    });

    it("warns about a tag in the note, and offers the server's rewrite when it refuses", async () => {
        const onAdd = vi
            .fn()
            .mockResolvedValue({ok: false, code: "note-tag", message: "hledger reads re: as a tag", body: {offending: "re:", suggestion: "re K-1"}});
        render(EntryForm, {today: "2026-11-26", accounts: ACCOUNTS, onAdd, onCancel: vi.fn(), testid: "add"});
        await fireEvent.input(screen.getByRole("combobox", {name: "Account"}), {target: {value: "internal:admin"}});
        await fill("add-duration", "15m");
        await fill("add-note", "re: K-1");
        expect(screen.getByTestId("add-note-warning").textContent).toContain("“re:”");
        await fireEvent.click(screen.getByTestId("add-save"));
        await fireEvent.click(await screen.findByTestId("add-use-suggestion"));
        expect((screen.getByTestId("add-note") as HTMLInputElement).value).toBe("re K-1");
        expect(screen.queryByTestId("add-note-warning")).toBeNull();
    });

    it("an edit sends only what changed", async () => {
        const onEdit = vi.fn().mockResolvedValue({ok: true});
        render(EntryForm, {entry: entry(), today: "2026-11-26", accounts: ACCOUNTS, onEdit, onCancel: vi.fn(), testid: "edit"});
        expect((screen.getByTestId("edit-duration") as HTMLInputElement).value).toBe("1h");
        await fill("edit-duration", "75m");
        await fireEvent.click(screen.getByTestId("edit-save"));
        expect(onEdit).toHaveBeenCalledWith({id: "time/pat/2026-11.timedot@20", duration: "75m"});
    });

    it("an edit with nothing changed just closes", async () => {
        const onEdit = vi.fn();
        const onCancel = vi.fn();
        render(EntryForm, {entry: entry(), today: "2026-11-26", accounts: ACCOUNTS, onEdit, onCancel, testid: "edit"});
        await fireEvent.click(screen.getByTestId("edit-save"));
        await waitFor(() => expect(onCancel).toHaveBeenCalled());
        expect(onEdit).not.toHaveBeenCalled();
    });

    it("a timeclock session changes by start and end", async () => {
        const onEdit = vi.fn().mockResolvedValue({ok: true});
        render(EntryForm, {
            entry: entry({id: "time/imported/2026-09.timeclock@s1", format: "timeclock"}),
            today: "2026-11-26",
            accounts: ACCOUNTS,
            onEdit,
            onCancel: vi.fn(),
            testid: "edit",
        });
        expect(screen.queryByTestId("edit-duration")).toBeNull();
        await fill("edit-end", "2026-09-29T14:45:30");
        await fireEvent.click(screen.getByTestId("edit-save"));
        expect(onEdit).toHaveBeenCalledWith({id: "time/imported/2026-09.timeclock@s1", end: "2026-09-29T14:45:30"});
    });

    it("Escape cancels", async () => {
        const onCancel = vi.fn();
        render(EntryForm, {entry: entry(), today: "2026-11-26", accounts: ACCOUNTS, onCancel, testid: "edit"});
        await fireEvent.keyDown(screen.getByTestId("edit-duration"), {key: "Escape"});
        expect(onCancel).toHaveBeenCalled();
    });
});
