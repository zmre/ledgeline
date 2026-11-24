// The shapes the invoicer API answers (the CLI's `--json` shapes, invoicer
// PLAN 1c and 3b), and the pure helpers the pages read them through. The
// server is the only calculator: amounts arrive as text and are shown as is.

/** The `paid:` mirror, as the file writes it (I7). */
export type Paid = {state: "no"} | {state: "partial"; parts: {date: string; amount: string}[]} | {state: "full"; date: string};

/** A contact snapshot on the document. */
export interface Party {
    name: string | null;
    email: string | null;
    phone: string | null;
    address: string | null;
    link: string | null;
}

/** One row of `GET /api/invoices`. */
export interface InvoiceEntry {
    path: string;
    title: string;
    number: string | null;
    doc_type: string | null;
    status: "draft" | "sent" | "void" | string | null;
    date: string | null;
    due_date: string | null;
    reference: string | null;
    invoicee: Party;
    total: string | null;
    paid: Paid | null;
    open: boolean;
    overdue: boolean;
    template: string | null;
    problems: {errors: number; warnings: number};
}

/** `GET /api/invoices/{number}`: the parts the invoice page reads. */
export interface InvoiceDetail {
    path: string;
    title: string;
    number: string | null;
    doc_type: string | null;
    status: string | null;
    date: string | null;
    due_date: string | null;
    template: string | null;
    invoicee: Party;
    paid: Paid | null;
    open: boolean;
    overdue: boolean;
    table: TableDetail | null;
}

/** A written amount the arithmetic disagrees with (`show`'s `table.discrepancies`). */
export interface Discrepancy {
    /** 1-based row. */
    row: number;
    line: number;
    /** As written (empty when blank). */
    written: string;
    /** What the arithmetic gives. */
    expected: string;
}

/** The bill table, computed: the parts the invoice page reads. */
export interface TableDetail {
    total: string | null;
    rows: {row: number; kind: string; description: string; rate: string | null}[];
    discrepancies: Discrepancy[];
}

/** One amount the totals notice lists: which row, as written, as computed. */
export interface Mismatch {
    row: number;
    /** The row's description, else its rate cell ("Subtotal", "Total due"), else "Row N". */
    label: string;
    written: string;
    computed: string;
}

/** The amounts that don't match their rows, labelled for the totals notice; empty when they all agree. */
export function mismatches(table: TableDetail | null): Mismatch[] {
    if (table === null) return [];
    return (table.discrepancies ?? []).map((d) => {
        const row = (table.rows ?? []).find((r) => r.row === d.row);
        const label = row?.description || row?.rate || `Row ${d.row}`;
        return {row: d.row, label, written: d.written === "" ? "(blank)" : d.written, computed: d.expected};
    });
}

/** `GET /api/books`. */
export interface BooksInfo {
    journal: string | null;
    basis: "accrual" | "cash";
    documents: number;
    customers: number;
    nextNumber: string;
}

/** A posting, amounts in the journal's style. */
export interface PostingView {
    account: string;
    amounts: string[];
}

/** A transaction the books hold under an invoice's number. */
export interface EntryView {
    date: string;
    description: string;
    file: string;
    line: number;
    postings: PostingView[];
}

/** A payment or credit against a receivable, and where it is in the journal. */
export interface ReceivedView {
    date: string;
    amount: string;
    kind: "payment" | "credit";
    description: string;
    /** The transaction's other accounts: where the money went (the bank), or the credit's account. */
    from: string[];
    file: string;
    line: number;
}

/** Where an invoice stands with the books, for the panel's first line. */
export interface Settlement {
    state: "paid" | "partial" | "unpaid" | "void";
    total: string;
    paid: string;
    outstanding: string;
    paid_on: string | null;
    due_date: string | null;
}

/** `GET /api/invoices/{number}/books`: the Books panel. */
export interface InvoiceBooks {
    basis: "accrual" | "cash";
    number: string | null;
    status: string | null;
    bookings: EntryView[];
    receivable: {
        account: string;
        amount: string;
        outstanding: string;
        state: "open" | "partial" | "paid" | "void";
        payments: ReceivedView[];
    } | null;
    voids: EntryView[];
    payments: (EntryView & {amount: string})[];
    books_paid_text: string | null;
    paid: Paid | null;
    in_step: boolean | null;
    settlement: Settlement | null;
    /** The file's `paid:` line disagrees with the books: that, in plain words. */
    mirror_problem: string | null;
    problems: {severity: string; message: string}[];
}

