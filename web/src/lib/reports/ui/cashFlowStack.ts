// The Cash Flow chart's model: the table's rows, stacked.
//
// Pure — no Svelte, no DOM, no live palette (the caller hands one in).
//
// The chart and the table below it must never disagree, so the chart is built
// from the SAME displayed row list the table renders: `compressPeriodRows` over
// the report's rows (`ReportTable`'s `periodRows`), with the same labels. There
// is no second source of accounts to drift from.
//
// # The rule
//
//   - SEGMENTS ARE THE DISPLAYED TREE'S PARTS. Every displayed row with no
//     displayed child is a segment carrying that row's values. A displayed
//     parent's figure is the sum of its children's plus its own postings, so it
//     is drawn as its children's segments stacked, plus — when the parent's
//     value differs from the sum of its displayed children in any bucket — one
//     OWN segment, `parent − Σ children`, computed on the exact `MixedAmount`s.
//     So each displayed row's value is exactly the sum of the segments under it,
//     and the roots' segments sum to the table's Net.
//   - LABELS ARE THE TABLE'S. A segment is labelled with its row's displayed
//     text (an own segment with its parent's). Only when two segments would
//     read the same (`cash` under two parents) do both fall back to the full
//     account name, which is what the table's indentation says about them.
//   - NO TAIL FOLDING. The table's accounts are the chart's accounts. Colours
//     are handed out in table row order: the first `SLOT_COUNT` drawn segments
//     get their own palette slot and every later one gets the muted `other`
//     colour — never a recycled slot, which would make two accounts look alike
//     — while still being its own segment, tooltip row and legend entry.
//   - ONE COMMODITY. Cash flow is not valued, so a `MixedAmount` has no single
//     number: the chart plots the report's most common commodity
//     (`stackCommodity`) and the panel names the rest. A segment with nothing
//     in that commodity has nothing to draw and is left out.
//   - THE NET is the table's Net row (`report.totals`) in that commodity.

import {maAdd, maIsZero, maNeg, type MixedAmount} from "$lib/domain/money";
import {colorAt, type ChartPalette} from "$lib/format/palette";
import {amountIn} from "$lib/projections/projectionView";
import {bucketLabel} from "../periods";
import type {PeriodReport, PeriodRow} from "../types";
import {compressPeriodRows, type DisplayRow} from "./displayRows";
import {splitBySign, stackCommodity, type PeriodStack, type StackEntity} from "./periodStack";

/** One non-overlapping part of the displayed tree, in exact amounts. */
export interface FlowSegment {
    /** The row's account, or `account (own)` for a parent's own postings. */
    key: string;
    /** The displayed row this segment belongs to. */
    account: string;
    /** True for a parent's own-postings segment. */
    own: boolean;
    /** What the legend and tooltip say: the table's text for the row. */
    label: string;
    /** Exact per-bucket values. */
    values: readonly MixedAmount[];
}

const difference = (a: MixedAmount, b: MixedAmount): MixedAmount => maAdd(a, maNeg(b));

/**
 * The displayed children of each displayed row, by index. `compressRows` emits
 * depth-first with each child one indent deeper than its parent, so a row's
 * children are the rows one indent deeper before the next row at its own
 * indent or shallower.
 */
function displayedChildren(display: readonly DisplayRow<PeriodRow>[]): number[][] {
    const children: number[][] = display.map(() => []);
    const open: number[] = [];
    display.forEach((row, i) => {
        while (open.length > 0 && display[open[open.length - 1]].indent >= row.indent) open.pop();
        if (open.length > 0) children[open[open.length - 1]].push(i);
        open.push(i);
    });
    return children;
}

/**
 * The table's displayed rows as stack segments, in table order: a leaf's own
 * values, and for a parent whose value is not its displayed children's sum, the
 * remainder as an own segment at the parent's place (before its children).
 */
export function cashFlowSegments(display: readonly DisplayRow<PeriodRow>[]): FlowSegment[] {
    const children = displayedChildren(display);
    const segments: FlowSegment[] = [];
    display.forEach((shown, i) => {
        const {row} = shown;
        const kids = children[i];
        if (kids.length === 0) {
            segments.push({key: row.account, account: row.account, own: false, label: shown.label, values: row.values});
            return;
        }
        const own = row.values.map((value, b) => kids.reduce((rest, k) => difference(rest, display[k].row.values[b] ?? new Map()), value));
        if (own.some((value) => !maIsZero(value))) {
            segments.push({key: `${row.account} (own)`, account: row.account, own: true, label: shown.label, values: own});
        }
    });

    const counts = new Map<string, number>();
    for (const segment of segments) counts.set(segment.label, (counts.get(segment.label) ?? 0) + 1);
    return segments.map((segment) => ((counts.get(segment.label) ?? 0) > 1 ? {...segment, label: segment.account} : segment));
}

const sum = (values: readonly number[]): number => values.reduce((a, b) => a + b, 0);

/**
 * The whole Cash Flow chart model for one `PeriodReport`, in the shape
 * `PeriodStackView` draws. Legend and draw order are the table's row order.
 *
 * `fallback` names the commodity when the report holds none at all (an empty
 * journal); `palette` is the resolved theme palette (`chartColors.current`).
 */
export function cashFlowStack(report: PeriodReport, palette: ChartPalette, fallback: string): PeriodStack {
    const {commodity, omitted} = stackCommodity(report, fallback);
    const entities = cashFlowSegments(compressPeriodRows(report.rows))
        .map((segment) => ({segment, values: segment.values.map((value) => amountIn(value, commodity))}))
        .filter(({values}) => values.some((v) => v !== 0))
        // Slots in table order; past `SLOT_COUNT`, `colorAt` gives the `other`
        // colour rather than cycling — see the header.
        .map(({segment, values}, slot): StackEntity => ({
            key: segment.key,
            label: segment.label,
            color: colorAt(palette, slot),
            side: sum(values) >= 0 ? "up" : "down",
            values,
        }));

    return {
        commodity,
        omitted,
        labels: report.buckets.map(bucketLabel),
        entities,
        series: splitBySign(entities),
        net: report.totals.map((total) => amountIn(total, commodity)),
    };
}
