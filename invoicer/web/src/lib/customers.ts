// Customers (invoicer PLAN 4b): the shapes `/api/customers` answers, the
// pure helpers the pages read them through, and the four calls. The server
// does every write by key (`invoicer_core::customer_edit`): the SPA sends
// only the fields the user changed, so a key it never touched is never
// rewritten, and names the content hash it was shown, so an outside edit is
// never clobbered (a `409`, [`ApiStatusError`]).
import {ApiShapeError, ApiUnreachableError, authHeaders} from "#ui/api/client.js";
import {failureMessage} from "./api.js";
import {localToday, type InvoiceEntry} from "./invoices.js";
import {linkName} from "./links.js";

/** `person` or `organization` (the note's `type:`). */
export type ContactKind = "person" | "organization";

/** One labelled value (`- billing: ap@x.com`); `label` null for a bare one. */
export interface Labelled {
    label: string | null;
    value: string;
}

/** `billing_portal:`: how invoices reach a customer through a vendor portal. */
export interface Portal {
    name: string | null;
    url: string | null;
    login: string | null;
    supplier_id: string | null;
    requires_reference: boolean;
    instructions: string | null;
}

/**
 * One open PO as the form edits it (`invoicer_core::customer_edit::PoFields`,
 * PLAN 5b S5): the keys the core reads. `was` is the reference it had when
 * read (null for a new one), so the note's entry keeps keys the form doesn't
 * know. A PO left out of the list is closed: it comes off the note.
 */
export interface PoForm {
    reference: string;
    /** As money (`$40,000`); null for none. */
    amount: string | null;
    /** YYYY-MM-DD; null for never. */
    expires: string | null;
    /**
     * Its file (the PO's PDF, say): an entry as `contracts:` takes them
     * (`[[Title]]`, a `/`-rooted path, `~/…` or `file:///…`); null for none.
     * Left out of a request, the note keeps the one it has.
     */
    attachment?: string | null;
    was: string | null;
}

/** An open PO and what has been invoiced against it (`customers show`'s `po_usage`, S6). */
export interface PoUsage {
    reference: string;
    amount: string | null;
    expires: string | null;
    /** Expired before today. */
    expired: boolean;
    /** The sent invoices quoting it, oldest first. */
    invoices: string[];
    /** Their totals added up; null when none (or in another commodity). */
    invoiced: string | null;
    /** The amount less what was invoiced; null without an amount. */
    remaining: string | null;
    /** Its attachment, resolved; null (or absent) for none. */
    attachment?: Attachment | null;
}

/** The form's POs for a usage list: each as read (`was` its own reference), its attachment kept. */
export function posOf(usage: readonly PoUsage[]): PoForm[] {
    return usage.map((po) => ({
        reference: po.reference,
        amount: po.amount,
        expires: po.expires,
        attachment: po.attachment == null ? null : entryOf(po.attachment),
        was: po.reference,
    }));
}

/**
 * The customer form (`invoicer_core::customer_edit::CustomerFields`): every
 * field optional. Absent leaves the key as the note has it; an empty string or
 * list, or null, removes it.
 */
export interface CustomerFields {
    title?: string;
    kind?: ContactKind;
    emails?: Labelled[];
    phones?: Labelled[];
    addresses?: Labelled[];
    company?: string;
    terms?: number | null;
    revenue_account?: string;
    invoice_template?: string;
    ledger_customer?: string;
    contracts?: string[];
    billing_portal?: Portal | null;
    open_pos?: PoForm[];
    /** The note's markdown body: the notes on the customer. */
    body?: string;
}

/** The form with every field present: what the page edits. */
export type CustomerForm = Required<CustomerFields>;

/** What `POST /api/customers` and `PUT /api/customers/{key}` answer. */
export interface CustomerSaved {
    path: string;
    title: string;
    /** What an invoice's `customer:` writes: `[[Title]]`. */
    link: string;
    ledger_customer: string;
    kind: ContactKind;
    /** The note's content hash now: what the next save names. */
    hash: string;
    written: boolean;
    changed: string[];
    /** Open POs this save closed, each noted under "Closed POs" in the note's body (PLAN 5b, S5). */
    noted?: string[];
}

/** An amount owed in one commodity, in the journal's style. */
export interface Balance {
    commodity: string;
    amount: string;
}

/** What a customer owes and has paid. */
export interface CustomerAccount {
    balance: Balance[];
    overdue: number;
    oldest_overdue: string | null;
    last_payment: {date: string; amount: string} | null;
}

/** Who invoices go to. */
export interface BillingContact {
    note: string;
    name: string;
    email: string | null;
    phone: string | null;
    address: string | null;
}

