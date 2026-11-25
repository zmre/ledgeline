// The customer form's pure parts: what a save sends (only what changed), the
// SPA's route and API paths, and the account in words.
import {afterEach, describe, expect, it, vi} from "vitest";
import {
    ApiStatusError,
    balanceText,
    changedFields,
    cloneForm,
    contractName,
    createCustomer,
    customerApiUrl,
    emptyForm,
    isDirty,
    kindLabel,
    lastPaymentText,
    linkName,
    newCustomerFields,
    routeParam,
    saveCustomer,
    type CustomerForm,
} from "./customers.js";

/** The form as `GET /api/customers/customers/Fabrikam.md` fills it. */
function fabrikam(): CustomerForm {
    return {
        title: "Fabrikam",
        kind: "organization",
        emails: [{label: "billing", value: "ap@fabrikam.example"}],
        phones: [],
        addresses: [{label: "billing", value: "1 Fabrikam Way\nSpringfield, IL 62701\nUSA"}],
        company: "",
        terms: 30,
        revenue_account: "",
        invoice_template: "/invoices/tpl/myco-default.html",
        ledger_customer: "",
        contracts: [],
        billing_portal: null,
        open_pos: [],
    };
}

describe("UNIT customer form", () => {
    it("sends only the fields that changed", () => {
        const original = fabrikam();
        const edited = cloneForm(original);
        expect(changedFields(original, edited)).toEqual({});
        expect(isDirty(original, edited)).toBe(false);
        edited.terms = 45;
        edited.phones.push({label: "billing", value: " 555 0100 "});
        expect(changedFields(original, edited)).toEqual({terms: 45, phones: [{label: "billing", value: "555 0100"}]});
        expect(isDirty(original, edited)).toBe(true);
    });

    it("treats white space and blank rows as no change", () => {
        const original = fabrikam();
        const edited = cloneForm(original);
        edited.title = " Fabrikam ";
        edited.emails.push({label: "work", value: "  "});
        edited.contracts.push("");
        expect(changedFields(original, edited)).toEqual({});
    });

    it("sends an emptied list or text as a removal, and a portal as a whole", () => {
        const original = fabrikam();
        const edited = cloneForm(original);
        edited.emails = [];
        edited.invoice_template = "";
        edited.terms = null;
        edited.billing_portal = {name: "Ariba", url: null, login: null, supplier_id: null, requires_reference: true, instructions: ""};
        expect(changedFields(original, edited)).toEqual({
            emails: [],
            invoice_template: "",
            terms: null,
            billing_portal: {name: "Ariba", url: null, login: null, supplier_id: null, requires_reference: true, instructions: null},
        });
        // An empty portal is no portal.
        const blank = cloneForm(original);
        blank.billing_portal = {name: "", url: null, login: null, supplier_id: null, requires_reference: false, instructions: null};
        expect(changedFields(original, blank)).toEqual({});
    });

    it("sends a label-less value with a null label", () => {
        const original = fabrikam();
        const edited = cloneForm(original);
        edited.emails[0].label = " ";
        expect(changedFields(original, edited)).toEqual({emails: [{label: null, value: "ap@fabrikam.example"}]});
    });

    it("creates with what was filled in", () => {
        const form = emptyForm("person");
        form.title = "Lee Park";
        form.company = "Fabrikam";
        form.emails[0].value = "lee@fabrikam.example";
        expect(newCustomerFields(form)).toEqual({
            kind: "person",
            title: "Lee Park",
            company: "Fabrikam",
            emails: [{label: "billing", value: "lee@fabrikam.example"}],
        });
        // The kind is always sent: the server needs it to create.
        expect(newCustomerFields(emptyForm("organization"))).toEqual({kind: "organization"});
    });
});

