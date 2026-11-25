// The header form, mounted: link fields show names and write [[Name]] (a
// space typed mid-name survives), the template is a dropdown by name, and
// Deposit for note shows only on a deposit invoice or where it is set.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {describe, expect, it, vi} from "vitest";
import {DEFAULT_WORDS, emptyForm, type EditorForm, type EditorOptions} from "#lib/editor.js";
import type {TemplateEntry} from "#lib/invoices.js";
import HeaderForm from "./HeaderForm.svelte";

const OPTIONS: EditorOptions = {
    customers: [
        {title: "Customer X", link: "[[Customer X]]", path: "customers/Customer X.md", kind: "organization", terms: 30, invoice_template: null, open_pos: []},
    ],
    catalog: [],
    accounts: [],
    deposit_account: "liabilities:customer-deposits",
    revenue_account: null,
    fields: [],
    words: DEFAULT_WORDS,
    next_number: "2005",
    customers_dir: "customers",
    link_notes: [{title: "Estimate 12", link: "[[Estimate 12]]", kind: "estimate"}],
};

const TEMPLATES: TemplateEntry[] = [
    {path: "/invoices/tpl/classic.html", name: "Classic", kinds: ["invoice"], starter: true, installed: true},
    {path: "/invoices/tpl/modern.html", name: "Modern", kinds: ["invoice"], starter: true, installed: false},
];

function mount(over: Partial<EditorForm> = {}, props: Record<string, unknown> = {}) {
    let form = $state(emptyForm({customer: "[[Customer X]]", date: "2026-11-17", template: "/invoices/tpl/classic.html", ...over}));
    const onChange = vi.fn((next: EditorForm) => (form = next));
    render(HeaderForm, {
        get form() {
            return form;
        },
        options: OPTIONS,
        templates: TEMPLATES,
        status: "draft",
        onChange,
        onAddCustomer: vi.fn(),
        fieldValues: () => Promise.resolve([]),
        ...props,
    });
    return {onChange, form: () => form};
}

describe("COMPONENT HeaderForm", () => {
    it("shows the customer by name and writes a link, spaces and all", async () => {
        const {form} = mount();
        await tick();
        const field = screen.getByTestId("header-customer") as HTMLInputElement;
        expect(field.value).toBe("Customer X");
        for (const typed of ["North", "North ", "North W"]) {
            await fireEvent.input(field, {target: {value: typed}});
            await tick();
        }
        expect(field.value).toBe("North W");
        expect(form().customer).toBe("[[North W]]");
    });

    it("picks the template from a list by name", async () => {
        const {form} = mount();
        const select = screen.getByTestId("header-template") as HTMLSelectElement;
        expect(Array.from(select.options).map((o) => o.textContent?.trim())).toEqual(["The books' default", "Classic", "Modern (starter)"]);
        await fireEvent.change(select, {target: {value: "/invoices/tpl/modern.html"}});
        expect(form().template).toBe("/invoices/tpl/modern.html");
    });

    it("offers Deposit for note on a deposit invoice, linking a known note", async () => {
        mount();
        expect(screen.queryByTestId("header-deposit-for")).toBeNull();
        document.body.innerHTML = "";
        const {form} = mount({}, {deposit: true});
        const field = screen.getByTestId("header-deposit-for") as HTMLInputElement;
        await fireEvent.input(field, {target: {value: "estimate 12"}});
        expect(form().deposit_for).toBe("[[Estimate 12]]");
        await fireEvent.input(field, {target: {value: "Phase 1 build"}});
        expect(form().deposit_for).toBe("Phase 1 build");
    });

    it("shows a deposit_for that is set, without its brackets", async () => {
        mount({deposit_for: "[[Estimate 12]]"});
        await tick();
        expect((screen.getByTestId("header-deposit-for") as HTMLInputElement).value).toBe("Estimate 12");
    });
});

describe("COMPONENT HeaderForm reference suggestions (PLAN 5b)", () => {
    it("offers the customer's open POs, one expired by the invoice's date last", async () => {
        const withPos: EditorOptions = {
            ...OPTIONS,
            customers: [
                {
                    ...OPTIONS.customers[0],
                    open_pos: [
                        {reference: "PO #OLD", amount: null, expires: "2026-01-31"},
                        {reference: "PO #PY75334", amount: "$40,000", expires: "2027-06-30"},
                    ],
                },
            ],
        };
        mount({}, {options: withPos});
        await tick();
        await fireEvent.click(screen.getByTestId("header-reference"));
        await tick();
        const shown = within(screen.getByRole("listbox"))
            .getAllByRole("option")
            .map((o) => o.textContent ?? "");
        expect(shown).toHaveLength(2);
        expect(shown[0]).toContain("PO #PY75334");
        expect(shown[0]).toContain("expires 2027-06-30");
        expect(shown[1]).toContain("PO #OLD");
        expect(shown[1]).toContain("expired 2026-01-31");
    });
});
