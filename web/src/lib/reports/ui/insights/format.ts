// Display helpers for the Insights dashboard. Pure formatting on top of the
// exact-decimal domain money helpers — no Svelte/DOM. Amounts are formatted
// through the shared reportStyles map (same styling as the report tables); the
// base commodity gets the headline figure, any others are secondary lines.
//
// The genuinely shared primitives (the fallback style, `fmt`, signed amount and
// percent, the sign→colour mapping) moved to `#lib/format` when it turned out
// the holdings UI carried its own drifted copies of each. They are re-exported
// here so this stays the dashboard's one import.

import {isZero, sub, toNumber, type Dec, type MixedAmount} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {absDec, fmt, ZERO} from "#lib/format/amounts.js";
import {sentimentClass} from "#lib/format/sign.js";
import type {MetricDelta} from "#lib/reports/insightsTypes.js";

export {fmt, fmtSignedAmount, fmtSignedPct} from "#lib/format/amounts.js";
export {signClass} from "#lib/format/sign.js";

/** The base-commodity part of a MixedAmount, formatted (a real "0" when absent). */
export function fmtBase(ma: MixedAmount, base: string, styles: ReadonlyMap<string, AmountStyle>): string {
    return fmt(base, ma.get(base) ?? ZERO, styles);
}

/** Non-base, non-zero commodity lines of a MixedAmount, for the secondary display. */
export function extras(ma: MixedAmount, base: string, styles: ReadonlyMap<string, AmountStyle>): string[] {
    return [...ma.entries()]
        .filter(([commodity, qty]) => commodity !== base && !isZero(qty))
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([commodity, qty]) => fmt(commodity, qty, styles));
}

/** A small delta line coloured by sentiment: `▲ $123.45 (12.3%)` / `▼ …`. */
export interface DeltaLine {
    arrow: string;
    text: string;
    /** daisyUI text colour class (success/error/neutral). */
    klass: string;
}

/**
 * Build the "small" delta line for a standard metric. `goodWhenUp` decides the
 * colour: an increase in revenue is green, an increase in expenses is red.
 */
export function deltaLine(metric: MetricDelta, base: string, styles: ReadonlyMap<string, AmountStyle>, goodWhenUp: boolean): DeltaLine {
    const change = metric.delta.get(base) ?? ZERO;
    const direction = change.m > 0n ? 1 : change.m < 0n ? -1 : 0;
    const arrow = direction > 0 ? "▲" : direction < 0 ? "▼" : "▪";
    // The arrow already carries the direction, so the number itself is unsigned.
    const amount = fmt(base, absDec(change), styles);
    const pct = metric.pct === null ? "" : ` (${Math.abs(metric.pct).toFixed(1)}%)`;
    return {arrow, text: `${amount}${pct}`, klass: sentimentClass(change.m, goodWhenUp)};
}

/** A single-commodity MetricDelta over the base, for figures the engine sends as two plain Decs. */
export function baseMetric(current: Dec, previous: Dec, base: string): MetricDelta {
    const delta = sub(current, previous);
    const pct = previous.m === 0n ? null : ((toNumber(current) - toNumber(previous)) / Math.abs(toNumber(previous))) * 100;
    return {current: new Map([[base, current]]), previous: new Map([[base, previous]]), delta: new Map([[base, delta]]), pct};
}

/** Divide a base-commodity Dec by a positive integer for a monthly average (display only). */
export function monthlyAverage(total: Dec | undefined, months: number): Dec {
    if (total === undefined || months <= 0) return ZERO;
    // Exact-enough for display: scale up two extra places, integer-divide, keep 2.
    const scaled = total.m * 100n;
    const per = scaled / BigInt(months);
    return {m: per, p: total.p + 2};
}

// --- Startup metrics (plans/37) ---------------------------------------------

