// The invoice list, mounted: every row links to its invoice, overdue due
// dates stand out, and an empty books folder offers New invoice.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
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

    it("opens an invoice from a click anywhere on its row, but not from a link in it", async () => {
        const onOpen = vi.fn();
        render(InvoiceList, {href, onOpen, invoices: [entry({}), entry({path: "b.md", number: "106"})]});
        const rows = screen.getAllByTestId("invoice-row");
        await fireEvent.click(within(rows[1]).getByText("$8,350.00"));
        expect(onOpen).toHaveBeenCalledOnce();
        expect(onOpen.mock.calls[0][0].number).toBe("106");
        // The number's own link is the keyboard's way in (Tab, Enter): one link per row, nothing nested.
        const link = within(rows[0]).getByRole("link", {name: "105"});
        link.addEventListener("click", (e) => e.preventDefault());
        await fireEvent.click(link);
        expect(onOpen).toHaveBeenCalledOnce();
        expect(within(rows[0]).getAllByRole("link")).toHaveLength(1);
        expect(rows[0].getAttribute("role")).toBeNull();
    });

    it("offers the editor's New invoice when there are none", () => {
        render(InvoiceList, {href, invoices: [], newHref: "/app/new"});
        const empty = screen.getByTestId("invoice-list-empty").textContent ?? "";
        expect(empty).toContain("No invoices yet");
        expect(screen.getByTestId("invoice-list-new").getAttribute("href")).toBe("/app/new");
        expect(empty).not.toContain("ledgeline-invoicer");
        expect(screen.getByTestId("invoice-list-new").className).toContain("btn-primary");
        expect(screen.queryByTestId("invoice-list")).toBeNull();
    });
});

describe("COMPONENT InvoiceList sorting and filters (PLAN 5b)", () => {
    it("sorts from its headers: a button each, aria-sort on the column", async () => {
        const onSort = vi.fn();
        render(InvoiceList, {href, invoices: [entry({})], shown: {sort: "total", dir: "desc"}, onSort});
        const headers = screen.getAllByRole("columnheader");
        expect(headers.map((h) => h.getAttribute("aria-sort"))).toEqual(["none", "none", "none", "none", "descending", "none"]);
        await fireEvent.click(screen.getByRole("button", {name: "Due"}));
        expect(onSort).toHaveBeenCalledExactlyOnceWith("due");
    });

    it("keeps plain headers without onSort", () => {
        render(InvoiceList, {href, invoices: [entry({})]});
        expect(screen.queryAllByRole("button", {name: "Number"})).toHaveLength(0);
    });

    it("tints overdue rows", () => {
        render(InvoiceList, {href, invoices: [entry({}), entry({path: "b.md", number: "106", overdue: true})]});
        const rows = screen.getAllByTestId("invoice-row");
        expect(rows[0].dataset.overdue).toBeUndefined();
        expect(rows[1].dataset.overdue).toBe("true");
        expect(rows[1].className).toContain("bg-error");
    });

    it("says nothing matches when filtered, with Clear filters, not the first-invoice card", async () => {
        const onClear = vi.fn();
        render(InvoiceList, {href, invoices: [], filtered: true, onClear});
        expect(screen.queryByTestId("invoice-list-empty")).toBeNull();
        expect(screen.getByTestId("invoice-list-no-match").textContent).toContain("No invoices match");
        await fireEvent.click(screen.getByTestId("invoice-list-clear"));
        expect(onClear).toHaveBeenCalledOnce();
    });
});
