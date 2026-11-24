// A snippet's form, mounted: its fields bound to the form, the body in a
// plain text field when MBR's can't load (jsdom), and where it is used.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import type {SnippetEntry, SnippetForm, SnippetUse} from "#lib/library.js";
import SnippetEditor from "./SnippetEditor.svelte";

const NET_30: SnippetEntry = {
    path: "invoices/tpl/snippets/net-30.md",
    title: "Net 30 ACH",
    kind: "payment",
    aliases: ["ACH terms"],
    body: "## Payment\n\nPay by ACH.\n",
    hash: "h1",
    used_by: [
        {path: "customers/Customer X.md", title: "Customer X", how: "default", number: null},
        {path: "invoices/105.md", title: "Invoice 105", how: "copy", number: "105"},
    ],
};

function mount(entry: SnippetEntry = NET_30): SnippetForm {
    const form = $state<SnippetForm>({title: entry.title, kind: entry.kind ?? "", body: entry.body});
    render(SnippetEditor, {
        get form() {
            return form;
        },
        set form(value: SnippetForm) {
            Object.assign(form, value);
        },
        entry,
        customerHref: (use: SnippetUse) => `/app/customers/${use.path}`,
        invoiceHref: (use: SnippetUse) => `/app/invoices/${use.number}`,
    });
    return form;
}

describe("COMPONENT SnippetEditor", () => {
    it("edits the title, kind and body into the form", async () => {
        const form = mount();
        const title = screen.getByTestId("snippet-title") as HTMLInputElement;
        expect(title.value).toBe("Net 30 ACH");
        await fireEvent.input(title, {target: {value: "ACH, net 30"}});
        expect(form.title).toBe("ACH, net 30");
        const body = screen.getByTestId("snippet-body") as HTMLTextAreaElement;
        expect(body.dataset.kind).toBe("text");
        expect(body.value).toBe("## Payment\n\nPay by ACH.\n");
        await fireEvent.input(body, {target: {value: "Pay within 30 days."}});
        expect(form.body).toBe("Pay within 30 days.");
        const kind = screen.getByLabelText("Kind") as HTMLInputElement;
        expect(kind.value).toBe("payment");
        await fireEvent.input(kind, {target: {value: "terms"}});
        expect(form.kind).toBe("terms");
        expect(screen.getByTestId("snippet-aliases").textContent).toContain("ACH terms");
    });

    it("lists the customers naming it and the invoices holding a copy", () => {
        mount();
        const customers = screen.getByTestId("snippet-customers");
        expect(customers.textContent).toContain("Customer X");
        expect(customers.querySelector("a")?.getAttribute("href")).toBe("/app/customers/customers/Customer X.md");
        expect(screen.getByTestId("snippet-copies").querySelector("a")?.getAttribute("href")).toBe("/app/invoices/105");
    });

    it("says when nothing uses it", () => {
        mount({...NET_30, used_by: [], aliases: []});
        expect(screen.getByTestId("snippet-uses").textContent).toContain("No customer names it");
        expect(screen.queryByTestId("snippet-aliases")).toBeNull();
    });
});
