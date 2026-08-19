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
// So the popup is `position: fixed` and portalled to <body>, positioned from the
// input's viewport rect. One mechanism covers both. The arithmetic lives here,
// pure, because jsdom has no layout engine and could not verify it in a
// component test.

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
 * Where to put a popup anchored to `rect`.
 *
 * Prefers below — that is where a dropdown belongs and where the eye already is
 * — and flips above only when below is genuinely cramped AND above is roomier.
 * The `MIN_ROOM` floor stops it flip-flopping over a pixel of difference as the
 * user types and the list resizes.
 */
export function popupPosition(rect: Rect, viewport: Viewport, desiredHeight: number): Placement {
    const roomBelow = viewport.height - rect.bottom - GAP - MARGIN;
    const roomAbove = rect.top - GAP - MARGIN;
    const below = roomBelow >= Math.min(desiredHeight, MIN_ROOM) || roomBelow >= roomAbove;

    const maxHeight = Math.max(0, below ? roomBelow : roomAbove);
    const height = Math.min(desiredHeight, maxHeight);
    const top = below ? rect.bottom + GAP : rect.top - GAP - height;

    // Clamp horizontally so a right-hand-edge input does not push the popup off
    // screen. The popup matches the input's width, which keeps the two visually
    // bound together.
    const width = Math.min(rect.width, viewport.width - MARGIN * 2);
    const left = Math.max(MARGIN, Math.min(rect.left, viewport.width - width - MARGIN));

    return {top, left, width, maxHeight, below};
}
