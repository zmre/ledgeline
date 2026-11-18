// Filters ⇄ URL sync (WP-04). The store is the source of truth; the URL is a
// debounced projection written with replaceState (no history entries, no
// loops — the URL is parsed into the store exactly once, at startup).
import {browser} from "$app/env";
import {expandKeyOf, parseExpand, recognitionExpand} from "#lib/journal/recognitionExpand.svelte.js";
import {defaultFilter, filters, subscribeFilters} from "#lib/stores/filters.svelte.js";
import {startSearchSync} from "#lib/url/searchSync.js";
import {filterToSearch, searchToFilter} from "./urlCodec";

export {filterToSearch, searchToFilter} from "./urlCodec";

/**
 * Restore filters from the current URL once, then mirror every filter change
 * back into the query string (debounced). Call after the SvelteKit router is
 * ready (e.g. from onMount in the page that hosts the FilterBar); the return
 * value stops syncing and works as an onMount cleanup.
 *
 * Restoring only when the query string is non-empty is this store's own rule:
 * an empty URL leaves whatever the session already had, unlike the holdings
 * scope, which always resets to today.
 */
export function startUrlSync(): () => void {
    if (!browser) return () => undefined;

    const dflt = defaultFilter();
    if (window.location.search !== "") {
        filters.replace(searchToFilter(window.location.search, dflt));
        // Plan 45 Q7: `expand=2026-11` opens that month's recognition group.
        // Read once, like the filter; the URL then mirrors the filter only.
        const expand = new URLSearchParams(window.location.search).get("expand");
        if (expand !== null) recognitionExpand.seed(parseExpand(expand), expandKeyOf(filters.value));
    }

    return startSearchSync(subscribeFilters, (f) => filterToSearch(f, dflt));
}
