// hledger-web JSON API client (WP-02). Fetch + error taxonomy only; wire-shape
// knowledge lives in normalize.ts.
//
// This module also owns the ACCESS TOKEN the Ledgeline engine requires on every
// wire and /api route: `authHeaders` is the single place it is attached, and
// both this client and the native one (native.ts) route every fetch through it.

/** localStorage key for the persisted settings blob. Owned here (rather than in
 * the settings store) so `apiToken` can read it without importing a runes module
 * into the plain-TS clients — settings.svelte.ts imports it back from here. */
export const SETTINGS_STORAGE_KEY = "ledgeline.settings.v1";

/**
 * The engine's per-process access token, or null when there is none to send.
 *
 * Two sources, in order:
 *  1. `window.__LEDGELINE_TOKEN__`, injected into index.html by the binary that
 *     serves the SPA. Same-origin pages get it for free; a cross-origin page
 *     cannot read it, which is the point.
 *  2. `serverToken` in the persisted settings, for the cross-origin cases — vite
 *     dev and the Playwright suite — where the SPA is served by something other
 *     than the engine and the token comes from $LEDGELINE_TOKEN.
 *  3. `$LEDGELINE_TOKEN` itself, for the node integration suite, which drives
 *     this client directly with neither a `window` nor a `localStorage` to read.
 *     Unreachable from a browser bundle (`process` is undefined there).
 *
 * A plain hledger-web has no token; null simply means "send no Authorization",
 * which is what that server expects.
 */
export function apiToken(): string | null {
    if (typeof window !== "undefined") {
        const injected = (window as {__LEDGELINE_TOKEN__?: string}).__LEDGELINE_TOKEN__;
        if (typeof injected === "string" && injected !== "") return injected;
    }
    if (typeof localStorage !== "undefined") {
        try {
            const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
            const parsed = raw === null ? {} : (JSON.parse(raw) as {serverToken?: unknown});
            if (typeof parsed.serverToken === "string" && parsed.serverToken !== "") return parsed.serverToken;
        } catch {
            // Corrupt settings blob: fall through to the env fallback below.
        }
    }
    if (typeof process !== "undefined") {
        const fromEnv = process.env?.LEDGELINE_TOKEN;
        if (typeof fromEnv === "string" && fromEnv !== "") return fromEnv;
    }
    return null;
}

/** `base` plus `Authorization: Bearer <token>` when a token is available. */
export function authHeaders(base: Record<string, string>): Record<string, string> {
    const token = apiToken();
    return token === null ? base : {...base, Authorization: `Bearer ${token}`};
}

/** Network/CORS/HTTP failure — the setup modal reacts by showing the launch command. */
export class ApiUnreachableError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "ApiUnreachableError";
    }
}

/** The server answered, but the JSON was not what an hledger-web API returns. */
export class ApiShapeError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "ApiShapeError";
    }
}

/**
 * The request was still outstanding when its deadline passed.
 *
 * A SUBCLASS of [`ApiUnreachableError`] on purpose: to every caller a request
 * that never came back is a server that cannot be reached, and they already
 * classify that (the setup modal's launch hint, `editing`'s "network" failure
 * kind). The distinct type only exists so the message can say what happened.
 */
export class ApiTimeoutError extends ApiUnreachableError {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "ApiTimeoutError";
    }
}

/**
 * How long any single request may take, headers AND body, before it is aborted.
 *
 * There was no deadline at all, and nothing in `web/src` passed a `signal`, so
 * one request to a server that accepted the connection and then went quiet hung
 * forever. That is worse than it sounds: the journal store dedups concurrent
 * refreshes behind one promise, so the poller, the toolbar's refresh button and
 * every error toast's Retry all handed back that same dead promise, and
 * `editing.run()`'s `finally { busy = false }` never ran — the edit popup froze
 * behind a spinner with no way out (FE-5f).
 */
export const REQUEST_TIMEOUT_MS = 30_000;

/**
 * Run `request` under a deadline, wired to an optional caller `signal` so a
 * superseded round can cancel it early.
 *
 * The deadline covers the whole exchange, not just the headers: `run` receives
 * the composed signal and the timer is only cleared once its promise settles,
 * so a server that sends `200 OK` and then stalls the body is caught too.
 */
