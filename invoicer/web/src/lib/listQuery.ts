// The two lists' filters and sort as URL query strings (invoicer PLAN 5b,
// S1–S4): one parse and one serialize each, so a link, a reload and
// Back/Forward all land on the same list. The server does the filtering and
// sorting (`invoicer_core::views::query`): the page hands it the URL's
// parameters as they are, and a value the server doesn't read is dropped here
// first, so a mistyped link shows the whole list instead of an error.

/** Which way a column sorts. */
export type SortDir = "asc" | "desc";

/** The invoice list's `status` filter. */
export const INVOICE_STATUSES = ["draft", "sent", "paid", "void", "open", "overdue"] as const;
export type InvoiceStatus = (typeof INVOICE_STATUSES)[number];

/** The invoice list's columns, as `sort` names them. */
export const INVOICE_SORTS = ["number", "customer", "date", "due", "total", "status"] as const;
export type InvoiceSort = (typeof INVOICE_SORTS)[number];

/** The customer list's columns, as `sort` names them. */
export const CUSTOMER_SORTS = ["name", "owed", "overdue", "last-payment"] as const;
export type CustomerSort = (typeof CUSTOMER_SORTS)[number];

/** `kind`: people or organizations. */
export const CUSTOMER_KINDS = ["person", "organization"] as const;
export type CustomerKind = (typeof CUSTOMER_KINDS)[number];

/** The aging buckets the filter offers (Ledgeline's); `aged` takes any `A-B` or `N+` too. */
export const AGED_BUCKETS = [
    {value: "current", label: "Not yet due"},
    {value: "1-30", label: "1–30 days late"},
    {value: "31-60", label: "31–60 days late"},
    {value: "61-90", label: "61–90 days late"},
    {value: "91+", label: "Over 90 days late"},
] as const;

/** The invoice list's URL state. Empty strings are "no filter". */
export interface InvoiceQuery {
    status: InvoiceStatus | "";
    /** A customer's name. */
    customer: string;
    /** Dated on or after (YYYY-MM-DD). */
    from: string;
    /** Dated on or before. */
    to: string;
    /** Open and this many days past due: `current`, `31-60`, `91+`. */
    aged: string;
    /** Number, customer, reference or total holding this text. */
    q: string;
    /** The column; "" is the server's order, newest first (shown as Date, descending). */
    sort: InvoiceSort | "";
    dir: SortDir;
}

/** The customer list's URL state. */
export interface CustomerQuery {
    owes: boolean;
    overdue: boolean;
    kind: CustomerKind | "";
    q: string;
    /** The column; "" is by name. */
    sort: CustomerSort | "";
    dir: SortDir;
}

/** What the parsers read: a URL's `searchParams` (SvelteKit's are read-only). */
type Params = Pick<URLSearchParams, "get">;

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const AGED = /^(current|\d+-\d+|\d+[+-])$/;

const oneOf = <T extends string>(values: readonly T[], text: string | null): T | "" => ((values as readonly string[]).includes(text ?? "") ? (text as T) : "");

/** A query value, trimmed; "" when absent. */
const param = (params: Params, name: string): string => (params.get(name) ?? "").trim();

const flag = (params: Params, name: string): boolean => ["true", "1", "yes"].includes(param(params, name));

/** Whether `aged` is a value the server reads (`current`, `A-B` with A ≤ B, `N+` or `N-`), for any bucket edges. */
export function isAged(text: string): boolean {
    if (!AGED.test(text)) return false;
    const range = /^(\d+)-(\d+)$/.exec(text);
    return range === null || Number(range[1]) <= Number(range[2]);
}

/** The invoice list's state from its URL; unreadable values are dropped. */
export function parseInvoiceQuery(params: Params): InvoiceQuery {
    const date = (name: string): string => {
        const value = param(params, name);
        return DATE.test(value) ? value : "";
    };
    const aged = param(params, "aged");
    const sort = oneOf(INVOICE_SORTS, param(params, "sort"));
    return {
        status: oneOf(INVOICE_STATUSES, param(params, "status")),
        customer: param(params, "customer"),
        from: date("from"),
        to: date("to"),
        // `91-` (the dashboard's first form) reads as `91+`.
        aged: isAged(aged) ? aged.replace(/^(\d+)-$/, "$1+") : "",
        q: params.get("q") ?? "",
        sort,
        dir: sort === "" ? "desc" : param(params, "dir") === "desc" ? "desc" : "asc",
    };
}

/** The customer list's state from its URL. */
export function parseCustomerQuery(params: Params): CustomerQuery {
    const sort = oneOf(CUSTOMER_SORTS, param(params, "sort"));
    return {
        owes: flag(params, "owes"),
        overdue: flag(params, "overdue"),
        kind: oneOf(CUSTOMER_KINDS, param(params, "kind")),
        q: params.get("q") ?? "",
        sort,
        dir: sort === "" ? "asc" : param(params, "dir") === "desc" ? "desc" : "asc",
    };
}

