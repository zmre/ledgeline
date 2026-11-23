// The invoicer's API client (invoicer PLAN 3a, 3b). The SPA is served by the
// ledgeline-invoicer server under /app, so every request is same-origin and
// relative; under `vite dev` and `vite preview` (port 4273) the vite proxy
// forwards everything outside /app to the `--server` on 5199. The token comes
// from the shared client in packages/ui: injected by the window, or adopted
// from the `#token=` launch fragment.
import {ApiShapeError, ApiUnreachableError, authHeaders} from "#ui/api/client.js";
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

/** `GET /api/books`: the open books. */
export function fetchBooks(): Promise<BooksInfo> {
    return json<BooksInfo>("GET", "/api/books", (body) => isObject(body) && "basis" in body);
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
