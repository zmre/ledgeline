// The bucket axis both period charts draw, decided once.
//
// Period charts plot the bucket INDEX, never the bucket's date. A month is 28
// to 31 days and a quarter is 90 to 92, so a time scale spaces the buckets
// unevenly and invites the reader to compare widths that mean nothing. An index
// scale spaces them evenly and the label — a string the caller already
// formatted — comes from the axis formatter.
//
// That trade costs exactly one thing, and this module is the price. A
// continuous scale over 0..n-1 will happily put a tick at 2.5, halfway between
// two buckets, and then `labelFormatter` rounds it and prints a label belonging
// to neither side of it. So the ticks are named explicitly, as integers.
//
// `HoldingsTrend.svelte` and `insights/ChartWidget.svelte` each carried their
// own copy of both functions, character for character. They live here so a
// third chart cannot invent a seventh spacing.

/**
 * How many x labels a period chart aims for.
 *
 * Six is what fits across the 375px viewport the app is used at without the
 * labels touching; the step is rounded UP from it, so the real count lands
 * between four and seven depending on where `count` falls.
 */
const TARGET_TICKS = 6;

/**
 * About six evenly spaced bucket indices, always including the last one.
 *
 * Integers only: handed to an axis as explicit `ticks` so no label can land
 * between two buckets. The last index is always present even when the stride
 * misses it, because an unlabelled final bucket reads as if the series stops
 * before it does.
 */
export function tickIndices(count: number): number[] {
    const step = Math.max(1, Math.ceil(count / TARGET_TICKS));
    const ticks: number[] = [];
    for (let i = 0; i < count; i++) {
        if (i % step === 0 || i === count - 1) ticks.push(i);
    }
    return ticks;
}

/**
 * A formatter turning a numeric bucket index back into its label.
 *
 * Takes `unknown` because that is what layerchart hands an axis or tooltip
 * formatter, and answers "" rather than "undefined" for an index outside the
 * range — a stray tick should print nothing, not a word.
 */
export function labelFormatter(labels: readonly string[]): (i: unknown) => string {
    return (i: unknown): string => labels[Math.round(Number(i))] ?? "";
}
