// A customer's invoices, mounted: its open invoices and the other ones
// (backlinked from `customer:`; owed and the last payment are the card
// header's now), and the other notes linking it, each saying what it is and
// opening inside the app.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import CustomerAccount from "./CustomerAccount.svelte";
import {customerX} from "./customerFixtures.js";

const hrefs = {
    invoiceHref: (number: string) => `/app/invoices/${number}`,
    customerHref: (path: string) => `/app/customers/${path.replace(/\.md$/, "")}`,
    noteHref: (path: string) => `/app/notes/${path.replace(/\.md$/, "")}`,
};

describe("COMPONENT CustomerAccount", () => {
    it("shows the open invoices and every other one, dates for people", () => {
        render(CustomerAccount, {customer: customerX(), ...hrefs});
        // The money line is the header's.
        expect(screen.queryByTestId("account-balance")).toBeNull();
        // Open first: 105 only; Other is everything else, never 105 again (H18).
        const open = screen.getAllByTestId("customer-open-invoice-row");
        expect(open.map((r) => r.dataset.number)).toEqual(["105"]);
        expect(within(open[0]).getByTestId("status-badge").textContent).toBe("Open");
        expect(within(open[0]).getByText("$8,350.00")).toBeDefined();
        // A formatted due date, the ISO date in its title.
        const due = within(open[0]).getByTitle("2026-12-10");
        expect(due.textContent).toMatch(/Dec 10/);
        expect(within(open[0]).getByRole("link", {name: "105"}).getAttribute("href")).toBe("/app/invoices/105");
        const rows = screen.getAllByTestId("customer-other-invoice-row");
        expect(rows.map((r) => r.dataset.number)).toEqual(["108", "104"]);
        expect(within(rows[0]).getByTestId("status-badge").textContent).toBe("Draft");
        expect(within(rows[1]).getByTestId("status-badge").textContent).toBe("Paid");
        expect(screen.getByRole("heading", {name: "Open"}).id).toBe("open-invoices");
        expect(screen.getByRole("heading", {name: "Other"})).toBeDefined();
        expect(screen.queryByText("All invoices")).toBeNull();
    });

    it("says what each other linking note is, and links it inside the app", () => {
        render(CustomerAccount, {customer: customerX(), ...hrefs});
        const others = screen.getByTestId("customer-backlinks");
        const items = within(others).getAllByTestId("customer-backlink");
        // Not its invoices again; never the linking key ("company").
        expect(items.map((i) => i.textContent?.replace(/\s+/g, " ").trim())).toEqual(["Fred Smith Person", "Kickoff", "Quote 7 Estimate"]);
        expect(within(others).queryByText(/company/i)).toBeNull();
        const href = (name: string) => within(others).getByRole("link", {name}).getAttribute("href");
        expect(href("Fred Smith")).toBe("/app/customers/customers/Fred Smith");
        expect(href("Kickoff")).toBe("/app/notes/notes/kickoff");
        expect(href("Quote 7")).toBe("/app/invoices/E7");
    });

    it("marks an overdue invoice with a badge, and opens an invoice from its row", async () => {
        const onOpen = vi.fn();
        render(CustomerAccount, {
            customer: customerX({
                account: {balance: [], overdue: 2, oldest_overdue: "2026-08-31", last_payment: null},
                backlinks: [],
            }),
            ...hrefs,
            onOpen,
        });
        expect(screen.queryByTestId("customer-backlinks")).toBeNull();
        await fireEvent.click(within(screen.getAllByTestId("customer-other-invoice-row")[1]).getByTitle("2026-10-31"));
        expect(onOpen.mock.calls[0][0].number).toBe("104");
    });

    it("says when no invoice names it", () => {
        render(CustomerAccount, {customer: customerX({documents: []}), ...hrefs});
        expect(screen.getByTestId("customer-no-invoices")).toBeDefined();
        expect(screen.queryByTestId("customer-open-invoices")).toBeNull();
        expect(screen.queryByTestId("customer-other-invoices")).toBeNull();
    });

    it("says nothing is open when every invoice is settled", () => {
        const settled = customerX().documents.filter((d) => !d.open);
        render(CustomerAccount, {customer: customerX({documents: settled}), ...hrefs});
        expect(screen.getByTestId("customer-nothing-open").textContent).toBe("Nothing open.");
        expect(screen.getAllByTestId("customer-other-invoice-row")).toHaveLength(2);
    });

    it("shows the newest five other invoices, then all of them on request", async () => {
        const docs = customerX().documents;
        const many = Array.from({length: 7}, (_, i) => ({...docs[2], path: `invoices/${200 + i}.md`, number: String(200 + i)}));
        render(CustomerAccount, {customer: customerX({documents: many}), ...hrefs});
        expect(screen.getAllByTestId("customer-other-invoice-row")).toHaveLength(5);
        const more = screen.getByTestId("other-show-all");
        expect(more.textContent).toBe("Show all 7");
        expect(more.getAttribute("aria-expanded")).toBe("false");
        await fireEvent.click(more);
        expect(screen.getAllByTestId("customer-other-invoice-row")).toHaveLength(7);
    });
});
