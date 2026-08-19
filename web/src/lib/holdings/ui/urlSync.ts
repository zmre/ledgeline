// Holdings scope + tab ⇄ URL sync (WP-10, extended by plans/14), the WP-04
// replaceState pattern: the store is the source of truth; the URL is a debounced
// projection. Unlike the journal filters, the scope is ALWAYS reset from the URL
// on mount — absent params mean today/empty/include/Stocks (plans/10), never a
// scope remembered from a previous visit in the same session.
//
// One `startSearchSync` covers BOTH the scope and the tab. That is the point:
// two writers over one query string would race on their debounces, and the later
// one would drop the other's keys.
import {browser} from "$app/environment";
import {localToday} from "$lib/stores/filters.svelte";
import {holdingsScope, holdingsTab, subscribeHoldingsUrlState} from "$lib/stores/holdings.svelte";
import {startSearchSync} from "$lib/url/searchSync";
import {searchToState, stateToSearch} from "./urlCodec";

/**
 * Reset the scope and tab from the current URL once (absent params → fresh
 * defaults), then mirror every change back into the query string (debounced).
 * Call from onMount in the holdings page; the return value stops syncing and
 * works as an onMount cleanup.
 *
 * The reset is UNCONDITIONAL, unlike the journal filters: absent params mean
 * today/empty/include/Stocks (plans/10), never a scope remembered from earlier
 * in the same session.
 */
export function startHoldingsUrlSync(): () => void {
    if (!browser) return () => undefined;

    const today = localToday();
    const restored = searchToState(window.location.search, today);
    holdingsScope.replace(restored.scope);
    holdingsTab.value = restored.tab;

    return startSearchSync(subscribeHoldingsUrlState, (state) => stateToSearch(state, today));
}
