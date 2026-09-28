// A `PeriodReport` as a stacked, diverging period chart: the numbers the Net
// Worth chart draws, decided here so they can be tested by calling a function
// rather than by reading SVG. The Cash Flow chart shares the shape
// (`PeriodStack`), the commodity pick and the sign split, but takes its parts
// from the table's displayed rows and folds nothing — see `cashFlowStack`.
//
// Pure — no Svelte, no DOM, no live palette (the caller hands one in).
//
// # The pipeline
//
//   1. PICK ONE COMMODITY. A `MixedAmount` has no single number, and cash flow
//      is not valued, so the chart plots the commodity the report carries most
//      figures in and names the rest (`omitted`) rather than quietly plotting a
//      fraction of the answer — the projections tab's rule, reused.
//   2. PARTS, NOT ROWS. A depth-clamped report lists a parent AND its children,
//      so stacking every row counts `assets:bank:checking` twice (once under
//      `assets:bank`). The stack is the LEAVES of the clamped tree, plus one
//      RESIDUAL part per parent whose own postings are not in any child row —
//      `parent − Σ children`, computed on the exact `MixedAmount`s, so a residual
//      is either exactly zero and dropped or real and drawn.
//   3. SIDES. Each part is assigned the side it stacks on: a net-worth row by the
//      ENGINE's classification (`kind`, by effective declared type — never sign
//      or name); anything else (a `mixed` row) by the sign of its total over the
//      window.
//   4. FOLD THE TAIL, PER SIDE. At most `MAX_NAMED` parts keep their names; the
//      rest of each side sums into one `OTHER_LABEL` entity in the muted tail
//      colour. Each side is guaranteed a couple of names so a large asset base
//      cannot crowd every liability into "(other)". A tail of ONE is not folded:
//      "(other)" standing for a single account just hides its name.
//   5. COLOUR BY ENTITY. Slots are handed out once, in the fixed legend order, so
//      an account is the same colour in every bucket by construction.
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

import {maAdd, maIsZero, maNeg, rankCommodities, type MixedAmount} from "$lib/domain/money";
import {colorAt, OTHER_LABEL, type ChartPalette} from "$lib/format/palette";
import {amountIn} from "$lib/projections/projectionView";
import {bucketLabel} from "../periods";
import type {PeriodReport, PeriodRow, PeriodRowKind} from "../types";

/** Which way from zero an entity stacks. */
export type StackSide = "up" | "down";

/** Named parts kept before the tail folds; ~6 named + "(other)" is legible at 375px. */
export const MAX_NAMED = 6;

/** Names each side keeps before the rest compete on magnitude, so one side cannot take every slot. */
const MIN_NAMED_PER_SIDE = 2;

/** A part of the stack before folding: a leaf row, or a parent's own residual. */
export interface StackPart {
    /** Stable identity: the account, or `account (own)` for a residual. */
    key: string;
    label: string;
    /** The engine's side, when it gave one (net worth). */
    kind?: PeriodRowKind;
    /** Exact per-bucket values. */
    values: readonly MixedAmount[];
}

/** One legend entry: an account (or folded tail) and its per-bucket signed value. */
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
    /**
     * Legend order. `periodStack`: the up side, then the down side, each largest
     * first, any "(other)" last on its side. `cashFlowStack`: the table's row order.
     */
    entities: readonly StackEntity[];
    /** Draw order (stacked outward from zero). */
    series: readonly StackSeries[];
    /** The engine's own per-bucket total, not a sum of the parts. */
    net: readonly number[];
}

/** The folded tail's key on one side. */
export const otherKey = (side: StackSide): string => `${OTHER_LABEL}:${side}`;

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

const parentOf = (account: string): string | null => {
    const cut = account.lastIndexOf(":");
    return cut < 0 ? null : account.slice(0, cut);
};

/**
 * The non-overlapping parts of a depth-clamped tree: every leaf row, plus a
 * residual for every parent whose value is not fully explained by its child rows
 * (its own postings, or children below the clamp that were not rows).
 *
 * A child is a row whose account's parent IS the parent's account; the rows are
 * the union across buckets, so "has a child row" is decided once for the whole
 * window and a bucket where the child happens to be zero still subtracts zero.
 */