/** One row of `GET /api/customers`. */
export interface CustomerRow {
    path: string;
    title: string;
    kind: ContactKind;
    ledger_customer: string;
    terms: number | null;
    billing: BillingContact;
    portal: string | null;
    invoices: number;
    open: number;
    account: CustomerAccount;
}

/** A note linking to the customer (MBR's backlinks, M1). */
export interface Backlink {
    url: string;
    path: string | null;
    title: string | null;
    /** The frontmatter key that links (`customer`); null for a body link. */
    key: string | null;
    /** The linking note's `type:` (`person`, `invoice`, `contract`, …); null when it has none. */
    kind: string | null;
    /** A document's number: the app's invoice page shows it. */
    number: string | null;
    /** Whether it is a customer: the app's customer page shows it. */
    customer: boolean;
}

/**
 * A contract or PO attachment as the server resolves it
 * (`invoicer_core::attachment::AttachmentView`): a note, a file in the books,
 * or a file outside them (`path` as written, `location` where it is:
 * `~/Documents/MSA.pdf`). `found` is the note's or file's path, or the
 * outside file's, null when it doesn't exist; `name` is what to show.
 */
export type Attachment = ({kind: "note"; link: string} | {kind: "file"; path: string} | {kind: "outside"; path: string; location: string}) & {
    found: string | null;
    name: string;
};

/** A `contracts:` entry as the server resolves it. */
export type Contract = Attachment;

/** What the note writes for an attachment: its link or path as written. */
export function entryOf(attachment: Attachment): string {
    return attachment.kind === "note" ? attachment.link : attachment.path;
}

/** A note or file a contract may name (`GET /api/files`). */
export interface FileCandidate {
    path: string;
    name: string;
    /** A note's `type:`; null for a file (or a note without one). */
    kind: string | null;
    /** What `contracts:` writes for it: `[[Title]]`, a `/`-rooted path, or for a file outside the books `~/…` or `file:///…`. */
    entry: string;
    /** Outside the books folder (`path` is then where it is: `~/Documents/MSA.pdf`). */
    outside?: boolean;
}

/** `GET /api/files`: the candidates, and what this process can do natively. */
export interface FileList {
    /** The system's open dialog is available (the window, macOS). */
    pick: boolean;
    /** A books file can open in its viewer (the window, macOS). */
    open: boolean;
    files: FileCandidate[];
}

/** `GET /api/customers/{key}`: the parts the customer page reads. */
export interface CustomerDetail extends CustomerRow {
    revenue_account: string | null;
    invoice_template: string | null;
    contracts: Contract[];
    open_pos: {reference: string; amount: string | null; expires: string | null}[];
    /** Each open PO with what was invoiced against it, in `open_pos` order. */
    po_usage: PoUsage[];
    snippets: string[];
    /** Its invoices, newest first. */
    documents: InvoiceEntry[];
    problems: {severity: string; message: string}[];
    hash: string;
    form: CustomerForm;
    backlinks: Backlink[];
}

/** A wikilink's name; one implementation, in links.ts (re-exported for the customer pages). */
export {linkName} from "./links.js";

/** A contract entry's name without the server: a link's target, a path's file name (a `file:` URL's decoded). */
export function contractName(entry: string): string {
    const text = entry.trim();
    if (text.startsWith("[[")) return linkName(text);
    const parts = text.replace(/\/+$/, "").split("/");
    const last = parts[parts.length - 1] || text;
    if (!/^file:/i.test(text)) return last;
    try {
        return decodeURIComponent(last);
    } catch {
        return last;
    }
}

/** Whether an entry names a file outside the books folder (`~/…`, `file:///…`). */
export function isOutsideEntry(entry: string): boolean {
    const text = entry.trim();
    return text.startsWith("~/") || /^file:/i.test(text);
}

/** What a note is, in words, from its `type:` ("Organization", "Credit note"). */
export function kindLabel(kind: string | null): string | null {
    if (kind === null || kind.trim() === "") return null;
    const words = kind.trim().replace(/[-_]+/g, " ");
    return words.charAt(0).toUpperCase() + words.slice(1);
}

/** The labels a list offers first: `billing` is what invoices pick. */
export const LABELS = ["billing", "work", "home", "mobile", "other"] as const;

/** An empty portal. */
export function emptyPortal(): Portal {
    return {name: null, url: null, login: null, supplier_id: null, requires_reference: false, instructions: null};
}

/** A blank form for a new customer. */
export function emptyForm(kind: ContactKind = "organization"): CustomerForm {
    return {
        title: "",
        kind,
        emails: [{label: "billing", value: ""}],
        phones: [],
        addresses: [{label: "billing", value: ""}],
        company: "",
        terms: null,
        revenue_account: "",
        invoice_template: "",
        ledger_customer: "",
        contracts: [],
        billing_portal: null,
        open_pos: [],
        body: "",
    };
}

