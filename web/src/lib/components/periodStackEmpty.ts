// When `PeriodStackChart` draws a plot, and when it says why it does not.
//
// One rule, used by the component itself, so a caller never has to guess
// whether its data will draw: the chart asks this with its own props.

/** Why a stack cannot be drawn: nothing non-zero in it, or fewer buckets than the caller's minimum. */
export type StackEmpty = "no-data" | "too-few";

/**
 * `"no-data"` when every bucket's series values AND net are zero (or there are
 * no buckets): a chart of nothing is a flat rule that says less than the words.
 * `"too-few"` when there is something, but in fewer than `minBuckets` buckets —
 * one bucket is a single column or a zero-width area, a number rather than a
 * trend. `null` when it draws.
 */
export function stackEmptyReason(
    labels: readonly unknown[],
    series: readonly {values: readonly number[]}[],
    net: readonly number[],
    minBuckets = 1
): StackEmpty | null {
    const zeroAt = (i: number): boolean => (net[i] ?? 0) === 0 && series.every((s) => (s.values[i] ?? 0) === 0);
    if (labels.every((_, i) => zeroAt(i))) return "no-data";
    return labels.length < minBuckets ? "too-few" : null;
}
