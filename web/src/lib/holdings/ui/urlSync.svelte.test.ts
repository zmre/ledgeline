// The browser glue urlCodec.test.ts deliberately leaves out: restore-on-
// navigation and the debounced mirror, wired together. A `.svelte.test.ts` on
// the same trade keymap.svelte.test.ts makes: nothing here mounts a component,
// but the store's `subscribeHoldingsUrlState` is a rune `$effect`, which only
// fires on Svelte's CLIENT build (the components project), and the sync itself
// reads `window.location` (jsdom).
//
// The bug these pin: the scope and tab were read from the URL exactly once, in
// onMount. A navigation that changes only the query string — the app-bar
// Holdings link clicked while already on /holdings, back/forward between two
// /holdings entries — reuses the page component, so the store kept the OLD
// state while the address bar showed the new one, and the next debounced
// mirror write replaceState-overwrote the URL the user had navigated to.

import {flushSync} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {defaultScope, holdingsScope, holdingsTab} from "$lib/stores/holdings.svelte";
import {startHoldingsUrlSync} from "./urlSync";

const mocks = vi.hoisted(() => ({
    /** Callbacks registered via afterNavigate; the tests fire them, playing router. */
    navigations: [] as Array<() => void>,
    /** Every URL the mirror pushed through SvelteKit's replaceState. */
    replaceStateCalls: [] as string[],
}));

vi.mock("$app/environment", () => ({browser: true}));
vi.mock("$app/navigation", () => ({
    afterNavigate: (cb: () => void): void => {
        mocks.navigations.push(cb);
    },
    // SvelteKit's shallow replaceState: commits the URL and fires NO navigation
    // callbacks — that skip is exactly how the sync tells its own mirror writes
    // apart from real navigations, so the mock must reproduce it.
    replaceState: (url: string | URL): void => {
        mocks.replaceStateCalls.push(String(url));
        window.history.replaceState(null, "", url);
    },
}));

/** A real navigation: the router commits the URL FIRST, then fires afterNavigate. */
function navigate(url: string): void {
    window.history.pushState(null, "", url);
    for (const cb of mocks.navigations) cb();
}

/** Flush the rune subscription, then let the mirror's debounce fire. */
function settle(): void {
    flushSync();
    vi.advanceTimersByTime(300);
}

// Module-level runes state and a module-level URL are both shared across the
// file, so every test starts from a bare /holdings with a fresh default store.
let stop: (() => void) | null = null;

beforeEach(() => {
    vi.useFakeTimers();
    window.history.replaceState(null, "", "/holdings");
    holdingsScope.replace(defaultScope());
    holdingsTab.value = "stocks";
    flushSync();
    mocks.navigations.length = 0;
    mocks.replaceStateCalls.length = 0;
});

afterEach(() => {
    stop?.();
    stop = null;
    vi.useRealTimers();
});

describe("COMPONENT holdings urlSync — restore on navigation", () => {
    it("re-restores tab AND scope when a navigation changes only the query string", () => {
        stop = startHoldingsUrlSync();
        settle();

        // Same route, new query — the component is reused, onMount does not re-run.
        navigate("/holdings?asof=2020-01-02&acct=Assets&mode=exclude&gain=ytd&tab=other");

        // Without the afterNavigate restore nothing re-reads the URL, and every
        // line below still sees the mount-time defaults.
        expect(holdingsTab.value).toBe("other");
        expect(holdingsScope.value.asOf).toBe("2020-01-02");
        expect([...holdingsScope.value.accounts]).toEqual(["Assets"]);
        expect(holdingsScope.value.mode).toBe("exclude");
        expect(holdingsScope.value.gainPeriod).toBe("ytd");
    });

    it("does not echo a navigation-triggered restore back into the URL, and a re-fire replaces nothing", () => {
        stop = startHoldingsUrlSync();
        settle();
        mocks.replaceStateCalls.length = 0;

        navigate("/holdings?tab=other");
        settle();

        // The restore happened (this is the line that fails without the fix)…
        expect(holdingsTab.value).toBe("other");
        // …and produced no write: the store now serializes to exactly the URL,
        // so the debounced mirror compares equal and stays silent. A restore
        // that echoed would fight the very navigation it was restoring.
        expect(mocks.replaceStateCalls).toEqual([]);

        // afterNavigate also fires once around mount ('enter') — a duplicate
        // with the same URL. Idempotence via the codec: the scope object is not
        // even replaced, so no subscription churn, no debounce woken.
        const scopeBefore = holdingsScope.value;
        for (const cb of mocks.navigations) cb();
        settle();
        expect(holdingsScope.value).toBe(scopeBefore);
        expect(mocks.replaceStateCalls).toEqual([]);
    });

    it("no longer rewrites a freshly navigated-to URL from stale store state (the app-bar repro)", () => {
        stop = startHoldingsUrlSync();
        settle();

        // On /holdings the user opens Other; the mirror writes ?tab=other.
        holdingsTab.value = "other";
        settle();
        expect(window.location.search).toBe("?tab=other");

        // App-bar "Holdings" link: a real navigation to the bare route.
        navigate("/holdings");
        expect(holdingsTab.value).toBe("stocks");

        // The next mirrored edit describes the NEW state only. Before the fix
        // the store still said "other", and this exact write silently put
        // `&tab=other` back into the address bar.
        holdingsScope.setAsOf("2020-01-02");
        settle();
        expect(window.location.search).toBe("?asof=2020-01-02");
    });
});