/** The invoice list's state as its URL query, without the `?`: only what isn't the default, in a fixed order. */
export function invoiceQueryString(query: InvoiceQuery): string {
    const out = new URLSearchParams();
    if (query.status !== "") out.set("status", query.status);
    if (query.customer.trim() !== "") out.set("customer", query.customer.trim());
    if (query.from !== "") out.set("from", query.from);
    if (query.to !== "") out.set("to", query.to);
    if (query.aged !== "") out.set("aged", query.aged);
    if (query.q.trim() !== "") out.set("q", query.q.trim());
    if (query.sort !== "") {
        out.set("sort", query.sort);
        out.set("dir", query.dir);
    }
    return out.toString();
}

/** The customer list's state as its URL query, without the `?`. */
export function customerQueryString(query: CustomerQuery): string {
    const out = new URLSearchParams();
    if (query.owes) out.set("owes", "true");
    if (query.overdue) out.set("overdue", "true");
    if (query.kind !== "") out.set("kind", query.kind);
    if (query.q.trim() !== "") out.set("q", query.q.trim());
    if (query.sort !== "") {
        out.set("sort", query.sort);
        out.set("dir", query.dir);
    }
    return out.toString();
}

/** Whether any filter (not the sort) narrows the invoice list. */
export function invoiceFiltered(query: InvoiceQuery): boolean {
    return [query.status, query.customer.trim(), query.from, query.to, query.aged, query.q.trim()].some((v) => v !== "");
}

/** Whether any filter narrows the customer list. */
export function customerFiltered(query: CustomerQuery): boolean {
    return query.owes || query.overdue || query.kind !== "" || query.q.trim() !== "";
}

/** The invoice list with every filter cleared, the sort kept. */
export function clearInvoiceFilters(query: InvoiceQuery): InvoiceQuery {
    return {status: "", customer: "", from: "", to: "", aged: "", q: "", sort: query.sort, dir: query.dir};
}

/** The customer list with every filter cleared, the sort kept. */
export function clearCustomerFilters(query: CustomerQuery): CustomerQuery {
    return {owes: false, overdue: false, kind: "", q: "", sort: query.sort, dir: query.dir};
}

/** Which way a column sorts on its first click: amounts and dates biggest/latest first, names A first. */
const FIRST_DIR: Record<InvoiceSort | CustomerSort, SortDir> = {
    number: "desc",
    customer: "asc",
    date: "desc",
    due: "asc",
    total: "desc",
    status: "asc",
    name: "asc",
    owed: "desc",
    overdue: "desc",
    "last-payment": "desc",
};

/**
 * The sort after a click on `column`'s header: the same column flips, another
 * starts its own way round. `shown` is what the header shows now (the
 * default order counts as its column).
 */
export function nextSort<T extends InvoiceSort | CustomerSort>(shown: {sort: T; dir: SortDir}, column: T): {sort: T; dir: SortDir} {
    if (shown.sort === column) return {sort: column, dir: shown.dir === "asc" ? "desc" : "asc"};
    return {sort: column, dir: FIRST_DIR[column]};
}

/** The invoice column the list is sorted by, as the headers show it: the default order is Date, newest first. */
export function shownInvoiceSort(query: InvoiceQuery): {sort: InvoiceSort; dir: SortDir} {
    return query.sort === "" ? {sort: "date", dir: "desc"} : {sort: query.sort, dir: query.dir};
}

/** The customer column the list is sorted by: the default is Name, A first. */
export function shownCustomerSort(query: CustomerQuery): {sort: CustomerSort; dir: SortDir} {
    return query.sort === "" ? {sort: "name", dir: "asc"} : {sort: query.sort, dir: query.dir};
}

/** The URL state after sorting by `column`; the default order (Date, newest first) is written as no sort. */
export function sortInvoicesBy(query: InvoiceQuery, column: InvoiceSort): InvoiceQuery {
    const next = nextSort(shownInvoiceSort(query), column);
    const isDefault = next.sort === "date" && next.dir === "desc";
    return {...query, sort: isDefault ? "" : next.sort, dir: next.dir};
}

/** The URL state after sorting by `column`; Name A–Z is written as no sort. */
export function sortCustomersBy(query: CustomerQuery, column: CustomerSort): CustomerQuery {
    const next = nextSort(shownCustomerSort(query), column);
    const isDefault = next.sort === "name" && next.dir === "asc";
    return {...query, sort: isDefault ? "" : next.sort, dir: next.dir};
}

/** A header's `aria-sort`. */
export function ariaSort(shown: {sort: string; dir: SortDir}, column: string): "ascending" | "descending" | "none" {
    if (shown.sort !== column) return "none";
    return shown.dir === "asc" ? "ascending" : "descending";
}

/** "3 invoices", "1 invoice". */
export function countText(count: number, one: string, many: string): string {
    return `${count} ${count === 1 ? one : many}`;
}

/**
 * Quick jump (S3): the invoice Enter in the search box opens. An exact
 * number wins (`105`, any case); else the one row the search left; else none.
 */
export function quickJump<T extends {number: string | null}>(rows: readonly T[], text: string): T | null {
    const wanted = text.trim().toLowerCase();
    if (wanted === "") return null;
    const exact = rows.find((r) => (r.number ?? "").toLowerCase() === wanted);
    if (exact !== undefined) return exact;
    return rows.length === 1 ? rows[0] : null;
}
