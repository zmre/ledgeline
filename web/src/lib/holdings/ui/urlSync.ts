// Holdings scope ⇄ URL sync (WP-10), the WP-04 replaceState pattern: the store
// is the source of truth; the URL is a debounced projection. Unlike the
// journal filters, the scope is ALWAYS reset from the URL on mount — absent
// params mean today/empty/include (plans/10), never a scope remembered from a
// previous visit in the same session.
import {browser} from "$app/environment";
import {localToday} from "$lib/stores/filters.svelte";
import {holdingsScope, subscribeHoldingsScope} from "$lib/stores/holdings.svelte";
import {startSearchSync} from "$lib/url/searchSync";
import {scopeToSearch, searchToScope} from "./urlCodec";

/**
 * Reset the scope from the current URL once (absent params → fresh defaults),
 * then mirror every scope change back into the query string (debounced). Call
 * from onMount in the holdings page; the return value stops syncing and works
 * as an onMount cleanup.
 *
 * The reset is UNCONDITIONAL, unlike the journal filters: absent params mean
 * today/empty/include (plans/10), never a scope remembered from earlier in the
 * same session.
 */
export function startHoldingsUrlSync(): () => void {
    if (!browser) return () => undefined;

    const today = localToday();
    holdingsScope.replace(searchToScope(window.location.search, today));

    return startSearchSync(subscribeHoldingsScope, (scope) => scopeToSearch(scope, today));
}
