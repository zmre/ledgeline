// Time billing's calls (plan 46 Phase 2, `app/time.rs`; B-unbilled,
// B-invoice, B-ap): what is unbilled per customer, the Bill dialog's preview
// and create, an orphaned tag's release, what is unpaid per timekeeper, and
// the Pay dialog's preview and create. Refusals come back as an
// EditorApiError with the server's `code` (`not-set-up`, `conflict`, …).
//
// Each call sends the browser's `today` (as the invoice calls do): the
// periods, the default dates and the reference follow the user's calendar.
// Every write tags time lines, so the caller fetches what it shows again
// afterwards (the files' change events do that for other pages).
import {send} from "#lib/editorApi.js";
import {localToday} from "#lib/invoices.js";
import type {Qty} from "../types.js";

/** One period to bill (or pay): its dates, inclusive, and its unbilled time. */
export interface PeriodView {
    from: string;
    to: string;
    quantity: Qty;
    entries: number;
}

/** One customer's unbilled time. */
export interface CustomerUnbilled {
    /** The `customer:` value the time accounts carry. */
    customer: string;
    /** The customer note's title, when one has that name. */
    title: string | null;
    path: string | null;
    cadence: string | null;
    quantity: Qty;
    entries: number;
    oldest: string;
    newest: string;
    /** At each account's item rate, plain decimal; null when some account has none. */
    estimate: string | null;
    commodity: string | null;
    /** The main suggestion first. */
    periods: PeriodView[];
}

/** Billable time on an account no customer is linked to. */
export interface NeedsCustomer {
    account: string;
    quantity: Qty;
    oldest: string;
}

/** Time tagged `invoice: N` where no invoice N exists. */
export interface Orphan {
    invoice: string;
    entries: string[];
    quantity: Qty;
}

/** `GET /api/time/unbilled`. */
export interface Unbilled {
    today: string;
    customers: CustomerUnbilled[];
    needs_customer: NeedsCustomer[];
    orphans: Orphan[];
    /** Every customer's main suggestion, summed (the banner). */
    suggested_total: Qty;
}

/** How the Bill dialog groups time into rows. */
export type Grouping = "account" | "account-person" | "account-note";

/** The timesheet table under the bill table. */
export type Detail = "none" | "daily" | "entries";

/** Where a row's suggested item came from. */
export type ItemSource = "choice" | "time-items" | "account" | "most-used";

/** One group's choices, by its key; absent fields keep the server's default. */
export interface GroupChoice {
    key: string;
    item?: string;
    rate?: string;
    account?: string;
    description?: string;
    remember?: boolean;
    exclude?: boolean;
}

/** `POST /api/time/bill`. */
export interface BillRequest {
    customer: string;
    from?: string;
    to?: string;
    /** Exactly these lines (Entries' Bill selected) instead of a period. */
    entries?: string[];
    group?: Grouping;
    groups?: GroupChoice[];
    detail?: Detail;
    date?: string;
    reference?: string;
    number?: string;
    template?: string;
    terms?: number;
}

/** The item a row bills as. */
export interface ItemView {
    code: string;
    source: ItemSource;
    /** Whether the catalog has it. */
    found: boolean;
    description: string | null;
    unit: string | null;
    rate: string | null;
    books: string | null;
}

/** One row-to-be. */
export interface BillGroup {
    key: string;
    account: string;
    person: string | null;
    note: string | null;
    entries: string[];
    actual: Qty;
    rounded: Qty;
    /** Rounded minus actual, in hours. */
    difference: string;
    /** The row's Hours cell. */
    hours: string;
    item: ItemView | null;
    rate: string | null;
    books: string | null;
    description: string;
    amount: string | null;
    excluded: boolean;
    needs: string[];
}

/** The rounding a customer's note asks for (B-round). */
export interface Rounding {
    increment: number | null;
    direction: "up" | "nearest";
    scope: "row" | "day" | "entry";
}

/** What a bill would be. */
export interface BillPreview {
    customer: {path: string; title: string; ledger_customer: string; cadence: string | null; detail: Detail; rounding: Rounding};
    from: string | null;
    to: string | null;
    group: Grouping;
    groups: BillGroup[];
    actual: Qty;
    rounded: Qty;
    difference: string;
    amount: string | null;
    detail: Detail;
    /** The timesheet table's markdown; null for no timesheet. */
    timesheet: string | null;
    entries: string[];
    periods: PeriodView[];
    ready: boolean;
    needs: string[];
}

/** What Create draft made. */
export interface BillCreated {
    path: string;
    number: string;
    total: string | null;
    tagged: string[];
    files: string[];
    remembered: string[];
    warnings: string[];
}

/** One person's time with no `bill` tag. */
export interface PersonUnpaid {
    /** `""`: unattributed. */
    person: string;
    /** Their timekeeper note. */
    note: string | null;
    title: string | null;
    quantity: Qty;
    entries: number;
    oldest: string;
    newest: string;
    periods: PeriodView[];
}

/** A timekeeper note, as the Pay dialog reads it. */
export interface Timekeeper {
    path: string;
    title: string;
    kind: "person" | "organization";
    person: string;
    pay_rate: string | null;
    vendor: string | null;
    expense_account: string | null;
    payable_account: string | null;
    pay_terms: number | null;
    pay_to: string | null;
}

/** What the Pay dialog still has to ask for. */
export type NeedField = "note" | "rate" | "expense_account" | "payable_account" | "time" | "reference";

export interface Need {
    field: NeedField;
    message: string;
    /** What to choose from: timekeeper notes, payable accounts. */
    candidates: string[];
}

