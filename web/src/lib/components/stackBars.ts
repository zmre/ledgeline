// The bar segments `PeriodStackChart` draws, as SVG path data.
//
// Drawn as plain paths rather than layerchart's `Bars`: every layerchart mark
// registers with the chart as it mounts, and each registration invalidates the
// chart's domains, scales and every other mark's position, so mounting N bars
// costs O(N²). A net worth chart has one bar per account per bucket, and with
// every table account charted that ran to tens of seconds. Paths here are one
// pass over the stacked bands `PeriodStackChart` already computes.
//
// Pure: the caller hands in the scale functions.

/** One bucket: each series' value and its stacked `[y0, y1]` band, index-aligned. */
export interface StackBarRow {
    v: readonly number[];
    band: readonly (readonly [number, number])[];
}

/** Where the bars go, in px. */
export interface StackBarLayout {
    /** The left edge of bucket `i`'s bar. */
    left: (i: number) => number;
    width: number;
    /** Value to px. */
    y: (value: number) => number;
    /** Px of surface between stacked segments. */
    gap: number;
    /** Corner radius of each stack's data end. */
    radius: number;
}

/** One drawn segment. */
export interface StackBar {
    key: string;
    /** Index into the chart's series. */
    series: number;
    bucket: number;
    d: string;
}

/**
 * A rectangle from `top` to `bottom` (px, top < bottom), with the corners at
 * one end rounded: the end away from zero, which is the data end.
 */
function segmentPath(x: number, width: number, top: number, bottom: number, round: "top" | "bottom" | "none", radius: number): string {
    const r = round === "none" ? 0 : Math.max(0, Math.min(radius, width / 2, bottom - top));
    const right = x + width;
    if (r <= 0) return `M${x},${top}H${right}V${bottom}H${x}Z`;
    if (round === "top") return `M${x},${bottom}V${top + r}A${r},${r} 0 0 1 ${x + r},${top}H${right - r}A${r},${r} 0 0 1 ${right},${top + r}V${bottom}Z`;
    return `M${x},${top}H${right}V${bottom - r}A${r},${r} 0 0 1 ${right - r},${bottom}H${x + r}A${r},${r} 0 0 1 ${x},${bottom - r}Z`;
}

/**
 * Every non-zero segment of every bucket, stacked as `band` says.
 *
 * A zero draws nothing. The segment at the base of each side's stack sits on
 * the zero rule; every one after it gives up `gap` px at its inner end, so
 * neighbours are separated by surface. The outermost segment on each side has
 * its data end rounded. `negative` is each SERIES' side (a zero of a negative
 * series still belongs below the axis), the same flag the bands were built from.
 */
export function stackBars(rows: readonly StackBarRow[], negative: readonly boolean[], layout: StackBarLayout): StackBar[] {
    const {left, width, y, gap, radius} = layout;
    return rows.flatMap((row, bucket) => {
        const drawn = row.v.flatMap((value, series) => (value === 0 ? [] : [series]));
        const x = left(bucket);
        return ([false, true] as const).flatMap((down) => {
            const side = drawn.filter((series) => negative[series] === down);
            return side.map((series, n): StackBar => {
                const [base, end] = row.band[series];
                const inner = y(base);
                const outer = y(end);
                // Toward the outer end by the gap, but never past it: a
                // segment thinner than the gap is drawn as nothing, not inverted.
                const shift = n === 0 ? 0 : Math.min(gap, Math.abs(outer - inner));
                const innerPx = outer < inner ? inner - shift : inner + shift;
                const round = n < side.length - 1 ? "none" : down ? "bottom" : "top";
                return {
                    key: `${bucket}:${series}`,
                    series,
                    bucket,
                    d: segmentPath(x, width, Math.min(innerPx, outer), Math.max(innerPx, outer), round, radius),
                };
            });
        });
    });
}
