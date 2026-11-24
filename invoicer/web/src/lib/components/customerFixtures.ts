// Customer responses as the server sends them for the accrual fixture
// (invoicer/fixtures/accrual), trimmed to what the components read. The Rust
// router tests pin the real shapes (src/app/customers_tests.rs).
import type {CustomerDetail, CustomerRow} from "#lib/customers.js";
import {entry} from "./fixtures.js";

export function row(over: Partial<CustomerRow>): CustomerRow {
    return {
        path: "customers/Contoso.md",
        title: "Contoso",
        kind: "organization",
        ledger_customer: "Contoso",
        terms: 30,
        billing: {note: "customers/Contoso.md", name: "Contoso", email: "ap@contoso.example", phone: null, address: null},
        portal: null,
        invoices: 2,
        open: 1,
        account: {
            balance: [{commodity: "$", amount: "$2,000.00"}],
            overdue: 1,
            oldest_overdue: "2026-11-14",
            last_payment: {date: "2026-11-05", amount: "$1,000.00"},
        },
        ...over,
    };
}

export function customerX(over: Partial<CustomerDetail> = {}): CustomerDetail {
    return {
        ...row({
            path: "customers/Customer X.md",
            title: "Customer X",
            ledger_customer: "Customer X",
            billing: {
                note: "customers/Customer X.md",
                name: "Customer X",
                email: "accounts_receivable@customerx.com",
                phone: "+1 (303) 555-0142",
                address: null,
            },
            invoices: 3,
            account: {
                balance: [{commodity: "$", amount: "$8,350.00"}],
                overdue: 0,
                oldest_overdue: null,
                last_payment: {date: "2026-10-08", amount: "$8,350.00"},
            },
        }),
        revenue_account: "revenue:consulting",
        invoice_template: "/invoices/tpl/myco-default.html",
        contracts: [
            {kind: "note", link: "[[Customer X MSA 2026]]", found: "customers/contracts/Customer X MSA 2026.md", name: "Customer X MSA 2026"},
            {kind: "file", path: "/customers/contracts/SOW 3.pdf", found: "customers/contracts/SOW 3.pdf", name: "SOW 3.pdf"},
            {kind: "file", path: "/customers/contracts/gone.pdf", found: null, name: "gone.pdf"},
        ],
        open_pos: [],
        snippets: [],
        documents: [
            entry({path: "invoices/108.md", number: "108", status: "draft", open: false, paid: null, total: "$1,200.00"}),
            entry({}),
            entry({path: "invoices/104.md", number: "104", open: false, paid: {state: "full", date: "2026-10-08"}, due_date: "2026-10-31"}),
        ],
        problems: [],
        hash: "1f2-00000000deadbeef",
        form: {
            title: "Customer X",
            kind: "organization",
            emails: [
                {label: "billing", value: "accounts_receivable@customerx.com"},
                {label: "work", value: "hello@customerx.com"},
            ],
            phones: [{label: "billing", value: "+1 (303) 555-0142"}],
            addresses: [{label: "billing", value: "1234 Main St. #555\nSpringfield, CO 80000"}],
            company: "",
            terms: 30,
            revenue_account: "revenue:consulting",
            invoice_template: "/invoices/tpl/myco-default.html",
            ledger_customer: "Customer X",
            contracts: ["[[Customer X MSA 2026]]", "/customers/contracts/SOW 3.pdf", "/customers/contracts/gone.pdf"],
            billing_portal: {
                name: "Coupa",
                url: "https://customerx.coupahost.com/",
                login: "finance@mycompany.com",
                supplier_id: "SUP-44821",
                requires_reference: true,
                instructions: "Upload the PDF under Invoices.\n",
            },
        },
        backlinks: [
            {
                url: "/invoices/2026-11-10 Customer X Invoice 105/",
                path: "invoices/2026-11-10 Customer X Invoice 105.md",
                title: "Invoice 105",
                key: "customer",
                kind: "invoice",
                number: "105",
                customer: false,
            },
            {url: "/customers/Fred Smith/", path: "customers/Fred Smith.md", title: "Fred Smith", key: "company", kind: "person", number: null, customer: true},
            {url: "/notes/kickoff/", path: "notes/kickoff.md", title: "Kickoff", key: null, kind: null, number: null, customer: false},
            {url: "/invoices/old%20quote/", path: "invoices/old quote.md", title: "Quote 7", key: null, kind: "estimate", number: "E7", customer: false},
        ],
        ...over,
    };
}
