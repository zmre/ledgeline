// The list pages themselves, mounted (PLAN 5b): a URL opened directly asks
// the server for that list, and shows its filters. Written after the
// customer page shipped without its URL state (the rewrite never landed;
// every component test passed), so the routes are covered, not only their
// parts.
import {render, screen, waitFor} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";

const nav = vi.hoisted(() => ({url: new URL("http://localhost/app/customers"), goto: vi.fn()}));

vi.mock("$app/state", () => ({
    page: {
        get url() {
            return nav.url;
        },
        params: {},
    },
}));
vi.mock("$app/navigation", () => ({goto: nav.goto, beforeNavigate: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => `/app${path}`}));

const api = vi.hoisted(() => ({customers: vi.fn(), invoices: vi.fn()}));
vi.mock("#lib/customers.js", async (original) => ({...(await original<object>()), fetchCustomers: api.customers}));
vi.mock("#lib/api.js", async (original) => ({...(await original<object>()), fetchInvoices: api.invoices}));

import CustomersPage from "../routes/customers/+page.svelte";
import InvoicesPage from "../routes/invoices/+page.svelte";
import {row} from "./components/customerFixtures.js";
import {entry} from "./components/fixtures.js";

describe("COMPONENT list pages read their URL on a direct load", () => {
    beforeEach(() => {
        api.customers.mockReset().mockResolvedValue([row({})]);
        api.invoices.mockReset().mockResolvedValue([entry({})]);
        nav.goto.mockReset();
    });

    it("the customer list asks for the URL's filters and shows them", async () => {
        nav.url = new URL("http://localhost/app/customers?overdue=true&sort=owed&dir=desc");
        render(CustomersPage);
        await waitFor(() => expect(api.customers).toHaveBeenCalled());
        expect(api.customers).toHaveBeenCalledWith(undefined, "overdue=true&sort=owed&dir=desc");
        expect(screen.getByTestId("customer-filter-overdue").getAttribute("aria-pressed")).toBe("true");
        await waitFor(() => expect(screen.getByTestId("customer-count").textContent).toBe("1 customer match"));
        expect(screen.getByRole("columnheader", {name: /Owed/}).getAttribute("aria-sort")).toBe("descending");
    });

    it("the invoice list asks for the URL's filters and shows them", async () => {
        nav.url = new URL("http://localhost/app/invoices?status=overdue&aged=91-");
        render(InvoicesPage);
        await waitFor(() => expect(api.invoices).toHaveBeenCalled());
        expect(api.invoices).toHaveBeenCalledWith(undefined, "status=overdue&aged=91%2B");
        expect((screen.getByTestId("invoice-filter-status") as HTMLSelectElement).value).toBe("overdue");
        await waitFor(() => expect(screen.getByTestId("invoice-count").textContent).toBe("1 invoice match"));
    });
});
