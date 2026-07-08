// Pure filter ⇄ URL-query codec (WP-04). No Svelte/DOM/$app imports so the
// round-trip is unit-testable under node; browser glue lives in urlSync.ts.
//
// Scheme: `?from=&to=&acct=a,b&q=` (plans/04). Dates are written as a PAIR
// whenever either end differs from the default range, so a shared URL is not
// re-interpreted against a different month later; an explicitly empty value
// (`from=`) means an open end (null). Account names are individually
// percent-encoded before the comma join so names containing commas survive.
import type {ISODate} from "$lib/domain/types";
import type {JournalFilter} from "$lib/stores/filters.svelte";

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Serialize to a query string ("" when everything matches `dflt`). No leading "?". */
export function filterToSearch(f: JournalFilter, dflt: JournalFilter): string {
    const params = new URLSearchParams();
    if (f.from !== dflt.from || f.to !== dflt.to) {
        params.set("from", f.from ?? "");
        params.set("to", f.to ?? "");
    }
    if (f.accounts.size > 0) {
        params.set("acct", [...f.accounts].sort().map(encodeURIComponent).join(","));
    }
    if (f.query !== "") {
        params.set("q", f.query);
    }
    return params.toString();
}

function parseDate(v: string | null, fallback: ISODate | null): ISODate | null {
    if (v === null) return fallback; // param absent → default
    if (v === "") return null; // explicitly empty → open end
    return ISO_DATE.test(v) ? v : fallback; // malformed → ignore
}

/** Parse a query string (with or without leading "?"); absent params fall back to `dflt`. */
export function searchToFilter(search: string, dflt: JournalFilter): JournalFilter {
    const params = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const acct = params.get("acct");
    const accounts =
        acct === null || acct === ""
            ? new Set(dflt.accounts)
            : new Set(
                  acct
                      .split(",")
                      .filter((s) => s !== "")
                      .map(decodeURIComponent)
              );
    return {
        from: parseDate(params.get("from"), dflt.from),
        to: parseDate(params.get("to"), dflt.to),
        accounts,
        query: params.get("q") ?? dflt.query,
    };
}
