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

import {rootPx} from "#lib/theme/scale.js";
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

/**
 * The size axis tick labels are drawn at: layerchart's own 10px
 * (`Axis.base.svelte`), as rem so it follows the interface scale. Layerchart
 * pins it in px, so the charts restyle their labels with `axisTickLabelProps`.
 */
export const AXIS_LABEL_REM = 0.625;

/** Clear space kept between two neighbouring tick labels (12px at 100 %). */
const LABEL_GAP_REM = 0.75;

/** A chart's padding, per side. */
export interface Padding {
    top: number;
    right: number;
    bottom: number;
    left: number;
}

/**
 * A chart padding written in rem, in px at a root of `root`. The paddings hold
 * the axis labels (the left one is the y labels' gutter), so they have to grow
 * with the labels or a 200 % axis is clipped at the chart's edge.
 */
export function remPadding(rem: Padding, root: number): Padding {
    return {top: rem.top * root, right: rem.right * root, bottom: rem.bottom * root, left: rem.left * root};
}

/**
 * Tick-label props for a layerchart axis drawn at `AXIS_LABEL_REM` on a root of
 * `root` px.
 *
 * Layerchart styles `.lc-axis-tick-label` with `font-size: 10px` and positions
 * the label with a fixed 7px cap height and 11px line height, so at any other
 * interface scale the labels would stay 10px while everything around them
 * grew. The class (a utility, which outranks layerchart's `components` layer)
 * restores the size in rem; the two metrics keep their 10px proportions in px
 * at the live root, which is the unit layerchart does its arithmetic in.
 */
export function axisTickLabelProps(root: number): {class: string; capHeight: string; lineHeight: string} {
    return {class: "text-[0.625rem]", capHeight: `${0.4375 * root}px`, lineHeight: `${0.6875 * root}px`};
}

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
 * been laid out, `plotWidth` 0 or less), it falls back to the fixed target.
 */
export interface AxisFit {
    /** The plot area's width in px: the chart's width less its left and right padding. */
    plotWidth: number;
    /** Every bucket's label, as it will be printed. */
    labels: readonly string[];
    /** The size the labels are drawn at, in rem. Defaults to `AXIS_LABEL_REM`. */
    fontRem?: number;
    /** CSS px per rem. Defaults to the live root (`rootPx()`). */
    root?: number;
}

/**
 * The widest label's px width plus the gap kept beside it: measured in the
 * document's font (`textMeasurer`), or estimated when nothing can measure.
 */
function labelSlotPx({labels, fontRem = AXIS_LABEL_REM, root = rootPx()}: AxisFit): number {
    const fontPx = fontRem * root;
    const widthOf = textMeasurer(fontPx) ?? ((text: string) => text.length * fontPx * FALLBACK_EM_PER_CHAR);
    const widest = labels.reduce((max, label) => Math.max(max, widthOf(label)), 0);
    return widest + LABEL_GAP_REM * root;
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
 *
 * The same rule places any other one-per-bucket text (the stacked chart's net
 * value labels): pass those strings and their size as the `fit`.
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

/** The px extent of a scale's range: the plot width for an x scale. Negative before layout (padding exceeds the box). */
export function rangeWidth(scale: {range(): readonly unknown[]}): number {
    const [start, end] = scale.range();
    return Number(end) - Number(start);
}

/**
 * The x axis `ticks` for a period chart with these bucket labels: a function
 * of the axis scale, which layerchart calls with the scale it has already
 * sized to the plot. So the chart's own measurement is the width the labels are
 * fitted to — no second `bind:clientWidth`, no copy of the chart's padding.
 * Before layout the range is empty and this is the fixed ~six-tick target.
 *
 * `root` is the px per rem the labels are drawn at; a chart passes `rootPx()`
 * from inside its `$derived` so a zoom refits the axis.
 */
export function fittedTicks(labels: readonly string[], root: number = rootPx()): (scale: {range(): readonly unknown[]}) => number[] {
    return (scale) => tickIndices(labels.length, {plotWidth: rangeWidth(scale), labels, root});
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
