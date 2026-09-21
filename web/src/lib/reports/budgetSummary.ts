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

import {dec, maAdd, maNeg, maScale, toNumber, type MixedAmount} from "../domain/money";
import {daysBetween} from "./periods";
import type {ISODate} from "../domain/types";
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

// ---------------------------------------------------------------------------
// Pace — where a line SHOULD be by now
// ---------------------------------------------------------------------------
//
// `summarizeBudget` sums a goal across every bucket in the span while the actual
// can only run to today, so in June a year-to-date view reports every expense as
// half-spent and every income line as half-earned. Judging a bar against the
// whole period's goal therefore reads a mid-year income line as a shortfall and
// a mid-year expense line as a large, reassuring underspend — the same lie in
// two directions.
//
// The fix is one number: the fraction of the span that has elapsed. Prorating by
// DAYS rather than by completed buckets is what the ask describes ("month
// six … would be half"), and it is correct for a `~ yearly` rule (whose whole
// figure lands in January's bucket) and a `~ monthly` rule × 12 alike, because
// `summarizeBudget` has already folded both into one span total. Counting
// buckets instead would step the mark on the 1st of each month and flip a bar's
// colour for reasons that have nothing to do with money.

/**
 * Fraction of the inclusive span `[from, to]` that has elapsed as of `asOf`,
 * clamped to [0, 1].
 *
 * 1 for a span that has already ended: looking at last year, the pace mark sits
 * exactly on the goal marker and every bar means what it means today. 0 for a
 * span that has not started. A single-day span is 1 on its own day.
 *
 * `asOf` is a PARAMETER, never a clock read — this module is purity-guarded and
 * ports to Rust, where `periods.rs` deliberately omits `today()`.
 */
export function elapsedFraction(from: ISODate, to: ISODate, asOf: ISODate): number {
    const span = daysBetween(from, to) + 1;
    if (span <= 0) return 1; // a malformed (inverted) span is not a span to pace against
    const done = daysBetween(from, asOf) + 1;
    if (done <= 0) return 0;
    return done >= span ? 1 : done / span;
}

/**
 * How many fractional digits a pace fraction is carried at before it multiplies
 * a goal. Six is far past what a day-count ratio can resolve (a 366-day span
 * steps by 0.0027) and keeps the intermediate mantissa small.
 */
const PACE_PLACES = 6;

/**
 * The goal prorated to `fraction` — where a line should be by now.
 *
 * Exact, like every other number here: the float fraction is snapped to a `Dec`
 * once and then multiplied into each commodity, rather than converting money to
 * a float and back. `fraction` outside [0, 1] is clamped and a non-finite one is
 * read as a finished span, so this is total for any input `elapsedFraction`
 * could not have produced.
 */
export function paceAmount(goal: MixedAmount, fraction: number): MixedAmount {
    const bounded = Number.isFinite(fraction) ? Math.min(Math.max(fraction, 0), 1) : 1;
    return maScale(goal, dec(BigInt(Math.round(bounded * 10 ** PACE_PLACES)), PACE_PLACES));
}

/** How a bar is doing. `behind` is the unhealthy side of pace in either column. */
export type BudgetHealth = "healthy" | "behind" | "over";

/**
 * Healthy iff the actual is on the right side of PACE — at or above it for
 * revenue, at or below it for expense. One sentence covering both columns is the
 * only way a two-section screen stays legible, and it is what stops "earned more
 * than target" rendering as a red overspend.
 *
 * `over` is the expense-only third state: past the GOAL, not merely past pace.
 * It is the one an envelope user must not miss, so it stays distinguishable even
 * though it paints the same colour as `behind`. Revenue past its goal is simply
 * `healthy` — there is no such thing as earning too much.
 *
 * All three arguments are MAGNITUDES (see `magnitudeAmount`), so revenue's
 * credit-normal sign never reaches this comparison.
 */
export function budgetHealth(actual: number, pace: number, goal: number, income: boolean): BudgetHealth {
    if (income) return actual >= pace ? "healthy" : "behind";
    if (actual > goal) return "over";
    return actual <= pace ? "healthy" : "behind";
}

/** Bullet-bar geometry: a single-colour fill, the goal marker, and the quieter pace tick. */
export interface BarGeometry {
    /** 0..100 — width of the fill. ONE colour; `budgetHealth` says which. */
    fillPct: number;
    /** 0..100 — position of the goal marker (the high-contrast line). */
    markerPct: number;
    /** 0..100 — position of the pace mark, or null when it coincides with the goal marker. */
    pacePct: number | null;
    /** spent / budget as a fraction (e.g. 0.88), or null when there is no positive budget. */
    ratio: number | null;
}

/**
 * Below this many percentage points apart, the pace tick and the goal marker
 * would render as one thicker line — so the pace tick is dropped instead of
 * drawn on top of the marker it is meant to be distinguished from. Half a
 * percent of a 300px track is under two pixels.
 */
const TICK_EPSILON_PCT = 0.5;

/**
 * Scale the track to `max(spent, budget*1.25)` so an under-budget bar leaves ~20%
 * headroom past the marker (overspend is always visible) and an over-budget bar
 * saturates the fill while the marker slides left of the fill's end.
 *
 * The fill is no longer split at the marker. That split is exactly what made an
 * income line "earning more than target" paint red past its goal; the colour is
 * now one decision, taken by `budgetHealth` from the same three numbers.
 */
export function barGeometry(spent: number, budget: number, pace: number): BarGeometry {
    const s = Math.abs(spent);
    const b = Math.abs(budget);
    const p = Math.abs(pace);
    if (b === 0) {
        return {fillPct: s > 0 ? 100 : 0, markerPct: 100, pacePct: null, ratio: null};
    }
    const scaleMax = Math.max(s, b * 1.25);
    const markerPct = (b / scaleMax) * 100;
    const pacePct = (p / scaleMax) * 100;
    return {
        fillPct: (s / scaleMax) * 100,
        markerPct,
        pacePct: Math.abs(pacePct - markerPct) < TICK_EPSILON_PCT ? null : pacePct,
        ratio: s / b,
    };
}