export function stackParts(rows: readonly PeriodRow[]): StackPart[] {
    const children = new Map<string, PeriodRow[]>();
    const present = new Set(rows.map((row) => row.account));
    for (const row of rows) {
        const parent = parentOf(row.account);
        if (parent !== null && present.has(parent)) children.set(parent, [...(children.get(parent) ?? []), row]);
    }
    const parts: StackPart[] = [];
    for (const row of rows) {
        const kids = children.get(row.account);
        if (kids === undefined) {
            parts.push({key: row.account, label: row.account, kind: row.kind, values: row.values});
            continue;
        }
        const residual = row.values.map((value, i) => kids.reduce((rest, kid) => maAdd(rest, maNeg(kid.values[i] ?? new Map())), value));
        if (residual.some((value) => !maIsZero(value))) {
            parts.push({key: `${row.account} (own)`, label: `${row.account} (own)`, kind: row.kind, values: residual});
        }
    }
    return parts;
}

interface Measured {
    key: string;
    label: string;
    side: StackSide;
    values: number[];
    magnitude: number;
}

const sum = (values: readonly number[]): number => values.reduce((a, b) => a + b, 0);

function sideOf(kind: PeriodRowKind | undefined, values: readonly number[]): StackSide {
    if (kind === "asset") return "up";
    if (kind === "liability") return "down";
    return sum(values) >= 0 ? "up" : "down";
}

/** Largest first; ties by key so the order never depends on input order. */
const byMagnitude = (a: Measured, b: Measured): number => b.magnitude - a.magnitude || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);

/**
 * Which parts keep their names: each side's `MIN_NAMED_PER_SIDE` largest, then
 * the largest of the rest up to `MAX_NAMED` — and then any side whose tail is a
 * single part keeps that one too (so never more than `MAX_NAMED + 2`, within
 * `SLOT_COUNT`).
 */
function namedKeys(measured: readonly Measured[]): Set<string> {
    const ranked = [...measured].sort(byMagnitude);
    const named = new Set<string>();
    for (const side of ["up", "down"] as const) {
        for (const part of ranked.filter((p) => p.side === side).slice(0, MIN_NAMED_PER_SIDE)) named.add(part.key);
    }
    for (const part of ranked) {
        if (named.size >= MAX_NAMED) break;
        named.add(part.key);
    }
    for (const side of ["up", "down"] as const) {
        const tail = ranked.filter((p) => p.side === side && !named.has(p.key));
        if (tail.length === 1) named.add(tail[0].key);
    }
    return named;
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
    const measured: Measured[] = stackParts(report.rows)
        .map((part) => {
            const values = part.values.map((value) => amountIn(value, commodity));
            return {key: part.key, label: part.label, side: sideOf(part.kind, values), values, magnitude: sum(values.map(Math.abs))};
        })
        .filter((part) => part.magnitude > 0);

    const named = namedKeys(measured);
    const entities: StackEntity[] = [];
    let slot = 0;
    for (const side of ["up", "down"] as const) {
        const onSide = measured.filter((p) => p.side === side).sort(byMagnitude);
        for (const part of onSide.filter((p) => named.has(p.key))) {
            // `namedKeys` keeps at most MAX_NAMED + 2 = SLOT_COUNT names, and
            // `colorAt` folds to the tail colour rather than cycling if it did not.
            entities.push({key: part.key, label: part.label, color: colorAt(palette, slot++), side, values: part.values});
        }
        const tail = onSide.filter((p) => !named.has(p.key));
        if (tail.length > 0) {
            const values = report.buckets.map((_, i) => sum(tail.map((p) => p.values[i] ?? 0)));
            entities.push({key: otherKey(side), label: OTHER_LABEL, color: palette.other, side, values});
        }
    }

    return {
        commodity,
        omitted,
        labels: report.buckets.map(bucketLabel),
        entities,
        series: splitBySign(entities),
        net: report.totals.map((total) => amountIn(total, commodity)),
    };
}
