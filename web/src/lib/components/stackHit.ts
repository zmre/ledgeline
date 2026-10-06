// Which stacked segment is where: what `PeriodStackChart`'s tooltip asks to
// name the one segment under the pointer, and to list a bucket's segments in
// the order the eye sees them.
//
// Pure, over the same stacked bands the chart draws (`StackBarRow`).

import type {StackBarRow} from "./stackBars";

/**
 * The series whose band is under `value` at bucket position `at`, or null.
 *
 * `at` may be fractional: an area between two buckets is the straight line
 * between their bands (layerchart's default linear curve), so the band there is
 * interpolated the same way. Bars pass a whole bucket index. A band of zero
 * thickness is under nothing.
 */
export function seriesAt(rows: readonly StackBarRow[], at: number, value: number): number | null {
    if (rows.length === 0 || !(at >= 0) || at > rows.length - 1) return null;
    const lo = Math.floor(at);
    const hi = Math.min(lo + 1, rows.length - 1);
    const t = at - lo;
    const lerp = (a: number, b: number): number => a + (b - a) * t;
    const count = rows[lo].band.length;
    for (let series = 0; series < count; series++) {
        const [a0, a1] = rows[lo].band[series];
        const [b0, b1] = rows[hi].band[series];
        const y0 = lerp(a0, b0);
        const y1 = lerp(a1, b1);
        if (y0 !== y1 && value >= Math.min(y0, y1) && value <= Math.max(y0, y1)) return series;
    }
    return null;
}

/**
 * One bucket's non-zero series, top of the chart to bottom: the up stack from
 * its outermost segment down to the zero rule, then the down stack from the
 * rule outward. `negative` is each series' side, as the bands were built.
 */
export function topToBottom(values: readonly number[], negative: readonly boolean[]): number[] {
    const drawn = values.flatMap((value, series) => (value === 0 ? [] : [series]));
    return [...drawn.filter((series) => !negative[series]).reverse(), ...drawn.filter((series) => negative[series])];
}
