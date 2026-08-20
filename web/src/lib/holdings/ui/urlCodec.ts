// Pure holdings-scope ⇄ URL-query codec (WP-10, extended by plans/14). No
// Svelte/DOM/$app imports so the round-trip is unit-testable under node; browser
// glue lives in urlSync.ts.
//
// Scheme: `?asof=&acct=&mode=&gain=&tab=` (plans/10, plans/14). Absent params
// ALWAYS mean the fresh-visit defaults — asOf today, no accounts, include mode,
// all-time gain, Stocks tab — never a remembered date, so `today` is threaded
// through both directions (injectable for tests). Account names are individually
// percent-encoded before the comma join so names containing commas survive (same
// as filters/urlCodec).
//
// `tab` is here rather than in a params module of its own because this screen
// must have exactly ONE writer to its query string. The scope is mirrored by a
// store subscription (urlSync.ts); a second `searchMirror` for the tab would
// race it, and whichever debounce fired last would erase the other's keys. So
// the tab travels with the scope through `stateToSearch`, while staying OUT of
// `HoldingsScope` itself — scope is the report resource's refetch key, and a tab
// in it would refetch the stock report on every tab click.
import type {ISODate} from "$lib/domain/types";
import {isTab, TAB_ORDER, type HoldingsTab} from "$lib/holdings/params";
import type {GainPeriod, HoldingsScope} from "$lib/holdings/types";
import {safeDecode} from "$lib/url/safeDecode";

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Derived, not restated, so the default tab and the strip's first tab cannot drift apart. */
const DEFAULT_TAB: HoldingsTab = TAB_ORDER[0];

/** Everything the holdings screen restores from — and mirrors into — the query string. */
export interface HoldingsUrlState {
    scope: HoldingsScope;
    tab: HoldingsTab;
}

/** The scope half, as params, so the tab can join it without a second codec re-encoding anything. */
function scopeParams(scope: HoldingsScope, today: ISODate): URLSearchParams {
    const params = new URLSearchParams();
    if (scope.asOf !== today) params.set("asof", scope.asOf);
    if (scope.accounts.size > 0) params.set("acct", [...scope.accounts].sort().map(encodeURIComponent).join(","));
    if (scope.mode !== "include") params.set("mode", scope.mode);
    if (scope.gainPeriod !== "all") params.set("gain", scope.gainPeriod);
    return params;
}

/** Serialize to a query string ("" when everything is the default for `today`). No leading "?". */
export function scopeToSearch(scope: HoldingsScope, today: ISODate): string {
    return scopeParams(scope, today).toString();
}

/**
 * Serialize scope AND tab — the one function the URL sync writes through.
 *
 * `tab` is omitted on the default, exactly like every other param here: a link
 * to the screen everyone opens on should not carry a key naming it, and a
 * fresh-visit URL stays bare.
 */
export function stateToSearch(state: HoldingsUrlState, today: ISODate): string {
    const params = scopeParams(state.scope, today);
    if (state.tab !== DEFAULT_TAB) params.set("tab", state.tab);
    return params.toString();
}

/** Parse a query string (with or without leading "?"); absent/malformed params fall back to today/empty/include. */
export function searchToScope(search: string, today: ISODate): HoldingsScope {
    const params = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const asof = params.get("asof");
    const acct = params.get("acct");
    const accounts =
        acct === null || acct === ""
            ? new Set<string>()
            : new Set(
                  acct
                      .split(",")
                      .filter((s) => s !== "")
                      .map(safeDecode) // never throws: `?acct=%` must not break the mount (SEC-12)
              );
    const gain = params.get("gain");
    const gainPeriod: GainPeriod = gain === "ytd" || gain === "12mo" ? gain : "all";
    return {
        asOf: asof !== null && ISO_DATE.test(asof) ? asof : today,
        accounts,
        mode: params.get("mode") === "exclude" ? "exclude" : "include",
        gainPeriod,
    };
}

/**
 * Parse scope AND tab. An absent, empty or unknown `tab` opens Stocks rather
 * than stranding the page on a blank sub-screen — the same refusal `isTab`
 * exists for, and the reason a stale link from another surface cannot break this
 * one.
 */
export function searchToState(search: string, today: ISODate): HoldingsUrlState {
    const params = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const tab = params.get("tab");
    return {scope: searchToScope(search, today), tab: tab !== null && isTab(tab) ? tab : DEFAULT_TAB};
}