describe("UNIT customer paths and words", () => {
    it("routes by the note's path without .md, each segment encoded", () => {
        expect(routeParam("customers/Customer X.md")).toBe("customers/Customer%20X");
        expect(routeParam("/clients/a#b/C?.md")).toBe("clients/a%23b/C%3F");
        expect(customerApiUrl("customers/Customer X")).toBe("/api/customers/customers/Customer%20X");
        expect(customerApiUrl("Café & Co")).toBe("/api/customers/Caf%C3%A9%20%26%20Co");
    });

    it("says what is owed and the last payment", () => {
        const none = {balance: [], overdue: 0, oldest_overdue: null, last_payment: null};
        expect(balanceText(none)).toBe("Nothing owed");
        expect(lastPaymentText(none)).toBe("—");
        const some = {
            balance: [
                {commodity: "$", amount: "$2,000.00"},
                {commodity: "EUR", amount: "100.00 EUR"},
            ],
            overdue: 1,
            oldest_overdue: "2026-11-14",
            last_payment: {date: "2026-11-05", amount: "$1,000.00"},
        };
        expect(balanceText(some)).toBe("$2,000.00 + 100.00 EUR");
        expect(lastPaymentText(some)).toBe("$1,000.00 on 2026-11-05");
    });
});

describe("UNIT customer requests", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("saves with the hash and only the fields given; a 409 is a status error", async () => {
        const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
            const body = JSON.parse(String(init?.body)) as {hash: string};
            if (body.hash === "stale") return new Response(JSON.stringify({error: "customers/Fabrikam.md changed on disk"}), {status: 409});
            return new Response(JSON.stringify({path: "customers/Fabrikam.md", hash: "new", written: true, changed: ["terms"]}), {status: 200});
        });
        vi.stubGlobal("fetch", fetch);
        const saved = await saveCustomer("customers/Fabrikam", "abc", {terms: 45});
        expect(saved.hash).toBe("new");
        const [url, init] = fetch.mock.calls[0];
        expect(url).toBe("/api/customers/customers/Fabrikam");
        expect(init?.method).toBe("PUT");
        expect(JSON.parse(String(init?.body))).toEqual({hash: "abc", fields: {terms: 45}});

        const refused = saveCustomer("customers/Fabrikam", "stale", {terms: 45});
        await expect(refused).rejects.toBeInstanceOf(ApiStatusError);
        await expect(saveCustomer("customers/Fabrikam", "stale", {})).rejects.toMatchObject({status: 409, message: "customers/Fabrikam.md changed on disk"});
    });

    it("creates by POST and answers the link", async () => {
        const fetch = vi.fn(
            async () =>
                new Response(JSON.stringify({path: "customers/Lee Park.md", title: "Lee Park", link: "[[Lee Park]]", hash: "h", written: true, changed: []}), {
                    status: 201,
                })
        );
        vi.stubGlobal("fetch", fetch);
        const made = await createCustomer({title: "Lee Park", kind: "person"});
        expect(made.link).toBe("[[Lee Park]]");
        const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(url).toBe("/api/customers");
        expect(init.method).toBe("POST");
    });
});

describe("UNIT names, never syntax", () => {
    it("shows a link's name and a contract's name", () => {
        expect(linkName("[[Customer X]]")).toBe("Customer X");
        expect(linkName(" [[MSA 2026|the MSA]] ")).toBe("MSA 2026");
        expect(linkName("[[Notes#Terms]]")).toBe("Notes");
        expect(linkName("Customer X")).toBe("Customer X");
        expect(contractName("[[Customer X MSA 2026]]")).toBe("Customer X MSA 2026");
        expect(contractName("/customers/contracts/SOW 3 (signed).pdf")).toBe("SOW 3 (signed).pdf");
        expect(contractName("SOW.pdf")).toBe("SOW.pdf");
    });

    it("says what a note is from its type", () => {
        expect(kindLabel("organization")).toBe("Organization");
        expect(kindLabel("credit-note")).toBe("Credit note");
        expect(kindLabel(null)).toBeNull();
        expect(kindLabel(" ")).toBeNull();
    });
});
