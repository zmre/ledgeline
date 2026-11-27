// The value-over-time chart's y-axis and colour (plan 47 D1–D4). Pure — no
// Svelte runtime — so the rules are node-tested rather than eyeballed.
//
// A chart that fits its y-axis to the shown window turns a 0.4 % week into a
// cliff. So the axis is ANCHORED: the engine reports the min/max over a horizon
// at least as long as the window (`anchor`, D1/D2, unpadded), the shown values
// join it so nothing is clipped, and this module pads the result (D3) — padding
// is presentation, which is why the engine leaves it to us.

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
    return values.slice(start).filter((n): n is number => n !== null && Number.isFinite(n));
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
    if (anchor !== null && Number.isFinite(anchor.min) && Number.isFinite(anchor.max)) values.push(anchor.min, anchor.max);
    if (values.length === 0) return null;
    const min = Math.min(...values);
    const max = Math.max(...values);
    const floor = (n: number): number => (min >= 0 ? Math.max(0, n) : n);
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
