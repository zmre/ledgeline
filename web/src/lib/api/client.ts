// hledger-web JSON API client (WP-02). Fetch + error taxonomy only; wire-shape
// knowledge lives in normalize.ts.
//
// The token, the error classes, the deadline and the conditional-GET marker are
// the shared base in `#ui/api/client` (packages/ui), re-exported here so the
// app has one client module to import from. What stays here is the part only
// Ledgeline has: the hledger-web routes and their ETag cache.

import {ApiShapeError, ApiUnreachableError, authHeaders, NOT_MODIFIED, REQUEST_TIMEOUT_MS, withDeadline, type Conditional} from "#ui/api/client.js";

export {
    adoptLaunchToken,
    apiToken,
    ApiShapeError,
    ApiTimeoutError,
    ApiUnreachableError,
    authHeaders,
    isNotModified,
    LAUNCH_TOKEN_STORAGE_KEY,
    NOT_MODIFIED,
    REQUEST_TIMEOUT_MS,
    SETTINGS_STORAGE_KEY,
    withDeadline,
    type Conditional,
} from "#ui/api/client.js";

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

    /** Headers for a request to `url`, which is passed on so the ambient token is
     * only ever attached to an origin it belongs to (SEC-16). An EXPLICIT token
     * skips that check on purpose: it is the one the user just typed into the
     * setup modal for the server they are pointing at. */
    private headers(url: string): Record<string, string> {
        const base = {Accept: "application/json"};
        return this.token === undefined ? authHeaders(base, url) : {...base, Authorization: `Bearer ${this.token}`};
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
        const headers = this.headers(url);
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
}
