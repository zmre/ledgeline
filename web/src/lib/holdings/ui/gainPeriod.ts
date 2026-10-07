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
//   ytd  → Dec 31 of the prior year — the engine measures a windowed gain
//          against the value AT gainSince (end of that day) and counts flows
//          strictly after it, so the year's reference is the last day of the
//          previous one; Jan 1 would leave anything dated Jan 1 out of the gain
//   12mo → asOf minus one year (Feb-29 normalizes forward, e.g. 2024-02-29 → 2023-03-01)
//   5yr  → asOf minus five years, normalized the same way
//   all  → undefined (send nothing ⇒ all-time gain = marketValue − basis)
import type {ISODate} from "$lib/domain/types";
import type {GainPeriod} from "$lib/holdings/types";
import {addDays, addMonths} from "$lib/reports/periods";

/**
 * The value-over-time series window for a period: what `/api/holdings/series`
 * (and `/other/series`, and `/benchmarks`) are asked for.
 *
 * Always a `since` START, and for every windowed period it is exactly
 * `gainSinceFor(period, asOf)`: the engine takes a dated window's FIRST point
 * at `since` itself, so the chart opens on the very value the gain beside it is
 * measured against, and the benchmark overlay is seeded there too. "all" sends
 * `since=inception`: only the engine knows when the scope's first holding
 * activity was (and "all time" gain is against cost basis, not a date).
 * `auto` lets the engine size the interval to the span, so an early-January
 * year to date is drawn daily and a twenty-year history quarterly — see
 * `holdings::window::auto_interval`.
 */
export interface TrendWindow {
    interval: "daily" | "weekly" | "monthly" | "auto";
    since: string;
}

/**
 * `asOf` minus `years` years. Feb 29 normalizes FORWARD to Mar 1 in a
 * non-leap target year (the 12mo rule), unlike `addMonths`, which clamps back
 * to Feb 28: the day is added to the 1st of the target month.
 */
function yearsBefore(asOf: ISODate, years: number): ISODate {
    return addDays(`${Number(asOf.slice(0, 4)) - years}${asOf.slice(4, 8)}01`, Number(asOf.slice(8, 10)) - 1);
}

/** Everything that differs by period, in one place. */
interface PeriodFacts {
    /** The selector option. */
    label: string;
    /** Appended to gain labels/headers so a windowed gain is not misread; "" for all-time. */
    suffix: string;
    /** The muted qualifier beside the chart heading, naming the window in words. */
    note: string;
    /** The grain the chart is drawn at: enough points for a line, few enough to read. */
    interval: TrendWindow["interval"];
    /** The `gainSince` date for `asOf`, or undefined for all-time. */
    since: (asOf: ISODate) => ISODate | undefined;
}

/** In display order, shortest window first. The default is "all" (see `defaultScope`). */
const PERIODS: Record<GainPeriod, PeriodFacts> = {
    "1wk": {label: "1 week", suffix: " (1wk)", note: "last 7 days", interval: "daily", since: (asOf) => addDays(asOf, -7)},
    "1mo": {label: "1 month", suffix: " (1mo)", note: "last month", interval: "daily", since: (asOf) => addMonths(asOf, -1)},
    "3mo": {label: "3 months", suffix: " (3mo)", note: "last 3 months", interval: "weekly", since: (asOf) => addMonths(asOf, -3)},
    ytd: {label: "Year to date", suffix: " (YTD)", note: "year to date", interval: "auto", since: (asOf) => `${Number(asOf.slice(0, 4)) - 1}-12-31`},
    "12mo": {label: "12 months", suffix: " (12mo)", note: "last 12 months", interval: "monthly", since: (asOf) => yearsBefore(asOf, 1)},
    "5yr": {label: "5 years", suffix: " (5yr)", note: "last 5 years", interval: "monthly", since: (asOf) => yearsBefore(asOf, 5)},
    all: {label: "All time", suffix: "", note: "all time", interval: "auto", since: () => undefined},
};

/** Selector options in display order, shortest window first. */
export const GAIN_PERIODS: ReadonlyArray<{value: GainPeriod; label: string}> = (Object.keys(PERIODS) as GainPeriod[]).map((value) => ({
    value,
    label: PERIODS[value].label,
}));

/** Narrow an untrusted string (a URL param) to a GainPeriod. */
export function isGainPeriod(value: string | null | undefined): value is GainPeriod {
    return typeof value === "string" && Object.hasOwn(PERIODS, value);
}

/**
 * The `gainSince` query value for `period`, relative to `asOf`, or undefined for
 * all-time (send no param). Pure date-parts math (`reports/periods`) — never
 * `new Date('YYYY-MM-DD')` (that parses UTC and can drift a day in negative zones).
 */
export function gainSinceFor(period: GainPeriod, asOf: ISODate): string | undefined {
    return PERIODS[period].since(asOf);
}

/** Short suffix appended to gain labels/headers so the active window is visible ("" for all-time, else e.g. " (YTD)"). */
export function gainWindowSuffix(period: GainPeriod): string {
    return PERIODS[period].suffix;
}

export function trendWindowFor(period: GainPeriod, asOf: ISODate): TrendWindow {
    return {interval: PERIODS[period].interval, since: gainSinceFor(period, asOf) ?? "inception"};
}

/** The muted qualifier beside the chart heading, naming its window in words. */
export function trendWindowNote(period: GainPeriod): string {
    return PERIODS[period].note;
}
