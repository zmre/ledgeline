// Where each Sankey label goes, as plain arithmetic.
//
// A bar's label used to sit at the bar's own middle, whatever its neighbours
// did. On a ledger with many small accounts the bars are a pixel or two tall
// and stacked 12px apart, so their two-line labels landed on top of each other
// and neither could be read. This module places one COLUMN of labels at a time,
// and the panel draws whatever it says. It knows nothing about Svelte, the DOM
// or LayerChart, which is what lets the claims below be tested over thousands of
// random columns rather than one picture.
//
// The rules, in the order they give way:
//
//   1. ORDER IS SACRED. Labels come in the column's top-to-bottom bar order and
//      leave in it. A label is pushed past its anchor, never past a neighbour,
//      so the n-th label always names the n-th bar and a leader line can never
//      cross another one.
//   2. SPACING, THEN BOUNDS. Each label is pushed down just far enough to clear
//      the one above it, then the column is pushed back up off the bottom edge,
//      then down off the top edge. A label that did not need to move stays on
//      its bar's middle to the pixel.
//   3. THE AMOUNT GOES BEFORE THE CHART GROWS. A column whose two-line labels
//      cannot fit at all drops the figure line from its SMALLEST bars first
//      (ties: the lower one), one at a time, until it fits. The figure is not
//      lost: the legend lists it for every account, and the tooltip for every
//      bar. Only when every label is down to its name and the column still
//      overflows does it spill, and the panel sizes the chart so that does not
//      happen (`SankeyPanel`'s `heightFor`).
//
// Coordinates are the panel's: `y` grows downward, and a label's `y` is the
// middle of its box, the same point an undisturbed label shares with its bar.

/** One bar's label, in the column's top-to-bottom order. */
export interface LabelItem {
    key: string;
    /** Where the label wants to sit: its bar's vertical middle. */
    anchor: number;
    /** The bar's value. Only compared, to decide whose figure line goes first. */
    size: number;
}

export interface LabelOptions {
    /** The highest a label's box may reach. */
    top: number;
    /** The lowest a label's box may reach. */
    bottom: number;
    /** Height of a name-and-amount label, air included. */
    twoLine: number;
    /** Height of a name-only label, air included. */
    oneLine: number;
}

export interface PlacedLabel {
    key: string;
    /** The middle of the label's box. */
    y: number;
    /** False once the column ran out of room and this label's amount was dropped. */
    twoLine: boolean;
    /** Whether the label left its bar, and so needs a leader line back to it. */
    displaced: boolean;
}

/** How far a label may drift before it reads as belonging somewhere else. */
export const DISPLACED_PX = 2;

/**
 * Place one column of labels: in order, at least a box apart, inside
 * `[top, bottom]` whenever the column fits at one line each.
 *
 * Two neighbours are spaced by half of each box (so two two-line labels sit
 * `twoLine` apart, two one-line labels `oneLine`, and a mixed pair halfway),
 * which makes "fits" exactly "the boxes' heights add up to no more than the
 * room".
 */
export function layoutLabels(items: readonly LabelItem[], opts: LabelOptions): PlacedLabel[] {
    const room = opts.bottom - opts.top;
    const twoLine = items.map(() => true);
    const heights = (): number[] => twoLine.map((two) => (two ? opts.twoLine : opts.oneLine));

    // Whether the column fits depends only on the summed heights, so the
    // demotions can be decided before anything is placed. Smallest bar first;
    // on a tie the later (lower) one, which is the one the engine ranked last.
    let used = items.length * opts.twoLine;
    if (used > room) {
        const victims = items.map((item, i) => ({size: item.size, i})).sort((a, b) => a.size - b.size || b.i - a.i);
        for (const {i} of victims) {
            if (used <= room) break;
            twoLine[i] = false;
            used -= opts.twoLine - opts.oneLine;
        }
    }

    const y = place(
        items.map((item) => item.anchor),
        heights(),
        opts
    );
    return items.map((item, i) => ({
        key: item.key,
        y: y[i],
        twoLine: twoLine[i],
        displaced: Math.abs(y[i] - item.anchor) > DISPLACED_PX,
    }));
}

/**
 * The three passes. Forward pushes each label clear of the one above; backward
 * pulls the tail up off `bottom`; forward again pushes the head down off `top`.
 * The last pass wins, so a column that cannot fit spills off the BOTTOM, never
 * the top, where its first (biggest) labels stay readable.
 */
function place(anchors: readonly number[], heights: readonly number[], {top, bottom}: LabelOptions): number[] {
    const n = anchors.length;
    const gap = (i: number): number => (heights[i - 1] + heights[i]) / 2;
    const y = anchors.slice();
    for (let i = 1; i < n; i++) y[i] = Math.max(y[i], y[i - 1] + gap(i));
    if (n > 0) y[n - 1] = Math.min(y[n - 1], bottom - heights[n - 1] / 2);
    for (let i = n - 2; i >= 0; i--) y[i] = Math.min(y[i], y[i + 1] - gap(i + 1));
    if (n > 0) y[0] = Math.max(y[0], top + heights[0] / 2);
    for (let i = 1; i < n; i++) y[i] = Math.max(y[i], y[i - 1] + gap(i));
    return y;
}

/** The geometry LayerChart's Sankey adds to each node: optional in its types, present once laid out. */
export interface Bar {
    key: string;
    side: string;
    value?: number;
    y0?: number;
    y1?: number;
}

/** One label's place, and whether its column stepped out to give leader lines room. */
export interface LabelSpot extends PlacedLabel {
    leaders: boolean;
}

/**
 * Every bar's label, keyed by bar: one `layoutLabels` per column (`side`), in
 * the order the Sankey stacked that column's bars. A column with even one
 * displaced label marks all of its labels `leaders`, so the panel steps the
 * whole column out together and its labels still start on one edge.
 */
export function placeLabels(bars: readonly Bar[], opts: LabelOptions): Record<string, LabelSpot> {
    const spots: Record<string, LabelSpot> = {};
    for (const side of new Set(bars.map((bar) => bar.side))) {
        const column = bars.filter((bar) => bar.side === side).sort((a, b) => (a.y0 ?? 0) - (b.y0 ?? 0));
        const placed = layoutLabels(
            column.map((bar) => ({key: bar.key, anchor: ((bar.y0 ?? 0) + (bar.y1 ?? 0)) / 2, size: bar.value ?? 0})),
            opts
        );
        const leaders = placed.some((label) => label.displaced);
        for (const label of placed) spots[label.key] = {...label, leaders};
    }
    return spots;
}
