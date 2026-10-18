// Insights chart display helpers (WP-05). Pure TS: no Svelte/DOM imports.
//
// The chart DATA — sign conventions, the magnitude ranking, the pie and line
// datasets, the big numbers and the footer total — is computed by the engine
// (`crates/ledgeline-core/src/view/series.rs`, served by `/api/journal/summary`
// and `/api/journal/chart`), held to goldens captured from the functions that
// used to live here (plan 26 B5). What remains is the display boundary: turning
// a chart's numbers back into styled text for axes and tooltips.

import {dec, formatAmount} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {OTHER_LABEL} from "#lib/format/palette.js";
import type {Interval as PeriodsInterval} from "#lib/reports/periods.js";

/**
 * The intervals the journal chart offers: a deliberate NARROWING of the
 * canonical `periods.Interval` (no quarterly/yearly), so picking one that
 * `ChartWidget`'s selector does not offer is a type error.
 */
export type Interval = Extract<PeriodsInterval, "daily" | "weekly" | "monthly">;

/** Label used for the folded tail of small accounts — one literal, shared with the holdings pie. */
export const OTHER = OTHER_LABEL;

/**
 * Format a display-boundary number (a chart value that already went through
 * `toNumber`) back into the commodity's display style, e.g. for axis ticks and
 * line tooltips. Exact Dec strings (PieDatum.formatted, big numbers) are still
 * preferred wherever the Dec is available.
 */
export function formatChartValue(value: number, commodity: string, style: AmountStyle): string {
    const scaled = Math.round(value * 10 ** style.precision);
    if (!Number.isSafeInteger(scaled)) return `${value} ${commodity}`; // out of exact range; charts never get here in practice
    return formatAmount({commodity, qty: dec(scaled, style.precision), style});
}

/** Magnitude → abbreviation, largest unit first (K = 10^3 … T = 10^12). */
const COMPACT_UNITS: [divisor: number, suffix: string][] = [
    [1e3, "K"],
    [1e6, "M"],
    [1e9, "B"],
    [1e12, "T"],
];

/** Abbreviate a non-negative magnitude: >=1000 → mantissa (~1 decimal) + K/M/B/T, else a rounded integer with no suffix. */
function compactParts(abs: number): {mantissa: string; suffix: string} {
    if (abs < 1e3) return {mantissa: String(Math.round(abs)), suffix: ""};
    let i = 0;
    while (i + 1 < COMPACT_UNITS.length && abs >= COMPACT_UNITS[i + 1][0]) i += 1;
    let rounded = Number((abs / COMPACT_UNITS[i][0]).toFixed(1));
    // Boundary carry: 999_999 rounds to "1000.0K" → promote to the next unit ("1.0M").
    if (rounded >= 1000 && i + 1 < COMPACT_UNITS.length) {
        i += 1;
        rounded = Number((abs / COMPACT_UNITS[i][0]).toFixed(1));
    }
    return {mantissa: rounded.toFixed(1), suffix: COMPACT_UNITS[i][1]};
}

/**
 * Compact axis-tick variant of formatChartValue: abbreviates magnitude with a
 * K/M/B/T suffix at ~1 decimal so left-axis ticks stay short and never clip
 * (1234 → "$1.2K", 5_269_875 → "$5.3M", 1e9 → "$1.0B"). Zero → "$0";
 * sub-thousand values render as a plain rounded amount ("$500"). Sign and
 * commodity placement follow the style exactly like formatAmount ("$-1.2K").
 * DISPLAY-ONLY and lossy by design — tooltips/hover keep full precision via
 * formatChartValue; only axis ticks use this.
 */
export function formatCompactChartValue(value: number, commodity: string, style: AmountStyle): string {
    const negative = value < 0;
    const {mantissa, suffix} = compactParts(Math.abs(value));
    const point = style.decimalPoint === "." ? mantissa : mantissa.replace(".", style.decimalPoint);
    const num = (negative ? "-" : "") + point + suffix;
    if (commodity === "") return num;
    const space = style.spaced ? " " : "";
    return style.side === "L" ? commodity + space + num : num + space + commodity;
}
