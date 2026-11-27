// The note field: it warns as you type when hledger would read a tag (and
// won't save until reworded), offers the rewrite, shows the server's own
// refusal and suggestion on a 422 note-tag, saves with Enter and cancels
// with Escape.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {EditorApiError} from "#lib/editorApi.js";
import NoteField from "./NoteField.svelte";

function mount(onSave = vi.fn(async () => undefined), initial = "") {
    const onCancel = vi.fn();
    render(NoteField, {label: "Acme Corp · advisory", initial, onSave, onCancel});
    return {onSave, onCancel, input: screen.getByTestId("time-note-input") as HTMLInputElement};
}

describe("COMPONENT the note field", () => {
    it("starts from the note there, and saves the edit with Enter", async () => {
        const {onSave, input} = mount(undefined, "call");
        expect(input.value).toBe("call");
        await fireEvent.input(input, {target: {value: "call about the K-1 "}});
        await fireEvent.keyDown(input, {key: "Enter"});
        expect(onSave).toHaveBeenCalledWith("call about the K-1");
    });

    it("warns while typing a word hledger reads as a tag, and offers the rewrite", async () => {
        const {onSave, input} = mount();
        await fireEvent.input(input, {target: {value: "call re: K-1"}});
        expect(screen.getByTestId("time-note-problem").textContent).toContain("“re:” would be read as a tag");
        expect(input.getAttribute("aria-invalid")).toBe("true");
        expect((screen.getByTestId("time-note-save") as HTMLButtonElement).disabled).toBe(true);
        await fireEvent.keyDown(input, {key: "Enter"});
        expect(onSave).not.toHaveBeenCalled();
        await fireEvent.click(screen.getByTestId("time-note-suggestion"));
        expect(input.value).toBe("call re K-1");
        expect(screen.queryByTestId("time-note-problem")).toBeNull();
    });

    it("shows the server's refusal and its suggestion", async () => {
        const refusal = new EditorApiError(422, "note-tag", "hledger would read a tag here", null, {offending: "x:", suggestion: "server's rewording"});
        const {input} = mount(vi.fn(async () => Promise.reject(refusal)));
        await fireEvent.input(input, {target: {value: "fine by the client"}});
        await fireEvent.click(screen.getByTestId("time-note-save"));
        await vi.waitFor(() => expect(screen.getByTestId("time-note-problem").textContent).toContain("hledger would read a tag here"));
        await fireEvent.click(screen.getByTestId("time-note-suggestion"));
        expect(input.value).toBe("server's rewording");
    });

    it("cancels with Escape", async () => {
        const {onCancel, input} = mount();
        await fireEvent.keyDown(input, {key: "Escape"});
        expect(onCancel).toHaveBeenCalled();
    });
});
