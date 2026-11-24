// The invoicer's API client (invoicer PLAN 3a, 3b). The SPA is served by the
// ledgeline-invoicer server under /app, so every request is same-origin and
// relative; under `vite dev` and `vite preview` (port 4273) the vite proxy
// forwards everything outside /app to the `--server` on 5199. The token comes
// from the shared client in packages/ui: injected by the window, or adopted
// from the `#token=` launch fragment.
import {ApiShapeError, ApiUnreachableError, authHeaders} from "#ui/api/client.js";
import type {Anchor, Delivery, Destination, Exported} from "./delivery.js";
import {localToday, type BooksInfo, type InvoiceBooks, type InvoiceDetail, type InvoiceEntry, type Preview, type TemplateEntry} from "./invoices.js";

/** `GET /api/ui-prefs`'s body: the shared theme and scale, among others. */
export interface UiPrefsBody {
    prefs: {theme?: unknown; uiScale?: unknown};
}

/** The saved theme and scale, or null when the body does not carry them. */
export function appearanceFrom(body: UiPrefsBody): {theme: string; uiScale: number} | null {
    const {theme, uiScale} = body.prefs ?? {};
    if (typeof theme !== "string" || typeof uiScale !== "number") return null;
    return {theme, uiScale};
}

/**
 * The server's own sentence for a failed request (`{"error": "…"}`), else the
 * status line: "no books are open; choose File → Open" says what to do, "HTTP
 * 409" does not.
 */
export async function failureMessage(method: string, path: string, response: Response): Promise<string> {
    try {
        const body: unknown = await response.json();
        if (typeof body === "object" && body !== null && "error" in body && typeof body.error === "string" && body.error !== "") {
            return body.error;
        }
    } catch {
        // Not JSON: the status line below.
    }
    return `${method} ${path}: HTTP ${response.status}`;
}

async function call(method: string, path: string, body?: unknown): Promise<Response> {
    let response: Response;
    const headers = authHeaders(body === undefined ? {Accept: "application/json"} : {Accept: "application/json", "Content-Type": "application/json"}, path);
    try {
        response = await fetch(path, {method, headers, body: body === undefined ? undefined : JSON.stringify(body)});
    } catch (error) {
        throw new ApiUnreachableError(`${method} ${path}: ${String(error)}`, {cause: error});
    }
    if (!response.ok) throw new ApiUnreachableError(await failureMessage(method, path, response));
    return response;
}

async function json<T>(method: string, path: string, check: (body: unknown) => boolean, body?: unknown): Promise<T> {
    const parsed: unknown = await (await call(method, path, body)).json();
    if (!check(parsed)) throw new ApiShapeError(`${method} ${path}: not the shape this app expects`);
    return parsed as T;
}

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);

/** The effective UI prefs. */
export async function fetchUiPrefs(): Promise<UiPrefsBody> {
    return json<UiPrefsBody>("GET", "/api/ui-prefs", (body) => isObject(body) && "prefs" in body);
}

let session: Promise<void> | null = null;

/**
 * The session cookie MBR's pages need (they load by navigation, so they
 * can't carry the bearer token). Asked for once; a failure is forgotten so
 * the next caller asks again.
 */
export function ensureSession(): Promise<void> {
    session ??= call("POST", "/api/session").then(
        () => undefined,
        (error: unknown) => {
            session = null;
            throw error;
        }
    );
    return session;
}

/** Forget the session (tests; File → Open gets a new server process anyway). */
export function resetSession(): void {
    session = null;
}

/** `GET /api/books`: the open books, with the sidebar's counts on `today` (overdue). */
export function fetchBooks(today: string = localToday()): Promise<BooksInfo> {
    return json<BooksInfo>("GET", `/api/books?today=${today}`, (body) => isObject(body) && "basis" in body);
}

/**
 * `GET /api/invoices`: every document, newest first. "Overdue" is measured
 * against `today`, the browser's local date by default: the user's calendar,
 * not the server process's.
 */
export function fetchInvoices(today: string = localToday()): Promise<InvoiceEntry[]> {
    return json<InvoiceEntry[]>("GET", `/api/invoices?today=${today}`, Array.isArray);
}

/** `GET /api/invoices/{number}`: one document in full (`today` as above). */
export function fetchInvoice(number: string, today: string = localToday()): Promise<InvoiceDetail> {
    return json<InvoiceDetail>("GET", `/api/invoices/${encodeURIComponent(number)}?today=${today}`, (body) => isObject(body) && "path" in body);
}

