// Reading a `Projection`: the numbers the three charts plot, and the runway
// sentence stated in words.
//
// Pure module — no Svelte, no DOM, no clock. The sentence in particular is a
// function rather than markup because "cash turns negative in March 2028 — 18
// months" is the single most load-bearing string this tab produces, and a claim
// that is a function can be tested by calling it.
//
// # Charts plot ONE commodity
//
// A `MixedAmount` has no single number in it, and there is no market price for a
// future date to value one with, so a chart has to pick a commodity and say
// which. [`chartCommodity`] picks the one the projection is mostly denominated
// in; [`otherCommodities`] names what that leaves out, so the surface can say so
// rather than quietly plotting a fraction of the answer.

import {toNumber, type MixedAmount} from "$lib/domain/money";
import type {PeriodReport} from "$lib/reports/types";
import type {ReportInterval} from "$lib/reports/ui/params";
import type {BalanceSeries, Projection} from "./types";

/** One number per bucket, for the commodity being charted. Absent is 0 — the amount is known and holds none of it. */
export function seriesOf(values: readonly MixedAmount[], commodity: string): number[] {
    return values.map((value) => amountIn(value, commodity));
}

/** One `MixedAmount`'s figure in `commodity`, or 0 when it holds none. */
export function amountIn(value: MixedAmount, commodity: string): number {
    const found = value.get(commodity);
    return found === undefined ? 0 : toNumber(found);
}

/** A balance series as plain numbers: the opening figure and one closing figure per bucket. */
export function balanceNumbers(series: BalanceSeries, commodity: string): {opening: number; values: number[]} {
    return {opening: amountIn(series.opening, commodity), values: seriesOf(series.values, commodity)};
}

/**
 * The commodity to chart: the one the projection carries the most figures in.
 *
 * Counted over every figure a chart would draw, so a scenario written in `$`
 * against an opening balance in `$` charts `$` even when one stray EUR line is
 * present. Ties break lexicographically so the choice is stable across reloads
 * rather than dependent on map order.
 */
export function chartCommodity(projection: Projection, fallback: string): string {
    const counts = new Map<string, number>();
    const tally = (value: MixedAmount): void => {
        for (const commodity of value.keys()) counts.set(commodity, (counts.get(commodity) ?? 0) + 1);
    };
    tally(projection.cash.opening);
    tally(projection.netWorth.opening);
    for (const value of projection.cash.values) tally(value);
    for (const value of projection.netWorth.values) tally(value);
    for (const total of projection.netIncome.totals) tally(total);

    let best: string | null = null;
    let bestCount = 0;
    for (const [commodity, count] of [...counts].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
        if (count > bestCount) {
            best = commodity;
            bestCount = count;
        }
    }
    return best ?? fallback;
}

/** Every commodity in the projection except the one being charted, in a stable order. */
export function otherCommodities(projection: Projection, charted: string): string[] {
    const all = new Set<string>();
    const tally = (value: MixedAmount): void => {
        for (const commodity of value.keys()) all.add(commodity);
    };
    tally(projection.cash.opening);
    tally(projection.netWorth.opening);
    for (const value of projection.cash.values) tally(value);
    for (const value of projection.netWorth.values) tally(value);
    for (const total of projection.netIncome.totals) tally(total);
    all.delete(charted);
    return [...all].sort();
}

/**
 * The three columns `PeriodFlowChart` takes, off a net-income `PeriodReport`.
 *
 * Read from the DEPTH-1 rows only. The report is a depth-clamped tree — a
 * parent row and its children both appear — so summing every row would count an
 * `expenses:rent` twice, once under `expenses`. The depth-1 rows are the ones
 * whose sum IS `totals`, which is the `PeriodReport` contract the engine pins.
 *
 * `inflows` and `outflows` come back as MAGNITUDES, because that is what
 * `PeriodFlowChart` takes: it owns the sign. `net` is `totals`, the engine's own
 * figure, rather than `inflow − outflow` — they agree here, and handing over the
 * authoritative one means a future row that is in neither column cannot make the
 * line disagree with the table above it.
 */
export function flowSeries(report: PeriodReport, commodity: string): {inflows: number[]; outflows: number[]; net: number[]} {
    const top = report.rows.filter((row) => row.depth === 1);
    const inflows = report.buckets.map(() => 0);
    const outflows = report.buckets.map(() => 0);
    for (const row of top) {
        row.values.forEach((value, i) => {
            if (i >= inflows.length) return;
            const n = amountIn(value, commodity);
            // Cash-flow orientation: positive IS an inflow. The engine already
            // flipped revenue; flipping again here is the bug this comment exists
            // to stop (plan 22, amendment 3).
            if (n > 0) inflows[i] += n;
            else outflows[i] += -n;
        });
    }
    return {inflows, outflows, net: seriesOf(report.totals, commodity)};
}

// ---------------------------------------------------------------------------
// The runway, in words
// ---------------------------------------------------------------------------

/** Singular and plural nouns for one bucket of each interval. */
const INTERVAL_NOUN: Record<ReportInterval, {one: string; many: string}> = {
    monthly: {one: "month", many: "months"},
    quarterly: {one: "quarter", many: "quarters"},
    yearly: {one: "year", many: "years"},
};

/** `18` + `monthly` → `"18 months"`. */
export function periodsPhrase(count: number, interval: ReportInterval): string {
    const noun = INTERVAL_NOUN[interval];
    return `${count} ${count === 1 ? noun.one : noun.many}`;
}

/** What the runway slot above the cash chart says. */
export interface RunwayStatement {
    /** `warn` when the cash crosses zero inside the window; `ok` when it does not; `none` when nothing is projected. */
    tone: "warn" | "ok" | "none";
    /** The sentence. Always a full sentence — it is read on its own. */
    headline: string;
    /** A second sentence qualifying the first, or null. */
    detail: string | null;
}

/**
 * The crossing point, stated in words — or, when there is none, said plainly
 * plus the average build instead.
 *
 * `format` is the caller's money formatter for the charted commodity, so the
 * figure in the sentence is spelled the way every other figure on the page is.
 */
export function runwayStatement(projection: Projection, interval: ReportInterval, commodity: string, format: (n: number) => string): RunwayStatement {
    if (projection.buckets.length === 0) {
        return {tone: "none", headline: "Nothing is projected over this window.", detail: null};
    }

    const runway = projection.runway;
    if (runway !== null) {
        return {
            tone: "warn",
            headline: `Cash turns negative in ${runway.label} — ${periodsPhrase(runway.periods, interval)}.`,
            detail: `That is the first period closing below zero, on ${runway.date}.`,
        };
    }

    const {opening, values} = balanceNumbers(projection.cash, commodity);
    const last = values[values.length - 1] ?? opening;
    const noun = INTERVAL_NOUN[interval].one;
    const build = (last - opening) / values.length;
    const headline = `Cash never turns negative over the ${periodsPhrase(projection.buckets.length, interval)} projected.`;
    if (build === 0) {
        return {tone: "ok", headline, detail: `It ends where it started, at ${format(last)}.`};
    }
    const verb = build > 0 ? "builds" : "draws down";
    return {tone: "ok", headline, detail: `It ${verb} by ${format(Math.abs(build))} a ${noun} on average, ending at ${format(last)}.`};
}
