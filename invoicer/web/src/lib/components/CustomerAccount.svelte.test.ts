// A customer's account, mounted: owed, overdue, last payment, its open
// invoices and all of them (backlinked from `customer:`), and the other
// notes linking it, each saying what it is and opening inside the app.
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
    it("shows the balance, the last payment, the open invoices and every invoice", () => {
        render(CustomerAccount, {customer: customerX(), ...hrefs});
        expect(screen.getByTestId("account-balance").textContent).toBe("$8,350.00");
        expect(screen.queryByTestId("account-overdue")).toBeNull();
        expect(screen.getByTestId("account-last-payment").textContent).toBe("$8,350.00 on 2026-10-08");
        // Open first: 105 only.
        const open = screen.getAllByTestId("customer-open-invoice-row");
        expect(open.map((r) => r.dataset.number)).toEqual(["105"]);
        const rows = screen.getAllByTestId("customer-invoice-row");
        expect(rows.map((r) => r.dataset.number)).toEqual(["108", "105", "104"]);
        expect(within(rows[0]).getByTestId("status-badge").textContent).toBe("Draft");
        expect(within(rows[1]).getByTestId("status-badge").textContent).toBe("Open");
        expect(within(rows[1]).getByText("$8,350.00")).toBeDefined();
        expect(within(rows[1]).getByText("2026-12-10")).toBeDefined();
        expect(within(rows[2]).getByTestId("status-badge").textContent).toBe("Paid");
        expect(within(rows[1]).getByRole("link", {name: "105"}).getAttribute("href")).toBe("/app/invoices/105");
        expect(screen.getByText("All invoices")).toBeDefined();
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

    it("flags overdue, says nothing is owed, and opens an invoice from its row", async () => {
        const onOpen = vi.fn();
        render(CustomerAccount, {
            customer: customerX({
                account: {balance: [], overdue: 2, oldest_overdue: "2026-08-31", last_payment: null},
                backlinks: [],
            }),
            ...hrefs,
            onOpen,
        });
        expect(screen.getByTestId("account-balance").textContent).toBe("Nothing owed");
        expect(screen.getByTestId("account-overdue").textContent?.trim()).toBe("2 overdue, since 2026-08-31");
        expect(screen.queryByTestId("customer-backlinks")).toBeNull();
        await fireEvent.click(within(screen.getAllByTestId("customer-invoice-row")[2]).getByText("2026-10-31"));
        expect(onOpen.mock.calls[0][0].number).toBe("104");
    });

    it("says when no invoice names it", () => {
        render(CustomerAccount, {customer: customerX({documents: []}), ...hrefs});
        expect(screen.getByTestId("customer-no-invoices")).toBeDefined();
        expect(screen.queryByTestId("customer-open-invoices")).toBeNull();
    });
});
