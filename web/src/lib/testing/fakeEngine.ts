// A stubbed global `fetch` that answers by URL suffix — how a test drives a REAL
// store with literal wire JSON instead of mocking the store away.
//
// This is the seam `importStore.svelte.ts` documents and `importStore.test.ts`
// already uses: every request in the app goes out through `LedgelineApi` /
// `HledgerApi`, both of which call the global `fetch`, so there is no injected
// transport to build and no second implementation of these calls to keep true.
//
// It lives here because the `components` project needs the same seam: a
// component test that mocked its store would prove the component renders
// whatever it is handed, which is never the thing that broke — what broke was
// the value the component was handed. Wiring the real store to fake bytes is
// what makes that assertable.

import {settings} from "#lib/stores/settings.svelte.js";
import {vi} from "vitest";

/** The address the fake engine answers on. Nothing listens there; `fetch` is stubbed. */
export const FAKE_ENGINE = "http://engine.test";

/**
 * A minimal `/api/journal` body: an untitled journal with nothing declared,
 * no classified accounts and a fixed generation. Spread `overrides` over it for
 * the fields a test is about.
 */
export function journalInfoBody(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        title: null,
        file: null,
        commodityStyles: {},
        reportStyles: {},
        dominantCommodity: "$",
        txnCount: 0,
        firstDate: null,
        lastDate: null,
        accounts: {},
        generation: "fake-1",
        ...overrides,
    };
}

export const FAKE_JOURNAL_INFO = journalInfoBody();

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

/**
 * A `fetch` answering any request whose URL ENDS WITH one of `table`'s keys.
 *
 * Suffix matching rather than exact, so a route can be written as the path the
 * API client builds (`/api/aliases`) without repeating the base URL, and an
 * unrouted request 404s loudly rather than hanging.
 */
export function routes(table: Record<string, unknown>): (url: string) => Promise<Response> {
    return (url: string) => {
        // A route matches the full URL OR its path alone, so a table entry need
        // not spell out a query string the client builds (`?asOf=…`).
        const path = url.split("?", 1)[0];
        const key = Object.keys(table).find((route) => url.endsWith(route)) ?? Object.keys(table).find((route) => path.endsWith(route));
        const body = key === undefined ? undefined : table[key];
        if (body === undefined) return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
        return Promise.resolve(body instanceof Response ? body : json(body));
    };
}

/**
 * Point `settings` at a fake engine serving `table`. Pair with
 * `afterEach(() => vi.unstubAllGlobals())`.
 *
 * `/version` is always answered because `settings.setServerUrl` verifies it
 * before it will persist an address, and it is overridable so a test can make
 * the probe fail. `/api/journal` and `/api/diagnostics` are answered too
 * (overridably): the journal round requires both, since the SPA requires the
 * native API.
 */
export async function connectFakeEngine(table: Record<string, unknown> = {}): Promise<void> {
    vi.stubGlobal(
        "fetch",
        routes({"/version": "1.52", "/api/journal": FAKE_JOURNAL_INFO, "/api/diagnostics": {count: 0, maxSeverity: null, diagnostics: []}, ...table})
    );
    // `settings` is a module singleton shared by every test in the file. Calling
    // `setServerUrl` twice would bump the reconnect nonce, which is the exact
    // signal every `onServerReady` latch keys on — so a second call silently
    // re-fires every mount-time fetch in the tree.
    if (settings.serverUrl === null) await settings.setServerUrl(FAKE_ENGINE);
}
