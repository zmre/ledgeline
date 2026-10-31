// The review overlay's motion (plan 32 D29, reworked as a card deck): cards
// knocked off a pile. The top card leaves the viewport entirely; the card
// waiting beneath it (the "peek") rises into its place at the same time.
//
// Pure. Every function is a Svelte transition (`(node, params) =>
// TransitionConfig`) or a style string whose output depends only on its params.
// The overlay passes in the moment (`exit`) and the reduced-motion preference,
// so the durations, easings and frames are unit-tested without a DOM (`node` is
// never read).
//
// # Easing lives inside `css`
//
// Every config leaves Svelte's `easing` at linear and eases inside `css`. That
// lets one animation hold still for a while and then move. The Reviewed exit
// holds while the ✓ stamp pops, then flies. It is one animation rather than a
// Svelte `delay`, because `finishMotion` must be able to end it in one call. A
// delayed Svelte transition is two animations, and finishing the first one only
// starts the second.
//
// # Non-blocking
//
// That part is structural, not something this file guarantees alone. The next
// card is mounted the moment the store advances (`{#key}` does not wait for
// the outro), and Svelte marks the outgoing card `inert`. A keypress
// mid-animation calls `finishMotion`, which ends every running animation at
// its final frame before the key acts.

import {backOut, cubicOut, quadIn} from "svelte/easing";
import type {TransitionConfig} from "svelte/transition";

/** How a card leaves the top of the deck, which also decides how the next one arrives. */
export type CardExit = "reviewed" | "saved" | "skip" | "back" | "ignore";

/**
 * The moment the next card transition belongs to: a user's exit, the run
 * opening (nothing has left yet), or the overlay closing (the layer fades as a
 * whole, so the cards do nothing of their own).
 */
export type DeckMoment = CardExit | "open" | "close";

export interface MotionParams {
    exit: DeckMoment;
    /** `prefers-reduced-motion: reduce`: no travel, a short cross-fade. */
    reduced: boolean;
}

/** Every duration, in ms. */
export const DURATIONS = {
    /** The first card rises in as the run opens. */
    open: 180,
    /** The top bar drops in with it. */
    bar: 180,
    /** The peek appears this long after the opening rise starts. */
    peekBeat: 120,
    /** The whole layer fades and drops. */
    close: 160,
    /** The ✓ stamp pops before a Reviewed card moves. */
    stamp: 90,
    /** Flight times, after any stamp hold. */
    reviewed: 280,
    saved: 280,
    skip: 260,
    ignore: 300,
    /** ← : the previous card comes back in from the left. */
    back: 280,
    /** ← : the card that was on top settles back into the peek slot. */
    settle: 220,
    /** The peek rises to the top. */
    rise: 220,
    /** A peek fading in or out, and its move between slots. */
    peek: 220,
    footer: 80,
    similar: 120,
    counter: 300,
    /** The reduced-motion cross-fade, standing in for every move above. */
    reduced: 120,
} as const;

/** One pose in the pile: how far down, how small and how dim a card sits. */
export interface Pose {
    y: number;
    scale: number;
    dim: number;
}

/** The top of the deck. */
export const TOP: Pose = {y: 0, scale: 1, dim: 1};

/**
 * The cards waiting beneath it, by depth (1 = the next card). Each one is
 * pinned at its bottom edge (`transform-origin: bottom`), so the shrink and the
 * drop together leave a strip showing below the card above.
 *
 * The cards are dimmed with `brightness`, not `opacity`: a translucent card
 * would show the hero of the one behind it through its own.
 */
export const PEEKS: readonly Pose[] = [
    {y: 14, scale: 0.95, dim: 0.6},
    {y: 28, scale: 0.9, dim: 0.38},
];

/** How many cards wait beneath the top one. */
export const PEEK_DEPTH = PEEKS.length;

/** Where each exit sends the outgoing card: off the viewport, in viewport units. */
const FLIGHTS: Record<"reviewed" | "saved" | "skip" | "ignore", {x: number; y: number; deg: number}> = {
    // Right and slightly up.
    reviewed: {x: 120, y: -8, deg: 12},
    saved: {x: 120, y: -8, deg: 12},
    // Left.
    skip: {x: -120, y: 0, deg: -8},
    // Straight down. quadIn is constant acceleration, so it reads as falling.
    ignore: {x: 0, y: 105, deg: 4},
};

/** `cubicOut` as CSS, for the peeks' slot-to-slot moves (a CSS transition, not a Svelte one). */
const CUBIC_OUT_CSS = "cubic-bezier(0.215, 0.61, 0.355, 1)";

