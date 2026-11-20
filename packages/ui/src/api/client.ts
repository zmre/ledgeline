// The shared fetch plumbing for an engine-backed SPA (WP-02, extracted for the
// invoicer, plan L6): the access token, the error taxonomy, request deadlines
// and the conditional-GET marker. The endpoint wrappers live in each app —
// Ledgeline's hledger-web routes are `HledgerApi` in web/src/lib/api/client.ts,
// which re-exports everything here.
//
// This module owns the ACCESS TOKEN the Ledgeline engine requires on every
// wire and /api route: `authHeaders` is the single place it is attached, and
// every client (`HledgerApi`, the native one in native.ts) routes every fetch
// through it. It is also the single place that decides WHERE it may be
// attached — see `tokenMayBeSentTo` (SEC-16).

/** localStorage key for the persisted settings blob. Owned here (rather than in
 * the settings store) so `apiToken` can read it without importing a runes module
 * into the plain-TS clients — settings.svelte.ts imports it back from here. */
export const SETTINGS_STORAGE_KEY = "ledgeline.settings.v1";

/** sessionStorage key for the token a headless (`--server`) engine handed this
 * tab in its launch URL's `#token=` fragment. See [`adoptLaunchToken`]. */
export const LAUNCH_TOKEN_STORAGE_KEY = "ledgeline.launchToken";

/**
 * Move a `#token=…` launch fragment into `sessionStorage` and strip it from the
 * address bar. Idempotent and cheap, so it is safe to call on every lookup.
 *
 * `ledgeline --server` prints `http://127.0.0.1:PORT/#token=…` rather than
 * putting the token in the served page, because the page is served to anyone
 * who asks and HTTP cannot tell the user's browser from another local process.
 * A fragment never goes over the wire. `sessionStorage` keeps it per tab and
 * gone with the tab, which matches the token's own lifetime (one engine run)
 * better than `localStorage` would. Stripping it keeps the token out of the
 * address bar, bookmarks and anything copied from there.
 *
 * Other fragment parameters, should there ever be any, are left alone.
 */
