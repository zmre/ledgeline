// A customer's account, mounted: owed, overdue, last payment, its invoices
// (backlinked from `customer:`) and the other notes linking it.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import CustomerAccount from "./CustomerAccount.svelte";
import {customerX} from "./customerFixtures.js";

const invoiceHref = (doc: {number: string | null}) => `/app/invoices/${doc.number}`;
const pageHref = (url: string) => url;

describe("COMPONENT CustomerAccount", () => {
    it("shows the balance, the last payment and each invoice's status, total and due date", () => {
        render(CustomerAccount, {customer: customerX(), invoiceHref, pageHref});
        expect(screen.getByTestId("account-balance").textContent).toBe("$8,350.00");
        expect(screen.queryByTestId("account-overdue")).toBeNull();
        expect(screen.getByTestId("account-last-payment").textContent).toBe("$8,350.00 on 2026-10-08");
        const rows = screen.getAllByTestId("customer-invoice-row");
        expect(rows.map((r) => r.dataset.number)).toEqual(["108", "105", "104"]);
        expect(within(rows[0]).getByTestId("status-badge").textContent).toBe("Draft");
        expect(within(rows[1]).getByTestId("status-badge").textContent).toBe("Open");
        expect(within(rows[1]).getByText("$8,350.00")).toBeDefined();
        expect(within(rows[1]).getByText("2026-12-10")).toBeDefined();
        expect(within(rows[2]).getByTestId("status-badge").textContent).toBe("Paid");
        expect(within(rows[1]).getByRole("link", {name: "105"}).getAttribute("href")).toBe("/app/invoices/105");
    });

    it("lists the other notes linking it, not its invoices again", () => {
        render(CustomerAccount, {customer: customerX(), invoiceHref, pageHref});
        const others = screen.getByTestId("customer-backlinks");
        const links = within(others).getAllByRole("link");
        expect(links.map((l) => l.textContent)).toEqual(["Fred Smith", "Kickoff"]);
        expect(within(others).getByText("(company)")).toBeDefined();
    });

    it("flags overdue, says nothing is owed, and opens an invoice from its row", async () => {
        const onOpen = vi.fn();
        render(CustomerAccount, {
            customer: customerX({
                account: {balance: [], overdue: 2, oldest_overdue: "2026-08-31", last_payment: null},
                backlinks: [],
            }),
            invoiceHref,
            pageHref,
            onOpen,
        });
        expect(screen.getByTestId("account-balance").textContent).toBe("Nothing owed");
        expect(screen.getByTestId("account-overdue").textContent?.trim()).toBe("2 overdue, since 2026-08-31");
        expect(screen.queryByTestId("customer-backlinks")).toBeNull();
        await fireEvent.click(within(screen.getAllByTestId("customer-invoice-row")[2]).getByText("2026-10-31"));
        expect(onOpen.mock.calls[0][0].number).toBe("104");
    });

    it("says when no invoice names it", () => {
        render(CustomerAccount, {customer: customerX({documents: []}), invoiceHref, pageHref});
        expect(screen.getByTestId("customer-no-invoices")).toBeDefined();
    });
});
