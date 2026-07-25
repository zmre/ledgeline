// Display helpers for the Insights dashboard. Pure formatting on top of the
// exact-decimal domain money helpers — no Svelte/DOM. Amounts are formatted
// through the shared reportStyles map (same styling as the report tables); the
// base commodity gets the headline figure, any others are secondary lines.

import {formatAmount, isZero, type Dec, type MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import type {MetricDelta} from "$lib/reports/insightsTypes";

/** Style for a commodity absent from the map (e.g. a period with no postings). */
const FALLBACK_STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};

function styleFor(styles: ReadonlyMap<string, AmountStyle>, commodity: string): AmountStyle {
    return styles.get(commodity) ?? FALLBACK_STYLE;
}

const ZERO: Dec = {m: 0n, p: 0};

function absDec(d: Dec): Dec {
    return {m: d.m < 0n ? -d.m : d.m, p: d.p};
}

/** Format one commodity amount, e.g. "$1,234.56". */
export function fmt(commodity: string, qty: Dec, styles: ReadonlyMap<string, AmountStyle>): string {
    return formatAmount({commodity, qty, style: styleFor(styles, commodity)});
}

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
    const klass = direction === 0 ? "text-base-content/50" : direction > 0 === goodWhenUp ? "text-success" : "text-error";
    const amount = fmt(base, absDec(change), styles);
    const pct = metric.pct === null ? "" : ` (${Math.abs(metric.pct).toFixed(1)}%)`;
    return {arrow, text: `${amount}${pct}`, klass};
}

/** Format a percent for the investment box, e.g. "+12.3%" / "−4.0%" / "—". */
export function fmtSignedPct(pct: number | null): string {
    if (pct === null) return "—";
    const sign = pct > 0 ? "+" : pct < 0 ? "−" : "";
    return `${sign}${Math.abs(pct).toFixed(1)}%`;
}

/** Format a signed base-commodity amount, e.g. "+$1,234.56" / "−$40.00" / "—". */
export function fmtSignedAmount(gain: Dec | null, base: string, styles: ReadonlyMap<string, AmountStyle>): string {
    if (gain === null) return "—";
    const sign = gain.m > 0n ? "+" : gain.m < 0n ? "−" : "";
    return `${sign}${fmt(base, absDec(gain), styles)}`;
}

/** daisyUI colour class for a signed figure: success (+), error (−), neutral (0/absent). */
export function signClass(value: number | bigint | null): string {
    if (value === null) return "text-base-content/50";
    const positive = typeof value === "bigint" ? value > 0n : value > 0;
    const negative = typeof value === "bigint" ? value < 0n : value < 0;
    return positive ? "text-success" : negative ? "text-error" : "text-base-content/50";
}

/** Divide a base-commodity Dec by a positive integer for a monthly average (display only). */
export function monthlyAverage(total: Dec | undefined, months: number): Dec {
    if (total === undefined || months <= 0) return ZERO;
    // Exact-enough for display: scale up two extra places, integer-divide, keep 2.
    const scaled = total.m * 100n;
    const per = scaled / BigInt(months);
    return {m: per, p: total.p + 2};
}