export function adoptLaunchToken(): void {
    if (typeof window === "undefined") return;
    const {location, history} = window;
    const hash = location?.hash;
    if (typeof hash !== "string" || !hash.startsWith("#")) return;
    const params = new URLSearchParams(hash.slice(1));
    const token = params.get("token");
    if (token === null) return;
    if (token !== "") {
        try {
            sessionStorage.setItem(LAUNCH_TOKEN_STORAGE_KEY, token);
        } catch {
            // Storage disabled: the token cannot be kept, and stripping it below
            // still keeps it out of the address bar.
        }
    }
    params.delete("token");
    const rest = params.toString();
    try {
        history.replaceState(history.state, "", `${location.pathname}${location.search}${rest === "" ? "" : `#${rest}`}`);
    } catch {
        // Not fatal: the token has been adopted either way.
    }
}

/** The token adopted from a launch URL by [`adoptLaunchToken`], or null. */
function launchToken(): string | null {
    adoptLaunchToken();
    if (typeof sessionStorage === "undefined") return null;
    try {
        const stored = sessionStorage.getItem(LAUNCH_TOKEN_STORAGE_KEY);
        return stored === null || stored === "" ? null : stored;
    } catch {
        return null;
    }
}

/**
 * The engine's per-process access token, or null when there is none to send.
 *
 * Four sources, in order:
 *  1. `window.__LEDGELINE_TOKEN__`, set by the packaged GUI through wry's
 *     initialization script — out of band, never in a response body, and only
 *     on the engine's own origin.
 *  2. The launch token a headless `ledgeline --server` printed as
 *     `http://127.0.0.1:PORT/#token=…`, adopted into this tab's
 *     `sessionStorage` by [`adoptLaunchToken`]. Ahead of source 3 on purpose: it
 *     is the token of the engine this tab was opened against, and a
 *     `serverToken` left in `localStorage` by an earlier run is not.
 *  3. `serverToken` in the persisted settings, for the cross-origin cases — vite
 *     dev and the Playwright suite — where the SPA is served by something other
 *     than the engine and the token comes from $LEDGELINE_TOKEN.
 *  4. `$LEDGELINE_TOKEN` itself, for the node integration suite, which drives
 *     this client directly with neither a `window` nor a `localStorage` to read.
 *     Unreachable from a browser bundle (`process` is undefined there).
 *
 * The served page itself never carries the token: it is fetched without one,
 * so anything that can reach loopback can read it.
 *
 * A plain hledger-web has no token; null simply means "send no Authorization",
 * which is what that server expects.
 *
 * # Why source 3 stays in `localStorage`
 *
 * It is worth asking, because a bearer token in `localStorage` is readable by
 * anything running on the page. The answer is that it is LOAD-BEARING and only
 * for the cross-origin cases: the SPA at `:4173`/`:5173` is a different origin
 * from the engine at `:5099`, so it cannot be handed the token the way the
 * packaged app is, and every e2e spec seeds it there before its first request.
 * Dropping it would break `just dev` and the whole Playwright suite.
 *
 * In the packaged app it is not used at all: `window.__LEDGELINE_TOKEN__` is
 * checked first and always wins, and nothing writes `serverToken` because the
 * embedded SPA never shows the setup modal that collects one. So the exposure is
 * confined to development, where the token is a fixed `$LEDGELINE_TOKEN` against
 * a fixture journal. What DOES change is where that token may be sent — see
 * `tokenMayBeSentTo`.
 */
export function apiToken(): string | null {
    if (typeof window !== "undefined") {
        const injected = (window as {__LEDGELINE_TOKEN__?: string}).__LEDGELINE_TOKEN__;
        if (typeof injected === "string" && injected !== "") return injected;
        const launched = launchToken();
        if (launched !== null) return launched;
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

/**
 * The origin of the engine the user actually configured, or null.
 *
 * Read from the persisted blob rather than the settings store for the same
 * reason [`apiToken`] is: this module must not import a runes module.
 */
function configuredEngineOrigin(): string | null {
    if (typeof localStorage === "undefined") return null;
    try {
        const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
        const parsed = raw === null ? {} : (JSON.parse(raw) as {serverUrl?: unknown});
        if (typeof parsed.serverUrl !== "string" || parsed.serverUrl === "") return null;
        return new URL(parsed.serverUrl).origin;
    } catch {
        // No stored URL, or one that is not parseable as an origin.
        return null;
    }
}

/**
 * May the access token be sent to `url`?
 *
 * SEC-16. `authHeaders` used to attach the token to whatever URL it was handed,
 * and the token is persisted in `localStorage`. `HledgerApi` is constructed with
 * an ARBITRARY base URL by `settings.setServerUrl` — which is the setup modal's
 * "verify this server" button — so typing any address into that field and
 * leaving the token box empty sent the running engine's credential to that
 * address. Two origins are legitimate targets and nothing else is:
 *
 *  1. **Our own.** The packaged app serves the SPA and the API same-origin, and
 *     `window.__LEDGELINE_TOKEN__` (or a headless engine's launch token) is
 *     handed to that page for exactly this.
 *  2. **The engine the user configured**, i.e. the origin of the stored
 *     `serverUrl`. This is what keeps the documented CROSS-ORIGIN flows working:
 *     `just dev` and `web/playwright.config.ts` run the SPA at one origin
 *     (`:5173` / `:4173`) and the engine at another (`:5099`), and every e2e spec
 *     seeds `{serverUrl, serverToken}` into `localStorage` before the first
 *     request. The token still goes to `:5099`, because that is what is stored.
 *
 * Verifying a NEW address with an empty token box now sends no token, which is
 * both what an empty token box means and the right answer anyway: a different
 * engine process has a different token, so the old one was never going to work.
 *
 * Outside a browser (the node integration suite, which drives this client
 * directly with `$LEDGELINE_TOKEN`) there is no origin to compare against and no
 * other page to leak to, so the caller's own target stands.
 */
function tokenMayBeSentTo(url: string): boolean {
    if (typeof window === "undefined") return true;
    let target: string;
    try {
        target = new URL(url, window.location.href).origin;
    } catch {
        // Not a resolvable URL: fail closed rather than guess.
        return false;
    }
    return target === window.location.origin || target === configuredEngineOrigin();
}

/**
 * `base` plus `Authorization: Bearer <token>` when a token is available AND
 * `url` is an origin the token may go to (see [`tokenMayBeSentTo`]).
 *
 * `url` is optional only because `native.ts` calls this from four places and is
 * always aimed at the configured engine already; pass it wherever the target can
 * be anything else.
 */
export function authHeaders(base: Record<string, string>, url?: string): Record<string, string> {
    if (url !== undefined && !tokenMayBeSentTo(url)) return base;
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
 * The server answered, but it is not the engine this SPA needs: a 404 or a
 * non-JSON body on an `/api/*` route. Shown bare (its message is a complete
 * sentence naming the remedy) and with no Retry, which cannot help — see
 * `AsyncSection`. Each app supplies its own message.
 */
export class NativeApiUnavailableError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "NativeApiUnavailableError";
    }
}
