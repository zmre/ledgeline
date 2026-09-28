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

import {textMeasurer} from "./textWidth";

/**
 * How many x labels a period chart aims for at most.
 *
 * Six is what fits across the 375px viewport the app is used at when labels
 * are short; the step is rounded UP from it, so the real count lands between
 * four and seven depending on where `count` falls. Long labels ("Jun 2026") in
 * a narrow chart get fewer — see `AxisFit`.
 */
const TARGET_TICKS = 6;

/** The px size layerchart draws axis tick labels at (`Axis.base.svelte`). */
export const AXIS_LABEL_PX = 10;

/** Clear space kept between two neighbouring tick labels. */
const LABEL_GAP_PX = 12;

/**
 * Average glyph advance as a fraction of the font size, for when text cannot be
 * measured (no canvas: tests, server rendering). Generous on purpose — digits and
 * capitals run wider than lowercase — since an overestimate only thins the
 * labels while an underestimate lets them collide.
 */
const FALLBACK_EM_PER_CHAR = 0.62;

/**
 * What a chart knows about its x axis beyond the bucket count: the width the
 * plot actually has, and the labels it will print. With it, `tickIndices`
 * spaces the ticks so no two labels touch; without it (or before the chart has
 * been laid out, `plotWidth` 0), it falls back to the fixed target.
 */
export interface AxisFit {
    /** The plot area's width in px: the chart's width less its left and right padding. */
    plotWidth: number;
    /** Every bucket's label, as the axis will print it. */
    labels: readonly string[];
    /**
     * Measures a label at `AXIS_LABEL_PX`. Omitted: the document's own font via
     * `textMeasurer`, falling back to an estimate. `null`: always estimate.
     */
    measure?: ((text: string) => number) | null;
}

/** The widest label's px width plus the gap kept beside it. */
function labelSlotPx(fit: AxisFit): number {
    const measure = fit.measure === undefined ? textMeasurer(AXIS_LABEL_PX) : fit.measure;
    const widthOf = measure ?? ((text: string) => text.length * AXIS_LABEL_PX * FALLBACK_EM_PER_CHAR);
    const widest = fit.labels.reduce((max, label) => Math.max(max, widthOf(label)), 0);
    return widest + LABEL_GAP_PX;
}

/**
 * Evenly spaced bucket indices, always including the last one.
 *
 * Integers only: handed to an axis as explicit `ticks` so no label can land
 * between two buckets. At most about six; fewer when `fit` says the labels are
 * too wide for the room — the stride grows until neighbouring labels clear each
 * other. The last index is always present even when the stride misses it,
 * because an unlabelled final bucket reads as if the series stops before it
 * does; when it would crowd the stride tick just before it, that one is dropped
 * instead.
 */
export function tickIndices(count: number, fit?: AxisFit): number[] {
    const fitted = fit !== undefined && fit.plotWidth > 0 && count > 0;
    // Px per bucket. A band scale (bars) gives each bucket `plotWidth / count`;
    // a point scale (lines) gives slightly more, so this is the safe bound.
    const bucketPx = fitted ? fit.plotWidth / count : Infinity;
    const labelPx = fitted ? labelSlotPx(fit) : 0;
    const step = Math.max(1, Math.ceil(count / TARGET_TICKS), Math.ceil(labelPx / bucketPx));
    const ticks: number[] = [];
    for (let i = 0; i < count; i += step) ticks.push(i);
    const last = count - 1;
    if (count > 0 && ticks[ticks.length - 1] !== last) {
        const previous = ticks[ticks.length - 1];
        if ((last - previous) * bucketPx < labelPx) ticks.pop();
        ticks.push(last);
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
