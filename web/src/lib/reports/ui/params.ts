// Report tab/controls ⇄ URL-query codec (WP-07). Pure module (no Svelte/DOM
// imports) so the round-trip is unit-testable under node; the reports page owns
// the replaceState glue (same pattern as filters/urlSync.ts).
//
// Scheme: `?tab=bs|is|cf|nw` plus the ACTIVE tab's controls, written in full
// (`asof`/`from`+`to`/`end`, `interval`, `count`, `depth`) so a shared or
// reloaded URL reproduces the exact report even when the defaults (today-based)
// have moved on. Params for inactive tabs are never written.
//
// Two older URL shapes are still read (plans/24): a P&L link with no
// `interval` predates that control and means Total, and a cash-flow link with
// `end`/`count` but no `from` is translated to the from/to range it showed.
// A balance-sheet link with no `interval` is the single-date sheet it always
// was; its multi-period mode adds `interval` and `from` (plans/28 follow-ups).

import type {ISODate} from "#lib/domain/types.js";
import {bucketEnd, bucketStart, lastNBuckets, today} from "#lib/reports/periods.js";
import {clampedInt, isIsoDate} from "#lib/url/params.js";

export type ReportTab = "insights" | "bs" | "is" | "cf" | "nw" | "subs";
/** A bucketing grain: one column per month, quarter or year. */
export type PeriodInterval = "monthly" | "quarterly" | "yearly";

/**
 * A report's column grain. `total` is ONE column over the whole range — the
 * P&L's comparative statement, or cash flow's single column — and is offered
 * only by the ranged reports (see `TAB_CONTROLS.*.intervals`).
 */
export type ReportInterval = "total" | PeriodInterval;

export const INTERVAL_LABELS: Record<ReportInterval, string> = {
    total: "Total",
    monthly: "Monthly",
    quarterly: "Quarterly",
    yearly: "Yearly",
};

/**
 * The Interval option's label on `tab`. The balance sheet's one-column grain
 * is the sheet as of a single date, and nothing is totalled on it, so it says
 * so rather than "Total". Every other tab reads `INTERVAL_LABELS`.
 */
export function intervalLabel(tab: ReportTab, interval: ReportInterval): string {
    return tab === "bs" && interval === "total" ? "Single date" : INTERVAL_LABELS[interval];
}

/** Whether the balance sheet is in its multi-period mode (one column per period end). */
export function bsMultiPeriod(p: Pick<ReportParams, "interval">): boolean {
    return p.interval !== "total";
}

/** Every grain, in the order the Interval select lists them. */
const ALL_INTERVALS: readonly ReportInterval[] = ["total", "monthly", "quarterly", "yearly"];
/**
 * The grains a snapshot report can take. Net worth's columns are balances AT a
 * date, and summing them into a Total means nothing, so it is never offered.
 */
const PERIODIC_INTERVALS: readonly ReportInterval[] = ["monthly", "quarterly", "yearly"];

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
    /** Balance sheet: point-in-time date (INCLUSIVE, engine semantics); the last column's end in multi-period mode. */
    asOf: ISODate;
    /**
     * Balance sheet, multi-period mode only: the first period's start (INCLUSIVE).
     * Its own field, like `cfFrom`, so the sheet's default (the trailing twelve
     * months) and its last-used start survive a tab switch. Written to the URL as
     * `from`, and only while the Interval is not Total.
     */
    bsFrom: ISODate;
    /** Income statement range (both INCLUSIVE). */
    from: ISODate;
    to: ISODate;
    /**
     * Cash flow range (both INCLUSIVE). Its own pair rather than the P&L's, so
     * each tab keeps its own default (a calendar year vs the trailing twelve
     * months) and its own last-used range across a tab switch. Written to the
     * URL as `from`/`to`, like the P&L's — only the active tab's are written.
     */
    cfFrom: ISODate;
    cfTo: ISODate;
    /** Net worth: date whose bucket is the last column (INCLUSIVE). */
    end: ISODate;
    interval: ReportInterval;
    /** Net worth lookback bucket count. */
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
    /**
     * A From date before As of, shown only while the Interval is not Total: the
     * balance sheet's multi-period mode, whose columns end at each period's end
     * and the last at As of. Total (labelled "Single date") is the one-column sheet.
     */
    asOfFrom: boolean;
    range: boolean;
    end: boolean;
    /** The Interval select's options, in order; empty = no Interval control. */
    intervals: readonly ReportInterval[];
    count: boolean;
    depth: boolean;
}

