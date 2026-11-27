// The customer card's header, mounted: the monogram and name, the subtitle
// ("Organization · Billed via Coupa · Net 30"), the actions (each a real
// link or button with one default target, the first one primary), and the
// money line (moved here from the account column).
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import CustomerCardHeader from "./CustomerCardHeader.svelte";
import {customerX} from "./customerFixtures.js";

const base = {newInvoiceHref: "/app/new?customer=Customer%20X"};

describe("COMPONENT CustomerCardHeader", () => {
    it("says who it is: a monogram, the name, kind, portal and terms", () => {
        render(CustomerCardHeader, {customer: customerX(), ...base});
        const monogram = screen.getByTestId("customer-monogram");
        expect(monogram.textContent?.trim()).toBe("CX");
        expect(monogram.className).toContain("rounded-box");
        expect(monogram.getAttribute("aria-hidden")).toBe("true");
        expect(screen.getByRole("heading", {level: 1}).textContent).toBe("Customer X");
        expect(screen.getByTestId("customer-subtitle").textContent?.replace(/\s+/g, " ").trim()).toBe("Organization · Billed via Coupa · Net 30");
        expect(screen.queryByTestId("view-company")).toBeNull();
    });

    it("names a person's company as a link to its page, in a circle", () => {
        const fred = customerX({title: "Fred Smith", kind: "person"});
        fred.form = {...fred.form, kind: "person", company: "[[Customer X]]", billing_portal: null, terms: null};
        render(CustomerCardHeader, {customer: fred, ...base, companyHref: "/app/customers/customers/Customer X"});
        expect(screen.getByTestId("customer-monogram").textContent?.trim()).toBe("FS");
        expect(screen.getByTestId("customer-monogram").className).toContain("rounded-full");
        const company = screen.getByTestId("view-company");
        expect(company.textContent).not.toContain("[[");
        expect(within(company).getByRole("link", {name: "Customer X"}).getAttribute("href")).toBe("/app/customers/customers/Customer X");
        expect(screen.getByTestId("customer-subtitle").textContent?.replace(/\s+/g, " ").trim()).toBe("Person · Customer X");
    });

    it("offers Email, Call, the portal and New invoice, the default targets first", async () => {
        const onOpenPortal = vi.fn();
        render(CustomerCardHeader, {customer: customerX(), ...base, onOpenPortal});
        const actions = within(screen.getByTestId("view-actions"));
        const email = screen.getByTestId("action-email");
        // Billing before work, as invoices pick.
        expect(email.getAttribute("href")).toBe("mailto:accounts_receivable@customerx.com");
        expect(email.className).toContain("btn-primary");
        expect(email.getAttribute("aria-keyshortcuts")).toBe("m");
        // Two emails: a menu with both, by label.
        const menu = screen.getByTestId("action-email-menu");
        expect(menu.getAttribute("aria-label")).toBe("Choose an email");
        const items = within(menu.closest("details") as HTMLElement)
            .getAllByRole("link")
            .map((a) => a.textContent);
        expect(items).toEqual(["Billing · accounts_receivable@customerx.com", "Work · hello@customerx.com"]);
        expect(screen.getByTestId("action-call").getAttribute("href")).toBe("tel:+13035550142");
        expect(screen.queryByTestId("action-call-menu")).toBeNull();
        expect(screen.getByTestId("action-call").className).not.toContain("btn-primary");
        await fireEvent.click(actions.getByRole("button", {name: /Open Coupa/}));
        expect(onOpenPortal).toHaveBeenCalledOnce();
        const fresh = screen.getByTestId("action-new-invoice");
        expect(fresh.getAttribute("href")).toBe("/app/new?customer=Customer%20X");
        expect(fresh.getAttribute("aria-keyshortcuts")).toBe("n");
    });

    it("without an email, the next action is primary; without a portal link, no portal button", () => {
        const quiet = customerX();
        quiet.form = {...quiet.form, emails: [], phones: []};
        render(CustomerCardHeader, {customer: quiet, ...base});
        expect(screen.queryByTestId("action-email")).toBeNull();
        expect(screen.queryByTestId("action-portal")).toBeNull();
        expect(screen.getByTestId("action-new-invoice").className).toContain("btn-primary");
    });

    it("Escape closes the email menu and gives its toggle focus", async () => {
        render(CustomerCardHeader, {customer: customerX(), ...base});
        const toggle = screen.getByTestId("action-email-menu");
        const menu = toggle.closest("details") as HTMLDetailsElement;
        menu.open = true;
        await fireEvent.keyDown(toggle, {key: "Escape"});
        expect(menu.open).toBe(false);
        expect(document.activeElement).toBe(toggle);
    });

    it("shows what is owed, overdue as a badge to the open invoices, and the last payment", () => {
        render(CustomerCardHeader, {
            customer: customerX({
                account: {
                    balance: [{commodity: "$", amount: "$16,482.21"}],
                    overdue: 2,
                    oldest_overdue: "2026-10-03",
                    last_payment: {date: "2026-10-12", amount: "$4,000.00"},
                },
            }),
            ...base,
        });
        expect(screen.getByTestId("account-balance").textContent).toBe("$16,482.21");
        const overdue = screen.getByTestId("account-overdue");
        expect(overdue.textContent).toMatch(/^2 overdue · since Oct 3/);
        expect(overdue.getAttribute("href")).toBe("#open-invoices");
        expect(overdue.className).toContain("badge-error");
        expect(screen.getByTestId("account-last-payment").textContent).toMatch(/^\$4,000\.00 · Oct 12/);
    });

    it("hides the money line with no invoices, and the actions while editing", () => {
        const view = render(CustomerCardHeader, {customer: customerX({documents: []}), ...base});
        expect(screen.queryByTestId("account-balance")).toBeNull();
        view.unmount();
        render(CustomerCardHeader, {customer: customerX(), ...base, editing: true});
        expect(screen.queryByTestId("view-actions")).toBeNull();
        expect(screen.queryByTestId("account-balance")).toBeNull();
        expect(screen.getByTestId("customer-heading").textContent).toBe("Customer X");
    });
});
