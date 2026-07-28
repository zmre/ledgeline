// Filters ⇄ URL sync (WP-04). The store is the source of truth; the URL is a
// debounced projection written with replaceState (no history entries, no
// loops — the URL is parsed into the store exactly once, at startup).
import {browser} from "$app/environment";
import {defaultFilter, filters, subscribeFilters} from "$lib/stores/filters.svelte";
import {startSearchSync} from "$lib/url/searchSync";
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
    }

    return startSearchSync(subscribeFilters, (f) => filterToSearch(f, dflt));
}