/** One decimal, grouped: `1,234.5`. */
function oneDecimal(n: number): string {
    return n.toLocaleString("en-US", {minimumFractionDigits: 1, maximumFractionDigits: 1});
}

/** A percentage in percent units, one decimal: `120.8%`. Null → em dash. */
export function fmtPct(pct: number | null): string {
    return pct === null ? "—" : `${oneDecimal(pct)}%`;
}

/**
 * A duration in months, for runway and CAC payback: whole months from 10 up
 * (`66 months`), one decimal below that, where the fraction still matters
 * (`2.5 months`, `1 month`).
 */
export function fmtMonths(months: number): string {
    const shown = months >= 10 ? Math.round(months) : Math.round(months * 10) / 10;
    const text = Number.isInteger(shown) ? shown.toLocaleString("en-US") : oneDecimal(shown);
    return `${text} ${shown === 1 ? "month" : "months"}`;
}

/** A change between two percentages, in percentage points: `+1.9 pp`, `-0.4 pp`, `0.0 pp`. */
export function fmtPctPoints(points: number): string {
    const rounded = Math.round(points * 10) / 10;
    const sign = rounded > 0 ? "+" : rounded < 0 ? "-" : "";
    return `${sign}${oneDecimal(Math.abs(rounded))} pp`;
}

/** A ratio with one decimal and a multiplication sign: `1.6×`. */
export function fmtRatio(ratio: number): string {
    return `${oneDecimal(ratio)}×`;
}

/** A day count, one decimal: `25.5 days`. */
export function fmtDays(days: number): string {
    return `${oneDecimal(days)} days`;
}

/**
 * A delta line between two percentages, in points (gross margin's change line).
 * Null when either side is missing: there is nothing to compare.
 */
export function pointsLine(current: number | null, previous: number | null, goodWhenUp: boolean): DeltaLine | null {
    if (current === null || previous === null) return null;
    const change = Math.round((current - previous) * 10) / 10;
    const arrow = change > 0 ? "▲" : change < 0 ? "▼" : "▪";
    return {arrow, text: `${oneDecimal(Math.abs(change))} pp`, klass: sentimentClass(change, goodWhenUp)};
}

/** A delta line for a percentage that is ITSELF the change (customer growth): `▲ 12.5% growth`. */
export function growthLine(pct: number | null, goodWhenUp: boolean): DeltaLine | null {
    if (pct === null) return null;
    const arrow = pct > 0 ? "▲" : pct < 0 ? "▼" : "▪";
    return {arrow, text: `${fmtPct(Math.abs(pct))} growth`, klass: sentimentClass(pct, goodWhenUp)};
}

/** One reading of a burn multiple. */
export interface BurnBand {
    /** The band's top: inclusive, except the first band's (which is "under 1"). */
    readonly upTo: number;
    readonly label: string;
    /** daisyUI text colour class, "" for neutral. */
    readonly klass: string;
}

/**
 * How to read a burn multiple (net burn ÷ net new ARR). The bands are David
 * Sacks's, from "The Burn Multiple" (Craft Ventures, 2020): under 1 is very
 * efficient, 1–2 good, over 2 suspect, over 3 bad. docs/reports/startup-metrics.md
 * prints the same table; change both together.
 */
export const BURN_MULTIPLE_BANDS: readonly BurnBand[] = [
    {upTo: 1, label: "Very efficient", klass: "text-success"},
    {upTo: 2, label: "Good", klass: ""},
    {upTo: 3, label: "Suspect", klass: "text-warning"},
    {upTo: Infinity, label: "Bad", klass: "text-error"},
];

/** The band a burn multiple falls in: exactly 1 is Good, exactly 2 still Good, exactly 3 still Suspect. */
export function burnBand(multiple: number): BurnBand {
    const [first, ...rest] = BURN_MULTIPLE_BANDS;
    if (multiple < first.upTo) return first;
    return rest.find((band) => multiple <= band.upTo) ?? rest[rest.length - 1];
}