/**
 * Where an invoice stands, in one plain line: "Paid in full on 2026-10-20",
 * "Partly paid: $1,000.00 of $3,000.00, $2,000.00 due 2026-11-14", "Unpaid: $8,350.00 due 2026-12-10".
 */
export function settlementText(s: Settlement): string {
    switch (s.state) {
        case "paid":
            return s.paid_on === null ? "Paid in full" : `Paid in full on ${s.paid_on}`;
        case "partial":
            return `Partly paid: ${s.paid} of ${s.total}, ${s.outstanding} due${s.due_date === null ? "" : ` ${s.due_date}`}`;
        case "unpaid":
            return `Unpaid: ${s.outstanding} due${s.due_date === null ? "" : ` ${s.due_date}`}`;
        case "void":
            return "Void: nothing is owed";
    }
}

/** `GET /api/templates`. */
export interface TemplateEntry {
    path: string;
    name: string;
    kinds: string[];
    starter: boolean;
    installed: boolean;
}

/** `POST /api/preview`. */
export interface Preview {
    html: string;
    path: string;
    template: string | null;
}

/** `YYYY-MM-DD` of `now` in the local zone (never via UTC: see eslint.config.js). */
export function localToday(now: Date = new Date()): string {
    const pad = (n: number): string => String(n).padStart(2, "0");
    return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/**
 * The URL MBR serves a note at: the path without its extension, each segment
 * percent-encoded, with a trailing slash (`invoices/A B.md` →
 * `/invoices/A%20B/`).
 */
export function pageUrl(path: string): string {
    const bare = path.replace(/^\/+/, "").replace(/\.(md|markdown)$/i, "");
    return `/${bare.split("/").map(encodeURIComponent).join("/")}/`;
}

/**
 * The preview page for the saved invoice `number` through `template`
 * (`GET /.ledgeline-invoicer/preview`): a page of its own under MBR's CSP,
 * behind the session cookie, for the invoice frame.
 */
export function previewUrl(number: string, template: string): string {
    const query = new URLSearchParams({number, template});
    return `/.ledgeline-invoicer/preview?${query.toString()}`;
}

/** daisyUI badge tones. */
export type Tone = "neutral" | "info" | "success" | "warning" | "error";

/**
 * Where a document stands, in one word: its status, and for a sent one what
 * the books (or its mirror) say. Overdue outranks open.
 */
export function standing(doc: Pick<InvoiceEntry, "status" | "paid" | "open" | "overdue">): {label: string; tone: Tone} {
    switch (doc.status) {
        case "draft":
            return {label: "Draft", tone: "neutral"};
        case "void":
            return {label: "Void", tone: "neutral"};
        case "sent":
            if (doc.overdue) return {label: "Overdue", tone: "error"};
            if (!doc.open) return {label: "Paid", tone: "success"};
            if (doc.paid?.state === "partial") return {label: "Partly paid", tone: "warning"};
            return {label: "Open", tone: "warning"};
        default:
            return {label: doc.status ?? "No status", tone: "neutral"};
    }
}

/** The `paid:` mirror in words: "No", "Paid 2026-10-05", "Partly: 2026-11-05 $1,000". */
export function paidText(paid: Paid | null): string {
    if (paid === null) return "—";
    switch (paid.state) {
        case "no":
            return "No";
        case "full":
            return `Paid ${paid.date}`;
        case "partial":
            return `Partly: ${paid.parts.map((p) => `${p.date} ${p.amount}`).join(", ")}`;
    }
}

/** A list row's document kind, when it isn't an invoice. */
export function kindLabel(docType: string | null): string | null {
    switch (docType) {
        case null:
        case "invoice":
            return null;
        case "estimate":
            return "Estimate";
        case "credit-note":
            return "Credit note";
        default:
            return docType;
    }
}

/**
 * The template an invoice names, in words: its name when the books (or the
 * starters) have it, else the path as written; null when it names none.
 */
export function templateName(templates: TemplateEntry[], own: string | null): {name: string; known: boolean} | null {
    if (own === null) return null;
    const wanted = `/${own.replace(/^\/+/, "")}`;
    const found = templates.find((t) => t.path === wanted);
    return found === undefined ? {name: own, known: false} : {name: found.name, known: true};
}

/** The templates an invoice can be previewed with: its own first, then the rest by name. */
export function templateChoices(templates: TemplateEntry[], own: string | null): TemplateEntry[] {
    const rest = templates.filter((t) => t.path !== own).sort((a, b) => a.name.localeCompare(b.name));
    const mine = templates.find((t) => t.path === own);
    return mine === undefined ? rest : [mine, ...rest];
}
