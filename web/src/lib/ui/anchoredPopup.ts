// Positioning for a popup that must escape its scroll container.
//
// The account combobox has two homes, and BOTH clip an absolutely-positioned
// child:
//
//   - Inside the transaction popup. daisyUI's `.modal-box` is
//     `overflow-y: auto; max-height: 100vh`, and per CSS spec a non-`visible`
//     value on one axis computes the other to `auto` — so it clips on both.
//   - Inside the journal's inline category editor, which lives in the
//     virtualized table's `overflow-y-auto` scroller.
//
// So the popup is `position: fixed` and portalled to <body> ([`portal`]),
// positioned from the input's viewport rect. One mechanism covers both. The
// arithmetic lives here, pure, because jsdom has no layout engine and could not
// verify it in a component test.
//
// # The portal is load-bearing, not tidiness
//
// `position: fixed` resolves against the VIEWPORT only while no ancestor
// establishes a containing block for it. daisyUI's `.modal-box` does:
//
//     .modal-box { scale: .95; translate: 0; transition: translate, scale, ... }
//
// A non-`none` `transform`/`scale`/`translate`/`filter` (and `will-change` on
// any of them, and `contain`) makes that element the containing block for fixed
// descendants — and, since `.modal-box` is also `overflow-y: auto`, their
// clipper. Without the portal the popup is therefore offset by the modal box's
// own position AND clipped by it, which reads on screen as "the autocomplete
// stopped working".
//
// That is not hypothetical: it is what daisyUI 5.7.19 does, and this module's
// comment claimed a portal that the component never actually performed. The
// inline category editor kept working the whole time — its scroller has no
// transform, so fixed still meant the viewport there — which is exactly why the
// breakage looked like it was about the transaction popup specifically.

export interface Rect {
    top: number;
    bottom: number;
    left: number;
    width: number;
}

export interface Viewport {
    width: number;
    height: number;
}

export interface Placement {
    top: number;
    left: number;
    width: number;
    /** How much vertical room the popup may use before it needs to scroll internally. */
    maxHeight: number;
    below: boolean;
}

/** Gap between the input and its popup, in px. */
const GAP = 2;
/** Keep this much clear of the viewport edge, so the popup never sits flush against it. */
const MARGIN = 8;
/** Below this much room, flipping up is worth it. */
const MIN_ROOM = 96;

/**
 * Which side of `rect` to open on, and how much room that leaves.
 *
 * Prefers below — that is where a dropdown belongs and where the eye already is
 * — and flips above only when below is genuinely cramped AND above is roomier.
 * The `MIN_ROOM` floor stops it flip-flopping over a pixel of difference as the
 * user types and the list resizes.
 *
 * Shared by both placements below, which differ only in how they pick a WIDTH.
 */
function verticalPlacement(rect: Rect, viewport: Viewport, desiredHeight: number): Omit<Placement, "left" | "width"> {
    const roomBelow = viewport.height - rect.bottom - GAP - MARGIN;
    const roomAbove = rect.top - GAP - MARGIN;
    const below = roomBelow >= Math.min(desiredHeight, MIN_ROOM) || roomBelow >= roomAbove;

    const maxHeight = Math.max(0, below ? roomBelow : roomAbove);
    const height = Math.min(desiredHeight, maxHeight);
    const top = below ? rect.bottom + GAP : rect.top - GAP - height;

    return {top, maxHeight, below};
}

/**
 * Where to put a popup anchored to `rect`, as wide as `rect` itself.
 *
 * The width is the anchor's because this is a COMBOBOX popup: a list of
 * completions for the field below it, which reads as one control with it.
 */
export function popupPosition(rect: Rect, viewport: Viewport, desiredHeight: number): Placement {
    // Clamp horizontally so a right-hand-edge input does not push the popup off
    // screen. The popup matches the input's width, which keeps the two visually
    // bound together.
    const width = Math.min(rect.width, viewport.width - MARGIN * 2);
    const left = Math.max(MARGIN, Math.min(rect.left, viewport.width - width - MARGIN));

    return {...verticalPlacement(rect, viewport, desiredHeight), left, width};
}

/**
 * Where to put a MENU anchored to `rect`: its own width, RIGHT edge aligned.
 *
 * Both differences from [`popupPosition`] are the reason this exists. A menu's
 * trigger is a 24px `⋯` button, so matching the anchor's width would give a
 * 24px menu; and a trigger in the last column of a table is close enough to the
 * right edge that a left-aligned menu would be clamped away from it. This is
 * daisyUI's `dropdown-end` written as arithmetic, because the CSS version is
 * positioned inside the scroll container it needs to escape (see the header).
 */
export function menuPosition(rect: Rect, viewport: Viewport, desired: {width: number; height: number}): Placement {
    const width = Math.min(desired.width, viewport.width - MARGIN * 2);
    const right = rect.left + rect.width;
    const left = Math.max(MARGIN, Math.min(right - width, viewport.width - width - MARGIN));

    return {...verticalPlacement(rect, viewport, desired.height), left, width};
}

/**
 * Move `node` to `<body>` for as long as it is mounted.
 *
 * A Svelte action rather than anything cleverer, because the requirement is
 * exactly "this element must not be a descendant of whatever rendered it". See
 * the header for why that is necessary and not merely neat.
 *
 * Teardown removes the node itself rather than asking its original parent to,
 * which is what makes the move safe: `node.remove()` is valid wherever the node
 * has ended up.
 */
export function portal(node: HTMLElement): {destroy(): void} {
    document.body.appendChild(node);
    return {
        destroy(): void {
            node.remove();
        },
    };
}
