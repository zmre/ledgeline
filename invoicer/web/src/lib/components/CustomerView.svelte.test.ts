// A customer, read-only, mounted: names, never `[[…]]` or paths; contracts
// open inside the app (a note) or in their viewer (a file).
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {TemplateEntry} from "#lib/invoices.js";
import CustomerView from "./CustomerView.svelte";
import {customerX} from "./customerFixtures.js";

const templates: TemplateEntry[] = [{path: "/invoices/tpl/myco-default.html", name: "MyCo default", kinds: ["invoice"], starter: false, installed: true}];
const hrefs = {
    noteHref: (path: string) => `/app/notes/${path.replace(/\.md$/, "")}`,
    fileHref: (path: string) => `/${path}`,
};

describe("COMPONENT CustomerView", () => {
    it("shows the contact, the invoicing settings and the portal", () => {
        render(CustomerView, {customer: customerX(), templates, ...hrefs});
        const emails = screen.getByTestId("view-emails");
        expect(emails.textContent).toContain("billing");
        expect(emails.textContent).toContain("accounts_receivable@customerx.com");
        expect(screen.getByTestId("view-addresses").textContent).toContain("Springfield, CO 80000");
        expect(screen.getByTestId("view-terms").textContent).toContain("Net 30 days");
        // The template by its name, not its path.
        expect(screen.getByTestId("view-template").textContent).toContain("MyCo default");
        expect(screen.getByTestId("view-template").textContent).not.toContain("/invoices/tpl");
        expect(screen.getByTestId("view-portal").textContent).toContain("Coupa");
        expect(screen.getByTestId("view-portal-reference")).toBeDefined();
        expect(screen.queryByTestId("view-company")).toBeNull();
    });

    it("names contracts, opens a note in the app and a file in its viewer", async () => {
        const onOpenFile = vi.fn();
        render(CustomerView, {customer: customerX(), templates, ...hrefs, onOpenFile});
        const contracts = within(screen.getByTestId("view-contracts"));
        expect(screen.getByTestId("view-contracts").textContent).not.toContain("[[");
        expect(contracts.getByRole("link", {name: "Customer X MSA 2026"}).getAttribute("href")).toBe("/app/notes/customers/contracts/Customer X MSA 2026");
        await fireEvent.click(contracts.getByRole("button", {name: "SOW 3.pdf"}));
        expect(onOpenFile).toHaveBeenCalledWith("customers/contracts/SOW 3.pdf");
        expect(contracts.getByText("missing")).toBeDefined();
    });

    it("without a viewer, a file opens in a new tab", () => {
        render(CustomerView, {customer: customerX(), templates, ...hrefs});
        const sow = within(screen.getByTestId("view-contracts")).getByRole("link", {name: "SOW 3.pdf"});
        expect(sow.getAttribute("href")).toBe("/customers/contracts/SOW 3.pdf");
        expect(sow.getAttribute("target")).toBe("_blank");
    });

    it("shows a person's company by name, linked to its page", () => {
        const fred = customerX({title: "Fred Smith", kind: "person"});
        fred.form = {...fred.form, kind: "person", company: "[[Customer X]]", billing_portal: null, invoice_template: "", terms: null};
        render(CustomerView, {customer: fred, templates, companyHref: "/app/customers/customers/Customer X", ...hrefs});
        const company = screen.getByTestId("view-company");
        expect(company.textContent).not.toContain("[[");
        expect(within(company).getByRole("link", {name: "Customer X"}).getAttribute("href")).toBe("/app/customers/customers/Customer X");
        expect(screen.getByTestId("view-template").textContent).toContain("The default");
        expect(screen.getByTestId("view-terms").textContent).toContain("The books' default");
        expect(screen.queryByTestId("view-portal")).toBeNull();
    });
});
