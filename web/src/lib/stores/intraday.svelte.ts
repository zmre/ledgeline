// Intraday quotes, from the page's side (plan 47 D10/D11).
//
// The engine holds one delayed quote per symbol in memory and fetches them only
// on `POST /api/prices/intraday`; a `live=1` read then values today's point with
// them, never touching the network. This store is the POST: asked after a
// short-window load (1wk/1mo, as of today), it refreshes the engine's cache and
// says whether anything landed, so the page reloads only when it has something
// new to show.
//
// The client paces itself too, not just the engine (which skips a symbol quoted
// in the last 15 minutes and backs off 15 minutes on any 429): one POST per
// server per `MIN_INTERVAL_MS`, and one in flight at a time. Every reload of the
// page — a scope change, a journal edit, the reload this very refresh triggers —
// asks again, and without this the page and the engine would play catch.
//
// Failures are silent by design: a missing quote means the saved prices stand,
// which is what the page showed before it asked. An engine from before plan 47
// (no such route) is remembered for the session and never asked again.

/* eslint-disable svelte/prefer-svelte-reactivity -- `lastAsked` and
   `unsupported` are pacing bookkeeping nothing renders, so they are plain
   collections on purpose; only `rateLimited` is shown, and it is `$state`. */
import {LedgelineApi, NativeApiUnavailableError, NotFoundError} from "#lib/api/native.js";
import {decodeIntradayResponse} from "#lib/api/nativeDecode.js";

/** At most one refresh per server this often. Shorter than the engine's 15-minute memo, so a long session still sees new quotes. */
export const MIN_INTERVAL_MS = 5 * 60_000;

/** Server → when it was last asked (ms). */
const lastAsked = new Map<string, number>();
/** Servers that have no intraday route (an engine from before plan 47). */
const unsupported = new Set<string>();
let inFlight: Promise<boolean> | null = null;
let rateLimited = $state(false);

async function ask(serverUrl: string): Promise<boolean> {
    try {
        const response = decodeIntradayResponse(await new LedgelineApi(serverUrl).refreshIntraday());
        rateLimited = response.rateLimited;
        return response.quotes.length > 0;
    } catch (cause) {
        if (cause instanceof NotFoundError || cause instanceof NativeApiUnavailableError) unsupported.add(serverUrl);
        return false;
    }
}

export const intraday = {
    /** The last refresh was cut short by a rate limit: saved prices are showing where a live quote was wanted. */
    get rateLimited(): boolean {
        return rateLimited;
    },
    /**
     * Refresh the engine's intraday cache for every held stock, unless this
     * server was asked within `MIN_INTERVAL_MS`, a refresh is already running,
     * or it has no such route. Resolves true when the reply carried at least
     * one quote — the caller's cue to reload with `live=1` — and false in every
     * other case, failures included. `now` is injectable for tests.
     */
    refresh(serverUrl: string, now: number = Date.now()): Promise<boolean> {
        if (unsupported.has(serverUrl) || inFlight !== null) return Promise.resolve(false);
        const last = lastAsked.get(serverUrl);
        if (last !== undefined && now - last < MIN_INTERVAL_MS) return Promise.resolve(false);
        lastAsked.set(serverUrl, now);
        inFlight = ask(serverUrl).finally(() => {
            inFlight = null;
        });
        return inFlight;
    },
    /** Forget every pacing and support decision (tests). */
    reset(): void {
        lastAsked.clear();
        unsupported.clear();
        inFlight = null;
        rateLimited = false;
    },
};
