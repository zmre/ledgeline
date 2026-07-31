// Moving one element of an ordered list, as a pure function.
//
// The imports editor is the first surface in this SPA with a user-reorderable
// list, and reordering is where an off-by-one is invisible: a rules file whose
// `if` blocks silently swapped would keep importing, just into the wrong
// accounts, because LATER MATCHES WIN. So the arithmetic lives here with its own
// tests and the component holds nothing but the array — the same split
// `holdings/ui/view.ts` uses for its comparators.
//
// Reorder is driven by ↑/↓ buttons rather than drag-and-drop: keyboard-reachable
// with no extra work, selectable by `getByRole` under Playwright, and no new
// dependency.

/**
 * `list` with the element at `from` moved to index `to`.
 *
 * Returns a NEW array; `list` is never mutated (the caller assigns the result,
 * which is what makes Svelte's `$state` notice). An out-of-range `from` or `to`,
 * or `from === to`, returns a copy unchanged — a disabled button that gets
 * clicked anyway must be a no-op, not a corrupted document.
 *
 * `to` is the destination index in the FINAL array, so `moveItem(l, 0, 1)` and
 * `moveItem(l, 1, 0)` are inverses.
 */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
    const out = [...list];
    if (!Number.isInteger(from) || !Number.isInteger(to)) return out;
    if (from < 0 || from >= out.length || to < 0 || to >= out.length || from === to) return out;
    const [moved] = out.splice(from, 1);
    // `splice` on a bounds-checked index always yields one element; the guard is
    // for `noUncheckedIndexedAccess`-style narrowing, not for a real case.
    if (moved === undefined) return [...list];
    out.splice(to, 0, moved);
    return out;
}
