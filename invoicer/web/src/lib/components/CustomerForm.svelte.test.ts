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

describe("COMPONENT CustomerForm open POs (PLAN 5b)", () => {
    it("adds, edits and closes POs in the form", async () => {
        const original = customerX().form;
        original.open_pos = [{reference: "PO #1", amount: "$1,000", expires: null, was: "PO #1"}];
        const form = $state(cloneForm(original));
        render(CustomerForm, {form, templates});
        expect((screen.getByTestId("form-po-reference") as HTMLInputElement).value).toBe("PO #1");
        await fireEvent.click(screen.getByTestId("form-po-add"));
        const refs = screen.getAllByTestId("form-po-reference");
        await fireEvent.input(refs[1], {target: {value: "PO #2"}});
        await fireEvent.input(screen.getAllByTestId("form-po-amount")[1], {target: {value: "$500"}});
        expect(changedFields(original, form).open_pos).toEqual([
            {reference: "PO #1", amount: "$1,000", expires: null, attachment: null, was: "PO #1"},
            {reference: "PO #2", amount: "$500", expires: null, attachment: null, was: null},
        ]);
        await fireEvent.click(screen.getByRole("button", {name: "Close PO #1"}));
        expect(changedFields(original, form).open_pos).toEqual([{reference: "PO #2", amount: "$500", expires: null, attachment: null, was: null}]);
    });

    it("leaves open_pos out of a save when only a blank row was added", async () => {
        const original = customerX().form;
        const form = $state(cloneForm(original));
        render(CustomerForm, {form, templates});
        await fireEvent.click(screen.getByTestId("form-po-add"));
        expect(changedFields(original, form)).toEqual({});
    });
});

/** `fetch` answering the picker: the books' files, and the system dialog's choice. */
function filesApi(picked: unknown): void {
    const files = {
        pick: true,
        open: true,
        files: [{path: "pos/PO 1.pdf", name: "PO 1.pdf", kind: null, entry: "/pos/PO 1.pdf"}],
    };
    vi.spyOn(globalThis, "fetch").mockImplementation((input) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        const body = url.includes("/api/files/pick") ? {cancelled: false, file: picked} : files;
        return Promise.resolve(new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}}));
    });
}

describe("COMPONENT CustomerForm PO attachments", () => {
    it("attaches a file to a PO by name, from the books or outside them, and removes it", async () => {
        const original = customerX().form;
        original.open_pos = [{reference: "PO #1", amount: null, expires: null, attachment: null, was: "PO #1"}];
        const form = $state(cloneForm(original));
        filesApi({path: "~/Documents/PO 1 signed.pdf", name: "PO 1 signed.pdf", kind: null, entry: "~/Documents/PO 1 signed.pdf", outside: true});
        render(CustomerForm, {form, templates});
        const row = within(screen.getByTestId("form-po-attachment"));
        await fireEvent.click(row.getByRole("button", {name: "Attach a file to PO #1"}));
        const picker = await screen.findByTestId("contract-picker");
        // Worded for the PO, not a contract.
        expect(within(picker).getByRole("heading").textContent).toBe("Attach a file to PO #1");
        expect(picker.textContent).not.toMatch(/contract/i);
        await fireEvent.click(await within(picker).findByTestId("contract-candidate"));
        expect(form.open_pos[0].attachment).toBe("/pos/PO 1.pdf");
        expect(screen.getByTestId("form-po-attachment-name").textContent).toBe("PO 1.pdf");
        expect(changedFields(original, form).open_pos).toEqual([{reference: "PO #1", amount: null, expires: null, attachment: "/pos/PO 1.pdf", was: "PO #1"}]);

        // Removed, then chosen with the system's dialog from outside the books.
        await fireEvent.click(screen.getByRole("button", {name: "Remove the attachment from PO #1"}));
        expect(form.open_pos[0].attachment).toBeNull();
        await fireEvent.click(screen.getByRole("button", {name: "Attach a file to PO #1"}));
        await fireEvent.click(await screen.findByTestId("contract-choose-file"));
        await vi.waitFor(() => expect(form.open_pos[0].attachment).toBe("~/Documents/PO 1 signed.pdf"));
        const attached = screen.getByTestId("form-po-attachment");
        expect(attached.textContent).toContain("PO 1 signed.pdf");
        expect(attached.textContent).toContain("Outside the books");
        expect(attached.textContent).not.toContain("~/");
        vi.restoreAllMocks();
    });

    it("marks a contract outside the books", () => {
        const form = $state({...cloneForm(customerX().form), contracts: ["file:///Volumes/Share/MSA%202026.pdf"]});
        render(CustomerForm, {form, templates});
        const row = screen.getByTestId("contract-row");
        expect(row.textContent).toContain("MSA 2026.pdf");
        expect(row.textContent).toContain("Outside the books");
        expect(row.textContent).not.toContain("file:");
    });
});

describe("COMPONENT CustomerForm revenue account and notes", () => {
    it("suggests the books' accounts as you type, revenue first", async () => {
        const form = $state(emptyForm());
        render(CustomerForm, {
            form,
            creating: true,
            accounts: ["expenses:consult", "revenue:consulting", "assets:cash"],
            revenueAccounts: ["revenue:consulting"],
        });
        const field = screen.getByRole("combobox", {name: "Revenue account"});
        await fireEvent.input(field, {target: {value: "consult"}});
        const list = await screen.findByRole("listbox", {name: "Account suggestions"});
        const options = within(list)
            .getAllByRole("option")
            .map((o) => o.textContent?.trim());
        expect(options).toEqual(["revenue:consulting", "expenses:consult"]);
        await fireEvent.keyDown(field, {key: "Enter"});
        expect(form.revenue_account).toBe("revenue:consulting");
    });

    it("edits the customer's notes", async () => {
        const original = customerX().form;
        const form = $state(cloneForm(original));
        render(CustomerForm, {form, templates});
        const notes = screen.getByRole("textbox", {name: "Notes"}) as HTMLTextAreaElement;
        expect(notes.value).toBe("Fred Smith is the champion.\n");
        await fireEvent.input(notes, {target: {value: "Fred Smith is the champion.\nLee pays.\n"}});
        expect(changedFields(original, form)).toEqual({body: "Fred Smith is the champion.\nLee pays.\n"});
    });
});
