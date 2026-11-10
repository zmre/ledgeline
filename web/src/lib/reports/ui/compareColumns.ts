// Compare vs actuals (plans/34-projection-backtest.md, D5): the two figure
// columns that replace a statement's Total when a projection is compared with
// what happened.
//
// A compared statement holds 2N columns: the N actual periods, then the same N
// periods as the scenario on screen would have projected them on their first
// day (`split` = N). Each row's window figure is read the way the row reads —
// a FLOW sums its block, a BALANCE takes its block's last column, an OPENING
// figure (the cash flow statement's beginning cash) its first — and then:
//
//   Δ  = actual − projected, per commodity, exact;
//   Δ% = Δ ÷ |projected| × 100, one decimal, half away from zero, in the base
//        commodity, or null.
//
// Computed here rather than in the engine for the reason the income
// statement's % change column is: every input is on the row, so a Δ can never
// disagree with the cells beside it, and none of the four statement wire types
// changes. Kept to one small pure module so plan 26 can port it.
//
// "Difference", never "miss" or "error": coming in above the projection is as
// worth reading as coming in below it, so nothing here (or in the cells that
// show it) is coloured by direction.

import {maAdd, maIsZero, maNeg, type MixedAmount} from "#lib/domain/money.js";
import {fmtPct} from "./incomeStatementRows";
import {pctChange} from "./percent";

/** How a row's window figure is read off its block. */
export type WindowReading = "flow" | "balance" | "opening";

/** One row's comparison. */
export interface CompareCells {
    /** The actual block's window figure. */
    actual: MixedAmount;
    /** The as-projected block's window figure. */
    projected: MixedAmount;
    /** `actual − projected`, per commodity. */
    delta: MixedAmount;
    /** `delta ÷ |projected|` as a percentage to one decimal, or null (D5). */
    pct: number | null;
}

/** A block's window figure: the sum, the last column or the first. */
export function windowFigure(block: readonly MixedAmount[], reading: WindowReading): MixedAmount {
    switch (reading) {
        case "flow":
            return block.reduce<MixedAmount>((sum, value) => maAdd(sum, value), new Map());
        case "balance":
            return block.at(-1) ?? new Map();
        case "opening":
            return block[0] ?? new Map();
    }
}

/** Whether `ma` holds anything but `base` (a commodity no price converted). */
function holdsOther(ma: MixedAmount, base: string): boolean {
    for (const [commodity, quantity] of ma) {
        if (commodity !== base && quantity.m !== 0n) return true;
    }
    return false;
}

/**
 * The comparison of one row whose 2N `values` are `split` actual columns then
 * the as-projected ones.
 *
 * Δ% is null when the projected figure is zero, absent or rounds to zero at
 * `precision` (the base commodity's display places); when either figure holds
 * a commodity other than `base`, because a ratio of the convertible part alone
 * would mislead; and when there is no base commodity.
 */
export function compareCells(values: readonly MixedAmount[], split: number, reading: WindowReading, base: string | null, precision?: number): CompareCells {
    const actual = windowFigure(values.slice(0, split), reading);
    const projected = windowFigure(values.slice(split), reading);
    const delta = maAdd(actual, maNeg(projected));
    const mixed = base === null || holdsOther(actual, base) || holdsOther(projected, base);
    return {actual, projected, delta, pct: mixed ? null : pctChange(actual, projected, base, precision)};
}

/** Whether a row's Δ is zero in every commodity. */
export function deltaIsZero(cells: CompareCells): boolean {
    return maIsZero(cells.delta);
}

/** The largest |Δ%| printed as a number; beyond it the cell reads `>999%` or `<-999%` and the exact figure is in its title. */
export const PCT_DISPLAY_CAP = 1000;

/** Δ% for display: one decimal, `—` for null, capped at ±999% (D5). */
export function fmtComparePct(pct: number | null): string {
    if (pct !== null && pct >= PCT_DISPLAY_CAP) return ">999%";
    if (pct !== null && pct <= -PCT_DISPLAY_CAP) return "<-999%";
    return fmtPct(pct);
}

/** The exact Δ% for a cell's `title`, uncapped; empty when there is none. */
export function comparePctTitle(pct: number | null): string {
    return pct === null ? "" : fmtPct(pct);
}

/**
 * Screen-reader text after each as-projected column heading: the visible
 * labels repeat ("Jul 2026" twice), so a heard one says which block it is. A
 * constant, not a template literal, because Svelte trims a literal leading
 * space inside an element.
 */
export const AS_PROJECTED_SR = " as projected";

/** The band's middle heading: "As projected from 1 Jul 2026". */
export function compareBandLabel(fromLabel: string): string {
    return `As projected from ${fromLabel}`;
}
