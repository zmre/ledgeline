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
    defaultOf,
    emptyForm,
    entryOf,
    humanizeLabel,
    initials,
    isDirty,
    isOutsideEntry,
    kindLabel,
    lastPaymentText,
    linkName,
    mailtoHref,
    newCustomerFields,
    notesParts,
    openCustomerPortal,
    portalHost,
    posOf,
    routeParam,
    saveCustomer,
    savedText,
    shortDate,
    telHref,
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
        body: "Fabrikam pays by ACH.\n\n- Contact: Lee\n",
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
        // Notes, when written.
        const noted = emptyForm("organization");
        noted.body = "Met at the expo.";
        expect(newCustomerFields(noted)).toEqual({kind: "organization", body: "Met at the expo."});
    });

    it("sends the notes only when they changed, and not for white space alone", () => {
        const original = fabrikam();
        const edited = cloneForm(original);
        edited.body = "\r\n\nFabrikam pays by ACH.   \r\n\r\n- Contact: Lee\n\n\n";
        expect(changedFields(original, edited)).toEqual({});
        expect(isDirty(original, edited)).toBe(false);
        edited.body = "Fabrikam pays by **wire**.\n";
        expect(changedFields(original, edited)).toEqual({body: "Fabrikam pays by **wire**.\n"});
        edited.body = "";
        expect(changedFields(original, edited)).toEqual({body: ""});
    });

    it("sends a PO's attachment with the PO list; an absent one is no change", () => {
        const original = fabrikam();
        original.open_pos = [{reference: "PO #1", amount: null, expires: null, attachment: null, was: "PO #1"}];
        const edited = cloneForm(original);
        edited.open_pos[0].attachment = " ~/Documents/PO 1.pdf ";
        expect(changedFields(original, edited)).toEqual({
            open_pos: [{reference: "PO #1", amount: null, expires: null, attachment: "~/Documents/PO 1.pdf", was: "PO #1"}],
        });
        const without = cloneForm(original);
        delete without.open_pos[0].attachment;
        expect(changedFields(original, without)).toEqual({});
    });

    it("keeps each PO's attachment entry when the view rebuilds the list", () => {
        expect(
            posOf([
                {
                    reference: "PO #1",
                    amount: null,
                    expires: null,
                    expired: false,
                    invoices: [],
                    invoiced: null,
                    remaining: null,
                    attachment: {kind: "outside", path: "file:///Volumes/Share/PO%201.pdf", location: "/Volumes/Share/PO 1.pdf", found: null, name: "PO 1.pdf"},
                },
                {reference: "PO #2", amount: null, expires: null, expired: false, invoices: [], invoiced: null, remaining: null},
            ])
        ).toEqual([
            {reference: "PO #1", amount: null, expires: null, attachment: "file:///Volumes/Share/PO%201.pdf", was: "PO #1"},
            {reference: "PO #2", amount: null, expires: null, attachment: null, was: "PO #2"},
        ]);
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
        const saved = await saveCustomer("customers/Fabrikam", "abc", {terms: 45}, "2026-11-24");
        expect(saved.hash).toBe("new");
        const [url, init] = fetch.mock.calls[0];
        expect(url).toBe("/api/customers/customers/Fabrikam");
        expect(init?.method).toBe("PUT");
        expect(JSON.parse(String(init?.body))).toEqual({hash: "abc", fields: {terms: 45}, today: "2026-11-24"});

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
        expect(contractName("~/Documents/MSA 2026.pdf")).toBe("MSA 2026.pdf");
        expect(contractName("file:///Volumes/Share/MSA%202026%20%231.pdf")).toBe("MSA 2026 #1.pdf");
        expect(contractName("file:///bad%ZZ.pdf")).toBe("bad%ZZ.pdf");
    });

    it("tells an outside file's entry, and an attachment's entry, apart", () => {
        expect(isOutsideEntry("~/Documents/MSA.pdf")).toBe(true);
        expect(isOutsideEntry(" FILE:///tmp/MSA.pdf")).toBe(true);
        expect(isOutsideEntry("/customers/contracts/MSA.pdf")).toBe(false);
        expect(isOutsideEntry("[[MSA]]")).toBe(false);
        expect(entryOf({kind: "note", link: "[[MSA]]", found: null, name: "MSA"})).toBe("[[MSA]]");
        expect(entryOf({kind: "outside", path: "~/MSA.pdf", location: "~/MSA.pdf", found: null, name: "MSA.pdf"})).toBe("~/MSA.pdf");
    });

    it("says what a note is from its type", () => {
        expect(kindLabel("organization")).toBe("Organization");
        expect(kindLabel("credit-note")).toBe("Credit note");
        expect(kindLabel(null)).toBeNull();
        expect(kindLabel(" ")).toBeNull();
    });
});