/** A deep copy (forms are plain JSON; a `$state` proxy copies too). */
export function cloneForm(form: CustomerForm): CustomerForm {
    return JSON.parse(JSON.stringify(form)) as CustomerForm;
}

const trimmed = (s: string | null | undefined): string => (s ?? "").trim();

/** A labelled list as the server will write it: blank rows dropped, labels and values trimmed. */
function normalList(items: Labelled[]): Labelled[] {
    return items.map((i) => ({label: trimmed(i.label) === "" ? null : trimmed(i.label), value: i.value.trim()})).filter((i) => i.value !== "");
}

function normalPortal(portal: Portal | null): Portal | null {
    if (portal === null) return null;
    const text = (s: string | null): string | null => (trimmed(s) === "" ? null : trimmed(s));
    const out = {
        name: text(portal.name),
        url: text(portal.url),
        login: text(portal.login),
        supplier_id: text(portal.supplier_id),
        requires_reference: portal.requires_reference,
        instructions: text(portal.instructions),
    };
    const empty = Object.entries(out).every(([k, v]) => (k === "requires_reference" ? v === false : v === null));
    return empty ? null : out;
}

/** POs as the server will write them: text trimmed, empty as null, wholly blank rows dropped. */
function normalPos(pos: PoForm[]): PoForm[] {
    const text = (s: string | null | undefined): string | null => (trimmed(s) === "" ? null : trimmed(s));
    return pos
        .map((po) => ({reference: trimmed(po.reference), amount: text(po.amount), expires: text(po.expires), attachment: text(po.attachment), was: po.was}))
        .filter((po) => po.reference !== "" || po.amount !== null || po.expires !== null || po.attachment !== null);
}

/**
 * A body as the server compares it: `\n` line ends, no white space at line
 * ends, no blank lines at either end. Edits in white space alone are no change.
 */
export function normalBody(body: string): string {
    return body
        .replace(/\r\n/g, "\n")
        .split("\n")
        .map((line) => line.trimEnd())
        .join("\n")
        .replace(/^\n+|\n+$/g, "");
}

/** Each field as the server compares it, so whitespace alone is no change. */
function normal(form: CustomerForm): CustomerForm {
    return {
        title: trimmed(form.title),
        kind: form.kind,
        emails: normalList(form.emails),
        phones: normalList(form.phones),
        addresses: normalList(form.addresses),
        company: trimmed(form.company),
        terms: form.terms === null || Number.isNaN(form.terms) ? null : form.terms,
        revenue_account: trimmed(form.revenue_account),
        invoice_template: trimmed(form.invoice_template),
        ledger_customer: trimmed(form.ledger_customer),
        contracts: form.contracts.map(trimmed).filter((c) => c !== ""),
        billing_portal: normalPortal(form.billing_portal),
        open_pos: normalPos(form.open_pos ?? []),
        body: normalBody(form.body ?? ""),
    };
}

/**
 * The fields `current` changes from `original`, and only those: what a save
 * sends. Empty when the form is clean.
 */
export function changedFields(original: CustomerForm, current: CustomerForm): CustomerFields {
    const before = normal(original);
    const after = normal(current);
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(after) as (keyof CustomerForm)[]) {
        if (JSON.stringify(before[key]) !== JSON.stringify(after[key])) out[key] = after[key];
    }
    // The body goes as typed (the server keeps its own white space rules).
    if ("body" in out) out.body = current.body;
    return out as CustomerFields;
}

/** Whether the form has unsaved changes. */
export function isDirty(original: CustomerForm, current: CustomerForm): boolean {
    return Object.keys(changedFields(original, current)).length > 0;
}

/** What a create sends: the kind, and every field the user filled in. */
export function newCustomerFields(form: CustomerForm): CustomerFields {
    return {kind: form.kind, ...changedFields(emptyForm(form.kind), form)};
}

/**
 * The SPA route's `path` parameter for a customer note: its path without the
 * `.md`, each segment percent-encoded (`customers/Customer X.md` →
 * `customers/Customer%20X`), for `/customers/[...path]`.
 */
export function routeParam(path: string): string {
    return path
        .replace(/^\/+/, "")
        .replace(/\.(md|markdown)$/i, "")
        .split("/")
        .map(encodeURIComponent)
        .join("/");
}

/** The API URL for a customer: `key` is a note path (decoded) or a name. */
export function customerApiUrl(key: string): string {
    return `/api/customers/${key.replace(/^\/+/, "").split("/").map(encodeURIComponent).join("/")}`;
}

/** What a customer owes, in one line ("$2,000.00", or "Nothing owed"). */
export function balanceText(account: CustomerAccount): string {
    return account.balance.length === 0 ? "Nothing owed" : account.balance.map((b) => b.amount).join(" + ");
}

