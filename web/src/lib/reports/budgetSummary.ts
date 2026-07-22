// Budget summary (period-total view). The engine returns per-bucket cells; the
// primary UI aggregates them into ONE {actual, goal} pair per account over the
// selected span — a $200/mo goal across 7 months reads as $1,400 budgeted to
// date — rendered as bullet bars (à la Goodbudget / YNAB envelopes). Pure TS
// (no Svelte/DOM, relative imports only — this ports to Rust later), so it's
// unit-testable under node.
//
// Exact-money discipline: amounts stay MixedAmount for every number the user
// reads; toNumber() is used ONLY to size the bars, a display concern the money
// module explicitly sanctions at chart boundaries.

import {maAdd, maNeg, toNumber, type MixedAmount} from "../domain/money";
import type {BudgetReport} from "./types";

/** The synthetic catch-all account for actuals with no matching goal (matches the engine). */
export const UNBUDGETED = "<unbudgeted>";

/** One account's period totals: actual summed across buckets, and goal summed (null ⇒ unbudgeted). */
export interface BudgetLine {
    account: string;
    /** Number of `:`-separated segments in `account`. */
    depth: number;
    actual: MixedAmount;
    /** Summed goal, or null when the account has no goal in any bucket (`<unbudgeted>` / non-budgeted). */
    goal: MixedAmount | null;
}

/** Sum each row's per-bucket cells into one period total. A row is "budgeted" (goal !== null) iff any cell had a goal. */
export function summarizeBudget(report: BudgetReport): BudgetLine[] {
    return report.rows.map((row) => {
        let actual: MixedAmount = new Map();
        let goal: MixedAmount | null = null;
        for (const cell of row.cells) {
            actual = maAdd(actual, cell.actual);
            if (cell.goal !== null) goal = maAdd(goal ?? new Map(), cell.goal);
        }
        return {account: row.account, depth: row.depth, actual, goal};
    });
}

/**
 * The leaf-most budgeted rows: hide an aggregate PARENT row when a deeper
 * budgeted row is also present. The engine reproduces hledger's tree budget,
 * which emits both `expenses` (the depth-clamped parent aggregate) and its
 * budgeted children — redundant in a flat bar list, where the parent's bar just
 * re-sums its children. Standalone budgets (no budgeted descendant) stay at
 * whatever depth. Unbudgeted rows (goal === null) are excluded (shown separately).
 */
export function budgetLeaves(lines: readonly BudgetLine[]): BudgetLine[] {
    const budgeted = lines.filter((l) => l.goal !== null);
    return budgeted.filter((leaf) => !budgeted.some((other) => other.account.startsWith(`${leaf.account}:`)));
}

/** True when any strict ancestor of `account` is itself a budgeted account (to avoid double-counting nested budgets). */
function hasBudgetedAncestor(account: string, budgeted: ReadonlySet<string>): boolean {
    let acct = account;
    for (;;) {
        const sep = acct.lastIndexOf(":");
        if (sep === -1) return false;
        acct = acct.slice(0, sep);
        if (budgeted.has(acct)) return true;
    }
}

/**
 * Overall "spent of budgeted" for the period: sums the actual and goal of the
 * TOP-LEVEL budgeted accounts only. Nested budgets (a budgeted child under a
 * budgeted parent) are inclusive, so counting both would double-count — we keep
 * the outermost. Unbudgeted rows (goal === null) are excluded from the budget bar.
 */
export function budgetTotals(lines: readonly BudgetLine[]): {actual: MixedAmount; goal: MixedAmount} {
    const budgeted = new Set(lines.filter((l) => l.goal !== null).map((l) => l.account));
    let actual: MixedAmount = new Map();
    let goal: MixedAmount = new Map();
    for (const line of lines) {
        if (line.goal === null || hasBudgetedAncestor(line.account, budgeted)) continue;
        actual = maAdd(actual, line.actual);
        goal = maAdd(goal, line.goal);
    }
    return {actual, goal};
}

/**
 * |ma| by the primary-commodity sign: flip a credit-normal (negative) amount to
 * its magnitude so income budgets (entered NEGATIVE per hledger's convention)
 * read as "earned $X of $Y". A no-op for positive (expense) and multi-commodity
 * amounts.
 */
export function magnitudeAmount(ma: MixedAmount): MixedAmount {
    const v = primaryValue(ma);
    return v !== null && v < 0 ? maNeg(ma) : ma;
}

/** Single-commodity numeric magnitude for bar geometry; 0 for empty, null for multi-commodity (no single bar). */
export function primaryValue(ma: MixedAmount): number | null {
    if (ma.size === 0) return 0;
    if (ma.size > 1) return null;
    const [qty] = ma.values();
    return toNumber(qty);
}

/** Bullet-bar geometry: fill split into an at/under-budget part and an over-budget part, plus the goal marker. */
export interface BarGeometry {
    /** 0..100 — width of the fill up to the goal marker. */
    underPct: number;
    /** 0..100 — width of the fill PAST the goal marker (the overspend, shown red). */
    overPct: number;
    /** 0..100 — position of the goal marker. */
    markerPct: number;
    /** spent / budget as a fraction (e.g. 0.88), or null when there is no positive budget. */
    ratio: number | null;
    over: boolean;
}

/**
 * Scale the track to `max(spent, budget*1.25)` so an under-budget bar leaves ~20%
 * headroom past the marker (overspend is always visible) and an over-budget bar
 * saturates the fill while the marker slides left of the fill's end.
 */
export function barGeometry(spent: number, budget: number): BarGeometry {
    const s = Math.abs(spent);
    const b = Math.abs(budget);
    if (b === 0) {
        return {underPct: s > 0 ? 100 : 0, overPct: 0, markerPct: 100, ratio: null, over: s > 0};
    }
    const scaleMax = Math.max(s, b * 1.25);
    const fillPct = (s / scaleMax) * 100;
    const markerPct = (b / scaleMax) * 100;
    return {underPct: Math.min(fillPct, markerPct), overPct: Math.max(0, fillPct - markerPct), markerPct, ratio: s / b, over: s > b};
}
