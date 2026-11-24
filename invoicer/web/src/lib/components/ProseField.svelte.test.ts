// A prose field, as the plain text field it is when MBR's can't load (jsdom):
// Insert table, and cells pasted from a spreadsheet (HTML or tab-separated)
// arriving as a markdown table.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {describe, expect, it, vi} from "vitest";
import {htmlTableRows, pastedTable} from "#lib/pasteTable.js";
import ProseField from "./ProseField.svelte";

function mount(value = "") {
    const onChange = vi.fn();
    render(ProseField, {value, label: "Prose above the table", onChange, testid: "prose"});
    return {onChange, field: screen.getByLabelText("Prose above the table") as HTMLTextAreaElement};
}

function paste(target: HTMLElement, data: Record<string, string>): Event {
    const event = new Event("paste", {bubbles: true, cancelable: true});
    Object.defineProperty(event, "clipboardData", {value: {getData: (type: string) => data[type] ?? ""}});
    target.dispatchEvent(event);
    return event;
}

const SHEETS_HTML =
    '<meta charset="utf-8"><google-sheets-html-origin><table><tbody><tr><td>Item</td><td>Hours</td></tr><tr><td>Design</td><td colspan="1">4</td></tr></tbody></table></google-sheets-html-origin>';

describe("COMPONENT ProseField", () => {
    it("reads a spreadsheet's HTML table", () => {
        expect(htmlTableRows(SHEETS_HTML)).toEqual([
            ["Item", "Hours"],
            ["Design", "4"],
        ]);
        expect(htmlTableRows("<p>no table</p>")).toBeNull();
        expect(pastedTable(SHEETS_HTML, "Item\tHours\nDesign\t4")).toBe("| Item | Hours |\n| --- | --- |\n| Design | 4 |");
    });

    it("turns pasted cells into a markdown table at the caret", async () => {
        const {onChange, field} = mount("Hello");
        field.setSelectionRange(5, 5);
        const event = paste(field, {"text/plain": "Item\tHours\nDesign\t4"});
        await tick();
        expect(event.defaultPrevented).toBe(true);
        expect(onChange).toHaveBeenLastCalledWith("Hello\n\n| Item | Hours |\n| --- | --- |\n| Design | 4 |\n");
    });

    it("leaves an ordinary paste to the field", () => {
        const {onChange, field} = mount("Hello");
        const event = paste(field, {"text/plain": "just words"});
        expect(event.defaultPrevented).toBe(false);
        expect(onChange).not.toHaveBeenCalled();
    });

    it("inserts an empty table, and offers nothing when read-only", async () => {
        const {onChange} = mount("Intro");
        await fireEvent.click(screen.getByTestId("prose-table"));
        expect(onChange).toHaveBeenLastCalledWith(expect.stringContaining("| Column | Column |\n| --- | --- |"));
        document.body.innerHTML = "";
        render(ProseField, {value: "", label: "Read only", readonly: true, onChange: vi.fn()});
        expect(screen.queryByText("Insert table")).toBeNull();
    });
});
