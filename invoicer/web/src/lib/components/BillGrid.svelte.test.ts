// The bill grid, mounted: the row-type buttons add the right cells above the
// final total, Alt+Arrow and the buttons reorder, an adjustment names its
// base, and amounts are the server's unless an answer is pending (then the
// display-only arithmetic, flagged).
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {computeTable} from "#lib/arithmetic.js";
import {DEFAULT_WORDS, type GridForm, type RowView, type TableView} from "#lib/editor.js";
import {listenForKeys, press} from "#lib/testing/keys.js";
import BillGrid from "./BillGrid.svelte";

const COLUMNS: GridForm["columns"] = [
    {header: "Description", role: "description"},
    {header: "Qty", role: "quantity"},
    {header: "Rate", role: "rate"},
    {header: "Amount", role: "amount"},
    {header: "*Account*", role: "books"},
];

const TABLE: GridForm = {
    columns: COLUMNS,
    rows: [
        ["Senior architect hours", "10", "$800", "", "revenue:consulting"],
        ["Discount", "", "-10%", "", ""],
        ["", "", "**Total due**", "", ""],
    ],
};

const row = (over: Partial<RowView>): RowView => ({
    row: 1,
    line: 1,
    kind: "item",
    description: "",
    code: null,
    quantity: null,
    unit: null,
    rate: null,
    amount: null,
    written: "",
    computed: true,
    applies_to: null,
    books: null,
    account: null,
    deferral: null,
    ...over,
});

const VIEW: TableView = {
    commodity: "$",
    total: "$7,200.00",
    rows: [
        row({row: 1, description: "Senior architect hours", amount: "$8,000.00"}),
        row({row: 2, kind: "adjustment", description: "Discount", rate: "-10%", amount: "-$800.00", applies_to: 1}),
        row({row: 3, kind: "total", rate: "**Total due**", amount: "$7,200.00"}),
    ],
    discrepancies: [],
};

function mount(over: Record<string, unknown> = {}) {
    const onChange = vi.fn();
    const table = (over.table as GridForm | undefined) ?? TABLE;
    render(BillGrid, {
        table,
        view: VIEW,
        local: computeTable(
            table.columns.map((c) => c.role),
            table.rows
        ),
        words: DEFAULT_WORDS,
        onChange,
        ...over,
    });
    return onChange;
}

const lastTable = (onChange: ReturnType<typeof vi.fn>): GridForm => onChange.mock.calls.at(-1)?.[0] as GridForm;

