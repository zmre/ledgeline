// The customer list, mounted: a row per customer with what it owes, overdue
// flagged, its last payment, and a click anywhere opening it.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import CustomerList from "./CustomerList.svelte";
import {row} from "./customerFixtures.js";

const href = (r: {path: string}) => `/app/customers/${r.path.replace(/\.md$/, "")}`;

describe("COMPONENT CustomerList", () => {
    it("shows each customer's balance, overdue flag and last payment", () => {
        render(CustomerList, {
            href,
            newHref: "/app/customers/new",
            customers: [
                row({}),
                row({
                    path: "customers/Fabrikam.md",
                    title: "Fabrikam",
                    account: {balance: [], overdue: 0, oldest_overdue: null, last_payment: {date: "2026-10-20", amount: "$2,900.00"}},
                }),
                row({
                    path: "customers/Fred Smith.md",
                    title: "Fred Smith",
                    kind: "person",
                    billing: {note: "customers/Customer X.md", name: "Customer X", email: "fred@customerx.com", phone: null, address: null},
                    account: {balance: [], overdue: 0, oldest_overdue: null, last_payment: null},
                }),
            ],
        });
        const rows = screen.getAllByTestId("customer-row");
        expect(rows.map((r) => r.dataset.path)).toEqual(["customers/Contoso.md", "customers/Fabrikam.md", "customers/Fred Smith.md"]);
        const contoso = within(rows[0]);
        expect(contoso.getByRole("link", {name: "Contoso"}).getAttribute("href")).toBe("/app/customers/customers/Contoso");
        expect(contoso.getByText("$2,000.00")).toBeDefined();
        expect(contoso.getByTestId("overdue-badge").getAttribute("title")).toBe("Due 2026-11-14");
        expect(contoso.getByText("$1,000.00 on 2026-11-05")).toBeDefined();
        const fabrikam = within(rows[1]);
        expect(fabrikam.queryByTestId("overdue-badge")).toBeNull();
        expect(fabrikam.getByText("$2,900.00 on 2026-10-20")).toBeDefined();
        const fred = within(rows[2]);
        expect(fred.getByText("Person")).toBeDefined();
        expect(fred.getByText("· bills Customer X")).toBeDefined();
    });

    it("opens a customer from a click on its row, not from its link", async () => {
        const onOpen = vi.fn();
        render(CustomerList, {href, newHref: "/new", onOpen, customers: [row({}), row({path: "customers/Fabrikam.md", title: "Fabrikam"})]});
        const rows = screen.getAllByTestId("customer-row");
        await fireEvent.click(within(rows[1]).getByText("ap@contoso.example"));
        expect(onOpen).toHaveBeenCalledOnce();
        expect(onOpen.mock.calls[0][0].title).toBe("Fabrikam");
        const link = within(rows[0]).getByRole("link", {name: "Contoso"});
        link.addEventListener("click", (e) => e.preventDefault());
        await fireEvent.click(link);
        expect(onOpen).toHaveBeenCalledOnce();
    });

    it("says how to add the first one", () => {
        render(CustomerList, {href, newHref: "/app/customers/new", customers: []});
        const empty = screen.getByTestId("customer-list-empty");
        expect(within(empty).getByRole("heading", {name: "No customers yet"})).toBeDefined();
        const button = within(empty).getByRole("link", {name: "New customer"});
        expect(button.getAttribute("href")).toBe("/app/customers/new");
        expect(button.className).toContain("btn-primary");
    });
});
