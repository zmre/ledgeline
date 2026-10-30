// What the list hands every entry's card besides the entry itself: the
// cursor, drag and menu wiring, which `EntryFrame` draws the same way for
// every kind. Bundled so each kind's component passes it through untouched.

export interface FrameMenuItem {
    label: string;
    onSelect: () => void;
    danger?: boolean;
}

export interface FrameProps {
    cursor: boolean;
    canDrag: boolean;
    outlined: boolean;
    hint: string | null;
    menu: FrameMenuItem[];
    onEdit: (() => void) | null;
    onActivate: () => void;
    onDragStart: (event: DragEvent) => void;
    onDragEnd: () => void;
    onHover: (inside: boolean) => void;
}

/**
 * The postings grid every card shares: account, amount, annotations. The
 * amount and annotation tracks are FIXED widths, so amounts line up down the
 * whole list, the way a journal's do — a per-card `auto` track put each card's
 * amounts wherever its own annotations left room.
 */
export const POSTING_GRID =
    "mt-1 grid grid-cols-[minmax(0,1fr)_minmax(0,9.5rem)] lg:grid-cols-[minmax(0,1fr)_minmax(0,9.5rem)_minmax(0,19rem)] items-baseline gap-x-4 gap-y-0.5 pl-6 font-mono text-[0.8125rem] tabular-nums";

/**
 * A posting's annotation cell: its own track from `lg` up; below that it
 * wraps under the posting, indented, so the account keeps its width.
 * Hidden when empty, so a plain posting adds no blank line.
 */
export const POSTING_NOTES = "col-span-2 pl-4 empty:hidden lg:col-span-1 lg:pl-0";
