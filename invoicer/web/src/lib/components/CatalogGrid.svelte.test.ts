// The catalog grid, mounted: a field per column by role, edits and reorders
// reported as new tables, Add item at the bottom, and problems
// marked on their cells and named below.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {GridForm} from "#lib/editor.js";
import CatalogGrid from "./CatalogGrid.svelte";

const TABLE: GridForm = {
    columns: [
        {header: "Code", role: "item"},
        {header: "Description", role: "description"},
        {header: "Unit", role: "unit"},
        {header: "Rate", role: "rate"},
        {header: "*Account*", role: "books"},
    ],
    rows: [
        ["ARCH", "Senior architect hours", "h", "$800", "revenue:consulting"],
        ["JR", "Junior engineer", "h", "$200", "revenue:consulting:delivery"],
    ],
};

function mount(over: Record<string, unknown> = {}) {
    const onChange = vi.fn();
    render(CatalogGrid, {table: TABLE, accounts: ["revenue:consulting", "revenue:saas"], onChange, ...over});
    return onChange;
}

const lastTable = (onChange: ReturnType<typeof vi.fn>): GridForm => onChange.mock.calls.at(-1)?.[0] as GridForm;

describe("COMPONENT CatalogGrid", () => {
    it("shows each column by role, the books column as Account", () => {
        mount();
        const headers = screen.getAllByRole("columnheader").map((h) => h.textContent?.trim());
        expect(headers).toEqual(["Move", "Code", "Description", "Unit", "Rate", "Account", "Row actions"]);
        expect((screen.getByLabelText("Row 1 code") as HTMLInputElement).value).toBe("ARCH");
        expect((screen.getByLabelText("Row 2 rate") as HTMLInputElement).value).toBe("$200");
        expect((screen.getByLabelText("Row 1 account") as HTMLInputElement).value).toBe("revenue:consulting");
        expect(screen.getAllByTestId("catalog-row")).toHaveLength(2);
    });

    it("reports an edited cell as a new table", async () => {
        const onChange = mount();
        await fireEvent.input(screen.getByLabelText("Row 2 rate"), {target: {value: "$250"}});
        expect(lastTable(onChange).rows[1]).toEqual(["JR", "Junior engineer", "h", "$250", "revenue:consulting:delivery"]);
        expect(TABLE.rows[1][3]).toBe("$200");
    });

    it("adds an item at the bottom whatever is focused, moves rows and deletes them", async () => {
        const onChange = mount();
        await fireEvent.focusIn(screen.getByLabelText("Row 1 description"));
        await fireEvent.click(screen.getByTestId("add-catalog-item"));
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["ARCH", "JR", ""]);

        await fireEvent.keyDown(screen.getByLabelText("Row 2 code"), {key: "ArrowUp", altKey: true});
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["JR", "ARCH"]);

        await fireEvent.click(screen.getByRole("button", {name: "Move row 1 down"}));
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["JR", "ARCH"]);

        await fireEvent.click(screen.getByRole("button", {name: "Delete row 1"}));
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["JR"]);
    });

    it("marks the cells with problems and names them", () => {
        mount({problems: [{row: 1, column: 3, message: "the rate ten: not a number"}]});
        const rate = screen.getByLabelText("Row 2 rate");
        expect(rate.getAttribute("aria-invalid")).toBe("true");
        expect(rate.classList.contains("input-error")).toBe(true);
        expect(screen.getByLabelText("Row 1 rate").getAttribute("aria-invalid")).toBeNull();
        expect(screen.getByTestId("catalog-problems").textContent).toContain("Row 2: the rate ten: not a number");
    });

    it("says when there are no items, and offers nothing to edit when read-only", () => {
        mount({table: {...TABLE, rows: []}});
        expect(screen.getByText("No items yet.")).toBeTruthy();
        document.body.innerHTML = "";
        mount({readonly: true});
        expect(screen.queryByTestId("add-catalog-item")).toBeNull();
        expect(screen.queryByRole("button", {name: "Delete row 1"})).toBeNull();
    });
});