export async function withDeadline<T>(what: string, timeoutMs: number, signal: AbortSignal | undefined, run: (signal: AbortSignal) => Promise<T>): Promise<T> {
    const controller = new AbortController();
    let timedOut = false;
    const timer = setTimeout(() => {
        timedOut = true;
        controller.abort();
    }, timeoutMs);
    const relay = (): void => controller.abort();
    if (signal !== undefined) {
        if (signal.aborted) controller.abort();
        else signal.addEventListener("abort", relay, {once: true});
    }
    try {
        return await run(controller.signal);
    } catch (cause) {
        if (timedOut) throw new ApiTimeoutError(`${what} timed out after ${Math.round(timeoutMs / 1000)}s`, {cause});
        throw cause;
    } finally {
        clearTimeout(timer);
        signal?.removeEventListener("abort", relay);
    }
}

/**
 * What a conditional GET returns when the server answered `304 Not Modified`:
 * the body we already have is still current, and no body was transferred.
 *
 * The journal endpoints ship the entire journal — hundreds of megabytes on a
 * large one — and the poller refetches every 30 seconds. Recognizing a 304 is
 * what makes an unchanged poll cost nothing at all rather than a full download,
 * parse and normalize (PERF-2).
 */
export const NOT_MODIFIED = Symbol("ledgeline.notModified");
/** A conditional-GET result: the value, or [`NOT_MODIFIED`]. */
export type Conditional<T> = T | typeof NOT_MODIFIED;

/** Narrowing helper for [`Conditional`] results. */
export function isNotModified(value: unknown): value is typeof NOT_MODIFIED {
    return value === NOT_MODIFIED;
}

/**
 * The last `ETag` seen per absolute URL.
 *
 * Module-level, not per-instance: `HledgerApi` is constructed fresh on every
 * refresh, so an instance field would never survive to be revalidated against.
 *
 * A plain hledger-web sends no `ETag`, so nothing is ever recorded for it and
 * every request stays unconditional. Same for a cross-origin SPA, where the
 * browser hides `ETag` unless the server exposes it via CORS — which also means
 * we never send `If-None-Match` cross-origin and never turn a simple request
 * into a preflighted one.
 */
const etagByUrl = new Map<string, string>();

/**
 * Forget every recorded `ETag`, so the next requests are unconditional.
 *
 * The caller needs this when a round comes back MIXED — some routes 304, some
 * 200 — which means the journal was swapped between the requests and the
 * unchanged halves cannot be combined with the changed ones.
 */
export function resetConditionalCache(): void {
    etagByUrl.clear();
}

function stringArray(value: unknown, route: string): string[] {
    if (!Array.isArray(value) || !value.every((item): item is string => typeof item === "string")) {
        throw new ApiShapeError(`GET ${route}: expected a JSON array of strings`);
    }
    return value;
}

/** Extra behaviour a caller can ask an [`HledgerApi`] for. */
export interface HledgerApiOptions {
    /**
     * Send `If-None-Match` on the big journal routes and let them answer
     * [`NOT_MODIFIED`].
     *
     * OFF by default, and deliberately so: a conditional GET can return
     * something that is not a body, so only a caller that keeps the previous
     * result and knows what to do with a 304 may switch it on. Today that is the
     * journal store's poller, which is also the only caller that repeats these
     * requests often enough for it to matter.
     */
    conditional?: boolean;
    /** Cancels every request this client makes — a superseded refresh round aborts its predecessor. */
    signal?: AbortSignal;
    /** Per-request deadline; defaults to [`REQUEST_TIMEOUT_MS`]. */
    timeoutMs?: number;
}

export class HledgerApi {
    readonly baseUrl: string;
    /** Token to send instead of the ambient one. Lets the setup flow verify a
     * candidate server+token without persisting either first. */
    private readonly token?: string;
    /** See [`HledgerApiOptions.conditional`]. */
    private readonly conditional: boolean;
    private readonly signal?: AbortSignal;
    private readonly timeoutMs: number;

