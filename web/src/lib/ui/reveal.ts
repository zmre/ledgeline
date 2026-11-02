// Bring content that a button press made appear BELOW the fold into view.
//
// The import flow is a column of steps, and every step's button sits at the
// bottom of what came before it — so the dry run, the result, the rules-file
// draft all land under the window's lower edge, where nothing says they exist
// (Patrick, 2026-10-31). `use:reveal` on the section that appears scrolls the
// page just far enough to show it, and no further:
//
//   - its top already on screen (with a sliver of it under that): nothing;
//   - otherwise, down — never up — until its top and up to 40% of a viewport of
//     its body are visible, or all of it when it is shorter;
//   - its top never goes above the sticky header, whose height the layout
//     publishes as the document's `scroll-padding-top`.
//
// # Armed, not automatic
//
// Content also "appears" when nobody pressed anything: the import store is a
// module singleton, so coming back to the tab remounts a dry run that was
// already there, and SvelteKit restores the scroll position around it. A reveal
// that fired on every mount would yank that page around. So a reveal fires only
// once its `RevealArm` is armed, and the arm is armed by the press itself.
//
// The arm belongs to the COMPONENT INSTANCE that owns the flow — created when
// it mounts, armed by its handlers, dropped when it unmounts. That makes
// "restored" and "new" structural rather than timed: a remount brings a fresh,
// disarmed arm, so restored content never moves the page, and everything that
// appears in a mounted flow after its first press is that press's consequence.
// A timed window ("armed for N seconds") was the alternative and is wrong both
// ways: an hledger run on a large statement outlasts a short N, and a long one
// leaves unrelated renders armed.
//
// The flow's sections are spread over several components, so the arm travels by
// context: the host calls `provideRevealArm()`, the panels inside it call
// `revealArm()`. A panel mounted with no host (its own tests) gets null, which
// never fires. The arm also collects a frame's requests and scrolls ONCE — see
// `revealTarget`.
//
// The arithmetic is `revealDelta`, pure, because jsdom has no layout engine and
// a component test could not check it. `toBeInViewport` in the e2e suite checks
// the composed result.
import {getContext, setContext} from "svelte";
import {prefersReducedMotion} from "#lib/review/ui/motion.js";

/** What of a viewport-relative rect the arithmetic needs. */
export interface RevealBox {
    /** Distance from the viewport's top edge, as `getBoundingClientRect().top`. */
    readonly top: number;
    readonly height: number;
}

export interface RevealViewport {
    readonly height: number;
    /** The sticky header's height: nothing above this line is visible. */
    readonly headerOffset: number;
}

/** How much of a viewport of the element's body to bring into view. */
export const REVEAL_SHARE = 0.4;

/**
 * How much of the element must already show for its top to count as "on
 * screen". A heading's worth: a 2px border at the very bottom edge is not
 * something anyone sees appear.
 */
export const REVEAL_MIN_VISIBLE = 48;

/**
 * How far to scroll DOWN (in CSS px, never negative) so `box` is visible
 * enough. Zero means leave the page alone.
 */
export function revealDelta(box: RevealBox, viewport: RevealViewport, share: number = REVEAL_SHARE): number {
    const height = Math.max(0, box.height);
    // Above the header line, i.e. scrolled past or under it: bringing it back
    // would be scrolling UP, which a reveal never does.
    if (box.top < viewport.headerOffset) return 0;
    if (box.top + Math.min(height, REVEAL_MIN_VISIBLE) <= viewport.height) return 0;
    const wanted = Math.min(height, share * viewport.height);
    const needed = box.top + wanted - viewport.height;
    // However tall it is, its top stops at the header.
    const ceiling = box.top - viewport.headerOffset;
    return Math.max(0, Math.round(Math.min(needed, ceiling)));
}

/**
 * A flow's reveal state: whether a press has happened in it yet, and the
 * sections that asked to be shown since the last frame.
 */
export interface RevealArm {
    readonly armed: boolean;
    arm(): void;
    /** Ask for `node` to be shown after the next layout. A no-op until armed. */
    request(node: HTMLElement): void;
}

/**
 * The one section a frame's requests scroll to: the first in document order of
 * those no other request contains.
 *
 * ONE scroll per frame, because sections appear together — the dry run mounts
 * with the review strip inside it — and two `scrollBy` calls measured from the
 * same pre-scroll layout would each scroll for themselves, the inner one
 * carrying the outer one's heading off the top. The outermost, highest one is
 * what the user has to see the start of; the rest follow it on screen.
 */
export function revealTarget(nodes: readonly HTMLElement[]): HTMLElement | null {
    const outer = nodes.filter((node) => !nodes.some((other) => other !== node && other.contains(node)));
    const ordered = [...outer].sort((a, b) => (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1));
    return ordered[0] ?? null;
}

export function createRevealArm(): RevealArm {
    let armed = false;
    const pending = new Set<HTMLElement>();
    let scheduled = false;

    function flush(): void {
        scheduled = false;
        const nodes = [...pending].filter((node) => node.isConnected);
        pending.clear();
        const target = revealTarget(nodes);
        if (target !== null) scrollToShow(target);
    }

    return {
        get armed(): boolean {
            return armed;
        },
        arm(): void {
            armed = true;
        },
        request(node: HTMLElement): void {
            if (!armed) return;
            pending.add(node);
            if (scheduled) return;
            scheduled = true;
            afterLayout(flush);
        },
    };
}

const ARM = Symbol("reveal-arm");

/** Create the flow's arm and hand it to every component below this one. Call during component init. */
export function provideRevealArm(): RevealArm {
    return setContext(ARM, createRevealArm());
}

/** The arm a host above this component provided, or null with no host. Call during component init. */
export function revealArm(): RevealArm | null {
    return getContext<RevealArm | undefined>(ARM) ?? null;
}

export interface RevealOptions {
    readonly arm: RevealArm | null;
    /**
     * Re-check when this changes (`Object.is`), for a section that stays
     * mounted while its content is replaced — a spinner becoming a result.
     */
    readonly key?: unknown;
}

/** The sticky header's height, as the layout published it. */
function headerOffset(): number {
    const padding = Number.parseFloat(getComputedStyle(document.documentElement).scrollPaddingTop);
    return Number.isFinite(padding) ? padding : 0;
}

/** After layout: the content that just mounted has not been measured yet. */
function afterLayout(run: () => void): void {
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(run);
    else setTimeout(run, 0);
}

/** Scroll the page by `revealDelta` for `node`, if that is anything. */
function scrollToShow(node: HTMLElement): void {
    const rect = node.getBoundingClientRect();
    const delta = revealDelta({top: rect.top, height: rect.height}, {height: window.innerHeight, headerOffset: headerOffset()});
    if (delta > 0) window.scrollBy({top: delta, behavior: prefersReducedMotion() ? "auto" : "smooth"});
}

/**
 * `use:reveal={{arm, key}}`: on mount, and whenever `key` changes, scroll the
 * page just enough to show this element — if `arm` is armed. See the head of
 * this file for the rules.
 *
 * Sets `data-reveal` on the node, so a test can find what is revealed.
 */
export function reveal(node: HTMLElement, options: RevealOptions): {update(next: RevealOptions): void} {
    let current = options;
    node.dataset.reveal = "";
    current.arm?.request(node);
    return {
        update(next: RevealOptions): void {
            const changed = !Object.is(next.key, current.key);
            current = next;
            if (changed) current.arm?.request(node);
        },
    };
}
