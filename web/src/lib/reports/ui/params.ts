// Report tab/controls ⇄ URL-query codec (WP-07). Pure module (no Svelte/DOM
// imports) so the round-trip is unit-testable under node; the reports page owns
// the replaceState glue (same pattern as filters/urlSync.ts).
//
// Scheme: `?tab=bs|is|cf|nw` plus the ACTIVE tab's controls, written in full
// (`asof`/`from`+`to`/`end`, `interval`, `count`, `depth`) so a shared or
// reloaded URL reproduces the exact report even when the defaults (today-based)
// have moved on. Params for inactive tabs are never written.

import type {ISODate} from "$lib/domain/types";
import {bucketEnd, bucketStart, lastNBuckets, today} from "$lib/reports/periods";

export type ReportTab = "insights" | "bs" | "is" | "cf" | "nw" | "subs";
export type ReportInterval = "monthly" | "quarterly" | "yearly";

// Insights is the first (default) tab — the dashboard shown when Reports opens.
export const TAB_ORDER: ReportTab[] = ["insights", "bs", "is", "cf", "nw", "subs"];

export const TAB_LABELS: Record<ReportTab, string> = {
    insights: "Insights",
    bs: "Balance Sheet",
    is: "P&L",
    cf: "Cash Flow",
    nw: "Net Worth",
    subs: "Subscriptions",
};

/** One flat parameter set shared by all tabs, so switching tabs keeps settings. */
export interface ReportParams {
    tab: ReportTab;
    /** Balance sheet: point-in-time date (INCLUSIVE, engine semantics). */
    asOf: ISODate;
    /** Income statement range (both INCLUSIVE). */
    from: ISODate;
    to: ISODate;
    /** Cash flow / net worth: date whose bucket is the last column (INCLUSIVE). */
    end: ISODate;
    interval: ReportInterval;
    /** Lookback bucket count. */
    count: number;
    /** Account depth clamp. */
    depth: number;
    /** Insights: inclusive comparison-span start (split at its midpoint by the engine). */
    insStart: ISODate;
    /** Insights: inclusive comparison-span end. */
    insEnd: ISODate;
}

/** Which controls each tab shows (drives ReportControls) and which params it serializes. */
export interface ControlsConfig {
    asOf: boolean;
    range: boolean;
    end: boolean;
    interval: boolean;
    count: boolean;
    depth: boolean;
}

export const TAB_CONTROLS: Record<ReportTab, ControlsConfig> = {
    // Insights owns its own period control (PeriodControl), so the shared
    // ReportControls bar shows nothing for it.
    insights: {asOf: false, range: false, end: false, interval: false, count: false, depth: false},
    // No depth on the balance sheet: groups ARE the reading and the accounts
    // inside one are a drill-down, so a clamp had almost nothing left to move —
    // and what it did move it moved wrongly, hiding accounts with no control on
    // screen to ask for them back. Expanding a group now shows all of it (the
    // engine is asked for an unclamped report), single-child chains still
    // compressed by `compressSectionRows`.
    bs: {asOf: true, range: false, end: false, interval: false, count: false, depth: false},
    // The income statement dropped its depth slider for the same reason (plans/13):
    // it is grouped now, and a clamp that hid accounts with no control on screen
    // to ask for them back was the worst of both. The range stays — a P&L is a
    // report ABOUT a period, and that is the one thing it cannot infer.
    is: {asOf: false, range: true, end: false, interval: false, count: false, depth: false},
    cf: {asOf: false, range: false, end: true, interval: true, count: true, depth: true},
    nw: {asOf: false, range: false, end: true, interval: true, count: true, depth: true},
    // Subscriptions scan a fixed trailing window, so they expose no controls.
    subs: {asOf: false, range: false, end: false, interval: false, count: false, depth: false},
};

/** Per-tab default interval/count, applied on tab activation (cash flow and net
 *  worth want different lookbacks: monthly/12 vs yearly/5). Depth is shared. */
export const TAB_DEFAULTS: Record<ReportTab, {interval: ReportInterval; count: number}> = {
    insights: {interval: "monthly", count: 12}, // inert (insights ignores interval/count)
    bs: {interval: "monthly", count: 12},
    is: {interval: "monthly", count: 12},
    cf: {interval: "monthly", count: 12},
    nw: {interval: "yearly", count: 5},
    subs: {interval: "monthly", count: 12}, // inert (subscriptions ignore interval/count)
};

export const MAX_COUNT = 120;

// --- Insights comparison-period presets ------------------------------------
// Each preset is a whole-month span of `spanMonths` COMPLETE months ending with
// the last fully-elapsed month; the engine splits it at its midpoint into two
// equal halves (previous vs current). "Year-over-year" (24 months → two clean
// 12-month halves) is the default.

export type InsightsPreset = "yoy" | "hoh" | "qoq" | "mom";

export const INSIGHTS_PRESETS: {id: InsightsPreset; label: string; spanMonths: number}[] = [
    {id: "yoy", label: "Year over year", spanMonths: 24},
    {id: "hoh", label: "Half over half", spanMonths: 12},
    {id: "qoq", label: "Quarter over quarter", spanMonths: 6},
    {id: "mom", label: "Month over month", spanMonths: 2},
];

/** The default insights preset: trailing two years, split into two 12-month halves. */
export const DEFAULT_INSIGHTS_PRESET: InsightsPreset = "yoy";

