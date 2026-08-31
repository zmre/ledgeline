// The Budget tab's params ⇄ URL-query codec, and the period presets the bars
// span.
//
// These lived in `$lib/reports/ui/params.ts` while Budget was a report tab. It
// is its own top-level tab now, with its own route and its own URL, so its
// params travel with it — leaving the reports codec to describe only the tabs
// that are still there.
//
// Pure module (no Svelte/DOM imports) so the round trip is unit-testable under
// node, exactly as the reports codec is.

import type {ISODate} from "$lib/domain/types";
import {bucketEnd, bucketStart, lastNBuckets, monthsBetween, today} from "$lib/reports/periods";

// --- Period presets ---------------------------------------------------------
// The budget summary is period-based; these presets set the from/to range that
// the store turns into monthly buckets. "Custom" = any range not matching one.

export type BudgetPreset = "this-month" | "last-month" | "ytd" | "this-year" | "trailing-12";

export const BUDGET_PRESETS: {id: BudgetPreset; label: string}[] = [
    {id: "this-month", label: "This month"},
    {id: "last-month", label: "Last month"},
    {id: "ytd", label: "Year to date"},
    {id: "this-year", label: "This year"},
    {id: "trailing-12", label: "Trailing 12 mo"},
];

/** The default budget range: year-to-date (Jan 1 → today). */
export const DEFAULT_BUDGET_PRESET: BudgetPreset = "ytd";

/** Resolve a preset to an inclusive from/to range, relative to `now`. */
export function budgetPresetRange(preset: BudgetPreset, now: ISODate = today()): {from: ISODate; to: ISODate} {
    const year = now.slice(0, 4);
    switch (preset) {
        case "this-month":
            return {from: bucketStart(now.slice(0, 7)), to: now};
        case "last-month": {
            const prev = lastNBuckets(now, "monthly", 2)[0];
            return {from: bucketStart(prev), to: bucketEnd(prev)};
        }
        case "ytd":
            return {from: `${year}-01-01`, to: now};
        case "this-year":
            return {from: `${year}-01-01`, to: `${year}-12-31`};
        case "trailing-12":
            return {from: bucketStart(lastNBuckets(now, "monthly", 12)[0]), to: now};
    }
}

/** Which preset (if any) the current from/to range matches; "custom" otherwise. */
export function activeBudgetPreset(from: ISODate, to: ISODate, now: ISODate = today()): BudgetPreset | "custom" {
    for (const {id} of BUDGET_PRESETS) {
        const range = budgetPresetRange(id, now);
        if (range.from === from && range.to === to) return id;
    }
    return "custom";
}

/**
 * The span the budget bars ACTUALLY cover, given the controls' `from`/`to`.
 *
 * The engine takes `{end, count}` and walks whole months backwards, so the first
 * bucket always starts on the 1st — `from`'s day-of-month is discarded. It does
 * truncate the last bucket at `end`, so only the START drifts. With
 * `from = 2026-01-15` the bar therefore includes 2026-01-01…01-14, which a
 * drill-down filtered to the raw controls excludes: measured $720.00 in the bar
 * against $20.00 in the journal it links to.
 *
 * The bars are the thing that cannot move — a monthly goal is a whole-month
 * figure, so comparing it to a partial month is what the envelope view is for.
 * So the drill-down is widened to this span instead, and `BudgetSummary` shows
 * it, rather than silently filtering to a narrower set than it charted.
 */
export function budgetSpan(from: ISODate, to: ISODate): {from: ISODate; to: ISODate; count: number} {
    return {from: `${from.slice(0, 7)}-01`, to, count: monthsBetween(from, to)};
}

// --- The tab's own params ---------------------------------------------------

/** The Budget tab's URL state: the range the bars cover, and the depth they clamp to. */
export interface BudgetParams {
    /** Inclusive range start. */
    from: ISODate;
    /** Inclusive range end. */
    to: ISODate;
    /** Account depth clamp. */
    depth: number;
}

/** The depth the tab opens at — the same default the period reports use. */
export const DEFAULT_BUDGET_DEPTH = 3;

/** Defaults: year-to-date at depth 3. */
export function defaultBudgetParams(now: ISODate = today()): BudgetParams {
    const range = budgetPresetRange(DEFAULT_BUDGET_PRESET, now);
    return {from: range.from, to: range.to, depth: DEFAULT_BUDGET_DEPTH};
}

/** Serialize to a query string (no leading "?"). */
export function budgetParamsToSearch(params: BudgetParams): string {
    const q = new URLSearchParams();
    q.set("from", params.from);
    q.set("to", params.to);
    q.set("depth", String(params.depth));
    return q.toString();
}

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Parse a query string (with or without a leading "?"); absent/malformed params fall back to `dflt`. */
export function searchToBudgetParams(search: string, dflt: BudgetParams): BudgetParams {
    const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const from = q.get("from");
    const to = q.get("to");
    const depth = q.get("depth");
    return {
        from: from !== null && ISO_DATE.test(from) ? from : dflt.from,
        to: to !== null && ISO_DATE.test(to) ? to : dflt.to,
        depth: depth !== null && /^\d+$/.test(depth) ? Math.min(Math.max(Number(depth), 1), 99) : dflt.depth,
    };
}
