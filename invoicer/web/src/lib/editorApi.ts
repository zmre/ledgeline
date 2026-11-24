// The invoice editor's API calls (invoicer PLAN 4a). Same-origin and
// token-gated like lib/api.ts. Writes the editor must tell apart ("the file
// changed", "confirm the re-booking") carry a `code`, so they go through
// `send`, which throws an EditorApiError with it instead of a bare sentence.
import {ApiShapeError, ApiUnreachableError, authHeaders} from "#ui/api/client.js";
import type {Computed, EditorForm, EditorOptions, EditorState, OpenDeposits} from "./editor.js";
import {localToday} from "./invoices.js";

/** A refused editor request: the HTTP status, the server's `code` and sentence, and (on `changed`) the file's current revision. */
export class EditorApiError extends Error {
    readonly status: number;
    readonly code: string;
    readonly revision: string | null;

    constructor(status: number, code: string, message: string, revision: string | null = null) {
        super(message);
        this.name = "EditorApiError";
        this.status = status;
        this.code = code;
        this.revision = revision;
    }
}

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);

/** A request whose refusal is an EditorApiError (`code` "unreachable" when the server can't be reached). */
async function send<T>(method: string, path: string, check: (body: unknown) => boolean, body?: unknown): Promise<T> {
    const headers = authHeaders(body === undefined ? {Accept: "application/json"} : {Accept: "application/json", "Content-Type": "application/json"}, path);
    let response: Response;
    try {
        response = await fetch(path, {method, headers, body: body === undefined ? undefined : JSON.stringify(body)});
    } catch (error) {
        throw new ApiUnreachableError(`${method} ${path}: ${String(error)}`, {cause: error});
    }
    let parsed: unknown;
    try {
        parsed = await response.json();
    } catch {
        parsed = null;
    }
    if (!response.ok) {
        const error = isObject(parsed) && typeof parsed.error === "string" && parsed.error !== "" ? parsed.error : `${method} ${path}: HTTP ${response.status}`;
        const code = isObject(parsed) && typeof parsed.code === "string" ? parsed.code : `http-${response.status}`;
        const revision = isObject(parsed) && typeof parsed.revision === "string" ? parsed.revision : null;
        throw new EditorApiError(response.status, code, error, revision);
    }
    if (!check(parsed)) throw new ApiShapeError(`${method} ${path}: not the shape this app expects`);
    return parsed as T;
}

const num = (number: string): string => encodeURIComponent(number);

/** `GET /api/editor/options`: customers, catalog, accounts, words. */
export function fetchEditorOptions(): Promise<EditorOptions> {
    return send<EditorOptions>("GET", "/api/editor/options", (b) => isObject(b) && Array.isArray(b.customers));
}

/** `GET /api/fields/{key}/values`: the values a custom field has across the books, most used first. */
export function fetchFieldValues(key: string): Promise<{value: string; count: number}[]> {
    return send("GET", `/api/fields/${encodeURIComponent(key)}/values`, Array.isArray);
}

/** What `POST /api/invoices` takes: the CLI's `new`. */
export interface NewInvoiceRequest {
    customer: string;
    template?: string | null;
    date: string;
    number?: string | null;
    terms?: number | null;
    reference?: string | null;
}

/** `POST /api/invoices`: write a new draft; its number. */
export function createInvoice(request: NewInvoiceRequest): Promise<{path: string; number: string; total: string | null; written: boolean}> {
    return send("POST", "/api/invoices", (b) => isObject(b) && typeof b.number === "string", request);
}

/** `GET /api/invoices/{num}/form`: the form, its revision, and its amounts. */
export function fetchForm(number: string): Promise<EditorState> {
    return send<EditorState>("GET", `/api/invoices/${num(number)}/form`, (b) => isObject(b) && isObject(b.form) && typeof b.revision === "string");
}

/** `POST /api/invoices/{num}/compute`: the amounts (and with `books`, the would-book) of an unsaved form. Writes nothing. */
export function computeForm(number: string, form: EditorForm, books = false): Promise<Computed> {
    return send<Computed>("POST", `/api/invoices/${num(number)}/compute`, (b) => isObject(b) && typeof b.text === "string", {form, books});
}

/** What a save did. */
export interface Saved {
    written: boolean;
    path: string;
    number: string;
    revision: string;
    status: string;
    rebooked: unknown;
}

/** `PUT /api/invoices/{num}/form`: save over `revision`; a sent invoice only with `rebook: true`. */
export function saveForm(number: string, form: EditorForm, revision: string, options: {rebook?: boolean} = {}): Promise<Saved> {
    return send<Saved>("PUT", `/api/invoices/${num(number)}/form`, (b) => isObject(b) && typeof b.revision === "string", {
        form,
        revision,
        rebook: options.rebook ?? false,
        today: localToday(),
    });
}

/** `POST /api/invoices/{num}/void`: the reversing entry (accrual), `status: void`. */
export function voidInvoice(number: string, date: string): Promise<unknown> {
    return send("POST", `/api/invoices/${num(number)}/void`, isObject, {date, today: localToday()});
}

/** `POST /api/invoices/{num}/rebook`: replace its transaction with what the file now says. */
export function rebookInvoice(number: string): Promise<unknown> {
    return send("POST", `/api/invoices/${num(number)}/rebook`, isObject, {today: localToday()});
}

/** `GET /api/invoices/{num}/deposits`: the customer's open deposits, each with the row that applies it. */
export function fetchDeposits(number: string): Promise<OpenDeposits> {
    return send<OpenDeposits>("GET", `/api/invoices/${num(number)}/deposits`, (b) => isObject(b) && Array.isArray(b.deposits));
}

/** `POST /api/preview/draft`: hold unsaved text for the preview page; the URL to frame. */
export function draftPreview(number: string, source: string, template: string | null = null): Promise<{url: string}> {
    return send("POST", "/api/preview/draft", (b) => isObject(b) && typeof b.url === "string", {number, source, template});
}
