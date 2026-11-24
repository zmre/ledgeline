// The bill grid, mounted: the row-type buttons add the right cells above the
// final total, Alt+Arrow and the buttons reorder, an adjustment names its
// base, and amounts are the server's unless an answer is pending (then the
// display-only arithmetic, flagged).
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {computeTable} from "#lib/arithmetic.js";
import {DEFAULT_WORDS, type GridForm, type RowView, type TableView} from "#lib/editor.js";
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
    it("shows the server's amounts and each adjustment's base", () => {
        mount();
        const amounts = screen.getAllByTestId("grid-amount").map((a) => a.textContent?.trim());
        expect(amounts).toEqual(["$8,000.00", "-$800.00", "$7,200.00"]);
        expect(screen.getAllByTestId("grid-amount").every((a) => a.getAttribute("data-optimistic") === "false")).toBe(true);
        expect(screen.getByTestId("grid-base").textContent?.replace(/\s+/g, " ").trim()).toBe("-10% of Senior architect hours, $8,000.00");
    });

    it("shows its own arithmetic, flagged, while the server's answer is pending", () => {
        mount({pending: true});
        const amounts = screen.getAllByTestId("grid-amount");
        expect(amounts.map((a) => a.textContent?.trim())).toEqual(["$8,000.00", "-$800.00", "$7,200.00"]);
        expect(amounts.every((a) => a.getAttribute("data-optimistic") === "true")).toBe(true);
        expect(screen.getAllByText("(calculating)")).toHaveLength(3);
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
        await fireEvent.keyDown(discount, {key: "ArrowUp", altKey: true});
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["Discount", "Senior architect hours", ""]);

        await fireEvent.click(screen.getByRole("button", {name: "Move row 1 down"}));
        expect(lastTable(onChange).rows.map((r) => r[0])).toEqual(["Discount", "Senior architect hours", ""]);
        // A plain arrow key is the field's own.
        const calls = onChange.mock.calls.length;
        await fireEvent.keyDown(discount, {key: "ArrowUp"});
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
});