/** One line of the bill. */
export interface PayLine {
    /** The time account; null for one line in total. */
    account: string | null;
    quantity: Qty;
    hours: string;
    amount: string | null;
}

/** `POST /api/time/pay`. */
export interface PayRequest {
    person: string;
    from?: string;
    to?: string;
    entries?: string[];
    lines?: "account" | "total";
    /** The note to use when none links the person (Pick a note). */
    note?: string;
    /** Or a new one (Create note…). */
    create_note?: {title: string; kind: "person" | "organization"};
    rate?: string;
    expense_account?: string;
    payable_account?: string;
    terms?: number;
    save_to_note?: boolean;
    date?: string;
    reference?: string;
}

/** What paying would book. */
export interface PayPreview {
    person: string;
    note: Timekeeper | null;
    /** Who is paid. */
    payee: string | null;
    /** The note paid instead (a contractor's company). */
    pay_to: string | null;
    from: string | null;
    to: string | null;
    entries: string[];
    lines: PayLine[];
    quantity: Qty;
    rate: string | null;
    amount: string | null;
    expense_account: string | null;
    payable_account: string | null;
    terms: number | null;
    due: string | null;
    reference: string;
    date: string;
    periods: PeriodView[];
    needs: Need[];
    ready: boolean;
}

/** What Create bill booked. */
export interface PayCreated {
    reference: string;
    date: string;
    vendor: string;
    amount: string;
    /** The transaction as written to the books. */
    text: string;
    tagged: string[];
    files: string[];
    saved_to_note: string[];
    note_created: string | null;
}

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);

const isQty = (body: unknown): boolean => isObject(body) && typeof body.seconds === "number" && typeof body.text === "string";

export function isUnbilled(body: unknown): body is Unbilled {
    return (
        isObject(body) &&
        Array.isArray(body.customers) &&
        body.customers.every((c) => isObject(c) && typeof c.customer === "string" && isQty(c.quantity) && Array.isArray(c.periods)) &&
        Array.isArray(body.needs_customer) &&
        Array.isArray(body.orphans) &&
        isQty(body.suggested_total)
    );
}

export function isBillPreview(body: unknown): body is BillPreview {
    return (
        isObject(body) &&
        Array.isArray(body.groups) &&
        body.groups.every((g) => isObject(g) && typeof g.key === "string" && isQty(g.actual) && isQty(g.rounded)) &&
        typeof body.ready === "boolean" &&
        Array.isArray(body.needs) &&
        isQty(body.rounded)
    );
}

const isBillCreated = (body: unknown): boolean => isObject(body) && typeof body.number === "string" && typeof body.path === "string";

export const isUnpaid = (body: unknown): body is PersonUnpaid[] =>
    Array.isArray(body) && body.every((p) => isObject(p) && typeof p.person === "string" && isQty(p.quantity));

export function isPayPreview(body: unknown): body is PayPreview {
    return isObject(body) && typeof body.person === "string" && Array.isArray(body.lines) && Array.isArray(body.needs) && typeof body.ready === "boolean";
}

const isPayCreated = (body: unknown): boolean => isObject(body) && typeof body.reference === "string" && typeof body.amount === "string";

/** `GET /api/time/unbilled`. */
export function fetchUnbilled(): Promise<Unbilled> {
    return send<Unbilled>("GET", `/api/time/unbilled?today=${localToday()}`, isUnbilled);
}

/** `POST /api/time/bill` with `preview`: nothing is written. */
export function previewBill(request: BillRequest): Promise<BillPreview> {
    return send<BillPreview>("POST", "/api/time/bill", isBillPreview, {...request, preview: true, today: localToday()});
}

/** `POST /api/time/bill`: the draft written and its time tagged `invoice: N`. */
export function createBill(request: BillRequest): Promise<BillCreated> {
    return send<BillCreated>("POST", "/api/time/bill", isBillCreated, {...request, today: localToday()});
}

/** `POST /api/time/release`: an invoice's time back to Unbilled, or (`nobill`) written off. */
export function releaseTime(invoice: string, how: "release" | "nobill" = "release"): Promise<unknown> {
    return send<unknown>("POST", "/api/time/release", isObject, {invoice, how});
}

/**
 * `POST /api/time/release` with `bill`: void the AP bill `reference` in the
 * books and take its `bill:` tags off (or, `nobill`, write the time off), in
 * one step.
 */
export function releaseBill(bill: string, how: "release" | "nobill" = "release"): Promise<unknown> {
    return send<unknown>("POST", "/api/time/release", isObject, {bill, how});
}

/** `GET /api/time/unpaid`: each timekeeper's time with no `bill` tag. */
export function fetchUnpaid(): Promise<PersonUnpaid[]> {
    return send<PersonUnpaid[]>("GET", `/api/time/unpaid?today=${localToday()}`, isUnpaid);
}

/** `POST /api/time/pay` with `preview`: nothing is written. */
export function previewPay(request: PayRequest): Promise<PayPreview> {
    return send<PayPreview>("POST", "/api/time/pay", isPayPreview, {...request, preview: true, today: localToday()});
}

/**
 * `POST /api/time/pay`: the bill booked and its time tagged `bill: REF`. When
 * something is still needed the server answers the preview instead, so the
 * answer is either.
 */
export function createPay(request: PayRequest): Promise<PayCreated | PayPreview> {
    return send<PayCreated | PayPreview>("POST", "/api/time/pay", (b) => isPayCreated(b) || isPayPreview(b), {...request, today: localToday()});
}

/** Whether a Create bill answer is the booked bill (else the preview: something is needed). */
export const isBooked = (answer: PayCreated | PayPreview): answer is PayCreated => "vendor" in answer && "text" in answer;
