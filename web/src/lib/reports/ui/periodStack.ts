// A `PeriodReport` as a stacked, diverging period chart: the numbers the Net
// Worth and Cash Flow charts draw, decided here so they can be tested by
// calling a function rather than by reading SVG.
//
// Pure — no Svelte, no DOM, no live palette (the caller hands one in).
//
// A chart and the table below it must never disagree, so the chart is built
// from the SAME displayed row list the table renders: `compressPeriodRows` over
// the report's rows (`ReportTable`'s `periodRows`), with the same labels. There
// is no second source of accounts to drift from.
//
// # The rule
//
//   1. PICK ONE COMMODITY. A `MixedAmount` has no single number, and cash flow
//      is not valued, so the chart plots the commodity the report carries most
//      figures in and names the rest (`omitted`) rather than quietly plotting a
//      fraction of the answer — the projections tab's rule, reused. A segment
//      with nothing in that commodity has nothing to draw and is left out.
//   2. SEGMENTS ARE THE DISPLAYED TREE'S PARTS. Every displayed row with no
//      displayed child is a segment carrying that row's values. A displayed
//      parent's figure is its children's plus its own postings, so it is drawn
//      as its children's segments stacked, plus — when the parent's value
//      differs from the sum of its displayed children in any bucket — one OWN
//      segment, `parent − Σ children`, computed on the exact `MixedAmount`s. So
//      each displayed row's value is exactly the sum of the segments under it,
//      and the roots' segments sum to the table's Net.
//   3. LABELS ARE THE TABLE'S. A segment is labelled with its row's displayed
//      text (an own segment with its parent's). Only when two segments would
//      read the same (`cash` under two parents) do both fall back to the full
//      account name, which is what the table's indentation says about them.
//   4. NO TAIL FOLDING. The table's accounts are the chart's accounts. Colours
//      are handed out in table row order: the first `SLOT_COUNT` drawn segments
//      get their own palette slot and every later one gets the muted `other`
//      colour — never a recycled slot, which would make two accounts look alike
//      — while still being its own segment, tooltip row and legend entry. An
//      account's colour is the same in every bucket by construction.
//   5. SIDES. A segment with an engine classification (a net-worth row's
//      `kind`, by effective declared type — never sign or name) is on the side
//      that kind says: assets up, liabilities down; a `mixed` row, or a row
//      with no kind (cash flow), by the sign of its total over the window. When
//      the engine classified the rows, the legend and stack list the up side
//      and then the down side, each in table order; otherwise they are plain
//      table order.
//   6. SPLIT BY SIGN. Every entity becomes a positive series and a negative
//      series (only those that are non-zero somewhere), sharing the entity's key,
//      colour and legend entry. The chart stacks its areas by SERIES, not by
//      point (`PeriodStackChart`'s `bandsAt`: a series with any negative value
//      stacks down in every bucket, zeros included), which is what keeps each
//      layer one unbroken band; a series that changed sign would have its
//      positive buckets stacked below the axis. Halves of one sign cannot, so
//      what is drawn above zero is exactly what was positive: an overdrawn asset
//      is a negative asset segment BELOW the axis, and a liability with a refund
//      on it is a positive segment above it.
//   7. THE NET is the table's Net row (`report.totals`) in the charted commodity.

import {maAdd, maIsZero, maNeg, rankCommodities, type MixedAmount} from "$lib/domain/money";
import {colorAt, type ChartPalette} from "$lib/format/palette";
import {amountIn} from "$lib/projections/projectionView";
import {bucketLabel} from "../periods";
import type {PeriodReport, PeriodRow, PeriodRowKind} from "../types";
import {compressPeriodRows, type DisplayRow} from "./displayRows";

/** Which way from zero an entity stacks. */
export type StackSide = "up" | "down";

/** One non-overlapping part of the displayed tree, in exact amounts. */
export interface StackSegment {
    /** The row's account, or `account (own)` for a parent's own postings. */
    key: string;
    /** The displayed row this segment belongs to. */
    account: string;
    /** True for a parent's own-postings segment. */
    own: boolean;
    /** What the legend and tooltip say: the table's text for the row. */
    label: string;
    /** The engine's side for the row, when it gave one (net worth). */
    kind?: PeriodRowKind;
    /** Exact per-bucket values. */
    values: readonly MixedAmount[];
}

/** One legend entry: an account and its per-bucket signed value. */
export interface StackEntity {
    key: string;
    label: string;
    color: string;
    side: StackSide;
    values: readonly number[];
}