const lerp = (from: number, to: number, q: number): number => from + (to - from) * q;
/** Round to 4 places: the frames stay readable, and a test can compare them as strings. */
const r = (n: number): number => Math.round(n * 1e4) / 1e4;

/** The CSS for a pose part-way (`q`) from `from` to `to`. */
export function poseCss(from: Pose, to: Pose, q: number): string {
    return `transform: translateY(${r(lerp(from.y, to.y, q))}px) scale(${r(lerp(from.scale, to.scale, q))}); filter: brightness(${r(lerp(from.dim, to.dim, q))})`;
}

/**
 * Progress `p` (0–1, linear) re-mapped so that the first `hold` ms of a
 * `total` ms animation stand still and the rest runs 0 → 1.
 */
export function afterHold(p: number, hold: number, total: number): number {
    if (hold <= 0) return p;
    const at = hold / total;
    return p <= at ? 0 : (p - at) / (1 - at);
}

/** How long the Reviewed hold is for this moment: the stamp's pop, or nothing. */
const holdFor = (exit: DeckMoment): number => (exit === "reviewed" ? DURATIONS.stamp : 0);

/** The reduced-motion stand-in for any card transition: a plain fade, in or out. */
function fade(): TransitionConfig {
    return {duration: DURATIONS.reduced, css: (t) => `opacity: ${r(t)}`};
}

/** Nothing: the card does not move of its own (the layer is closing around it). */
const still = (): TransitionConfig => ({duration: 0});

/** How long the outgoing card is on screen for `exit`, stamp hold included. */
export function exitDuration(exit: CardExit): number {
    if (exit === "back") return DURATIONS.settle;
    return holdFor(exit) + DURATIONS[exit];
}

/**
 * The outgoing top card. Svelte runs an outro's `t` from 1 down to 0, so
 * `u = 1 - t` is how far it has gone.
 *
 * - reviewed / saved: flies off to the right and slightly up, turning 12°.
 *   Reviewed holds still first while its ✓ stamp pops.
 * - skip: flies off to the left, turning −8°, and loses most of its colour.
 * - ignore: drops off the bottom.
 * - back: the previous card is coming in on top. This one shrinks back into
 *   the peek slot and fades out there, handing over to the peek that replaces it.
 */
export function cardOut(_node: Element, params: MotionParams): TransitionConfig {
    const {exit} = params;
    if (exit === "close") return still();
    if (params.reduced || exit === "open") return fade();
    if (exit === "back") {
        const peek = PEEKS[0] as Pose;
        return {duration: DURATIONS.settle, css: (_t, u) => `${poseCss(TOP, peek, cubicOut(u))}; opacity: ${r(1 - cubicOut(u))}`};
    }
    const flight = FLIGHTS[exit];
    const hold = holdFor(exit);
    const duration = exitDuration(exit);
    return {
        duration,
        css: (_t, u) => {
            const q = quadIn(afterHold(u, hold, duration));
            const move = `transform: translate(${r(flight.x * q)}vw, ${r(flight.y * q)}vh) rotate(${r(flight.deg * q)}deg)`;
            return exit === "skip" ? `${move}; filter: saturate(${r(1 - 0.7 * q)})` : move;
        },
    };
}

/** How long the incoming top card (or the summary under the last card) takes to arrive for `exit`, any hold included. */
export function enterDuration(exit: DeckMoment): number {
    if (exit === "open") return DURATIONS.open;
    if (exit === "back") return DURATIONS.back;
    if (exit === "close") return 0;
    return holdFor(exit) + DURATIONS.rise;
}

/**
 * The incoming top card.
 *
 * - open: the deck rises in, from 24px down at 0.97 scale.
 * - back: the previous card comes back in from the left, on top.
 * - anything else: the peek rises into the top slot while the old top card
 *   flies. After a Reviewed it waits out the stamp first, so the two moves
 *   start together.
 */
export function cardIn(_node: Element, params: MotionParams): TransitionConfig {
    const {exit} = params;
    if (exit === "close") return still();
    if (params.reduced) return fade();
    const duration = enterDuration(exit);
    if (exit === "open") {
        return {
            duration,
            css: (t) => {
                const q = cubicOut(t);
                return `transform: translateY(${r(24 * (1 - q))}px) scale(${r(0.97 + 0.03 * q)}); opacity: ${r(q)}`;
            },
        };
    }
    if (exit === "back") {
        return {
            duration,
            css: (t) => {
                const q = 1 - cubicOut(t);
                return `transform: translate(${r(-120 * q)}vw, 0vh) rotate(${r(-8 * q)}deg)`;
            },
        };
    }
    const peek = PEEKS[0] as Pose;
    const hold = holdFor(exit);
    return {duration, css: (t) => poseCss(peek, TOP, cubicOut(afterHold(t, hold, duration)))};
}

