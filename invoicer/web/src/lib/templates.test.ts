// The template editor's pure helpers (invoicer PLAN 4d).
import {describe, expect, it} from "vitest";
import type {InvoiceEntry} from "./invoices.js";
import {
    baseText,
    blockScaffold,
    definedBlocks,
    insertBlock,
    invoiceLabel,
    kindsText,
    latestInvoice,
    templateChanges,
    templateDirty,
    templateFile,
    templateOfParam,
    templateParam,
    usageText,
    type TemplateForm,
} from "./templates.js";

const invoice = (number: string | null, date: string | null, extra: Partial<InvoiceEntry> = {}): InvoiceEntry => ({
    path: `invoices/${number ?? "x"}.md`,
    title: `Invoice ${number ?? "x"}`,
    number,
    doc_type: "invoice",
    status: "sent",
    date,
    due_date: null,
    reference: null,
    invoicee: {name: "Customer X", email: null, phone: null, address: null, link: null},
    total: null,
    paid: null,
    open: false,
    overdue: false,
    template: null,
    problems: {errors: 0, warnings: 0},
    ...extra,
});

describe("UNIT templates", () => {
    it("routes a template by its file without .html, and back", () => {
        expect(templateFile("/invoices/tpl/blue.html")).toBe("invoices/tpl/blue.html");
        expect(templateParam("/invoices/tpl/Ocean Blue.html")).toBe("invoices/tpl/Ocean%20Blue");
        expect(templateOfParam("invoices/tpl/Ocean%20Blue")).toBe("/invoices/tpl/Ocean Blue.html");
        expect(templateOfParam("invoices/tpl/x.html")).toBe("/invoices/tpl/x.html");
        expect(templateOfParam("invoices/tpl/100% blue")).toBe("/invoices/tpl/100% blue.html");
    });

    it("sends only what changed", () => {
        const original: TemplateForm = {name: "Blue", kinds: ["invoice"], body: '{% extends "b" %}\n'};
        expect(templateChanges(original, {...original})).toEqual({});
        expect(templateDirty(original, {...original, name: " Blue "})).toBe(false);
        expect(templateChanges(original, {...original, name: "Sea "})).toEqual({name: "Sea"});
        // Kinds are compared as a set and sent in the app's order.
        expect(templateChanges({...original, kinds: ["estimate", "invoice"]}, {...original, kinds: ["invoice", "estimate"]})).toEqual({});
        expect(templateChanges(original, {...original, kinds: ["credit-note", "invoice"]})).toEqual({kinds: ["invoice", "credit-note"]});
        // The body is code: whitespace counts.
        expect(templateChanges(original, {...original, body: original.body + " "})).toEqual({body: original.body + " "});
    });

    it("says kinds, uses and the base in words", () => {
        expect(kindsText(["estimate", "invoice"])).toBe("Invoices, estimates");
        expect(kindsText([])).toBe("—");
        expect(usageText({used_by: []})).toBe("Not used");
        expect(
            usageText({
                used_by: [
                    {path: "a", title: "A", how: "invoice", number: "1"},
                    {path: "b", title: "B", how: "invoice", number: "2"},
                    {path: "c", title: "C", how: "customer", number: null},
                ],
            })
        ).toBe("2 invoices, 1 customer");
        expect(baseText({extends: "_invoice_base.v1.html", base_version: 1})).toBe("Invoice base, version 1");
        expect(baseText({extends: null, base_version: null})).toBe("A whole page");
        expect(baseText({extends: "_mine.html", base_version: null})).toBe("Extends _mine.html");
    });

    it("inserts a block's scaffolding on lines of its own, the caret inside", () => {
        const body = '{% extends "_invoice_base.v1.html" %}\n';
        const end = insertBlock(body, "theme");
        expect(end.body).toBe(body + blockScaffold("theme"));
        expect(end.body).toContain("{% block theme %}\n{{ super() }}\n{% endblock theme %}\n");
        expect(end.body.slice(end.caret)).toBe("{% endblock theme %}\n");
        // A caret mid-line moves to the end of that line.
        const mid = insertBlock("ab\ncd", "footer", 1);
        expect(mid.body).toBe(`ab\n${blockScaffold("footer")}cd`);
        // No final newline: one is added first.
        expect(insertBlock("x", "head").body).toBe(`x\n${blockScaffold("head")}`);
    });

    it("reads the blocks a body defines, at the top level, outside comments and raw", () => {
        const body = `{# invoicer: name="x" #}
{% extends "_invoice_base.v1.html" %}
{#- {% block parties %} in a comment -#}
{% block theme %}{{ super() }}{% block inner %}{% endblock %}{% endblock theme %}
{% raw %}{% block body %}{% endraw %}
{%- block footer -%}Thanks{%- endblock footer -%}
{% block theme %}again{% endblock %}`;
        expect(definedBlocks(body)).toEqual(["theme", "footer"]);
        expect(definedBlocks("<p>no blocks</p>")).toEqual([]);
    });

    it("previews with the latest invoice by date, then number", () => {
        expect(latestInvoice([])).toBeNull();
        const list = [invoice("9", "2026-11-17"), invoice("108", "2026-11-17"), invoice("2", "2026-10-01"), invoice(null, "2026-12-01")];
        expect(latestInvoice(list)?.number).toBe("108");
        expect(latestInvoice([invoice("E1", "2026-12-01", {doc_type: "estimate"}), invoice("5", "2026-01-01")])?.number).toBe("5");
        expect(invoiceLabel(invoice("108", "2026-11-17"))).toBe("108 · Customer X · 2026-11-17");
    });
});
