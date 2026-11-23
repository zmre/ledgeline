// The invoice list, mounted: every row links to its invoice, overdue due
// dates stand out, and an empty books folder says how to make one.
import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import InvoiceList from "./InvoiceList.svelte";
import {entry} from "./fixtures.js";

const href = (doc: {number: string | null}) => `/app/invoices/${doc.number}`;

describe("COMPONENT InvoiceList", () => {
    it("shows one row per document, linked by number, with the server's total", () => {
        render(InvoiceList, {
            href,
            invoices: [
                entry({}),
                entry({
                    path: "b.md",
                    number: "106",
                    invoicee: {...entry({}).invoicee, name: "Wingtip"},
                    total: "$2,400.00",
                    overdue: true,
                    due_date: "2026-08-31",
                }),
                entry({path: "c.md", number: "108", status: "draft", open: false, paid: null, doc_type: "estimate"}),
            ],
        });
        const rows = screen.getAllByTestId("invoice-row");
        expect(rows.map((r) => r.dataset.number)).toEqual(["105", "106", "108"]);
        const first = within(rows[0]);
        expect(first.getByRole("link", {name: "105"}).getAttribute("href")).toBe("/app/invoices/105");
        expect(first.getByText("Customer X")).toBeDefined();
        expect(first.getByText("$8,350.00")).toBeDefined();
        expect(first.getByTestId("status-badge").textContent).toBe("Open");

        const overdue = within(rows[1]);
        expect(overdue.getByTestId("status-badge").textContent).toBe("Overdue");
        expect(overdue.getByText("2026-08-31").className).toContain("text-error");

        const draft = within(rows[2]);
        expect(draft.getByTestId("status-badge").textContent).toBe("Draft");
        expect(draft.getByText("Estimate")).toBeDefined();
    });

    it("flags a document with errors", () => {
        render(InvoiceList, {href, invoices: [entry({problems: {errors: 2, warnings: 0}})]});
        expect(screen.getByText("2 errors")).toBeDefined();
    });

    it("says how to make the first invoice when there are none", () => {
        render(InvoiceList, {href, invoices: []});
        expect(screen.getByTestId("invoice-list-empty").textContent).toContain("ledgeline-invoicer new");
        expect(screen.queryByTestId("invoice-list")).toBeNull();
    });
});
