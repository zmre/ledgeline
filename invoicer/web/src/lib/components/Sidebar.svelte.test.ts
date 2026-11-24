// The sidebar, mounted: its sections with their counts, and the open books
// by name, with the basis as plain text.
import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import Sidebar, {type SidebarLink} from "./Sidebar.svelte";

const links: SidebarLink[] = [
    {id: "invoices", href: "/app/invoices", label: "Invoices", active: true, count: 3, countLabel: "open", overdue: 1},
    {id: "customers", href: "/app/customers", label: "Customers", active: false, count: 6},
    {id: "catalog", href: "/app/catalog", label: "Catalog", active: false, count: null},
];
const books = {
    journal: "books.journal",
    title: "Acme Books",
    basis: "accrual" as const,
    documents: 8,
    invoices: {all: 8, drafts: 1, open: 3, overdue: 1},
    customers: 6,
    catalog: 4,
    snippets: 2,
    nextNumber: "2005",
};

describe("COMPONENT Sidebar", () => {
    it("marks the current section and shows each section's count", () => {
        render(Sidebar, {links, books});
        const invoices = screen.getByTestId("sidebar-invoices");
        expect(invoices.getAttribute("href")).toBe("/app/invoices");
        expect(invoices.getAttribute("aria-current")).toBe("page");
        expect(within(invoices).getByTestId("sidebar-invoices-count").textContent).toBe("3 open");
        expect(within(invoices).getByTestId("sidebar-invoices-overdue").textContent).toBe("1 overdue");
        expect(screen.getByTestId("sidebar-customers-count").textContent).toBe("6");
        expect(screen.queryByTestId("sidebar-catalog-count")).toBeNull();
        expect(screen.queryByTestId("sidebar-customers-overdue")).toBeNull();
    });

    it("names the open books, with the basis as text and no document count", () => {
        render(Sidebar, {links, books});
        const foot = screen.getByTestId("sidebar-books");
        expect(within(foot).getByTestId("sidebar-books-name").textContent).toBe("Acme Books");
        expect(foot.textContent).toContain("books.journal");
        const basis = within(foot).getByTestId("sidebar-basis");
        expect(basis.textContent).toBe("Accrual basis");
        expect(basis.className).not.toContain("badge");
        expect(basis.closest("button")).toBeNull();
        expect(foot.textContent).not.toContain("documents");
    });

    it("falls back to the journal's name, and says when no books are open", () => {
        const {unmount} = render(Sidebar, {links, books: {...books, title: null}});
        expect(screen.getByTestId("sidebar-books-name").textContent).toBe("books.journal");
        unmount();
        render(Sidebar, {links: [{...links[0], active: false}], books: null});
        expect(screen.getByTestId("sidebar-books").textContent).toContain("No books open");
        expect(screen.getByTestId("sidebar-invoices").getAttribute("aria-current")).toBeNull();
    });
});
