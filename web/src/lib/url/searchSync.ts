// Store ⇄ query-string mirroring (the WP-04 replaceState pattern), written once.
//
// The store is the source of truth; the URL is a debounced projection. The URL
// is parsed INTO the store exactly once, at startup, by the caller — never in
// response to a store change — which is what keeps this from looping.
//
// This existed three times: `filters/urlSync.ts`, `holdings/ui/urlSync.ts`
// (near byte-identical, down to the same eslint-disable comment) and a third
// re-implementation inline in the reports route. The `try/catch` fallback below
// is the reason a third copy is a real cost rather than an aesthetic one — it
// is load-bearing and easy to omit.

import {browser} from "$app/environment";
import {replaceState} from "$app/navigation";

const DEBOUNCE_MS = 250;

/**
 * Write `search` to the address bar, replacing the current entry.
 *
 * Uses SvelteKit's `replaceState` so the router's own notion of the URL stays
 * in step, and falls back to the raw History API when the router is not
 * initialized (unit tests, embedding) — where `replaceState` throws.
 */
function writeSearch(search: string): void {
    if (window.location.search.replace(/^\?/, "") === search) return;
    const url = search === "" ? window.location.pathname : `${window.location.pathname}?${search}`;
    try {
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- URL is the CURRENT pathname (from window.location), not a route id to resolve
        replaceState(url, {});
    } catch {
        // Router not initialized (tests, embedding) — degrade to the raw History API.
        history.replaceState(history.state, "", url);
    }
}

/** A debounced writer to the address bar. `stop` cancels a pending write. */
export interface SearchMirror {
    write(search: string): void;
    stop(): void;
}

/**
 * A debounced query-string writer.
 *
 * For callers that already have their own change notification (a rune `$effect`
 * over reactive params). Callers driven by a store subscription want
 * `startSearchSync`, which wires both halves.
 */
export function searchMirror(debounceMs = DEBOUNCE_MS): SearchMirror {
    let timer: ReturnType<typeof setTimeout> | null = null;
    return {
        write(search: string): void {
            if (timer !== null) clearTimeout(timer);
            timer = setTimeout(() => {
                timer = null;
                writeSearch(search);
            }, debounceMs);
        },
        stop(): void {
            if (timer !== null) clearTimeout(timer);
            timer = null;
        },
    };
}

/**
 * Mirror every value a store publishes into the query string, debounced.
 *
 * `subscribe` is the store's own observer contract (fires once immediately,
 * then on every change, returning an unsubscribe). The return value stops
 * syncing and works directly as an `onMount` cleanup.
 *
 * Restoring FROM the URL is the caller's job and must happen before this: the
 * two stores differ on it (the journal filters restore only when the query
 * string is non-empty; the holdings scope always resets, so absent params mean
 * today/empty/include rather than a scope remembered from earlier in the
 * session).
 */
export function startSearchSync<T>(subscribe: (cb: (value: T) => void) => () => void, toSearch: (value: T) => string): () => void {
    if (!browser) return () => undefined;
    const mirror = searchMirror();
    const unsubscribe = subscribe((value) => mirror.write(toSearch(value)));
    return () => {
        mirror.stop();
        unsubscribe();
    };
}
