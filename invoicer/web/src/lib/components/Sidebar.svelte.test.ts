// The sidebar, mounted: its sections and the open books.
import {render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import Sidebar from "./Sidebar.svelte";

const links = [{id: "invoices", href: "/app/invoices", label: "Invoices", active: true}];

describe("COMPONENT Sidebar", () => {
    it("marks the current section and names the open books", () => {
        render(Sidebar, {links, books: {journal: "books.journal", basis: "accrual", documents: 8, customers: 6, nextNumber: "2005"}});
        const invoices = screen.getByRole("link", {name: "Invoices"});
        expect(invoices.getAttribute("href")).toBe("/app/invoices");
        expect(invoices.getAttribute("aria-current")).toBe("page");
        const books = screen.getByTestId("sidebar-books");
        expect(books.textContent).toContain("books.journal");
        expect(books.textContent).toContain("Accrual basis");
        expect(books.textContent).toContain("8 documents");
    });

    it("says when no books are open", () => {
        render(Sidebar, {links: [{...links[0], active: false}], books: null});
        expect(screen.getByTestId("sidebar-books").textContent).toContain("No books open");
        expect(screen.getByRole("link", {name: "Invoices"}).getAttribute("aria-current")).toBeNull();
    });
});
