// Responses as the server sends them for the accrual fixture
// (invoicer/fixtures/accrual), trimmed to what the components read. The Rust
// router tests pin the real shapes (src/app/tests.rs).
import type {Delivery} from "#lib/delivery.js";
import type {InvoiceBooks, InvoiceEntry} from "#lib/invoices.js";

const customerX = {
    name: "Customer X",
    email: "accounts_receivable@customerx.com",
    phone: "+1 (303) 555-0142",
    address: "1234 Main St. #555\nSpringfield, CO 80000\n",
    link: "[[Customer X]]",
};

export function entry(over: Partial<InvoiceEntry>): InvoiceEntry {
    return {
        path: "invoices/2026-11-10 Customer X Invoice 105.md",
        title: "2026-11-10 Customer X Invoice 105",
        number: "105",
        doc_type: "invoice",
        status: "sent",
        date: "2026-11-10",
        due_date: "2026-12-10",
        reference: "PO #PY75334, rev. 2",
        invoicee: customerX,
        total: "$8,350.00",
        paid: {state: "no"},
        open: true,
        overdue: false,
        template: "/invoices/tpl/myco-default.html",
        problems: {errors: 0, warnings: 0},
        ...over,
    };
}

export const BOOKS_105: InvoiceBooks = {
    basis: "accrual",
    number: "105",
    status: "sent",
    bookings: [
        {
            date: "2026-11-10",
            description: "Customer X | Invoice 105",
            file: "books.journal",
            line: 212,
            postings: [
                {account: "assets:accounts receivable", amounts: ["$8,350.00"]},
                {account: "revenue:consulting", amounts: ["-$7,200.00"]},
                {account: "revenue:consulting:delivery", amounts: ["-$9,500.00"]},
                {account: "liabilities:customer-deposits", amounts: ["$8,350.00"]},
            ],
        },
    ],
    receivable: {account: "assets:accounts receivable", amount: "$8,350.00", outstanding: "$8,350.00", state: "open", payments: []},
    voids: [],
    payments: [],
    books_paid_text: "no",
    paid: {state: "no"},
    in_step: true,
    settlement: {state: "unpaid", total: "$8,350.00", paid: "$0.00", outstanding: "$8,350.00", paid_on: null, due_date: "2026-12-10"},
    mirror_problem: null,
    problems: [],
};

/** `GET /api/invoices/105/pdf`: sent, kept twice, Customer X's portal. */
export const DELIVERY_105: Delivery = {
    number: "105",
    document: "invoices/2026-11-10 Customer X Invoice 105.md",
    status: "sent",
    sent_date: "2026-11-10",
    sent_via: "portal",
    destination: "kept",
    file: "2026-11-10 Customer X Invoice 105.pdf",
    pdf_dir: "invoices/pdf",
    sent_hash: "0123456789abcdef",
    changed_since_sent: false,
    exports: [
        {
            file: "2026-11-10 Customer X Invoice 105.pdf",
            kept_at: "invoices/pdf/2026-11-10 Customer X Invoice 105.pdf",
            date: "2026-11-20",
            bytes: 48_000,
            latest: true,
        },
        {
            file: "2026-11-10 Customer X Invoice 105 2026-11-10.pdf",
            kept_at: "invoices/pdf/2026-11-10 Customer X Invoice 105 2026-11-10.pdf",
            date: "2026-11-10",
            bytes: 47_000,
            latest: false,
        },
    ],
    portal: {
        name: "Coupa",
        url: "https://customerx.coupahost.com/",
        login: "finance@mycompany.com",
        supplier_id: "SUP-44821",
        requires_reference: true,
        instructions: "Upload the PDF under Invoices → Create from PO, then attach the timesheet.",
    },
    copy: {number: "105", reference: "PO #PY75334, rev. 2", total: "$8,350.00"},
    capabilities: {pdf: true, share: true, copy_file: true, open_url: true, open_file: true},
};

/** `GET /api/invoices/108/pdf`: a draft, never exported. */
export const DELIVERY_108: Delivery = {
    ...DELIVERY_105,
    number: "108",
    document: "invoices/2026-11-17 Customer X Invoice 108.md",
    status: "draft",
    sent_date: null,
    sent_via: null,
    destination: "temp",
    file: "2026-11-17 Customer X Invoice 108.pdf",
    sent_hash: null,
    changed_since_sent: null,
    exports: [],
    copy: {number: "108", reference: "PO #PY75334", total: "$13,600.00"},
};