/** Resolve a preset to an inclusive `[start, end]` span relative to `now`. */
export function insightsPresetRange(preset: InsightsPreset, now: ISODate = today()): {start: ISODate; end: ISODate} {
    const spanMonths = INSIGHTS_PRESETS.find((p) => p.id === preset)?.spanMonths ?? 24;
    // `spanMonths + 1` monthly buckets: the newest is the current (partial)
    // month, so the span runs from the oldest bucket to the last COMPLETE month.
    const monthly = lastNBuckets(now, "monthly", spanMonths + 1);
    return {start: bucketStart(monthly[0]), end: bucketEnd(monthly[monthly.length - 2])};
}

/** Which insights preset (if any) the current span matches; "custom" otherwise. */
export function activeInsightsPreset(start: ISODate, end: ISODate, now: ISODate = today()): InsightsPreset | "custom" {
    for (const {id} of INSIGHTS_PRESETS) {
        const range = insightsPresetRange(id, now);
        if (range.start === start && range.end === end) return id;
    }
    return "custom";
}

/** Defaults: Insights (year-over-year) is the landing tab; bs as-of today, P&L this year, cf/nw last 12 months. */
export function defaultReportParams(now: ISODate = today()): ReportParams {
    const year = now.slice(0, 4);
    const ins = insightsPresetRange(DEFAULT_INSIGHTS_PRESET, now);
    return {
        tab: "insights",
        asOf: now,
        from: `${year}-01-01`,
        to: `${year}-12-31`,
        end: now,
        interval: "monthly",
        count: 12,
        // Shared by cf/nw/budget, and by nothing else: neither statement reads
        // `depth` any more (see TAB_CONTROLS.bs and .is — both are grouped and
        // both ask the engine for an unclamped report). It was raised from 2 to
        // 3 for the balance sheet's sake and is left there deliberately —
        // moving it back would change what the period reports show, which is a
        // separate decision from removing a control from two tabs.
        depth: 3,
        insStart: ins.start,
        insEnd: ins.end,
    };
}

/** Serialize the ACTIVE tab's params to a query string (no leading "?"). */
export function paramsToSearch(p: ReportParams): string {
    const q = new URLSearchParams();
    q.set("tab", p.tab);
    if (p.tab === "insights") {
        q.set("istart", p.insStart);
        q.set("iend", p.insEnd);
        return q.toString();
    }
    const c = TAB_CONTROLS[p.tab];
    if (c.asOf) q.set("asof", p.asOf);
    if (c.range) {
        q.set("from", p.from);
        q.set("to", p.to);
    }
    if (c.end) q.set("end", p.end);
    if (c.interval) q.set("interval", p.interval);
    if (c.count) q.set("count", String(p.count));
    if (c.depth) q.set("depth", String(p.depth));
    return q.toString();
}

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;
const isTab = (v: string): v is ReportTab => (TAB_ORDER as string[]).includes(v);
const isInterval = (v: string): v is ReportInterval => v === "monthly" || v === "quarterly" || v === "yearly";

function parseDate(v: string | null, fallback: ISODate): ISODate {
    return v !== null && ISO_DATE.test(v) ? v : fallback;
}

function parseInt1(v: string | null, fallback: number, max: number): number {
    if (v === null || !/^\d+$/.test(v)) return fallback;
    const n = Number(v);
    return n < 1 ? 1 : n > max ? max : n;
}

/** Parse a query string (with or without leading "?"); absent/malformed params fall back to `dflt`. */
export function searchToParams(search: string, dflt: ReportParams): ReportParams {
    const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const tab = q.get("tab");
    const interval = q.get("interval");
    return {
        tab: tab !== null && isTab(tab) ? tab : dflt.tab,
        asOf: parseDate(q.get("asof"), dflt.asOf),
        from: parseDate(q.get("from"), dflt.from),
        to: parseDate(q.get("to"), dflt.to),
        end: parseDate(q.get("end"), dflt.end),
        interval: interval !== null && isInterval(interval) ? interval : dflt.interval,
        count: parseInt1(q.get("count"), dflt.count, MAX_COUNT),
        depth: parseInt1(q.get("depth"), dflt.depth, 99),
        insStart: parseDate(q.get("istart"), dflt.insStart),
        insEnd: parseDate(q.get("iend"), dflt.insEnd),
    };
}

// --- The Budget tab's old address -------------------------------------------

/**
 * The `/budget` query a `?tab=budget` reports URL should be forwarded to, or
 * `null` when this URL is not one.
 *
 * Budget was a report tab for several releases and is its own route now. A
 * bookmark or a shared link still naming it must not quietly land on Insights,
 * which is where an unknown `tab` value otherwise falls back to.
 *
 * Here rather than in the page for two reasons: this is testable arithmetic with
 * no router around it (the same split `filters/journalTarget.ts` makes), and
 * `URLSearchParams` is a mutable built-in the SPA's lint rules keep out of
 * `.svelte` files.
 */
export function budgetRedirect(search: string): string | null {
    const query = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    if (query.get("tab") !== "budget") return null;
    // Everything except the tab itself: `from`, `to` and `depth` are exactly the
    // params the Budget route still honours, so the forwarded link shows the same
    // budget the old one did.
    query.delete("tab");
    const carried = query.toString();
    return carried === "" ? "" : `?${carried}`;
}
