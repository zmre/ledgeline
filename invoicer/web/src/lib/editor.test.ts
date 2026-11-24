// The editor's grid operations and helpers.
import {describe, expect, it} from "vitest";
import {
    addDays,
    baseText,
    cellOf,
    ensureColumn,
    ensureTotal,
    fillFromCatalog,
    finalTotal,
    formatBooksCell,
    insertRow,
    isSpacerRow,
    isTotalRow,
    moveRow,
    onDiskChange,
    parseBooksCell,
    removeRow,
    setTotalLabel,
    totalLabel,
    totalWords,
    type GridForm,
    type RowView,
} from "./editor";

/** A new draft's table, as `new` writes it: header and a total row. */
const fresh = (): GridForm => ({
    columns: [
        {header: "Description", role: "description"},
        {header: "Qty", role: "quantity"},
        {header: "Rate", role: "rate"},
        {header: "Amount", role: "amount"},
        {header: "*Account*", role: "books"},
    ],
    rows: [["", "", "**Total due**", "**$0.00**", ""]],
});

describe("grid operations", () => {
    it("inserts each row type above the final total", () => {
        let table = fresh();
        for (const kind of ["item", "discount-percent", "discount-fixed", "subtotal", "tax", "spacer"] as const) {
            table = insertRow(table, kind).table;
        }
        expect(table.rows).toEqual([
            ["", "1", "", "", ""],
            ["Discount", "", "-10%", "", ""],
            ["Discount", "", "", "-$0.00", ""],
            ["", "", "**Subtotal**", "", ""],
            ["Sales tax", "", "0%", "", ""],
            ["", "", "", "", ""],
            ["", "", "**Total due**", "**$0.00**", ""],
        ]);
        expect(finalTotal(table)).toBe(6);
    });

    it("always inserts at the bottom, never below the final total, and adds a total when there is none", () => {
        let table = insertRow(fresh(), "item").table;
        table = insertRow(table, "item").table;
        const next = insertRow(table, "discount-percent");
        expect(next.at).toBe(2);
        expect(next.table.rows[2][2]).toBe("-10%");
        expect(finalTotal(next.table)).toBe(3);

        const bare: GridForm = {...fresh(), rows: [["Hours", "1", "$100", "", ""]]};
        const totalled = insertRow(bare, "item");
        expect(totalled.at).toBe(1);
        expect(totalled.table.rows[2]).toEqual(["", "", "**Total due**", "", ""]);
        expect(ensureTotal(totalled.table)).toBe(totalled.table);
    });

    it("reads and relabels total rows; a blank label keeps the row a total", () => {
        const table = insertRow(fresh(), "subtotal").table;
        expect(isTotalRow(table, 0)).toBe(true);
        expect(totalLabel(table, 1)).toBe("Total due");
        const renamed = setTotalLabel(table, 1, "Amount due");
        expect(renamed.rows[1][2]).toBe("**Amount due**");
        expect(finalTotal(renamed)).toBe(1);
        expect(setTotalLabel(table, 1, "  ")).toBe(table);
        // A label the books' words add is a total too.
        const custom = setTotalLabel(table, 1, "Amount owing");
        expect(finalTotal(custom)).toBe(0);
        expect(finalTotal(custom, undefined, ["Amount owing"])).toBe(1);
        expect(isSpacerRow(insertRow(fresh(), "spacer").table, 0)).toBe(true);
        expect(isSpacerRow(table, 0)).toBe(false);
    });

    it("knows the books' total words, with the defaults until the options load", () => {
        expect(totalWords(null).total[0]).toBe("Total due");
        expect(totalWords(null).subtotal).toEqual(["Subtotal"]);
    });

    it("moves and removes rows", () => {
        let table = insertRow(fresh(), "item").table;
        table = insertRow(table, "discount-percent").table;
        const moved = moveRow(table, 1, 0);
        expect(moved.rows.map((r) => r[0])).toEqual(["Discount", "", ""]);
        expect(moveRow(table, 0, -5).rows).toEqual(table.rows);
        expect(removeRow(moved, 0).rows).toHaveLength(2);
        // The original is untouched (the grid's ops are pure).
        expect(table.rows[0][1]).toBe("1");
    });

    it("adds the Code and Unit columns a catalog item needs", () => {
        const table = insertRow(fresh(), "item").table;
        const filled = fillFromCatalog(table, 0, {code: "ARCH", description: "Senior architect hours", unit: "h", rate: "$800", books: "revenue:consulting"});
        expect(filled.columns.map((c) => c.role)).toEqual(["item", "description", "quantity", "unit", "rate", "amount", "books"]);
        expect(filled.rows[0]).toEqual(["ARCH", "Senior architect hours", "1", "h", "$800", "", "revenue:consulting"]);
        expect(filled.rows[1]).toEqual(["", "", "", "", "**Total due**", "**$0.00**", ""]);
        expect(cellOf(filled, 0, "unit")).toBe("h");
        expect(ensureColumn(filled, "unit")).toBe(filled);
    });
});

describe("books cells", () => {
    it("round-trips the deferral grammar", () => {
        const text = "liabilities:deferred-revenue → revenue:saas, 12 months from 2026-12-01";
        const cell = parseBooksCell(text);
        expect(cell).toEqual({account: "liabilities:deferred-revenue", deferral: {target: "revenue:saas", months: 12, from: "2026-12-01", calendar: false}});
        expect(formatBooksCell(cell!)).toBe(text);
        expect(formatBooksCell(parseBooksCell("a -> b, 1 month calendar")!)).toBe("a → b, 1 month calendar");
        expect(parseBooksCell("revenue:consulting")).toEqual({account: "revenue:consulting", deferral: null});
        expect(parseBooksCell("a → b, through 2027-01-31")).toBeNull();
    });
});

describe("helpers", () => {
    it("adds days across months and years", () => {
        expect(addDays("2026-11-17", 30)).toBe("2026-12-17");
        expect(addDays("2026-12-20", 15)).toBe("2027-01-04");
        expect(addDays("nope", 3)).toBeNull();
    });

    it("decides what a change on disk means", () => {
        expect(onDiskChange("a", "a", true)).toBe("ignore");
        expect(onDiskChange("a", "b", false)).toBe("replace");
        expect(onDiskChange("a", "b", true)).toBe("conflict");
    });

    it("names an adjustment's base", () => {
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
        const rows = [
            row({row: 1, description: "Senior architect hours", amount: "$2,500.00"}),
            row({row: 2, kind: "adjustment", applies_to: 1, rate: "-10%"}),
            row({row: 3, kind: "total", rate: "**Subtotal**", amount: "$2,250.00"}),
            row({row: 4, kind: "adjustment", applies_to: 3, rate: "8.25%"}),
        ];
        expect(baseText(rows, rows[1])).toBe("of Senior architect hours, $2,500.00");
        expect(baseText(rows, rows[3])).toBe("of Subtotal, $2,250.00");
        expect(baseText(rows, rows[0])).toBeNull();
    });
});