export const TAB_CONTROLS: Record<ReportTab, ControlsConfig> = {
    // Insights owns its own period control (PeriodControl), so the shared
    // ReportControls bar shows nothing for it.
    insights: {asOf: false, asOfFrom: false, range: false, end: false, intervals: [], count: false, depth: false},
    // No depth on the balance sheet: groups ARE the reading and the accounts
    // inside one are a drill-down, so a clamp had almost nothing left to move —
    // and what it did move it moved wrongly, hiding accounts with no control on
    // screen to ask for them back. Expanding a group now shows all of it (the
    // engine is asked for an unclamped report), single-child chains still
    // compressed by `compressSectionRows`.
    //
    // The Interval defaults to Total, shown as "Single date": the sheet as of
    // one day, exactly as before. Monthly, Quarterly or Yearly turn it into one
    // column per period end from a From date, the last column at As of: the
    // P&L's range-and-grain parameterisation (plans/24), anchored on As of.
    bs: {asOf: true, asOfFrom: true, range: false, end: false, intervals: ALL_INTERVALS, count: false, depth: false},
    // The income statement dropped its depth slider for the same reason (plans/13):
    // it is grouped now, and a clamp that hid accounts with no control on screen
    // to ask for them back was the worst of both. The range stays — a P&L is a
    // report ABOUT a period, and that is the one thing it cannot infer. The
    // Interval is Total (one column, compared with the prior window) or one
    // column per bucket plus a Total (plans/24).
    is: {asOf: false, asOfFrom: false, range: true, end: false, intervals: ALL_INTERVALS, count: false, depth: false},
    // Cash flow is a report about a period too (plans/24): From/To replaced
    // End + Periods, so the range is named rather than counted backwards.
    cf: {asOf: false, asOfFrom: false, range: true, end: false, intervals: ALL_INTERVALS, count: false, depth: true},
    nw: {asOf: false, asOfFrom: false, range: false, end: true, intervals: PERIODIC_INTERVALS, count: true, depth: true},
    // Subscriptions scan a fixed trailing window, so they expose no controls.
    subs: {asOf: false, asOfFrom: false, range: false, end: false, intervals: [], count: false, depth: false},
};

/**
 * The params a tab's From/To controls read and write: the P&L's `from`/`to`
 * or cash flow's own `cfFrom`/`cfTo`. One answer, used by the URL codec, the
 * controls, the query builder and the export title alike.
 */
export function rangeKeys(tab: ReportTab): {from: "from" | "cfFrom"; to: "to" | "cfTo"} {
    return tab === "cf" ? {from: "cfFrom", to: "cfTo"} : {from: "from", to: "to"};
}

/** Per-tab default interval/count, applied on tab activation (the P&L opens on
 *  its Total statement, cash flow monthly, net worth yearly/5). Depth is shared. */
