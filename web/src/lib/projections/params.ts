// The Projections tab's params ⇄ URL-query codec.
//
// Modelled on `$lib/budget/params.ts`, which is the worked example of a
// standalone feature route owning its own codec. Pure module (no Svelte/DOM
// imports) so the round trip is unit-testable under node.
//
// Scheme: `?tab=net|cash|worth&interval=&count=&depth=`. All four are written in
// full, never only the non-defaults, so a shared or reloaded URL reproduces the
// same three charts — the reports codec's rule, for the reason it gives there.
//
// The SCENARIO is deliberately not in the URL. It is a table of money with
// account names in it, a query string is the one part of a page that gets pasted
// into chat windows and server logs, and Phase 3's file save is the durable home
// the ask actually asked for.

import {MAX_COUNT, type ReportInterval} from "$lib/reports/ui/params";
import {clampedInt, oneOf} from "$lib/url/params";

/** Which of the three report tabs the bottom half is showing. */
export type ProjectionTab = "net" | "cash" | "worth";

export const PROJECTION_TABS: readonly ProjectionTab[] = ["net", "cash", "worth"];

export const PROJECTION_TAB_LABELS: Record<ProjectionTab, string> = {
    net: "Net income",
    cash: "Cash & runway",
    worth: "Net worth",
};

/** The Projections tab's URL state. */
export interface ProjectionParams {
    tab: ProjectionTab;
    interval: ReportInterval;
    /** How many buckets forward to project. */
    count: number;
    /** Account depth clamp for the net-income rows. Never clamps the totals. */
    depth: number;
}

/**
 * The horizon the tab opens on: **monthly × 24**.
 *
 * Two years is the span a runway question is usually asked over — long enough
 * for a hiring ramp or a rent review to show up, short enough that a monthly
 * bucket is still legible — and it is well inside `MAX_COUNT`.
 */
export const DEFAULT_PROJECTION_INTERVAL: ReportInterval = "monthly";
export const DEFAULT_PROJECTION_COUNT = 24;

/**
 * Depth 2, not the reports' 3.
 *
 * The seed writes its unbudgeted categories at depth 2 (`expenses:housing`), so
 * opening any deeper shows a tree whose leaves are exactly its depth-2 rows
 * restated — one row per row, indented. Depth 2 is the reading that matches the
 * table above it.
 */
export const DEFAULT_PROJECTION_DEPTH = 2;

/** Defaults: net income, monthly × 24, depth 2. */
export function defaultProjectionParams(): ProjectionParams {
    return {
        tab: "net",
        interval: DEFAULT_PROJECTION_INTERVAL,
        count: DEFAULT_PROJECTION_COUNT,
        depth: DEFAULT_PROJECTION_DEPTH,
    };
}

/** Serialize to a query string (no leading "?"). */
export function projectionParamsToSearch(params: ProjectionParams): string {
    const q = new URLSearchParams();
    q.set("tab", params.tab);
    q.set("interval", params.interval);
    q.set("count", String(params.count));
    q.set("depth", String(params.depth));
    return q.toString();
}

const INTERVALS: readonly ReportInterval[] = ["monthly", "quarterly", "yearly"];

/**
 * Parse a query string (with or without a leading "?"); absent or malformed
 * params fall back to `dflt`.
 *
 * Every field is validated against its own vocabulary or range rather than
 * merely typechecked: a stale link naming a tab that no longer exists would
 * otherwise render an empty box with no control on screen to get back out of it.
 */
export function searchToProjectionParams(search: string, dflt: ProjectionParams): ProjectionParams {
    const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    return {
        tab: oneOf(PROJECTION_TABS, q.get("tab"), dflt.tab),
        interval: oneOf(INTERVALS, q.get("interval"), dflt.interval),
        count: clampedInt(q.get("count"), 1, MAX_COUNT, dflt.count),
        depth: clampedInt(q.get("depth"), 1, 99, dflt.depth),
    };
}
