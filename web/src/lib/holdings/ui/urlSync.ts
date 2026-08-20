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
import {afterNavigate} from "$app/navigation";
import {localToday} from "$lib/stores/filters.svelte";
import {holdingsScope, holdingsTab, subscribeHoldingsUrlState} from "$lib/stores/holdings.svelte";
import {startSearchSync} from "$lib/url/searchSync";
import {searchToState, stateToSearch} from "./urlCodec";

/**
 * Reset the scope and tab from the current URL (absent params → fresh
 * defaults), then mirror every change back into the query string (debounced).
 * Call from onMount in the holdings page; the return value stops syncing and
 * works as an onMount cleanup.
 *
 * The reset is UNCONDITIONAL, unlike the journal filters: absent params mean
 * today/empty/include/Stocks (plans/10), never a scope remembered from earlier
 * in the same session.
 *
 * It runs now AND after every real navigation, because onMount alone was not
 * enough: SvelteKit reuses this page component when only the query string
 * changes — the app-bar Holdings link clicked while already on /holdings,
 * back/forward between two /holdings entries — so the store kept the OLD
 * tab/scope while the address bar showed the new one, and the next debounced
 * mirror write then replaceState-overwrote the URL the user had navigated to.
 *
 * The one-writer design survives this: `afterNavigate` never fires for the
 * mirror's own writes, because SvelteKit's shallow `replaceState` skips the
 * navigation callbacks by design — so a mirror write cannot trigger a restore
 * that would clobber an in-flight debounced edit. And the restore itself is
 * idempotent (compared through the codec below), so the extra mount-time
 * `afterNavigate` call replaces nothing and wakes no debounce.
 */
export function startHoldingsUrlSync(): () => void {
    if (!browser) return () => undefined;

    const today = localToday();

    const restore = (): void => {
        const restored = searchToState(window.location.search, today);
        // Idempotence via the codec, not field-by-field: when the store already
        // serializes to what the URL parses to, skip the writes entirely — no
        // replaced scope object, no re-fired subscription, no mirror debounce.
        if (stateToSearch(restored, today) === stateToSearch({scope: holdingsScope.value, tab: holdingsTab.value}, today)) return;
        holdingsScope.replace(restored.scope);
        holdingsTab.value = restored.tab;
    };

    restore();
    // Legal here despite running inside onMount: Svelte 5 restores the component
    // context while an effect runs, so afterNavigate's own onMount registration
    // works, and it unregisters itself when this page is destroyed.
    afterNavigate(restore);

    return startSearchSync(subscribeHoldingsUrlState, (state) => stateToSearch(state, today));
}