/** One drawn series: one sign of one entity. Halves share `entity`, colour and legend entry. */
export interface StackSeries {
    key: string;
    entity: string;
    label: string;
    color: string;
    values: readonly number[];
}

/** Everything the chart component needs. */
export interface PeriodStack {
    commodity: string;
    /** Other commodities the report holds that the chart does not show, alphabetical. */
    omitted: readonly string[];
    labels: readonly string[];
    /** Legend order: table order, grouped up side then down side when the engine classified the rows. */
    entities: readonly StackEntity[];
    /** Draw order (stacked outward from zero). */
    series: readonly StackSeries[];
    /** The table's Net row, not a sum of the parts. */
    net: readonly number[];
}

/**
 * The commodity to chart and what that leaves out.
 *
 * Counted over every figure the chart could draw (rows and totals), ties broken
 * lexicographically by `rankCommodities`, so the pick is stable across reloads.
 */
export function stackCommodity(report: PeriodReport, fallback: string): {commodity: string; omitted: string[]} {
    const amounts = [...report.totals, ...report.rows.flatMap((row) => row.values)];
    const ranked = rankCommodities(amounts);
    const commodity = ranked[0] ?? fallback;
    return {commodity, omitted: ranked.filter((c) => c !== commodity).sort()};
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
export function stackSegments(display: readonly DisplayRow<PeriodRow>[]): StackSegment[] {
    const children = displayedChildren(display);
    const segments: StackSegment[] = [];
    display.forEach((shown, i) => {
        const {row} = shown;
        const kids = children[i];
        const base = {account: row.account, label: shown.label, ...(row.kind === undefined ? {} : {kind: row.kind})};
        if (kids.length === 0) {
            segments.push({...base, key: row.account, own: false, values: row.values});
            return;
        }
        const own = row.values.map((value, b) => kids.reduce((rest, k) => difference(rest, display[k].row.values[b] ?? new Map()), value));
        if (own.some((value) => !maIsZero(value))) segments.push({...base, key: `${row.account} (own)`, own: true, values: own});
    });

    const counts = new Map<string, number>();
    for (const segment of segments) counts.set(segment.label, (counts.get(segment.label) ?? 0) + 1);
    return segments.map((segment) => ((counts.get(segment.label) ?? 0) > 1 ? {...segment, label: segment.account} : segment));
}

const sum = (values: readonly number[]): number => values.reduce((a, b) => a + b, 0);

function sideOf(kind: PeriodRowKind | undefined, values: readonly number[]): StackSide {
    if (kind === "asset") return "up";
    if (kind === "liability") return "down";
    return sum(values) >= 0 ? "up" : "down";
}

/** Split every entity into its non-zero positive and negative halves. */
export function splitBySign(entities: readonly StackEntity[]): StackSeries[] {
    const halves: StackSeries[] = [];
    for (const entity of entities) {
        for (const [suffix, keep] of [
            ["+", (v: number) => (v > 0 ? v : 0)],
            ["-", (v: number) => (v < 0 ? v : 0)],
        ] as const) {
            const values = entity.values.map(keep);
            if (values.some((v) => v !== 0))
                halves.push({key: `${entity.key}|${suffix}`, entity: entity.key, label: entity.label, color: entity.color, values});
        }
    }
    return halves;
}

/**
 * The whole chart model for one `PeriodReport`.
 *
 * `fallback` names the commodity when the report holds none at all (an empty
 * journal); `palette` is the resolved theme palette (`chartColors.current`).
 */
export function periodStack(report: PeriodReport, palette: ChartPalette, fallback: string): PeriodStack {
    const {commodity, omitted} = stackCommodity(report, fallback);
    const drawn = stackSegments(compressPeriodRows(report.rows))
        .map((segment) => ({segment, values: segment.values.map((value) => amountIn(value, commodity))}))
        .filter(({values}) => values.some((v) => v !== 0));
    // Slots in table order; past `SLOT_COUNT`, `colorAt` gives the `other`
    // colour rather than cycling — see rule 4.
    const inTableOrder = drawn.map(({segment, values}, slot): StackEntity => ({
        key: segment.key,
        label: segment.label,
        color: colorAt(palette, slot),
        side: sideOf(segment.kind, values),
        values,
    }));
    const classified = drawn.some(({segment}) => segment.kind !== undefined);
    const entities = classified ? (["up", "down"] as const).flatMap((side) => inTableOrder.filter((e) => e.side === side)) : inTableOrder;

    return {
        commodity,
        omitted,
        labels: report.buckets.map(bucketLabel),
        entities,
        series: splitBySign(entities),
        net: report.totals.map((total) => amountIn(total, commodity)),
    };
}
