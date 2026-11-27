// Billing's test answers, shaped as the server gives them for the time
// fixture (invoicer/fixtures/time, clock 2026-11-26). Each takes overrides.
import type {BillCreated, BillGroup, BillPreview, CustomerUnbilled, PayCreated, PayPreview, PersonUnpaid, Unbilled} from "../api/billing.js";
import type {Qty} from "../types.js";

/** `seconds` as the server's Qty. */
export function q(seconds: number): Qty {
    const minutes = Math.round(seconds / 60);
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    const text = h === 0 ? `${m}m` : m === 0 ? `${h}h` : `${h}h ${String(m).padStart(2, "0")}m`;
    return {seconds, hours: String(seconds / 3600), text};
}

export function acme(over: Partial<CustomerUnbilled> = {}): CustomerUnbilled {
    return {
        customer: "Acme Corp",
        title: "Acme Corp",
        path: "customers/Acme Corp.md",
        cadence: "monthly",
        quantity: q(29040),
        entries: 7,
        oldest: "2026-10-06",
        newest: "2026-11-26",
        estimate: "1583.33",
        commodity: "$",
        periods: [{from: "2026-10-01", to: "2026-10-31", quantity: q(3000), entries: 1}],
        ...over,
    };
}

export function unbilled(over: Partial<Unbilled> = {}): Unbilled {
    return {
        today: "2026-11-26",
        customers: [
            acme(),
            acme({
                customer: "Globex",
                title: "Globex",
                path: "customers/Globex.md",
                cadence: null,
                quantity: q(59400),
                entries: 6,
                oldest: "2026-09-29",
                newest: "2026-11-10",
                estimate: "2970.00",
                periods: [{from: "2026-09-29", to: "2026-10-31", quantity: q(10800), entries: 2}],
            }),
        ],
        needs_customer: [{account: "clients:orphan", quantity: q(900), oldest: "2026-10-20"}],
        orphans: [],
        suggested_total: q(13800),
        ...over,
    };
}

export function billGroup(over: Partial<BillGroup> = {}): BillGroup {
    return {
        key: "clients:acme:advisory",
        account: "clients:acme:advisory",
        person: null,
        note: null,
        entries: ["time/pat/2026-10.timedot@6"],
        actual: q(3000),
        rounded: q(3240),
        difference: "0.066666667",
        hours: "0.90",
        item: {code: "ARCH", source: "account", found: true, description: "Senior advisory hours", unit: "h", rate: "$250", books: "revenue:consulting"},
        rate: "$250",
        books: "revenue:consulting",
        description: "Advisory, Oct 2026",
        amount: "$225.00",
        excluded: false,
        needs: [],
        ...over,
    };
}

export function billPreview(over: Partial<BillPreview> = {}): BillPreview {
    return {
        customer: {
            path: "customers/Acme Corp.md",
            title: "Acme Corp",
            ledger_customer: "Acme Corp",
            cadence: "monthly",
            detail: "daily",
            rounding: {increment: 6, direction: "up", scope: "row"},
        },
        from: "2026-10-01",
        to: "2026-10-31",
        group: "account",
        groups: [billGroup()],
        actual: q(3000),
        rounded: q(3240),
        difference: "0.066666667",
        amount: "$225.00",
        detail: "daily",
        timesheet: "## Timesheet\n\n| Date | Who | Work | Hours |\n|:-----|:----|:-----|------:|\n| 2026-10-06 | pat | year-end planning | 0.83 |\n",
        entries: ["time/pat/2026-10.timedot@6"],
        periods: [{from: "2026-10-01", to: "2026-10-31", quantity: q(3000), entries: 1}],
        ready: true,
        needs: [],
        ...over,
    };
}

export const billCreated = (over: Partial<BillCreated> = {}): BillCreated => ({
    path: "invoices/2026-11-26 Acme Corp Invoice 102.md",
    number: "102",
    total: "$225.00",
    tagged: ["time/pat/2026-10.timedot@6"],
    files: ["time/pat/2026-10.timedot"],
    remembered: [],
    warnings: [],
    ...over,
});

export const riley = (over: Partial<PersonUnpaid> = {}): PersonUnpaid => ({
    person: "riley",
    note: "people/Riley Chen.md",
    title: "Riley Chen",
    quantity: q(10800),
    entries: 1,
    oldest: "2026-11-10",
    newest: "2026-11-10",
    periods: [],
    ...over,
});

/** Riley's bill: no rate or expense account on the note yet. */
export function payPreview(over: Partial<PayPreview> = {}): PayPreview {
    return {
        person: "riley",
        note: {
            path: "people/Riley Chen.md",
            title: "Riley Chen",
            kind: "person",
            person: "riley",
            pay_rate: null,
            vendor: null,
            expense_account: null,
            payable_account: null,
            pay_terms: null,
            pay_to: null,
        },
        payee: "Riley Chen",
        pay_to: null,
        from: null,
        to: null,
        entries: ["time/riley/2026-11.timedot@4"],
        lines: [{account: "clients:globex:audit", quantity: q(10800), hours: "3.00", amount: null}],
        quantity: q(10800),
        rate: null,
        amount: null,
        expense_account: null,
        payable_account: "liabilities:accounts payable",
        terms: null,
        due: null,
        reference: "riley timesheet 2026-11",
        date: "2026-11-26",
        periods: [],
        needs: [
            {field: "rate", message: "riley's hourly rate", candidates: []},
            {field: "expense_account", message: "the expense account the time is booked to", candidates: []},
        ],
        ready: false,
        ...over,
    };
}

export const payCreated = (over: Partial<PayCreated> = {}): PayCreated => ({
    reference: "riley timesheet 2026-11",
    date: "2026-11-26",
    vendor: "Riley Chen",
    amount: "$180.00",
    text: "2026-11-26 Riley Chen | riley timesheet 2026-11  ; reference: riley timesheet 2026-11\n    liabilities:accounts payable  $-180.00\n    expenses:contractors  $180.00\n",
    tagged: ["time/riley/2026-11.timedot@4"],
    files: ["time/riley/2026-11.timedot"],
    saved_to_note: ["pay_rate", "expense_account"],
    note_created: null,
    ...over,
});