export const TAB_DEFAULTS: Record<ReportTab, {interval: ReportInterval; count: number}> = {
    insights: {interval: "monthly", count: 12}, // inert (insights ignores interval/count)
    bs: {interval: "total", count: 12}, // the single-date sheet; count inert (a range, not a lookback)
    is: {interval: "total", count: 12}, // count inert: the P&L has a range, not a lookback
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

/** The P&L's default From for a range ending on `end`: January 1 of its year (year to date). */
const yearStart = (end: ISODate): ISODate => `${end.slice(0, 4)}-01-01`;

/** The first day of the month `months - 1` months before `end`'s: `months` monthly columns ending with `end`'s. */
const monthsBack =
    (months: number) =>
    (end: ISODate): ISODate =>
        bucketStart(lastNBuckets(end, "monthly", months)[0]);

/** Cash flow's default From for a range ending on `end`: the trailing twelve monthly columns. */
const trailingYear = monthsBack(12);

/** Defaults: Insights (year-over-year) is the landing tab; bs as-of today, P&L this year, cf/nw last 12 months. */
export function defaultReportParams(now: ISODate = today()): ReportParams {
    const year = now.slice(0, 4);
    const ins = insightsPresetRange(DEFAULT_INSIGHTS_PRESET, now);
    return {
        tab: "insights",
        asOf: now,
        bsFrom: trailingYear(now),
        from: yearStart(now),
        to: `${year}-12-31`,
        // The first of the month eleven months back through today: the same
        // twelve monthly columns the old End + 12 Periods default drew.
        cfFrom: trailingYear(now),
        cfTo: now,
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

// --- The end date shared across tabs ----------------------------------------
// The reports page remembers the end date the user last set on any tab and
// moves the next tab it opens to it, so a balance sheet as of 2026-09-30 is
// followed by a P&L and a cash flow through 2026-09-30 without re-picking the
// date (see `stores/reportsEnd.ts` for the memory, the page for the wiring).
// Only the END is shared; a tab's start follows the rule in `alignToEnd`.

type EndKey = "asOf" | "to" | "cfTo" | "end" | "insEnd";
type StartKey = "from" | "cfFrom" | "bsFrom" | "insStart";

/** Each tab's end-date param, or `null` for a tab with no date control (Subscriptions). */
const END_KEYS: Record<ReportTab, EndKey | null> = {
    insights: "insEnd",
    bs: "asOf",
    is: "to",
    cf: "cfTo",
    nw: "end",
    subs: null,
};

/** The URL params that carry each tab's end (cash flow also reads its legacy `end`). */
const END_URL_PARAMS: Record<ReportTab, readonly string[]> = {
    insights: ["iend"],
    bs: ["asof"],
    is: ["to"],
    cf: ["to", "end"],
    nw: ["end"],
    subs: [],
};

/**
 * Each ranged tab's start param and the start rules RELATIVE to the end that it
 * knows, the first being its default: the P&L's year to date, cash flow's
 * trailing twelve months, and one rule per Insights preset (each a whole-month
 * span ending with the end's month; the default, year over year, first).
 */
const START_RULES: Partial<Record<ReportTab, {key: StartKey; rules: readonly ((end: ISODate) => ISODate)[]}>> = {
    is: {key: "from", rules: [yearStart]},
    // The balance sheet's multi-period start, kept in step with As of whether or
    // not the mode is on, so switching it on reads the trailing year to As of.
    bs: {key: "bsFrom", rules: [trailingYear]},
    cf: {key: "cfFrom", rules: [trailingYear]},
    insights: {
        key: "insStart",
        // INSIGHTS_PRESETS lists the default (year over year) first.
        rules: INSIGHTS_PRESETS.map((preset) => monthsBack(preset.spanMonths)),
    },
};

/** The active end date of `tab` in `p`, or `null` when the tab has none. */
export function tabEnd(p: ReportParams, tab: ReportTab): ISODate | null {
    const key = END_KEYS[tab];
    return key === null ? null : p[key];
}

/** Whether a reports URL names the end date of the tab it opens (a deep link's end beats a remembered one). */
export function searchNamesEnd(search: string): boolean {
    const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const rawTab = q.get("tab");
    if (rawTab === null || !isTab(rawTab)) return false;
    return END_URL_PARAMS[rawTab].some((name) => {
        const value = q.get(name);
        return value !== null && isIsoDate(value);
    });
}

/**
 * The params that move `tab` to end on `end`: `{}` when it has no end or
 * already ends there.
 *
 * A ranged tab's start is not shared; it is decided like this:
 * 1. If it stands in one of the tab's relative relationships to the OLD end
 *    (P&L: January 1 of its year; cash flow: twelve monthly columns; Insights:
 *    a preset's whole-month span), it keeps that relationship to the new end.
 *    The defaults are all relative, so an untouched P&L moved to 2026-09-30
 *    reads 2026-01-01..2026-09-30.
 * 2. Otherwise the user chose it, and it is kept while it is still on or
 *    before the new end.
 * 3. Otherwise (it would invert the range) it falls back to the tab's default
 *    relative to the new end.
 *
 * Pure: no clock, every date comes from `p` and `end`.
 */
export function alignToEnd(p: ReportParams, tab: ReportTab, end: ISODate): Partial<ReportParams> {
    const endKey = END_KEYS[tab];
    if (endKey === null || p[endKey] === end) return {};
    const patch: Partial<ReportParams> = {[endKey]: end};
    const start = START_RULES[tab];
    if (start === undefined) return patch;
    const oldEnd = p[endKey];
    const oldStart = p[start.key];
    const relative = start.rules.find((rule) => rule(oldEnd) === oldStart);
    if (relative !== undefined) patch[start.key] = relative(end);
    else if (oldStart > end) patch[start.key] = start.rules[0](end);
    return patch;
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
    if (c.asOfFrom) {
        // Single date writes nothing more, so its URL is the one it always was.
        if (bsMultiPeriod(p)) {
            q.set("from", p.bsFrom);
            q.set("interval", p.interval);
        }
        return q.toString();
    }
    if (c.range) {
        const keys = rangeKeys(p.tab);
        q.set("from", p[keys.from]);
        q.set("to", p[keys.to]);
    }
    if (c.end) q.set("end", p.end);
    if (c.intervals.length > 0) q.set("interval", p.interval);
    if (c.count) q.set("count", String(p.count));
    if (c.depth) q.set("depth", String(p.depth));
    return q.toString();
}

const isTab = (v: string): v is ReportTab => (TAB_ORDER as string[]).includes(v);
const isInterval = (v: string): v is ReportInterval => (ALL_INTERVALS as string[]).includes(v);

function parseDate(v: string | null, fallback: ISODate): ISODate {
    return v !== null && isIsoDate(v) ? v : fallback;
}

/**
 * The interval a URL asks of `tab`: its own value when the tab offers it (any
 * grain at all on a tab with no Interval control, so decoding stays un-gated),
 * otherwise a fallback.
 *
 * The P&L's fallback is Total, not `dflt`: a P&L link with no `interval`
 * predates the control, and what it showed was the Total statement. Every
 * other tab keeps `dflt.interval` as it always did, unless that grain is not
 * on offer (net worth has no Total).
 */
function parseInterval(raw: string | null, tab: ReportTab, dflt: ReportParams): ReportInterval {
    const offered = TAB_CONTROLS[tab].intervals;
    if (raw !== null && isInterval(raw) && (offered.length === 0 || offered.includes(raw))) return raw;
    // The P&L and the balance sheet both predate their Interval control, and
    // what an older link showed was the one-column report.
    if (tab === "is" || tab === "bs") return "total";
    return offered.length === 0 || offered.includes(dflt.interval) ? dflt.interval : TAB_DEFAULTS[tab].interval;
}

/**
 * Cash flow's range from a URL. `from`/`to` when present; otherwise a link
 * from before the range existed (`end` + `count`, no `from`) is translated to
 * the range it drew — the first day of its oldest bucket through `end` — so a
 * bookmark keeps showing the same columns.
 */
function cashFlowRange(q: URLSearchParams, interval: ReportInterval, dflt: ReportParams): {cfFrom: ISODate; cfTo: ISODate} {
    const legacy = q.get("from") === null && (q.get("end") !== null || q.get("count") !== null);
    if (legacy && interval !== "total") {
        const end = parseDate(q.get("end"), dflt.end);
        const count = clampedInt(q.get("count"), 1, MAX_COUNT, TAB_DEFAULTS.cf.count);
        return {cfFrom: bucketStart(lastNBuckets(end, interval, count)[0]), cfTo: end};
    }
    return {cfFrom: parseDate(q.get("from"), dflt.cfFrom), cfTo: parseDate(q.get("to"), dflt.cfTo)};
}

/** Parse a query string (with or without leading "?"); absent/malformed params fall back to `dflt`. */
export function searchToParams(search: string, dflt: ReportParams): ReportParams {
    const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
    const rawTab = q.get("tab");
    const tab = rawTab !== null && isTab(rawTab) ? rawTab : dflt.tab;
    const interval = parseInterval(q.get("interval"), tab, dflt);
    // `from`/`to` name the ACTIVE tab's range: cash flow's own pair on the cf
    // tab, the balance sheet's multi-period start on bs, the P&L's everywhere
    // else (where, as before, they are not gated).
    const cf = tab === "cf";
    const bs = tab === "bs";
    const asOf = parseDate(q.get("asof"), dflt.asOf);
    return {
        tab,
        asOf,
        // Absent: the trailing year to THIS link's As of, as the controls would
        // have set it, rather than to today's.
        bsFrom: bs ? parseDate(q.get("from"), trailingYear(asOf)) : dflt.bsFrom,
        from: cf || bs ? dflt.from : parseDate(q.get("from"), dflt.from),
        to: cf || bs ? dflt.to : parseDate(q.get("to"), dflt.to),
        ...(cf ? cashFlowRange(q, interval, dflt) : {cfFrom: dflt.cfFrom, cfTo: dflt.cfTo}),
        end: parseDate(q.get("end"), dflt.end),
        interval,
        count: clampedInt(q.get("count"), 1, MAX_COUNT, dflt.count),
        depth: clampedInt(q.get("depth"), 1, 99, dflt.depth),
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
