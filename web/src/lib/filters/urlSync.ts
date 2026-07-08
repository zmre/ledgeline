// Filters ⇄ URL sync (WP-04). The store is the source of truth; the URL is a
// debounced projection written with replaceState (no history entries, no
// loops — the URL is parsed into the store exactly once, at startup).
import {browser} from "$app/environment";
import {replaceState} from "$app/navigation";
import {defaultFilter, filters, subscribeFilters} from "$lib/stores/filters.svelte";
import {filterToSearch, searchToFilter} from "./urlCodec";

export {filterToSearch, searchToFilter} from "./urlCodec";

const DEBOUNCE_MS = 250;

/**
 * Restore filters from the current URL once, then mirror every filter change
 * back into the query string (debounced). Call after the SvelteKit router is
 * ready (e.g. from onMount in the page that hosts the FilterBar); the return
 * value stops syncing and works as an onMount cleanup.
 */
export function startUrlSync(): () => void {
    if (!browser) return () => undefined;

    const dflt = defaultFilter();
    if (window.location.search !== "") {
        filters.replace(searchToFilter(window.location.search, dflt));
    }

    let timer: ReturnType<typeof setTimeout> | null = null;
    const unsubscribe = subscribeFilters((f) => {
        const search = filterToSearch(f, dflt);
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
            timer = null;
            if (window.location.search.replace(/^\?/, "") === search) return;
            const url = search === "" ? window.location.pathname : `${window.location.pathname}?${search}`;
            try {
                // eslint-disable-next-line svelte/no-navigation-without-resolve -- URL is the CURRENT pathname (from window.location), not a route id to resolve
                replaceState(url, {});
            } catch {
                // Router not initialized (tests, embedding) — degrade to the raw History API.
                history.replaceState(history.state, "", url);
            }
        }, DEBOUNCE_MS);
    });

    return () => {
        if (timer !== null) clearTimeout(timer);
        unsubscribe();
    };
}