/** The last payment in words ("$1,000.00 on 2026-11-05", or "—"). */
export function lastPaymentText(account: CustomerAccount): string {
    return account.last_payment === null ? "—" : `${account.last_payment.amount} on ${account.last_payment.date}`;
}

/** A request refused with a status the page acts on: `409` is a conflict (the note changed, or the name is taken). */
export class ApiStatusError extends Error {
    constructor(
        message: string,
        readonly status: number
    ) {
        super(message);
        this.name = "ApiStatusError";
    }
}

async function request<T>(method: string, path: string, check: (body: unknown) => boolean, body?: unknown): Promise<T> {
    const headers = authHeaders(body === undefined ? {Accept: "application/json"} : {Accept: "application/json", "Content-Type": "application/json"}, path);
    let response: Response;
    try {
        response = await fetch(path, {method, headers, body: body === undefined ? undefined : JSON.stringify(body)});
    } catch (error) {
        throw new ApiUnreachableError(`${method} ${path}: ${String(error)}`, {cause: error});
    }
    if (!response.ok) throw new ApiStatusError(await failureMessage(method, path, response), response.status);
    const parsed: unknown = await response.json();
    if (!check(parsed)) throw new ApiShapeError(`${method} ${path}: not the shape this app expects`);
    return parsed as T;
}

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);
const isSaved = (body: unknown): boolean => isObject(body) && typeof body.path === "string" && typeof body.hash === "string";

/** `GET /api/customers`: every customer, by title, with its account on `today`; `query` (the list's URL query) filters and sorts them. */
export function fetchCustomers(today: string = localToday(), query: string = ""): Promise<CustomerRow[]> {
    const rest = query.replace(/^[?&]+/, "");
    return request<CustomerRow[]>("GET", `/api/customers?today=${today}${rest === "" ? "" : `&${rest}`}`, Array.isArray);
}

/** `GET /api/customers/{key}`: one customer, its form, invoices, account and backlinks. */
export function fetchCustomer(key: string, today: string = localToday()): Promise<CustomerDetail> {
    return request<CustomerDetail>("GET", `${customerApiUrl(key)}?today=${today}`, (body) => isObject(body) && typeof body.hash === "string");
}

/**
 * `POST /api/customers`: write a new customer note in `customers-dir`. The
 * answer's `link` is what an invoice's `customer:` writes. `409`: a note
 * already has the name.
 */
export function createCustomer(fields: CustomerFields): Promise<CustomerSaved> {
    return request<CustomerSaved>("POST", "/api/customers", isSaved, fields);
}

/**
 * `PUT /api/customers/{key}`: write `fields` (only those) into the note, if
 * it is still the bytes `hash` names; `409` otherwise. A PO closed by the
 * save is noted in the note's body, dated `today`.
 */
export function saveCustomer(key: string, hash: string, fields: CustomerFields, today: string = localToday()): Promise<CustomerSaved> {
    return request<CustomerSaved>("PUT", customerApiUrl(key), isSaved, {hash, fields, today});
}

/** What a save says it did: "Saved: Open POs." plus, when POs were closed, where they were noted. */
export function savedText(saved: Pick<CustomerSaved, "written" | "changed" | "noted">, labels: (keys: readonly string[]) => string): string {
    if (!saved.written) return "Nothing to save.";
    const noted = saved.noted ?? [];
    const closed = noted.length === 0 ? "" : ` ${noted.join(", ")} ${noted.length === 1 ? "is" : "are"} noted under Closed POs in the customer's note.`;
    return `Saved: ${labels(saved.changed)}.${closed}`;
}

const isFileList = (body: unknown): boolean => isObject(body) && Array.isArray(body.files);

/** `GET /api/files`: the notes and documents a contract may name (`list: false`: only what the app can do natively). */
export function fetchFiles(options: {list?: boolean} = {}): Promise<FileList> {
    return request<FileList>("GET", options.list === false ? "/api/files?list=false" : "/api/files", isFileList);
}

/**
 * `POST /api/files/pick`: the system's open dialog at the books folder (the
 * window). `null` when cancelled; a file outside the books folder is refused
 * with the reason (`ApiStatusError`, `422`).
 */
export async function pickFile(): Promise<FileCandidate | null> {
    const body = await request<{cancelled: boolean; file?: FileCandidate}>(
        "POST",
        "/api/files/pick",
        (b) => isObject(b) && typeof b.cancelled === "boolean",
        {}
    );
    return body.cancelled || body.file === undefined ? null : body.file;
}

/**
 * `POST /api/files/open`: a file in its default viewer (the window). `path`
 * is a path in the books folder, or an outside file's entry as written
 * (`~/…`, `file:///…`), which opens only when a customer names it.
 */
export async function openBooksFile(path: string): Promise<void> {
    await request<unknown>("POST", "/api/files/open", isObject, {path});
}
