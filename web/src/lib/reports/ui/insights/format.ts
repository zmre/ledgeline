// Display helpers for the Insights dashboard. Pure formatting on top of the
// exact-decimal domain money helpers — no Svelte/DOM. Amounts are formatted
// through the shared reportStyles map (same styling as the report tables); the
// base commodity gets the headline figure, any others are secondary lines.
//
// The genuinely shared primitives (the fallback style, `fmt`, signed amount and
// percent, the sign→colour mapping) moved to `$lib/format` when it turned out
// the holdings UI carried its own drifted copies of each. They are re-exported
// here so this stays the dashboard's one import.

import {isZero, type Dec, type MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import {absDec, fmt, ZERO} from "$lib/format/amounts";
import {sentimentClass} from "$lib/format/sign";
import type {MetricDelta} from "$lib/reports/insightsTypes";

export {fmt, fmtSignedAmount, fmtSignedPct} from "$lib/format/amounts";
export {signClass} from "$lib/format/sign";

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

/** Divide a base-commodity Dec by a positive integer for a monthly average (display only). */
export function monthlyAverage(total: Dec | undefined, months: number): Dec {
    if (total === undefined || months <= 0) return ZERO;
    // Exact-enough for display: scale up two extra places, integer-divide, keep 2.
    const scaled = total.m * 100n;
    const per = scaled / BigInt(months);
    return {m: per, p: total.p + 2};
}
