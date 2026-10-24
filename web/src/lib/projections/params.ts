// The Projections tab's params ⇄ URL-query codec.
//
// Modelled on `#lib/budget/params.ts`, which is the worked example of a
// standalone feature route owning its own codec. Pure module (no Svelte/DOM
// imports) so the round trip is unit-testable under node.
//
// Scheme: `?tab=cash|bs|is|cf&interval=&count=&lookback=&depth=`. All five are
// written in full, never only the non-defaults, so a shared or reloaded URL
// reproduces the same readings — the reports codec's rule, for the reason it
// gives there.
//
// `cash` is the Runway tab's value, unchanged since it was "Cash & runway", so
// old links keep working. The statements take the Reports tab's own codes.
// `net` and `worth` (the Net income and Net worth readings plan 28 replaced)
// are no longer tabs: an old link naming one opens the default, Runway.
//
// The SCENARIO is deliberately not in the URL. It is a table of money with
// account names in it, a query string is the one part of a page that gets pasted
// into chat windows and server logs, and Phase 3's file save is the durable home
// the ask actually asked for.

import {MAX_COUNT, type PeriodInterval} from "#lib/reports/ui/params.js";
import {clampedInt, oneOf} from "#lib/url/params.js";

/** Which reading the bottom half is showing. */
export type ProjectionTab = "cash" | "bs" | "is" | "cf";

/**
 * In strip order, which is also the digit order (1–4).
 *
 * Runway first: it is the one reading only this tab has, and the question the
 * tab is most often opened to answer. The three statements follow in the
 * Reports tab's own order.
 */
export const PROJECTION_TABS: readonly ProjectionTab[] = ["cash", "bs", "is", "cf"];

export const PROJECTION_TAB_LABELS: Record<ProjectionTab, string> = {
    cash: "Runway",
    bs: "Balance sheet",
    is: "Income statement",
    cf: "Cash flow",
};

/** The Projections tab's URL state. */
export interface ProjectionParams {
    tab: ProjectionTab;
    interval: PeriodInterval;
    /** How many buckets forward to project. */
    count: number;
    /** How many full ACTUAL periods the statements show before the forecast (plan 28 Decision 6). `0` is allowed. */
    lookback: number;
    /** The cash flow statement's account roll-up depth. Never clamps the totals. */
    depth: number;
}

/**
 * The horizon the tab opens on: **monthly × 24**.
 *
 * Two years is the span a runway question is usually asked over — long enough
 * for a hiring ramp or a rent review to show up, short enough that a monthly
 * bucket is still legible — and it is well inside `MAX_COUNT`.
 */
export const DEFAULT_PROJECTION_INTERVAL: PeriodInterval = "monthly";
export const DEFAULT_PROJECTION_COUNT = 24;

/** Two full periods back (plan 28 Decision 6) — enough to see where the forecast starts from. */
export const DEFAULT_PROJECTION_LOOKBACK = 2;

/**
 * Depth 2, not the reports' 3.
 *
 * The seed writes its unbudgeted categories at depth 2 (`expenses:housing`), so
 * opening any deeper shows a tree whose leaves are exactly its depth-2 rows
 * restated — one row per row, indented. Depth 2 is the reading that matches the
 * table above it.
 */
export const DEFAULT_PROJECTION_DEPTH = 2;

/** Defaults: runway, monthly × 24 with two periods of actuals, depth 2. */
export function defaultProjectionParams(): ProjectionParams {
    return {
        tab: "cash",
        interval: DEFAULT_PROJECTION_INTERVAL,
        count: DEFAULT_PROJECTION_COUNT,
        lookback: DEFAULT_PROJECTION_LOOKBACK,
        depth: DEFAULT_PROJECTION_DEPTH,
    };
}

/** Serialize to a query string (no leading "?"). */
export function projectionParamsToSearch(params: ProjectionParams): string {
    const q = new URLSearchParams();
    q.set("tab", params.tab);
    q.set("interval", params.interval);
    q.set("count", String(params.count));
    q.set("lookback", String(params.lookback));
    q.set("depth", String(params.depth));
    return q.toString();
}

const INTERVALS: readonly PeriodInterval[] = ["monthly", "quarterly", "yearly"];

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
        lookback: clampedInt(q.get("lookback"), 0, MAX_COUNT, dflt.lookback),
        depth: clampedInt(q.get("depth"), 1, 99, dflt.depth),
    };
}