/** `GET /api/invoices/{number}/books`: what the journal holds under it. */
export function fetchInvoiceBooks(number: string): Promise<InvoiceBooks> {
    return json<InvoiceBooks>("GET", `/api/invoices/${encodeURIComponent(number)}/books`, (body) => isObject(body) && "basis" in body);
}

/** `GET /api/templates`: the books' templates, then the starters not copied in. */
export function fetchTemplates(): Promise<TemplateEntry[]> {
    return json<TemplateEntry[]>("GET", "/api/templates", Array.isArray);
}

/** What `POST /api/preview` renders. */
export interface PreviewRequest {
    number?: string;
    path?: string;
    source?: string;
    template?: string;
}

/** `POST /api/preview`: a whole page, rendered; nothing is written. */
export function renderPreview(request: PreviewRequest): Promise<Preview> {
    return json<Preview>("POST", "/api/preview", (body) => isObject(body) && typeof body.html === "string", request);
}

/** `GET /api/invoices/{number}/pdf`: what the Send panel needs (PLAN 3c). */
export function fetchDelivery(number: string): Promise<Delivery> {
    return json<Delivery>("GET", `/api/invoices/${encodeURIComponent(number)}/pdf`, (body) => isObject(body) && "destination" in body);
}

/**
 * `POST /api/invoices/{number}/pdf`: export its PDF. A sent invoice's is kept
 * in `pdf-dir`; a draft's (or `temp: true`) goes to a temporary file.
 */
export function exportPdf(number: string, options: {temp?: boolean} = {}): Promise<Exported> {
    return json<Exported>("POST", `/api/invoices/${encodeURIComponent(number)}/pdf`, (body) => isObject(body) && typeof body.file === "string", {
        temp: options.temp ?? false,
        today: localToday(),
    });
}

/** `POST /api/invoices/{number}/send`: mark it sent (books it), with how it went out. */
export async function sendInvoice(number: string, via: string): Promise<void> {
    await call("POST", `/api/invoices/${encodeURIComponent(number)}/send`, {via, today: localToday()});
}

/**
 * `POST /api/invoices/{number}/portal`: the window opens the customer's portal
 * in the browser; elsewhere (`--server` in a browser) the URL comes back and
 * this opens it in a new tab.
 */
export async function openPortal(number: string): Promise<void> {
    const body = await json<{url: string; opened: boolean}>(
        "POST",
        `/api/invoices/${encodeURIComponent(number)}/portal`,
        (b) => isObject(b) && typeof b.url === "string"
    );
    if (!body.opened) window.open(body.url, "_blank", "noopener,noreferrer");
}

/** `POST /api/pdf/share`: Share… an exported PDF (macOS, the window). */
export async function sharePdf(destination: Destination, file: string, anchor?: Anchor): Promise<void> {
    await call("POST", "/api/pdf/share", {destination, file, anchor});
}

/** `POST /api/pdf/open`: open an exported PDF in the default viewer (the window, macOS). */
export async function openPdf(destination: Destination, file: string): Promise<void> {
    await call("POST", "/api/pdf/open", {destination, file});
}

/** `POST /api/pdf/copy`: Copy file, to paste into Mail, Finder or a chat. */
export async function copyPdf(destination: Destination, file: string): Promise<void> {
    await call("POST", "/api/pdf/copy", {destination, file});
}

/** What `POST /api/invoices/{number}/template` did. */
export interface TemplateUsed {
    template: string;
    /** Whether the file was rewritten (false: it already named it). */
    changed: boolean;
    /** The starter's file, when it was copied into the books now. */
    copied: string | null;
    status: string | null;
}

/**
 * `POST /api/invoices/{number}/template`: write the invoice's `template:` (as
 * `set` does), copying a starter into the books on first use.
 */
export function chooseTemplate(number: string, template: string): Promise<TemplateUsed> {
    return json<TemplateUsed>("POST", `/api/invoices/${encodeURIComponent(number)}/template`, (body) => isObject(body) && typeof body.template === "string", {
        template,
    });
}

/**
 * `POST /api/invoices/{number}/recalc`: rewrite its amounts from its rows. A
 * sent invoice is re-booked too, and only with `rebook: true` (the user
 * confirmed); the server refuses one dated in a closed period.
 */
export async function recalculate(number: string, options: {rebook?: boolean} = {}): Promise<void> {
    await call("POST", `/api/invoices/${encodeURIComponent(number)}/recalc`, {rebook: options.rebook ?? false, today: localToday()});
}