describe("COMPONENT BillGrid", () => {
    // Alt+Arrow and the row-type keys are keymap bindings (PLAN 5k).
    listenForKeys();

    it("shows the server's amounts and each adjustment's base", () => {
        mount();
        const amounts = screen.getAllByTestId("grid-amount").map((a) => a.textContent?.trim());
        expect(amounts).toEqual(["$8,000.00", "-$800.00"]);
        expect(screen.getAllByTestId("grid-amount").every((a) => a.getAttribute("data-optimistic") === "false")).toBe(true);
        expect(screen.getByTestId("grid-total").textContent?.trim()).toBe("$7,200.00");
        expect(screen.getByTestId("grid-base").textContent?.replace(/\s+/g, " ").trim()).toBe("-10% of Senior architect hours, $8,000.00");
    });

    it("shows its own arithmetic, flagged, while the server's answer is pending", () => {
        mount({pending: true});
        const amounts = screen.getAllByTestId("grid-amount");
        expect(amounts.map((a) => a.textContent?.trim())).toEqual(["$8,000.00", "-$800.00"]);
        expect(amounts.every((a) => a.getAttribute("data-optimistic") === "true")).toBe(true);
        expect(screen.getAllByText("(updating)")).toHaveLength(2);
        expect(screen.getByTestId("grid-total").getAttribute("data-optimistic")).toBe("true");
    });

    it("adds each row type above the final total", async () => {
        const cases: [string, string[]][] = [
            ["add-item", ["", "1", "", "", ""]],
            ["add-discount-percent", ["Discount", "", "-10%", "", ""]],
            ["add-discount-fixed", ["Discount", "", "", "-$0.00", ""]],
            ["add-subtotal", ["", "", "**Subtotal**", "", ""]],
            ["add-tax", ["Sales tax", "", "0%", "", ""]],
            ["add-spacer", ["", "", "", "", ""]],
        ];
        for (const [button, cells] of cases) {
            const onChange = mount();
            await fireEvent.click(screen.getAllByTestId(button).at(-1)!);
            const next = lastTable(onChange);
            expect(next.rows).toHaveLength(4);
            expect(next.rows[2]).toEqual(cells);
            expect(next.rows[3][2]).toBe("**Total due**");
            document.body.innerHTML = "";
        }
    });

    it("moves the focused row with Alt+Arrow, and with the buttons", async () => {
        const onChange = mount();
        const discount = screen.getByLabelText("Row 2 description");
        await press("ArrowUp", discount, {altKey: true});
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["Discount", "Senior architect hours", ""]);

        await fireEvent.click(screen.getByRole("button", {name: "Move row 1 down"}));
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["Discount", "Senior architect hours", ""]);
        // A plain arrow key is the field's own.
        const calls = onChange.mock.calls.length;
        await press("ArrowUp", discount);
        expect(onChange.mock.calls.length).toBe(calls);
    });

    it("deletes a row and edits a cell", async () => {
        const onChange = mount();
        await fireEvent.click(screen.getByRole("button", {name: "Delete row 2"}));
        expect(lastTable(onChange).rows).toHaveLength(2);
        await fireEvent.input(screen.getByLabelText("Row 1 quantity"), {target: {value: "12"}});
        expect(lastTable(onChange).rows[0][1]).toBe("12");
    });

    it("fills a row from the catalog, adding the Code and Unit columns", async () => {
        const onChange = mount({
            catalog: [{code: "ARCH", description: "Senior architect hours", unit: "h", rate: "$800", books: "revenue:consulting"}],
            table: {
                columns: COLUMNS,
                rows: [
                    ["", "1", "", "", ""],
                    ["", "", "**Total due**", "", ""],
                ],
            },
        });
        const field = screen.getByLabelText("Row 1 description");
        await fireEvent.input(field, {target: {value: "senior"}});
        await fireEvent.keyDown(field, {key: "Enter"});
        const next = lastTable(onChange);
        expect(next.columns.map((c) => c.role)).toEqual(["item", "description", "quantity", "unit", "rate", "amount", "books"]);
        expect(next.rows[0]).toEqual(["ARCH", "Senior architect hours", "1", "h", "$800", "", "revenue:consulting"]);
    });

    it("applies an open deposit above the final total", async () => {
        const loadDeposits = vi.fn().mockResolvedValue({
            customer: "Customer X",
            account: "liabilities:customer-deposits",
            commodity: "$",
            balance: "8350.00",
            deposits: [
                {
                    number: "104",
                    path: "invoices/2026-10-01 Customer X Invoice 104.md",
                    date: "2026-10-01",
                    description: "Customer X | Invoice 104",
                    credited: "8350.00",
                    remaining: "8350.00",
                    row: {description: "Less deposit (Invoice 104)", amount: "-$8,350.00", books: "liabilities:customer-deposits"},
                },
            ],
        });
        const onChange = mount({loadDeposits});
        await fireEvent.click(screen.getByTestId("add-deposit"));
        await fireEvent.click(await screen.findByRole("button", {name: "Apply"}));
        expect(lastTable(onChange).rows[2]).toEqual(["Less deposit (Invoice 104)", "", "", "-$8,350.00", "liabilities:customer-deposits"]);
    });

    it("is read-only for a void invoice", () => {
        mount({readonly: true});
        expect(screen.queryByTestId("add-item")).toBeNull();
        expect(screen.queryByRole("button", {name: "Delete row 1"})).toBeNull();
    });

    it("shows the final total as the footer, not a row, its label one of the total words", async () => {
        const onChange = mount({labels: {total: ["Total due", "Amount due"], subtotal: ["Subtotal"]}});
        expect(screen.getAllByTestId("grid-row")).toHaveLength(2);
        expect(screen.queryByLabelText("Row 3 rate")).toBeNull();
        const label = screen.getByLabelText("Total label") as HTMLSelectElement;
        expect(label.value).toBe("Total due");
        await fireEvent.change(label, {target: {value: "Amount due"}});
        expect(lastTable(onChange).rows[2]).toEqual(["", "", "**Amount due**", "", ""]);
    });

    it("adds a total row with the first row when the table has none", async () => {
        const onChange = mount({table: {columns: COLUMNS, rows: []}, view: null});
        expect(screen.getByTestId("grid-empty")).toBeTruthy();
        expect(screen.getByTestId("grid-total").textContent?.trim()).toBe("$0.00");
        await fireEvent.click(screen.getByTestId("add-item"));
        expect(lastTable(onChange).rows).toEqual([
            ["", "1", "", "", ""],
            ["", "", "**Total due**", "", ""],
        ]);
    });

    it("draws a subtotal as its label and amount, and a spacer as an empty separator", async () => {
        const table: GridForm = {
            columns: COLUMNS,
            rows: [
                ["Hours", "2", "$100", "", ""],
                ["", "", "**Subtotal**", "", ""],
                ["", "", "", "", ""],
                ["", "", "**Total due**", "", ""],
            ],
        };
        const onChange = mount({table, view: null, pending: true});
        const rows = screen.getAllByTestId("grid-row");
        expect(rows.map((r) => r.getAttribute("data-kind"))).toEqual(["item", "subtotal", "spacer"]);
        // No editable cells on either.
        expect(screen.queryByLabelText("Row 2 rate")).toBeNull();
        expect(screen.queryByLabelText("Row 3 description")).toBeNull();
        expect(screen.getByLabelText("Row 2 amount").textContent?.trim()).toBe("$200.00");
        expect(screen.getByTestId("grid-spacer")).toBeTruthy();
        await fireEvent.change(screen.getByLabelText("Row 2 subtotal label"), {target: {value: "Total due"}});
        expect(lastTable(onChange).rows[1][2]).toBe("**Total due**");
        // The spacer still moves and deletes.
        await fireEvent.click(screen.getByRole("button", {name: "Delete row 3"}));
        expect(lastTable(onChange).rows).toHaveLength(3);
    });

    it("offers the Unit and Code columns a table lacks", async () => {
        const onChange = mount();
        await fireEvent.click(screen.getByTestId("add-unit-column"));
        expect(lastTable(onChange).columns.map((c) => c.role)).toEqual(["description", "quantity", "unit", "rate", "amount", "books"]);
        await fireEvent.click(screen.getByTestId("add-code-column"));
        expect(lastTable(onChange).columns[0].role).toBe("item");
    });

    it("never moves a row below the final total", async () => {
        const onChange = mount();
        expect((screen.getByRole("button", {name: "Move row 2 down"}) as HTMLButtonElement).disabled).toBe(true);
        await press("ArrowDown", screen.getByLabelText("Row 2 description"), {altKey: true});
        expect(onChange).not.toHaveBeenCalled();
    });
});

