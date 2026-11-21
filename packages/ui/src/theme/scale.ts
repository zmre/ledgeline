// Interface scale (plan 39, "Scale"): one number, a whole percent, that every
// text size and every piece of text-bearing geometry derives from.
//
// The mechanism is the root font size. Tailwind and daisyUI size everything in
// rem, so setting `html { font-size: 125% }` scales the whole stylesheet at
// once; arbitrary px text was converted to rem so nothing is left behind. JS
// that has to know a rendered size before the browser lays it out (the
// journal's virtualized row pitch, the chip text measurer, chart paddings)
// writes that size in rem and converts it with `rootPx()`, the live px per rem,
// which `applyScale` keeps current.

import {BASE_ROOT_PX, measureRootPx, rootPx, setRootPx} from "./scale.svelte";

export {BASE_ROOT_PX, rootPx};

/** The scale steps the UI moves through, in whole percent. The server accepts 50–300 from a hand edit; the UI snaps to these. */
export const SCALE_STEPS: readonly number[] = [70, 80, 90, 100, 110, 125, 150, 175, 200];

/** Actual size. */
export const DEFAULT_SCALE = 100;

export type ZoomDirection = "in" | "out";

/**
 * The next step from `current` in `dir`.
 *
 * An off-step value (133, from a hand-edited prefs file) snaps to the nearest
 * step in the direction asked — in from 133 is 150, out is 125 — so one press
 * always moves the way the user asked and lands on a step. Clamps at both ends:
 * zooming in at 200 stays 200. A non-finite `current` is treated as actual size.
 */
export function stepScale(current: number, dir: ZoomDirection): number {
    const from = Number.isFinite(current) ? current : DEFAULT_SCALE;
    if (dir === "in") return SCALE_STEPS.find((step) => step > from) ?? SCALE_STEPS[SCALE_STEPS.length - 1];
    return SCALE_STEPS.findLast((step) => step < from) ?? SCALE_STEPS[0];
}

/** `rem` in CSS px at a root of `root` px (default: the live root). */
export function remToPx(rem: number, root: number = rootPx()): number {
    return rem * root;
}

/**
 * Scale the interface to `percent`.
 *
 * Sets `--ui-scale` (1.25 for 125) for any stylesheet rule that wants the
 * factor, and the root's font size INLINE as a percentage, so every rem
 * follows whether or not a stylesheet rule reads the variable. A percentage
 * rather than px keeps the user's own browser default size in play. Then
 * publishes the new px per rem through `rootPx()`.
 */
export function applyScale(percent: number, root: HTMLElement = document.documentElement): void {
    root.style.setProperty("--ui-scale", String(percent / 100));
    root.style.fontSize = `${percent}%`;
    setRootPx(measureRootPx(root) ?? (BASE_ROOT_PX * percent) / 100);
}