/**
 * A waiting card's resting style at `depth` (1 = next), with the CSS transition
 * that carries it to a new slot when the deck moves up. Under reduced motion it
 * still sits in the pile, but it changes slot in place.
 */
export function peekStyle(depth: number, reduced: boolean): string {
    const pose = PEEKS[Math.min(Math.max(depth, 1), PEEK_DEPTH) - 1] as Pose;
    const rest = poseCss(pose, pose, 1);
    if (reduced) return rest;
    return `${rest}; transition: transform ${DURATIONS.peek}ms ${CUBIC_OUT_CSS}, filter ${DURATIONS.peek}ms ${CUBIC_OUT_CSS}`;
}

/** A waiting card joining the pile: a fade, a beat late when the run is just opening. */
export function peekIn(_node: Element, params: {reduced: boolean; opening?: boolean}): TransitionConfig {
    const delay = params.opening === true ? DURATIONS.peekBeat : 0;
    return {delay, duration: params.reduced ? DURATIONS.reduced : DURATIONS.peek, css: (t) => `opacity: ${r(cubicOut(t))}`};
}

/** A waiting card leaving the pile (it became the top card, or ← pushed it a slot further down). */
export function peekOut(_node: Element, params: {reduced: boolean}): TransitionConfig {
    return {duration: params.reduced ? DURATIONS.reduced : DURATIONS.peek, css: (t) => `opacity: ${r(t)}`};
}

/** The ✓ stamp on Reviewed: 0.6 → 1.1 → 1. Reduced motion shows it still (the card border flashes instead). */
export function stampPop(_node: Element, params: {reduced: boolean}): TransitionConfig {
    if (params.reduced) return fade();
    return {duration: DURATIONS.stamp, easing: backOut, css: (t) => `transform: scale(${r(0.6 + 0.4 * t)}); opacity: ${r(Math.min(1, t * 2))}`};
}

/** The top bar as the overlay opens: drops 8px into place and fades in. */
export function barIn(_node: Element, params: {reduced: boolean}): TransitionConfig {
    if (params.reduced) return fade();
    return {duration: DURATIONS.bar, css: (t) => `transform: translateY(${r(-8 * (1 - cubicOut(t)))}px); opacity: ${r(cubicOut(t))}`};
}

/** The whole layer (bar and deck) as the overlay closes: fades and drops 16px. */
export function layerOut(_node: Element, params: {reduced: boolean}): TransitionConfig {
    if (params.reduced) return fade();
    return {duration: DURATIONS.close, css: (t, u) => `transform: translateY(${r(16 * quadIn(u))}px); opacity: ${r(t)}`};
}

/** The backdrop: a fade, as long as the opening rise, or the closing drop. */
export function backdrop(_node: Element, params: {reduced: boolean; closing?: boolean}): TransitionConfig {
    return {duration: params.reduced ? DURATIONS.reduced : params.closing === true ? DURATIONS.close : DURATIONS.open, css: (t) => `opacity: ${r(t)}`};
}

/** The footer's clean ↔ dirty swap: a fade in place (the footer's height is fixed). */
export function footerFade(_node: Element, params: {reduced: boolean}): TransitionConfig {
    return {duration: params.reduced ? DURATIONS.reduced : DURATIONS.footer, css: (t) => `opacity: ${r(t)}`};
}

/** The user's `prefers-reduced-motion`, read when asked (it can change mid-session). */
export function prefersReducedMotion(): boolean {
    return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * End every animation running under `root` at its last frame, at once (D29: a
 * keystroke mid-animation finishes it). Svelte runs `css` transitions through
 * the Web Animations API, so `finish()` also completes the outro and removes
 * the outgoing card. The peeks' slot moves are CSS transitions, which
 * `getAnimations` returns too.
 */
export function finishMotion(root: Element | null): void {
    if (root === null || typeof root.getAnimations !== "function") return;
    for (const animation of root.getAnimations({subtree: true})) {
        try {
            animation.finish();
        } catch {
            // An animation with an infinite iteration (the saving dot's pulse)
            // cannot be finished; it is not motion this rule is about.
        }
    }
}
