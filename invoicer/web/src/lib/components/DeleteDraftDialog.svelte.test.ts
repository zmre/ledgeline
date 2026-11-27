// Delete draft… and Void…'s time choice (plan 46 T10): each says how much
// time the invoice holds and what becomes of it; an invoice with no time
// says nothing about time.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import {listenForKeys} from "#lib/testing/keys.js";
import {entry, list, qty} from "#lib/time/testing.js";

const api = vi.hoisted(() => ({entries: vi.fn(), del: vi.fn()}));
vi.mock("#lib/time/api/entries.js", async (original) => ({...(await original<object>()), fetchEntries: api.entries}));
vi.mock("#lib/editorApi.js", async (original) => ({...(await original<object>()), deleteDraft: api.del}));

import {EditorApiError} from "#lib/editorApi.js";
import DeleteDraftDialog from "./DeleteDraftDialog.svelte";
import VoidTimeChoice from "./VoidTimeChoice.svelte";

describe("COMPONENT the invoice's time on delete and void", () => {
    listenForKeys();

    beforeEach(() => {
        api.entries.mockReset();
        api.del.mockReset();
        api.del.mockResolvedValue({deleted: "invoices/x.md", number: "102", time: {released: 1, how: "release"}});
    });

    it("deleting a draft with time says its hours go back to Unbilled", async () => {
        api.entries.mockResolvedValue(list([entry({quantity: qty(300), invoice: "102", billed: "invoiced"})]));
        const onDeleted = vi.fn();
        render(DeleteDraftDialog, {number: "102", onDeleted, onCancel: vi.fn()});
        expect((await screen.findByTestId("confirm-delete-draft-time")).textContent).toBe("Its 5 hours of time go back to Unbilled.");
        expect(api.entries).toHaveBeenCalledWith("invoice=102");
        await fireEvent.click(screen.getByTestId("confirm-delete-draft-confirm"));
        await waitFor(() => expect(onDeleted).toHaveBeenCalledWith(1));
        expect(api.del).toHaveBeenCalledWith("102");
    });

    it("a draft with no time, or books with no time files, say nothing about time", async () => {
        api.entries.mockRejectedValue(new EditorApiError(409, "not-set-up", "time isn't set up"));
        render(DeleteDraftDialog, {number: "103", onDeleted: vi.fn(), onCancel: vi.fn()});
        await waitFor(() => expect(api.entries).toHaveBeenCalled());
        expect(screen.getByTestId("confirm-delete-draft").textContent).toContain("Its file is deleted");
        expect(screen.queryByTestId("confirm-delete-draft-time")).toBeNull();
    });

    it("voiding asks Release (the default) or Mark not billed", async () => {
        render(VoidTimeChoice, {time: qty(300)});
        expect(screen.getByTestId("void-time").textContent).toContain("Its 5 hours of time:");
        expect((screen.getByTestId("void-time-release") as HTMLInputElement).checked).toBe(true);
        await fireEvent.click(screen.getByTestId("void-time-nobill"));
        expect((screen.getByTestId("void-time-nobill") as HTMLInputElement).checked).toBe(true);
    });

    it("no time, no question", () => {
        render(VoidTimeChoice, {time: null});
        expect(screen.queryByTestId("void-time")).toBeNull();
    });
});
