// Reading a `Projection`: the numbers the runway chart plots, and the runway
// sentence stated in words. The three statements need nothing from here: they
// render through the Reports tab's own views.
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

import type {Dec} from "#lib/domain/money.js";
import {rankCommodities, toNumber, type MixedAmount} from "#lib/domain/money.js";
import type {PeriodInterval} from "#lib/reports/ui/params.js";
import type {AssetRow, BalanceSeries, Projection} from "./types";

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
export function balanceNumbers(series: BalanceSeries, commodity: string): {opening: number; values: number[]; history: number[]} {
    return {opening: amountIn(series.opening, commodity), values: seriesOf(series.values, commodity), history: seriesOf(series.history, commodity)};
}

/**
 * Every figure the runway chart would draw, in one pass.
 *
 * The two questions below — what to plot, and what that leaves out — are asked
 * of the same series. A generator rather than an array so neither materialises
 * a copy of the projection to count it.
 */
function* projectionAmounts(projection: Projection): Generator<MixedAmount> {
    yield projection.cash.opening;
    yield* projection.cash.history;
    yield* projection.cash.values;
}

/**
 * The commodity to chart: the one the projection carries the most figures in.
 *
 * Counted over every figure a chart would draw, so a scenario written in `$`
 * against an opening balance in `$` charts `$` even when one stray EUR line is
 * present. Ties break lexicographically so the choice is stable across reloads
 * rather than dependent on map order — which is `rankCommodities`' own
 * guarantee, so the head of its ranking IS this answer.
 */
export function chartCommodity(projection: Projection, fallback: string): string {
    return rankCommodities(projectionAmounts(projection))[0] ?? fallback;
}

/**
 * Every commodity in the projection except the one being charted, in a stable
 * order.
 *
 * ALPHABETICAL, not by use, and deliberately not `rankCommodities`' order: this
 * is a list the surface names to say what it is NOT showing, and a reader
 * scanning it for their own currency wants it where the alphabet puts it.
 */
export function otherCommodities(projection: Projection, charted: string): string[] {
    const all = new Set<string>();
    for (const value of projectionAmounts(projection)) {
        for (const commodity of value.keys()) all.add(commodity);
    }
    all.delete(charted);
    return [...all].sort();
}

// ---------------------------------------------------------------------------
// Asset rows (plan 23, Phase 3)
// ---------------------------------------------------------------------------

/**
 * The journal's own balance per asset row, ready for the table's Balance
 * column: one `Dec` per row's `group`, in the commodity that row is written in.
 *
 * ONE COMMODITY, deliberately, and it is the ROW's. The file format states an
 * `opening:` override as a bare number whose commodity comes from the row's own
 * amount (plan 23, amendment 3), so the greyed figure a user is about to type
 * over has to be the figure in that same commodity — showing them a mixed total
 * they could not restate would be offering an edit that cannot be made.
 *
 * `commodityOf` is the caller's, because only the table knows which row is
 * which. A row whose account holds nothing in its own commodity is a real zero.
 *
 * Read from the LAST GOOD projection rather than a matching one: this is a fact
 * about the journal, not about the scenario, and it does not change while an
 * edit is being recomputed. What DOES change is which account a row names, so
 * every entry carries the account it was computed for and the caller is
 * expected to check it — see `journalBalanceFor`.
 */
export function assetRowsByGroup(projection: Projection | null): Map<string, AssetRow> {
    const byGroup = new Map<string, AssetRow>();
    for (const row of projection?.assets ?? []) byGroup.set(row.group, row);
    return byGroup;
}

/**
 * The journal's balance for one row, or null when it is not known YET.
 *
 * Null in three cases, and all three are "no answer" rather than "zero":
 * nothing has been projected yet, the engine declined to model the row (a
 * non-asset account — the warnings say so), or the attribution was computed for
 * a DIFFERENT account because the user has just retyped this row's. That last
 * check is the point: a stale figure under a freshly-typed account is a claim
 * about the ledger that the ledger is not making.
 */
export function journalBalanceFor(rows: ReadonlyMap<string, AssetRow>, group: string, account: string, commodity: string): Dec | null {
    const row = rows.get(group);
    if (row === undefined || row.account !== account.trim()) return null;
    return row.journalOpening.get(commodity) ?? {m: 0n, p: 0};
}

// ---------------------------------------------------------------------------
// The runway, in words
// ---------------------------------------------------------------------------

/** Singular and plural nouns for one bucket of each interval. */
const INTERVAL_NOUN: Record<PeriodInterval, {one: string; many: string}> = {
    monthly: {one: "month", many: "months"},
    quarterly: {one: "quarter", many: "quarters"},
    yearly: {one: "year", many: "years"},
};

/** `18` + `monthly` → `"18 months"`. */
export function periodsPhrase(count: number, interval: PeriodInterval): string {
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
export function runwayStatement(projection: Projection, interval: PeriodInterval, commodity: string, format: (n: number) => string): RunwayStatement {
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
