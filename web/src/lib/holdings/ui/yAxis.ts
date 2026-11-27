// The value-over-time chart's y-axis and colour (plan 47 D1–D4). Pure — no
// Svelte runtime — so the rules are node-tested rather than eyeballed.
//
// A chart that fits its y-axis to the shown window turns a 0.4 % week into a
// cliff. So the axis is ANCHORED: the engine reports the min/max over a horizon
// at least as long as the window (`anchor`, D1/D2, unpadded), the shown values
// join it so nothing is clipped, and this module pads the result (D3) — padding
// is presentation, which is why the engine leaves it to us.
//
// Amendment A1 adds the Performance chart's axis (`paddedReturnDomain`, the
// same padding over return fractions) and the two colour rules that replaced
// "value first vs last" where it misled: the Value chart follows the period's
// gain (`gainTrend`), the Performance chart the sign of its last return
// (`returnTrend`).

/** The period's direction, first point against last (D4). Picks the hero line's colour. */
export type Trend = "up" | "down" | "flat";

/** Share of the range (or, for a flat range, of the value) added above the max. */
const PAD_ABOVE = 0.1;
/** Share added below the min — more than above, so a line never sits on the axis. */
const PAD_BELOW = 0.2;

/**
 * The values from the first one above zero on (D2). An opening balance jumps
 * from $0 the day the account was opened; counting that 0 would pin every
 * axis to the floor. Points ≤ 0 AFTER the first positive one are real (a sold-out
 * position) and stay. Gaps (`null`) are skipped.
 */
function fromFirstPositive(values: readonly (number | null)[]): number[] {
    const start = values.findIndex((n) => n !== null && n > 0);
    if (start < 0) return [];
    return finite(values.slice(start));
}

/**
 * The padded y-domain for the chart, or null when there is nothing positive to
 * anchor on (the caller then leaves the chart's own domain alone).
 *
 * `anchor` is the engine's unpadded min/max over the anchor horizon (null when
 * it sent none). `shown` is every line on the chart — the hero and each
 * overlay — whose points join the min/max with the same leading-zero skip, so
 * a benchmark that outran the portfolio is never cut off.
 *
 * Padding (D3) is a share of the RANGE: 10 % above, 20 % below, floored at 0
 * when nothing is negative. A flat range pads by a share of the VALUE instead,
 * since 10 % of nothing is nothing.
 */
export function paddedDomain(anchor: {min: number; max: number} | null, shown: readonly (readonly (number | null)[])[]): [number, number] | null {
    const values = shown.flatMap(fromFirstPositive);
    if (isRange(anchor)) values.push(anchor.min, anchor.max);
    if (values.length === 0) return null;
    const min = Math.min(...values);
    return pad(values, (n) => (min >= 0 ? Math.max(0, n) : n));
}

/**
 * The Performance chart's padded y-domain (plan 47 A1c), in return fractions.
 *
 * The same D3 padding over `anchor` (the engine's `returnAnchor`, unpadded,
 * rebased to the window's first point) and every shown line — the portfolio's
 * return and each benchmark's — but with none of the value axis's special
 * cases: a return of 0 is the first point, not an account opening, so nothing
 * is skipped, and returns go negative, so nothing is floored. A flat range pads
 * by a share of the value; a flat ZERO range (a window in which no price moved)
 * gets `FLAT_RETURN_SPAN` so the 0 % line is still drawn somewhere sensible.
 * Null only when there is no return at all.
 */
export function paddedReturnDomain(anchor: {min: number; max: number} | null, shown: readonly (readonly (number | null)[])[]): [number, number] | null {
    const values = shown.flatMap(finite);
    if (isRange(anchor)) values.push(anchor.min, anchor.max);
    if (values.length === 0) return null;
    return pad(values, (n) => n) ?? [-FLAT_RETURN_SPAN * PAD_BELOW, FLAT_RETURN_SPAN * PAD_ABOVE];
}

/** The span (a fraction: 10 %) a window whose every return is 0 is drawn against. */
const FLAT_RETURN_SPAN = 0.1;

/** A min/max pair worth using: both finite. */
function isRange(range: {min: number; max: number} | null): range is {min: number; max: number} {
    return range !== null && Number.isFinite(range.min) && Number.isFinite(range.max);
}

/** Every finite value, gaps dropped. */
function finite(values: readonly (number | null)[]): number[] {
    return values.filter((n): n is number => n !== null && Number.isFinite(n));
}

/**
 * D3 over a non-empty list: 10 % of the range above, 20 % below, `floor`
 * applied to both ends. A flat range pads by a share of the value instead;
 * a flat range at exactly 0 has no share to take and answers null.
 */
function pad(values: readonly number[], floor: (n: number) => number): [number, number] | null {
    const min = Math.min(...values);
    const max = Math.max(...values);
    const range = max - min;
    if (range > 0) return [floor(min - range * PAD_BELOW), max + range * PAD_ABOVE];
    if (max === 0) return null;
    const size = Math.abs(max);
    return [floor(max - size * PAD_BELOW), max + size * PAD_ABOVE];
}

/** First non-null value against the last (D4); fewer than two values is flat. */
export function trendOf(values: readonly (number | null)[]): Trend {
    const present = values.filter((n): n is number => n !== null && Number.isFinite(n));
    if (present.length < 2) return "flat";
    const first = present[0];
    const last = present[present.length - 1];
    return last > first ? "up" : last < first ? "down" : "flat";
}

/**
 * The Value chart's colour (plan 47 A1a): the period's GAIN, not its value.
 * Selling $60k of a fund that rose 12 % leaves less VALUE at the end, and a
 * chart that compared first point with last painted that sale as a loss beside
 * a green Gain tile. `gain` is that tile's number (the stat tiles'
 * `totals.gain`); null — unknown, or a chart with no gain to follow (the Other
 * tab) — falls back to value first-vs-last (`trendOf`).
 */
export function gainTrend(gain: number | null, values: readonly (number | null)[]): Trend {
    if (gain === null || !Number.isFinite(gain)) return trendOf(values);
    return gain > 0 ? "up" : gain < 0 ? "down" : "flat";
}

/**
 * The Performance chart's colour (A1b): the sign of the LAST return. A return
 * line starts at 0 by construction, so "last against first" is just its sign.
 */
export function returnTrend(values: readonly (number | null)[]): Trend {
    const last = finite(values).at(-1);
    return last === undefined || last === 0 ? "flat" : last > 0 ? "up" : "down";
}
