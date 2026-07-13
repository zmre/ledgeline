// Holdings scope ⇄ URL sync (WP-10), the WP-04 replaceState pattern: the store
// is the source of truth; the URL is a debounced projection. Unlike the
// journal filters, the scope is ALWAYS reset from the URL on mount — absent
// params mean today/empty/include (plans/10), never a scope remembered from a
// previous visit in the same session.
import {browser} from "$app/environment";
import {replaceState} from "$app/navigation";
import {localToday} from "$lib/stores/filters.svelte";
import {holdingsScope, subscribeHoldingsScope} from "$lib/stores/holdings.svelte";
import {scopeToSearch, searchToScope} from "./urlCodec";

const DEBOUNCE_MS = 250;

/**
 * Reset the scope from the current URL once (absent params → fresh defaults),
 * then mirror every scope change back into the query string (debounced). Call
 * from onMount in the holdings page; the return value stops syncing and works
 * as an onMount cleanup.
 */
export function startHoldingsUrlSync(): () => void {
    if (!browser) return () => undefined;

    const today = localToday();
    holdingsScope.replace(searchToScope(window.location.search, today));

    let timer: ReturnType<typeof setTimeout> | null = null;
    const unsubscribe = subscribeHoldingsScope((scope) => {
        const search = scopeToSearch(scope, today);
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
