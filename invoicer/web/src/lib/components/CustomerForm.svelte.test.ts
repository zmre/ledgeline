// The customer form, mounted: it edits the form it is given in place (the
// page sends only what changed), with labelled lists, a person's company and
// a vendor portal.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {changedFields, cloneForm, emptyForm} from "#lib/customers.js";
import type {TemplateEntry} from "#lib/invoices.js";
import CustomerForm from "./CustomerForm.svelte";
import {customerX} from "./customerFixtures.js";

const templates: TemplateEntry[] = [
    {path: "/invoices/tpl/myco-default.html", name: "MyCo default", kinds: ["invoice"], starter: false, installed: true},
    {path: "/invoices/tpl/modern.html", name: "Modern", kinds: ["invoice", "estimate"], starter: true, installed: false},
    {path: "/invoices/tpl/estimate.html", name: "Estimate only", kinds: ["estimate"], starter: false, installed: true},
];

describe("COMPONENT CustomerForm", () => {
    it("fills in a customer's fields and edits them in place", async () => {
        const original = customerX().form;
        const form = $state(cloneForm(original));
        render(CustomerForm, {form, templates});
        expect((screen.getByTestId("customer-title") as HTMLInputElement).value).toBe("Customer X");
        const emails = within(screen.getByTestId("labelled-emails"));
        expect((emails.getByLabelText("Emails 1") as HTMLInputElement).value).toBe("accounts_receivable@customerx.com");
        expect((emails.getByLabelText("Emails 2: label") as HTMLInputElement).value).toBe("work");
        const address = within(screen.getByTestId("labelled-addresses")).getByLabelText("Addresses 1") as HTMLTextAreaElement;
        expect(address.value).toBe("1234 Main St. #555\nSpringfield, CO 80000");
        // A person's company only shows for a person.
        expect(screen.queryByTestId("customer-company")).toBeNull();

        await fireEvent.input(screen.getByRole("spinbutton", {name: "Terms (net days)"}), {target: {value: "45"}});
        await fireEvent.click(emails.getByRole("button", {name: "Move Emails 2 up"}));
        expect(changedFields(original, form)).toEqual({
            terms: 45,
            emails: [
                {label: "work", value: "hello@customerx.com"},
                {label: "billing", value: "accounts_receivable@customerx.com"},
            ],
        });
    });

    it("offers invoice templates only, and keeps one it doesn't know", () => {
        const form = $state({...cloneForm(customerX().form), invoice_template: "/old/gone.html"});
        render(CustomerForm, {form, templates});
        const select = screen.getByRole("combobox", {name: "Invoice template"}) as HTMLSelectElement;
        expect([...select.options].map((o) => o.textContent)).toEqual(["The default", "MyCo default", "Modern (starter)", "/old/gone.html (not found)"]);
        expect(select.value).toBe("/old/gone.html");
    });

    it("adds and removes labelled values", async () => {
        const form = $state(emptyForm());
        render(CustomerForm, {form, creating: true});
        const phones = within(screen.getByTestId("labelled-phones"));
        expect(phones.queryAllByTestId("phones-row")).toHaveLength(0);
        await fireEvent.click(phones.getByRole("button", {name: "Add phone"}));
        await fireEvent.input(phones.getByLabelText("Phones 1"), {target: {value: "555 0100"}});
        expect(form.phones).toEqual([{label: "billing", value: "555 0100"}]);
        await fireEvent.click(phones.getByRole("button", {name: "Add phone"}));
        expect(form.phones[1].label).toBe("work");
        await fireEvent.click(phones.getByRole("button", {name: "Remove Phones 1"}));
        expect(form.phones).toEqual([{label: "work", value: ""}]);
    });

    it("shows a person's company, and a portal once it is switched on", async () => {
        const form = $state(emptyForm("person"));
        render(CustomerForm, {form, companies: ["Customer X", "Contoso"]});
        const company = screen.getByTestId("customer-company") as HTMLInputElement;
        // Suggested by name, never as a [[link]].
        const list = document.getElementById(company.getAttribute("list") ?? "");
        expect([...(list?.querySelectorAll("option") ?? [])].map((o) => o.value)).toEqual(["Customer X", "Contoso"]);
        await fireEvent.input(company, {target: {value: "Customer X"}});
        expect(form.company).toBe("Customer X");
        expect(screen.queryByRole("textbox", {name: "Portal"})).toBeNull();
        await fireEvent.click(screen.getByTestId("portal-toggle"));
        await fireEvent.input(screen.getByRole("textbox", {name: "Portal"}), {target: {value: "Ariba"}});
        await fireEvent.click(screen.getByRole("checkbox", {name: /rejects invoices without a reference/}));
        expect(form.billing_portal).toMatchObject({name: "Ariba", requires_reference: true});
        await fireEvent.click(screen.getByTestId("portal-toggle"));
        expect(form.billing_portal).toBeNull();
    });

    it("lists contracts by name, removes one, and adds one from the picker", async () => {
        const original = customerX().form;
        const form = $state(cloneForm(original));
        const files = {
            pick: false,
            open: false,
            files: [
                {path: "customers/contracts/Customer X MSA 2026.md", name: "Customer X MSA 2026", kind: "contract", entry: "[[Customer X MSA 2026]]"},
                {path: "customers/contracts/SOW 4.pdf", name: "SOW 4.pdf", kind: null, entry: "/customers/contracts/SOW 4.pdf"},
            ],
        };
        vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify(files), {status: 200, headers: {"Content-Type": "application/json"}}));
        render(CustomerForm, {form, contractNames: {"/customers/contracts/SOW 3.pdf": "SOW 3 (signed)"}});
        const rows = () => screen.getAllByTestId("contract-row").map((r) => r.textContent?.replace("✕", "").trim());
        // The server's name, else the entry's: never [[…]] or a path.
        expect(rows()).toEqual(["Customer X MSA 2026", "SOW 3 (signed)", "gone.pdf"]);
        await fireEvent.click(screen.getByRole("button", {name: "Remove contract gone.pdf"}));
        expect(form.contracts).toEqual(["[[Customer X MSA 2026]]", "/customers/contracts/SOW 3.pdf"]);

        await fireEvent.click(screen.getByTestId("add-contract"));
        const picker = await screen.findByTestId("contract-picker");
        // The MSA is in the list already: not offered again.
        const offered = await within(picker).findAllByTestId("contract-candidate");
        expect(offered.map((b) => b.textContent)).toEqual([expect.stringContaining("SOW 4.pdf")]);
        await fireEvent.click(offered[0]);
        expect(screen.queryByTestId("contract-picker")).toBeNull();
        expect(form.contracts.at(-1)).toBe("/customers/contracts/SOW 4.pdf");
        expect(rows().at(-1)).toBe("SOW 4.pdf");
        expect(changedFields(original, form)).toEqual({
            contracts: ["[[Customer X MSA 2026]]", "/customers/contracts/SOW 3.pdf", "/customers/contracts/SOW 4.pdf"],
        });
        vi.restoreAllMocks();
    });
});
