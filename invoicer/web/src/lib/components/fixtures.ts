// Responses as the server sends them for the accrual fixture
// (invoicer/fixtures/accrual), trimmed to what the components read. The Rust
// router tests pin the real shapes (src/app/tests.rs).
import type {InvoiceBooks, InvoiceEntry} from "#lib/invoices.js";

const customerX = {
    name: "Customer X",
    email: "accounts_receivable@customerx.com",
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
    problems: [],
};