describe("COMPONENT BillGrid keys (PLAN 5k)", () => {
    listenForKeys();

    it("adds each row type with Alt+Shift and its letter, from a cell, and focuses the new row", async () => {
        const cases: [string, string[]][] = [
            ["I", ["", "1", "", "", ""]],
            ["D", ["Discount", "", "-10%", "", ""]],
            ["F", ["Discount", "", "", "-$0.00", ""]],
            ["S", ["", "", "**Subtotal**", "", ""]],
            ["T", ["Sales tax", "", "0%", "", ""]],
            ["B", ["", "", "", "", ""]],
        ];
        for (const [letter, cells] of cases) {
            const onChange = mount();
            const cell = screen.getAllByLabelText("Row 1 description").at(-1)!;
            const handled = !(await press(letter, cell, {altKey: true, shiftKey: true, code: `Key${letter}`}));
            expect(handled).toBe(true);
            expect(lastTable(onChange).rows[2]).toEqual(cells);
        }
    });

    it("reads a Mac's Option+Shift character as its letter", async () => {
        const onChange = mount();
        // Option+Shift+S types "Í" on a US Mac.
        await press("Í", screen.getByLabelText("Row 1 description"), {altKey: true, shiftKey: true, code: "KeyS"});
        expect(lastTable(onChange).rows[2]).toEqual(["", "", "**Subtotal**", "", ""]);
    });

    it("leaves Option+S without Shift to the field (it types ß)", async () => {
        const onChange = mount();
        const notHandled = await press("ß", screen.getByLabelText("Row 1 description"), {altKey: true, code: "KeyS"});
        expect(notHandled).toBe(true);
        expect(onChange).not.toHaveBeenCalled();
    });

    it("has no row keys on a void invoice", async () => {
        const onChange = mount({readonly: true});
        await press("I", document.body, {altKey: true, shiftKey: true, code: "KeyI"});
        await press("ArrowUp", screen.getByLabelText("Row 2 description"), {altKey: true});
        expect(onChange).not.toHaveBeenCalled();
    });

    it("moves a row with Alt+Arrow from a catalog cell, whose list keeps plain arrows", async () => {
        const onChange = mount({
            table: {columns: COLUMNS, rows: [TABLE.rows[0], ["Design review", "2", "$400", "", ""], TABLE.rows[2]]},
            catalog: [{code: "ARCH", description: "Senior architect hours", unit: "h", rate: "$800", books: "revenue:consulting"}],
        });
        await press("ArrowUp", screen.getByLabelText("Row 2 description"), {altKey: true});
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["Design review", "Senior architect hours", ""]);
    });

    it("moves a row only from inside the grid", async () => {
        const onChange = mount();
        const outside = document.createElement("textarea");
        document.body.append(outside);
        const notHandled = await press("ArrowUp", outside, {altKey: true});
        outside.remove();
        expect(notHandled).toBe(true);
        expect(onChange).not.toHaveBeenCalled();
    });
});
