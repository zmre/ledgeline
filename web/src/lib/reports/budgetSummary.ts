// Budget bars — presentation only (plan 26 B6).
//
// The engine computes the span summary the Budget tab draws
// (`crates/ledgeline-core/src/reports/budget_summary.rs`, served as
// `BudgetReport.summary`): one {actual, goal} pair per leaf budget over the
// whole span, in magnitudes, with its pace, health and label. What is left here
// is the part that is about pixels: turning those exact amounts into bar
// geometry. Pure TS (no Svelte/DOM), so it is unit-testable under node.
//
// Exact-money discipline: `toNumber()` is used ONLY to size the bars, a display
// concern the money module explicitly sanctions at chart boundaries. Nothing
// here decides health or wording — the engine already has, exactly.

import {toNumber, type MixedAmount} from "../domain/money";

/** Single-commodity numeric magnitude for bar geometry; 0 for empty, null for multi-commodity (no single bar). */
export function primaryValue(ma: MixedAmount): number | null {
    if (ma.size === 0) return 0;
    if (ma.size > 1) return null;
    const [qty] = ma.values();
    return toNumber(qty);
}

/** Bullet-bar geometry: a single-colour fill, the goal marker, and the quieter pace tick. */
export interface BarGeometry {
    /** 0..100 — width of the fill. ONE colour; the bar's `health` says which. */
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
 * now one decision, the engine's `health`, taken from the same three numbers.
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

/**
 * Where the expense ramp reaches amber, as a fraction of the way to the goal
 * marker: a line three-quarters spent is the one worth a second look.
 */
export const EXPENSE_AMBER_AT = 0.75;

/** Round a stop position so the CSS is stable (and readable in a test). */
const stop = (pct: number): string => `${Math.round(Math.min(100, Math.max(0, pct)) * 100) / 100}%`;

/**
 * The wide table's bar colour (plan 41 WP-B): a gradient FIXED TO THE TRACK,
 * which the fill only reveals.
 *
 * The stops are laid out in track coordinates, keyed to the goal marker, so a
 * bar's colour says how far along it is the way a fuel gauge does: an expense
 * at 40 % of its goal shows only green, at 85 % it has reached amber, at 100 %
 * it ends in orange, and anything past the marker is red. Stretching the
 * gradient over the FILL instead would paint every bar the same green-to-orange
 * sweep whatever its value, which is decoration rather than information.
 *
 * - Expense: success → warning (at {@link EXPENSE_AMBER_AT} of the goal) →
 *   orange (at the goal) → a hard step to error past it.
 * - Revenue: the opposite meaning, so a calm info → success ramp with NO red:
 *   earning more than the target must not read as overspend (plans/20).
 *
 * A goal of zero has no ramp to speak of (the marker sits at the end of the
 * track): any spending against it is all overspend, and any income all win.
 *
 * Returns a CSS `background-image`. Colours are theme tokens only — the
 * `--color-orange` one is derived in `app.css` so every theme has it.
 */
export function barGradient(kind: "revenue" | "expense", geom: BarGeometry): string {
    const marker = geom.markerPct;
    if (kind === "revenue") {
        if (geom.ratio === null) return "linear-gradient(to right, var(--color-success), var(--color-success))";
        return `linear-gradient(to right, var(--color-info) 0%, var(--color-success) ${stop(marker)}, var(--color-success) 100%)`;
    }
    if (geom.ratio === null) return "linear-gradient(to right, var(--color-error), var(--color-error))";
    return (
        "linear-gradient(to right, var(--color-success) 0%, " +
        `var(--color-warning) ${stop(marker * EXPENSE_AMBER_AT)}, ` +
        `var(--color-orange) ${stop(marker)}, ` +
        `var(--color-error) ${stop(marker)}, var(--color-error) 100%)`
    );
}

/**
 * The CSS that reveals `fillPct` of a full-track layer: `clip-path` rather than
 * a narrower element, so the gradient underneath keeps the TRACK's width (see
 * {@link barGradient}) without a `background-size` that divides by the fill —
 * which a zero-width fill would turn into infinity.
 */
export function fillClip(fillPct: number): string {
    return `inset(0 ${stop(100 - fillPct)} 0 0)`;
}