    constructor(baseUrl: string, token?: string, options?: HledgerApiOptions) {
        this.baseUrl = baseUrl.replace(/\/+$/, "");
        this.token = token;
        this.conditional = options?.conditional ?? false;
        this.signal = options?.signal;
        this.timeoutMs = options?.timeoutMs ?? REQUEST_TIMEOUT_MS;
    }

    private headers(): Record<string, string> {
        const base = {Accept: "application/json"};
        return this.token === undefined ? authHeaders(base) : {...base, Authorization: `Bearer ${this.token}`};
    }

    /**
     * `GET route`, returning the parsed JSON body.
     *
     * `revalidatable` marks a route whose payload is worth an `If-None-Match`;
     * it only takes effect on a client constructed with `conditional: true`, so
     * a caller that cannot handle [`NOT_MODIFIED`] never receives it. When both
     * hold, the last `ETag` seen for this URL is replayed and a `304` returns
     * [`NOT_MODIFIED`] instead of a body.
     *
     * `cache: "no-store"` stays either way: revalidation is ours to do
     * explicitly, so the browser cache never gets to answer for the engine.
     */
    private async get(route: string, revalidatable = false): Promise<unknown> {
        const conditional = revalidatable && this.conditional;
        const url = `${this.baseUrl}${route}`;
        const headers = this.headers();
        const known = conditional ? etagByUrl.get(url) : undefined;
        if (known !== undefined) headers["If-None-Match"] = known;
        return withDeadline(`GET ${url}`, this.timeoutMs, this.signal, async (signal) => {
            let response: Response;
            try {
                // no-store: journal data must always come from the live server, never the HTTP cache
                response = await fetch(url, {headers, cache: "no-store", signal});
            } catch (cause) {
                throw new ApiUnreachableError(`Cannot reach hledger-web at ${this.baseUrl} (network or CORS failure)`, {cause});
            }
            if (response.status === 304) return NOT_MODIFIED;
            if (!response.ok) {
                throw new ApiUnreachableError(`GET ${url} responded ${response.status} ${response.statusText}`);
            }
            let body: unknown;
            try {
                body = (await response.json()) as unknown;
            } catch (cause) {
                throw new ApiShapeError(`GET ${route}: response is not valid JSON`, {cause});
            }
            // Recorded only once the body is IN HAND. Recording it off the
            // headers meant an aborted or timed-out read — which now happens by
            // design, on every superseded round — left behind a tag for a
            // payload we never parsed; the next poll would send it, be told 304,
            // and keep the older journal it still had while believing it current.
            if (conditional) {
                const etag = response.headers.get("ETag");
                // Drop a stale entry when the server stops sending one, so we never
                // revalidate against a tag it no longer knows.
                if (etag === null) etagByUrl.delete(url);
                else etagByUrl.set(url, etag);
            }
            return body;
        });
    }

    async version(): Promise<string> {
        const value = await this.get("/version");
        if (typeof value !== "string") throw new ApiShapeError("GET /version: expected a JSON string");
        return value;
    }

    /** Raw wire JSON; pass through normalizeTransactions separately. */
    transactions(): Promise<Conditional<unknown>> {
        return this.get("/transactions", true);
    }

    /** Deliberately UNCONDITIONAL. Every wire payload derives from one server
     * snapshot and carries its one ETag, so `/transactions` + `/prices` +
     * `/accounts` all answering 304 already proves the account names are
     * unchanged too — and this list is kilobytes, not megabytes, so making its
     * callers handle a 304 buys nothing. */
    async accountNames(): Promise<string[]> {
        return stringArray(await this.get("/accountnames"), "/accountnames");
    }

    /** Raw wire JSON; pass through normalizePrices separately. */
    prices(): Promise<Conditional<unknown>> {
        return this.get("/prices", true);
    }

    async commodities(): Promise<string[]> {
        return stringArray(await this.get("/commodities"), "/commodities");
    }

    /** Raw wire JSON (account tree with declaration info); pass through normalizeAccounts separately. */
    accounts(): Promise<Conditional<unknown>> {
        return this.get("/accounts", true);
    }
}
