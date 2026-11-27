// A customer, read-only, mounted: the contact card's body. Names, never
// `[[…]]` or paths; labels for people; every contact value its own action
// with a copy button; empty optional groups hidden and named in one line;
// contracts open inside the app (a note) or in their viewer (a file).
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {copyStatus} from "#lib/copy.svelte.js";
import type {TemplateEntry} from "#lib/invoices.js";
import CustomerView from "./CustomerView.svelte";
import {customerX} from "./customerFixtures.js";

const templates: TemplateEntry[] = [{path: "/invoices/tpl/myco-default.html", name: "MyCo default", kinds: ["invoice"], starter: false, installed: true}];
const hrefs = {
    noteHref: (path: string) => `/app/notes/${path.replace(/\.md$/, "")}`,
    fileHref: (path: string) => `/${path}`,
};

afterEach(() => {
    vi.restoreAllMocks();
});

describe("COMPONENT CustomerView", () => {
    it("shows the contact with labels for people, each value its own action", () => {
        render(CustomerView, {customer: customerX(), templates, ...hrefs});
        const emails = within(screen.getByTestId("view-emails"));
        expect(emails.getByText("Billing")).toBeDefined();
        expect(emails.getByText("Work")).toBeDefined();
        expect(emails.getByRole("link", {name: "accounts_receivable@customerx.com"}).getAttribute("href")).toBe("mailto:accounts_receivable@customerx.com");
        const phone = within(screen.getByTestId("view-phones")).getByRole("link", {name: "+1 (303) 555-0142"});
        expect(phone.getAttribute("href")).toBe("tel:+13035550142");
        const address = screen.getByTestId("view-addresses").querySelector("address");
        expect(address?.textContent).toBe("1234 Main St. #555\nSpringfield, CO 80000");
        // A copy button per value, named for it.
        const copies = screen.getAllByTestId("copy-button").map((b) => b.getAttribute("aria-label"));
        expect(copies).toEqual(
            expect.arrayContaining(["Copy billing email", "Copy work email", "Copy billing phone", "Copy billing address", "Copy supplier ID"])
        );
    });

    it("hands mail and phone links to the window when asked", () => {
        render(CustomerView, {customer: customerX(), templates, ...hrefs, external: (href: string) => `/handoff?url=${encodeURIComponent(href)}`});
        const link = within(screen.getByTestId("view-emails")).getByRole("link", {name: "accounts_receivable@customerx.com"});
        expect(link.getAttribute("href")).toBe("/handoff?url=mailto%3Aaccounts_receivable%40customerx.com");
        expect(link.getAttribute("rel")).toBe("external");
    });

    it("copies a value, says so, and says how to copy by hand when it can't", async () => {
        const writeText = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("denied"));
        Object.defineProperty(navigator, "clipboard", {value: {writeText}, configurable: true});
        render(CustomerView, {customer: customerX(), templates, ...hrefs});
        const button = screen.getByRole("button", {name: "Copy supplier ID"});
        await fireEvent.click(button);
        expect(writeText).toHaveBeenCalledWith("SUP-44821");
        await vi.waitFor(() => expect(button.getAttribute("aria-label")).toBe("Copied"));
        expect(screen.getByTestId("copy-status").textContent).toBe("Copied supplier ID");
        expect(screen.getByTestId("copy-status").getAttribute("role")).toBe("status");
        await fireEvent.click(screen.getByRole("button", {name: "Copy billing email"}));
        await vi.waitFor(() => expect(copyStatus.message).toBe("Couldn't copy the billing email; select it and copy by hand."));
    });

    it("shows how the customer is invoiced, with defaults in plain words", () => {
        const view = render(CustomerView, {customer: customerX(), templates, ...hrefs});
        expect(screen.getByTestId("view-terms").textContent).toContain("Net 30");
        expect(screen.getByTestId("view-terms").textContent).not.toContain("days");
        // The template by its name, not its path.
        expect(screen.getByTestId("view-template").textContent).toContain("MyCo default");
        expect(screen.getByTestId("view-template").textContent).not.toContain("/invoices/tpl");
        expect(screen.getByTestId("view-revenue-account").textContent).toContain("revenue:consulting");
        // The name in the books is the title: not repeated.
        expect(screen.queryByTestId("view-ledger-customer")).toBeNull();
        view.unmount();
        const bare = customerX();
        bare.form = {...bare.form, terms: null, invoice_template: "", revenue_account: "", ledger_customer: "Customer X Holdings"};
        render(CustomerView, {customer: bare, templates, ...hrefs});
        expect(screen.getByTestId("view-terms").textContent).toContain("Books default");
        expect(screen.getByTestId("view-template").textContent).toContain("Default");
        expect(screen.getByTestId("view-revenue-account").textContent).toContain("Books default");
        expect(screen.getByTestId("view-ledger-customer").textContent).toContain("Customer X Holdings");
    });

    it("shows the portal: its host, the login and supplier ID to copy, the PO rule", async () => {
        const onOpenPortal = vi.fn();
        render(CustomerView, {customer: customerX(), templates, ...hrefs, onOpenPortal});
        const portal = within(screen.getByTestId("view-portal"));
        expect(portal.getByRole("heading", {name: "Coupa portal"})).toBeDefined();
        const url = portal.getByRole("button", {name: /customerx\.coupahost\.com/});
        expect(url.getAttribute("title")).toBe("https://customerx.coupahost.com/");
        await fireEvent.click(url);
        expect(onOpenPortal).toHaveBeenCalledOnce();
        expect(screen.getByTestId("view-portal-login").textContent).toContain("finance@mycompany.com");
        expect(screen.getByTestId("view-portal-reference").textContent).toContain("PO required");
        expect(screen.getByTestId("view-portal-instructions")).toBeDefined();
    });

    it("names contracts with their kind, opens a note in the app and a file in its viewer", async () => {
        const onOpenFile = vi.fn();
        render(CustomerView, {customer: customerX(), templates, ...hrefs, onOpenFile});
        const contracts = within(screen.getByTestId("view-contracts"));
        expect(screen.getByTestId("view-contracts").textContent).not.toContain("[[");
        expect(contracts.getByRole("link", {name: "Customer X MSA 2026"}).getAttribute("href")).toBe("/app/notes/customers/contracts/Customer X MSA 2026");
        const rows = screen.getAllByTestId("view-contract");
        expect(rows[0].textContent).toContain("Note");
        expect(rows[1].textContent).toContain("PDF");
        await fireEvent.click(contracts.getByRole("button", {name: "SOW 3.pdf"}));
        expect(onOpenFile).toHaveBeenCalledWith("customers/contracts/SOW 3.pdf");
        expect(contracts.getByText("Missing")).toBeDefined();
    });

    it("without a viewer, a file opens in a new tab", () => {
        render(CustomerView, {customer: customerX(), templates, ...hrefs});
        const sow = within(screen.getByTestId("view-contracts")).getByRole("link", {name: "SOW 3.pdf"});
        expect(sow.getAttribute("href")).toBe("/customers/contracts/SOW 3.pdf");
        expect(sow.getAttribute("target")).toBe("_blank");
    });

    it("marks a contract outside the books, and opens it in the window by its entry", async () => {
        const outside = customerX({
            contracts: [
                {
                    kind: "outside",
                    path: "~/Documents/MSA 2026.pdf",
                    location: "~/Documents/MSA 2026.pdf",
                    found: "/Users/me/Documents/MSA 2026.pdf",
                    name: "MSA 2026.pdf",
                },
                {kind: "outside", path: "file:///Volumes/Share/SOW.pdf", location: "/Volumes/Share/SOW.pdf", found: null, name: "SOW.pdf"},
            ],
        });
        const onOpenFile = vi.fn();
        render(CustomerView, {customer: outside, templates, ...hrefs, onOpenFile});
        const [msa, sow] = screen.getAllByTestId("view-contract");
        expect(within(msa).getByTestId("view-contract-outside").textContent).toBe("Outside the books");
        // Openable here: no path shown.
        expect(msa.textContent).not.toContain("~/");
        await fireEvent.click(within(msa).getByRole("button", {name: "MSA 2026.pdf"}));
        expect(onOpenFile).toHaveBeenCalledWith("~/Documents/MSA 2026.pdf");
        // Missing: said so, with where it was looked for.
        expect(sow.textContent).toContain("Missing");
        expect(sow.textContent).toContain("/Volumes/Share/SOW.pdf");
        expect(within(sow).queryByRole("button")).toBeNull();
        expect(screen.getByTestId("view-contracts").textContent).not.toContain("file:");
    });

    it("in a browser, an outside contract is its name and place, as text", () => {
        const outside = customerX({
            contracts: [{kind: "outside", path: "~/Documents/MSA.pdf", location: "~/Documents/MSA.pdf", found: "/Users/me/Documents/MSA.pdf", name: "MSA.pdf"}],
        });
        render(CustomerView, {customer: outside, templates, ...hrefs});
        const contract = screen.getByTestId("view-contract");
        expect(within(contract).queryByRole("link")).toBeNull();
        expect(within(contract).queryByRole("button")).toBeNull();
        expect(contract.textContent).toContain("MSA.pdf");
        expect(contract.textContent).toContain("~/Documents/MSA.pdf");
    });

    it("shows the notes read-only, folding away the closed POs the app writes", () => {
        const customer = customerX();
        customer.form = {
            ...customer.form,
            body: "Fred Smith is the champion.\n\n## Closed POs\n\n- PO #1; nothing invoiced; closed 2026-01-01.\n- PO #2; nothing invoiced; closed 2026-02-01.\n",
        };
        render(CustomerView, {customer, templates, ...hrefs});
        const notes = within(screen.getByTestId("view-notes")).getByRole("textbox", {name: "Notes"}) as HTMLTextAreaElement;
        expect(notes.value).toBe("Fred Smith is the champion.");
        expect(notes.readOnly).toBe(true);
        const closed = screen.getByTestId("view-closed-pos") as HTMLDetailsElement;
        expect(closed.open).toBe(false);
        expect(closed.querySelector("summary")?.textContent).toBe("Closed POs (2)");
        // Short notes aren't clamped.
        expect(screen.queryByTestId("notes-show-all")).toBeNull();
    });

    it("hides empty optional groups, and says what Edit can add", () => {
        const bare = customerX({contracts: [], snippets: []});
        bare.form = {...bare.form, emails: [], phones: [], addresses: [], billing_portal: null, body: ""};
        render(CustomerView, {customer: bare, templates, ...hrefs});
        expect(screen.getByTestId("view-no-contact").textContent).toBe("No contact details yet. Edit to add an email, phone or address.");
        for (const hidden of ["view-contracts", "view-snippets", "view-portal", "view-notes"]) expect(screen.queryByTestId(hidden)).toBeNull();
        expect(screen.getByTestId("view-missing-extras").textContent).toBe("Add contracts, a vendor portal, default snippets or notes with Edit.");
    });

    it("shows default snippets by name as chips", () => {
        render(CustomerView, {customer: customerX({snippets: ["[[Net 30 ACH]]", "[[Late fee]]"]}), templates, ...hrefs});
        const chips = within(screen.getByTestId("view-snippets")).getAllByRole("listitem");
        expect(chips.map((c) => c.textContent)).toEqual(["Net 30 ACH", "Late fee"]);
        // Everything optional is there: no closing line.
        expect(screen.queryByTestId("view-missing-extras")).toBeNull();
    });
});