describe("UNIT saved text (PLAN 5b, S5)", () => {
    const labels = (keys: readonly string[]) => keys.join(", ");
    it("says what was saved, and where closed POs were noted", () => {
        expect(savedText({written: false, changed: []}, labels)).toBe("Nothing to save.");
        expect(savedText({written: true, changed: ["Open POs"]}, labels)).toBe("Saved: Open POs.");
        expect(savedText({written: true, changed: ["Open POs"], noted: ["PO #1"]}, labels)).toBe(
            "Saved: Open POs. PO #1 is noted under Closed POs in the customer's note."
        );
        expect(savedText({written: true, changed: ["Open POs"], noted: ["PO #1", "PO #2"]}, labels)).toContain("PO #1, PO #2 are noted");
    });
});

describe("UNIT the contact card's words", () => {
    it("labels for people, as MBR writes them", () => {
        expect(humanizeLabel("billing")).toBe("Billing");
        expect(humanizeLabel("work_cell")).toBe("Work cell");
        expect(humanizeLabel("home-fax")).toBe("Home fax");
        expect(humanizeLabel("linkedin")).toBe("LinkedIn");
        expect(humanizeLabel("Accounts Payable")).toBe("Accounts Payable");
        expect(humanizeLabel("ébène")).toBe("Ébène");
    });

    it("initials: a person's first and last, an organization's first two that count", () => {
        expect(initials("Fred Smith", "person")).toBe("FS");
        expect(initials("Mary Ann de la Cruz", "person")).toBe("MC");
        expect(initials("Customer X", "organization")).toBe("CX");
        expect(initials("The Contoso Group, Inc.", "organization")).toBe("CG");
        expect(initials("Fabrikam LLC", "organization")).toBe("F");
        expect(initials("Cher", "person")).toBe("C");
        expect(initials("élan vital", "organization")).toBe("ÉV");
        expect(initials("北京 公司", "organization")).toBe("北公");
        expect(initials("The", "organization")).toBe("T");
    });

    it("picks the default value: billing, then work, then the first", () => {
        const items = [
            {label: "home", value: "a"},
            {label: "Work", value: "b"},
        ];
        expect(defaultOf(items)?.value).toBe("b");
        expect(defaultOf([...items, {label: "billing", value: "c"}])?.value).toBe("c");
        expect(defaultOf([{label: null, value: "d"}])?.value).toBe("d");
        expect(defaultOf([])).toBeNull();
    });

    it("links an email and a phone only when they are one", () => {
        expect(mailtoHref(" ap@x.com ")).toBe("mailto:ap@x.com");
        expect(mailtoHref("not an email")).toBeNull();
        expect(mailtoHref("javascript:alert(1)@x")).toBeNull();
        expect(telHref("+1 (303) 555-0142")).toBe("tel:+13035550142");
        expect(telHref("303.555.0100 ext. 12")).toBe("tel:3035550100,12");
        expect(telHref("303 555 0100 x7")).toBe("tel:3035550100,7");
        expect(telHref("ask for Pam")).toBeNull();
        expect(telHref("12")).toBeNull();
    });

    it("shows a portal by its host", () => {
        expect(portalHost("https://customerx.coupahost.com/")).toBe("customerx.coupahost.com");
        expect(portalHost("not a url")).toBe("not a url");
    });

    it("splits the closed POs the app writes from the user's notes", () => {
        expect(notesParts("Hi.\n")).toEqual({notes: "Hi.", closed: "", closedCount: 0});
        expect(notesParts("Hi.\n\n## Closed POs\n\n- PO #1; closed.\n- PO #2; closed.\n\n## Contacts\n\nLee.\n")).toEqual({
            notes: "Hi.\n\n## Contacts\n\nLee.",
            closed: "- PO #1; closed.\n- PO #2; closed.",
            closedCount: 2,
        });
        expect(notesParts("## Closed POs\n- A.\n")).toEqual({notes: "", closed: "- A.", closedCount: 1});
    });

    it("writes dates for people, with the year when it isn't this one", () => {
        expect(shortDate("2026-12-17", "2026-11-26")).toBe("Dec 17");
        expect(shortDate("2027-06-30", "2026-11-26")).toBe("Jun 30, 2027");
        expect(shortDate("soon", "2026-11-26")).toBe("soon");
    });

    it("opens the portal through the server, or in a tab when no window can", async () => {
        const fetchMock = vi
            .spyOn(globalThis, "fetch")
            .mockResolvedValueOnce(new Response(JSON.stringify({url: "https://p.example/", opened: true}), {status: 200}))
            .mockResolvedValueOnce(new Response(JSON.stringify({url: "https://p.example/", opened: false}), {status: 200}));
        const open = vi.fn();
        vi.stubGlobal("window", {open});
        await openCustomerPortal("customers/Customer X.md");
        expect(fetchMock.mock.calls[0][0]).toBe("/api/customer-portal");
        expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body))).toEqual({customer: "customers/Customer X.md"});
        expect(open).not.toHaveBeenCalled();
        await openCustomerPortal("Customer X");
        expect(open).toHaveBeenCalledWith("https://p.example/", "_blank", "noopener,noreferrer");
        vi.unstubAllGlobals();
        fetchMock.mockRestore();
    });
});
