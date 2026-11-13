// Pure filter ⇄ URL-query codec (WP-04). No Svelte/DOM/$app imports so the
// round-trip is unit-testable under node; browser glue lives in urlSync.ts.
//
// Scheme: `?preset=ytd` OR `?from=&to=`, plus `&acct=a,b&q=` (plans/04).
// Preset-produced ranges store the PRESET NAME so they stay live — a restored
// "ytd" recomputes against the current day instead of pinning the dates from
// whenever it was clicked. Hand-picked ranges are written as an explicit date
// PAIR whenever either end differs from the default range, so a shared URL is
// not re-interpreted against a different month later; an explicitly empty
// value (`from=`) means an open end (null). Account names are individually
// percent-encoded before the comma join so names containing commas survive.
//
// A pinned running balance (plan 36) is `&bal=<account>`, plus `&balc=<c>` only
// when a commodity was chosen: an absent `balc` means "the engine's default",
// so a shared link stays short and keeps meaning "the usual one".
//
// The calendar view (plan 40) is `&cal=1`. Its month is the ordinary from/to
// pair; the store, not the codec, holds a restored range to one month.
import type {ISODate} from "#lib/domain/types.js";
import {localToday, presetRange, type DatePreset, type JournalFilter, type RunningPin} from "#lib/stores/filters.svelte.js";
import {safeDecode} from "#lib/url/safeDecode.js";

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

const PRESETS: ReadonlySet<string> = new Set(["thisMonth", "lastMonth", "last90", "ytd", "thisYear", "lastYear", "all"] satisfies DatePreset[]);

/** Serialize to a query string ("" when everything matches `dflt`). No leading "?". */
export function filterToSearch(f: JournalFilter, dflt: JournalFilter): string {
    const params = new URLSearchParams();
    const preset = f.preset ?? null;
    if (preset !== null) {
        if (preset !== (dflt.preset ?? null)) params.set("preset", preset);
    } else if (f.from !== dflt.from || f.to !== dflt.to) {
        params.set("from", f.from ?? "");
        params.set("to", f.to ?? "");
    }
    if (f.accounts.size > 0) {
        params.set("acct", [...f.accounts].sort().map(encodeURIComponent).join(","));
    }
    if (f.query !== "") {
        params.set("q", f.query);
    }
    const pin = f.pin ?? null;
    if (pin !== null) {
        params.set("bal", pin.account);
        if (pin.commodity !== null) params.set("balc", pin.commodity);
    }
    if (f.calendar === true) params.set("cal", "1");
    return params.toString();
}

function parseDate(v: string | null, fallback: ISODate | null): ISODate | null {
    if (v === null) return fallback; // param absent → default
    if (v === "") return null; // explicitly empty → open end
    return ISO_DATE.test(v) ? v : fallback; // malformed → ignore
}

/** Parse a query string (with or without leading "?"); absent params fall back to `dflt`. `today` is injectable for tests. */
export function searchToFilter(search: string, dflt: JournalFilter, today: ISODate = localToday()): JournalFilter {
    const params = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const acct = params.get("acct");
    const accounts =
        acct === null || acct === ""
            ? new Set(dflt.accounts)
            : new Set(
                  acct
                      .split(",")
                      .filter((s) => s !== "")
                      .map(safeDecode) // never throws: `?acct=%` must not break the mount (SEC-12)
              );
    const query = params.get("q") ?? dflt.query;
    const pin = parsePin(params, dflt.pin ?? null);
    // Present only when on, so a table filter reads exactly as it did before the calendar existed.
    const calendar = (params.get("cal") === null ? dflt.calendar === true : params.get("cal") === "1") ? {calendar: true} : {};
    const rawPreset = params.get("preset");
    if (rawPreset !== null && PRESETS.has(rawPreset)) {
        const preset = rawPreset as DatePreset;
        const {from, to} = presetRange(preset, today); // live: recomputed against the CURRENT day
        return {from, to, accounts, query, preset, pin, ...calendar};
    }
    const hasDates = params.get("from") !== null || params.get("to") !== null;
    return {
        from: parseDate(params.get("from"), dflt.from),
        to: parseDate(params.get("to"), dflt.to),
        accounts,
        query,
        preset: hasDates ? null : (dflt.preset ?? null),
        pin,
        ...calendar,
    };
}

/** `bal`/`balc` → a pin. A blank `bal` is no pin; `balc` without `bal` means nothing on its own. */
function parsePin(params: URLSearchParams, fallback: RunningPin | null): RunningPin | null {
    const account = params.get("bal");
    if (account === null) return fallback;
    if (account === "") return null;
    const commodity = params.get("balc");
    return {account, commodity: commodity === null || commodity === "" ? null : commodity};
}
