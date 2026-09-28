// Pure gain-window helpers (WP-10 gain period): map a GainPeriod selection to
// the `gainSince=YYYY-MM-DD` query param the engine's /api/holdings accepts, to
// the label suffix the UI appends so a windowed gain number isn't misread, and
// to the window the value-over-time chart draws. One control drives all three:
// a chart of the last twelve months beside a one-week gain would be two answers
// to the same question. No Svelte/DOM/$app imports — node-testable, ports cleanly.
//
// The window ALWAYS ends at the scope's asOf (which itself defaults to today),
// so gainSince ≤ asOf by construction and every window stays coherent even when
// the user views holdings as of a past date:
//   1wk  → asOf minus seven days
//   1mo  → asOf minus one calendar month, the day clamped to the month's end (03-31 → 02-28)
//   3mo  → asOf minus three calendar months, clamped the same way
//   ytd  → Jan 1 of asOf's year
//   12mo → asOf minus one year (Feb-29 normalizes forward, e.g. 2024-02-29 → 2023-03-01)
//   5yr  → asOf minus five years, normalized the same way
//   all  → undefined (send nothing ⇒ all-time gain = marketValue − basis)
import type {GainPeriod} from "$lib/holdings/types";
import type {ISODate} from "$lib/domain/types";

/** Selector options in display order, shortest window first. The default is "all" (see `defaultScope`). */
export const GAIN_PERIODS: ReadonlyArray<{value: GainPeriod; label: string}> = [
    {value: "1wk", label: "1 week"},
    {value: "1mo", label: "1 month"},
    {value: "3mo", label: "3 months"},
    {value: "ytd", label: "Year to date"},
    {value: "12mo", label: "12 months"},
    {value: "5yr", label: "5 years"},
    {value: "all", label: "All time"},
];

/** Narrow an untrusted string (a URL param) to a GainPeriod. */
export function isGainPeriod(value: string | null | undefined): value is GainPeriod {
    return GAIN_PERIODS.some((p) => p.value === value);
}

function pad(n: number): string {
    return String(n).padStart(2, "0");
}

function isoOf(t: Date): ISODate {
    return `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`;
}

function parts(asOf: ISODate): [number, number, number] {
    return [Number(asOf.slice(0, 4)), Number(asOf.slice(5, 7)), Number(asOf.slice(8, 10))];
}

/** `asOf` minus `days` days. */
function minusDays(asOf: ISODate, days: number): ISODate {
    const [y, m, d] = parts(asOf);
    return isoOf(new Date(Date.UTC(y, m - 1, d - days)));
}

/** `asOf` minus `months` calendar months, the day clamped into the target month. */
function minusMonths(asOf: ISODate, months: number): ISODate {
    const [y, m, d] = parts(asOf);
    const lastDay = new Date(Date.UTC(y, m - 1 - months + 1, 0)).getUTCDate();
    return isoOf(new Date(Date.UTC(y, m - 1 - months, Math.min(d, lastDay))));
}

/** `asOf` minus `years` years; Feb 29 normalizes forward to Mar 1 (the existing 12mo rule). */
function minusYears(asOf: ISODate, years: number): ISODate {
    const [y, m, d] = parts(asOf);
    return isoOf(new Date(Date.UTC(y - years, m - 1, d)));
}

/**
 * The `gainSince` query value for `period`, relative to `asOf`, or undefined for
 * all-time (send no param). Pure date-parts math — never `new Date('YYYY-MM-DD')`
 * (that parses UTC and can drift a day in negative zones).
 */
export function gainSinceFor(period: GainPeriod, asOf: ISODate): string | undefined {
    switch (period) {
        case "all":
            return undefined;
        case "1wk":
            return minusDays(asOf, 7);
        case "1mo":
            return minusMonths(asOf, 1);
        case "3mo":
            return minusMonths(asOf, 3);
        case "ytd":
            return `${asOf.slice(0, 4)}-01-01`;
        case "12mo":
            return minusYears(asOf, 1);
        case "5yr":
            return minusYears(asOf, 5);
    }
}

/** Short suffix appended to gain labels/headers so the active window is visible ("" for all-time, else e.g. " (YTD)"). */
export function gainWindowSuffix(period: GainPeriod): string {
    switch (period) {
        case "all":
            return "";
        case "1wk":
            return " (1wk)";
        case "1mo":
            return " (1mo)";
        case "3mo":
            return " (3mo)";
        case "ytd":
            return " (YTD)";
        case "12mo":
            return " (12mo)";
        case "5yr":
            return " (5yr)";
    }
}

/**
 * The value-over-time series window for a period: what `/api/holdings/series`
 * (and `/other/series`, and `/benchmarks`) are asked for.
 *
 * Either a trailing `count` of buckets (twelve and sixty month-ends — the
 * twelve-month chart is exactly what it always was) or a `since` START the
 * engine counts from. "all" sends `since=inception`: only the engine knows when
 * the scope's first holding activity was. `auto` lets the engine size the
 * interval to the span, so an early-January year to date is drawn daily and a
 * twenty-year history quarterly — see `holdings::window::auto_interval`.
 */
export interface TrendWindow {
    interval: "daily" | "weekly" | "monthly" | "auto";
    count?: number;
    since?: string;
}

export function trendWindowFor(period: GainPeriod, asOf: ISODate): TrendWindow {
    switch (period) {
        case "1wk":
            return {interval: "daily", since: minusDays(asOf, 7)};
        case "1mo":
            return {interval: "daily", since: minusMonths(asOf, 1)};
        case "3mo":
            return {interval: "weekly", since: minusMonths(asOf, 3)};
        case "ytd":
            // The last day of the prior year, so the first point is where the
            // year started from — the value the YTD gain is measured against.
            return {interval: "auto", since: `${Number(asOf.slice(0, 4)) - 1}-12-31`};
        case "12mo":
            return {interval: "monthly", count: 12};
        case "5yr":
            return {interval: "monthly", count: 60};
        case "all":
            return {interval: "auto", since: "inception"};
    }
}

/** The muted qualifier beside the chart heading, naming its window in words. */
export function trendWindowNote(period: GainPeriod): string {
    switch (period) {
        case "1wk":
            return "last 7 days";
        case "1mo":
            return "last month";
        case "3mo":
            return "last 3 months";
        case "ytd":
            return "year to date";
        case "12mo":
            return "last 12 months";
        case "5yr":
            return "last 5 years";
        case "all":
            return "all time";
    }
}
