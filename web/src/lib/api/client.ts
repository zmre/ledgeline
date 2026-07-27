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

function stringArray(value: unknown, route: string): string[] {
    if (!Array.isArray(value) || !value.every((item): item is string => typeof item === "string")) {
        throw new ApiShapeError(`GET ${route}: expected a JSON array of strings`);
    }
    return value;
}

export class HledgerApi {
    readonly baseUrl: string;
    /** Token to send instead of the ambient one. Lets the setup flow verify a
     * candidate server+token without persisting either first. */
    private readonly token?: string;

    constructor(baseUrl: string, token?: string) {
        this.baseUrl = baseUrl.replace(/\/+$/, "");
        this.token = token;
    }

    private headers(): Record<string, string> {
        const base = {Accept: "application/json"};
        return this.token === undefined ? authHeaders(base) : {...base, Authorization: `Bearer ${this.token}`};
    }

    private async get(route: string): Promise<unknown> {
        const url = `${this.baseUrl}${route}`;
        let response: Response;
        try {
            // no-store: journal data must always come from the live server, never the HTTP cache
            response = await fetch(url, {headers: this.headers(), cache: "no-store"});
        } catch (cause) {
            throw new ApiUnreachableError(`Cannot reach hledger-web at ${this.baseUrl} (network or CORS failure)`, {cause});
        }
        if (!response.ok) {
            throw new ApiUnreachableError(`GET ${url} responded ${response.status} ${response.statusText}`);
        }
        try {
            return (await response.json()) as unknown;
        } catch (cause) {
            throw new ApiShapeError(`GET ${route}: response is not valid JSON`, {cause});
        }
    }

    async version(): Promise<string> {
        const value = await this.get("/version");
        if (typeof value !== "string") throw new ApiShapeError("GET /version: expected a JSON string");
        return value;
    }

    /** Raw wire JSON; pass through normalizeTransactions separately. */
    transactions(): Promise<unknown> {
        return this.get("/transactions");
    }

    async accountNames(): Promise<string[]> {
        return stringArray(await this.get("/accountnames"), "/accountnames");
    }

    /** Raw wire JSON; pass through normalizePrices separately. */
    prices(): Promise<unknown> {
        return this.get("/prices");
    }

    async commodities(): Promise<string[]> {
        return stringArray(await this.get("/commodities"), "/commodities");
    }

    /** Raw wire JSON (account tree with declaration info); pass through normalizeAccounts separately. */
    accounts(): Promise<unknown> {
        return this.get("/accounts");
    }
}
