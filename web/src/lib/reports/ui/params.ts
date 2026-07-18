// Report tab/controls ⇄ URL-query codec (WP-07). Pure module (no Svelte/DOM
// imports) so the round-trip is unit-testable under node; the reports page owns
// the replaceState glue (same pattern as filters/urlSync.ts).
//
// Scheme: `?tab=bs|is|cf|nw` plus the ACTIVE tab's controls, written in full
// (`asof`/`from`+`to`/`end`, `interval`, `count`, `depth`) so a shared or
// reloaded URL reproduces the exact report even when the defaults (today-based)
// have moved on. Params for inactive tabs are never written.

import type {ISODate} from "$lib/domain/types";
import {today} from "$lib/reports/periods";

export type ReportTab = "bs" | "is" | "cf" | "nw";
export type ReportInterval = "monthly" | "quarterly" | "yearly";

export const TAB_ORDER: ReportTab[] = ["bs", "is", "cf", "nw"];

export const TAB_LABELS: Record<ReportTab, string> = {
    bs: "Balance Sheet",
    is: "P&L",
    cf: "Cash Flow",
    nw: "Net Worth",
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
    bs: {asOf: true, range: false, end: false, interval: false, count: false, depth: true},
    is: {asOf: false, range: true, end: false, interval: false, count: false, depth: true},
    cf: {asOf: false, range: false, end: true, interval: true, count: true, depth: true},
    nw: {asOf: false, range: false, end: true, interval: true, count: true, depth: true},
};

/** Per-tab default interval/count, applied on tab activation (cash flow and net
 *  worth want different lookbacks: monthly/12 vs yearly/5). Depth is shared. */
export const TAB_DEFAULTS: Record<ReportTab, {interval: ReportInterval; count: number}> = {
    bs: {interval: "monthly", count: 12},
    is: {interval: "monthly", count: 12},
    cf: {interval: "monthly", count: 12},
    nw: {interval: "yearly", count: 5},
};

export const MAX_COUNT = 120;

/** Defaults per plans/07: bs as-of today, P&L this calendar year, cf/nw last 12 months. */
export function defaultReportParams(now: ISODate = today()): ReportParams {
    const year = now.slice(0, 4);
    return {
        tab: "bs",
        asOf: now,
        from: `${year}-01-01`,
        to: `${year}-12-31`,
        end: now,
        interval: "monthly",
        count: 12,
        depth: 2,
    };
}

/** Serialize the ACTIVE tab's params to a query string (no leading "?"). */
export function paramsToSearch(p: ReportParams): string {
    const q = new URLSearchParams();
    q.set("tab", p.tab);
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
    };
}
