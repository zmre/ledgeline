// A `GET /api/settings` body for tests (synthetic books), shaped as the
// wire's golden: defaults everywhere, the file setting the default terms
// and the quantity words, in cash books with one company and one template.
import type {SettingsPage, SettingsValues} from "#lib/settings.js";

export function defaultValues(): SettingsValues {
    return {
        company: null,
        "invoices-dir": "invoices",
        "customers-dir": "customers",
        "templates-dir": "invoices/tpl",
        catalog: "invoices/catalog.md",
        "pdf-dir": "invoices/pdf",
        filename: "{date} {customer} Invoice {invoice_num}.md",
        "number-start": null,
        "default-terms": null,
        "default-template": null,
        "receivable-account": null,
        "revenue-account": null,
        "deposit-account": null,
        "discount-account": null,
        "cash-deposits": "income-on-receipt",
        "payment-allocation": "match-rows",
        "recalc-on-save": "drafts",
        "aging-buckets": [30, 60, 90],
        "dso-days": 90,
        "paid-window-days": 30,
        words: {
            description: ["Description", "Item", "Service", "Product"],
            quantity: ["Qty", "Quantity", "Hours", "Units"],
            rate: ["Rate", "Price", "Unit price", "Unit cost"],
            amount: ["Amount", "Total", "Line total"],
            date: ["Date", "Service date"],
            unit: ["Unit"],
            item: ["Code", "SKU"],
            subtotal: ["Subtotal"],
            total: ["Total due", "Total", "Balance due", "Amount due"],
            deposit: ["Less deposit"],
            tax: ["Sales tax"],
            "customer-tag": "customer",
            "billing-labels": ["billing", "work"],
        },
    };
}

export function settingsPage(over: Partial<SettingsPage> = {}): SettingsPage {
    const defaults = defaultValues();
    const values = {...defaultValues(), "default-terms": 30, words: {...defaults.words, quantity: ["Qty", "Hours"]}};
    return {
        file: "present",
        problem: null,
        problems: [],
        readOnly: false,
        revision: "rev-1",
        values,
        defaults,
        set: ["default-terms", "words.quantity"],
        inferred: {"receivable-account": "assets:receivable", "revenue-account": "revenue:consulting", "deposit-account": "liabilities:customer-deposits"},
        nextNumber: {number: "208", after: "207", source: "document"},
        books: {basis: "cash", closedThrough: "2026-06-30"},
        booksError: null,
        choices: {
            templates: [{value: "/invoices/tpl/classic.html", name: "Classic", kinds: ["invoice"], starter: true}],
            companies: [{title: "My Company", link: "[[My Company]]"}],
            accounts: ["assets:checking", "assets:receivable", "revenue:consulting"],
        },
        affectsBookings: ["receivable-account", "revenue-account", "deposit-account", "discount-account", "cash-deposits", "payment-allocation"],
        coming: [
            {id: "oldest-row-first", setting: "payment-allocation", section: "payments", label: "Oldest row first"},
            {id: "keep-deferral", setting: "cash-deposits", section: "payments", label: "Cash books: keep a deposit as a deferral"},
            {id: "first-open-day", setting: null, section: "editing", label: "Sending into a closed period: book it on the first open day"},
            {id: "reverse-and-rebook", setting: null, section: "editing", label: "Editing a sent invoice: reverse it and book it again"},
        ],
        ...over,
    };
}
