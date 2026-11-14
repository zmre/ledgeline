// Swipe actions on the narrow journal's cards (plan 42 phase 3): the pure
// arithmetic, so the thresholds are tested by calling functions rather than by
// a finger. `swipeAction.svelte.ts` feeds pointer events through these.
//
// A card follows a horizontal drag:
//   - LEFT reveals the trailing actions (Edit, Delete); let go past
//     `openAt` of their width and they stay open, else the card springs back.
//   - RIGHT is the leading action (mark reviewed, else edit); let go past
//     `commitAt` and it runs, else the card springs back.
// Nothing moves until the drag has gone `slop` px and is clearly sideways
// (`axisRatio`); a mostly vertical drag is left to the page's scroll, which
// `touch-action: pan-y` hands to the browser in the first place.

export interface SwipeConfig {
    /** px the pointer must travel before the axis is decided. */
    slop: number;
    /** A drag is sideways when |dx| exceeds |dy| by this factor. */
    axisRatio: number;
    /** px width of the revealed trailing actions. */
    actionsWidth: number;
    /** Fraction of `actionsWidth` past which a released drag leaves them open. */
    openAt: number;
    /** px a rightward drag must reach for the leading action to run. */
    commitAt: number;
    /** px/ms: a flick this fast decides by direction alone. */
    flick: number;
}

/** Defaults, in px at 100 %; the caller scales the widths by `rootPx() / 16`. */
export const SWIPE_DEFAULTS: SwipeConfig = {slop: 10, axisRatio: 1.2, actionsWidth: 144, openAt: 0.4, commitAt: 96, flick: 0.6};

/** Scale the px distances in `config` by `factor` (the interface scale); speeds and ratios stay. */
export function scaledSwipe(factor: number, config: SwipeConfig = SWIPE_DEFAULTS): SwipeConfig {
    const f = Number.isFinite(factor) && factor > 0 ? factor : 1;
    return {...config, slop: config.slop * f, actionsWidth: config.actionsWidth * f, commitAt: config.commitAt * f};
}

export type SwipeAxis = "undecided" | "horizontal" | "vertical";

/** Which way a drag of (dx, dy) from its start is going; undecided inside the slop. */
export function decideAxis(dx: number, dy: number, config: SwipeConfig = SWIPE_DEFAULTS): SwipeAxis {
    const ax = Math.abs(dx);
    const ay = Math.abs(dy);
    if (Math.hypot(ax, ay) < config.slop) return "undecided";
    return ax > ay * config.axisRatio ? "horizontal" : "vertical";
}

/** Past a limit the card moves at a third of the finger's pace (rubber band). */
function rubber(over: number): number {
    return over / 3;
}

/**
 * Where the card sits for a drag of `dx` that began with it at `base` (0
 * closed, `-actionsWidth` open). Free between the two limits, rubber-banded
 * beyond them: left of the actions' width, and right of `commitAt`.
 */
export function dragOffset(base: number, dx: number, config: SwipeConfig = SWIPE_DEFAULTS): number {
    const raw = base + dx;
    const min = -config.actionsWidth;
    const max = config.commitAt;
    if (raw < min) return min + rubber(raw - min);
    if (raw > max) return max + rubber(raw - max);
    return raw;
}

export type SwipeEnd = "close" | "open" | "commit";

/**
 * What a released card does: from `offset` (where it was let go) moving at
 * `velocity` px/ms (negative = leftward).
 */
export function settle(offset: number, velocity: number, config: SwipeConfig = SWIPE_DEFAULTS): SwipeEnd {
    if (offset >= config.commitAt) return "commit";
    if (offset > 0) return "close";
    if (velocity <= -config.flick) return "open";
    if (velocity >= config.flick) return "close";
    return -offset >= config.actionsWidth * config.openAt ? "open" : "close";
}

/** The offset a settled card rests at. A committed card springs back: its action opens a sheet or saves. */
export function restingOffset(end: SwipeEnd, config: SwipeConfig = SWIPE_DEFAULTS): number {
    return end === "open" ? -config.actionsWidth : 0;
}

/** Whether moving from `before` to `after` crossed the commit line (either way): the haptic tick. */
export function crossedCommit(before: number, after: number, config: SwipeConfig = SWIPE_DEFAULTS): boolean {
    return before >= config.commitAt !== after >= config.commitAt;
}

/** ms a finger must rest on a card, unmoved past the slop, to open its menu. */
export const LONG_PRESS_MS = 500;

/** One swipe action: the leading one, or a revealed trailing button. */
export interface SwipeAction {
    label: string;
    tone: "success" | "info" | "error" | "neutral";
    run: () => void;
    testid?: string;
}
