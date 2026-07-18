// Pure holdings-scope ⇄ URL-query codec (WP-10). No Svelte/DOM/$app imports so
// the round-trip is unit-testable under node; browser glue lives in urlSync.ts.
//
// Scheme: `?asof=&acct=&mode=` (plans/10). Absent params ALWAYS mean the
// fresh-visit defaults — asOf today, no accounts, include mode — never a
// remembered date, so `today` is threaded through both directions (injectable
// for tests). Account names are individually percent-encoded before the comma
// join so names containing commas survive (same as filters/urlCodec).
import type {ISODate} from "$lib/domain/types";
import type {GainPeriod, HoldingsScope} from "$lib/holdings/types";

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Serialize to a query string ("" when everything is the default for `today`). No leading "?". */
export function scopeToSearch(scope: HoldingsScope, today: ISODate): string {
    const params = new URLSearchParams();
    if (scope.asOf !== today) params.set("asof", scope.asOf);
    if (scope.accounts.size > 0) params.set("acct", [...scope.accounts].sort().map(encodeURIComponent).join(","));
    if (scope.mode !== "include") params.set("mode", scope.mode);
    if (scope.gainPeriod !== "all") params.set("gain", scope.gainPeriod);
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
                      .map(decodeURIComponent)
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
