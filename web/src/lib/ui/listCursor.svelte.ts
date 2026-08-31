// A keyboard cursor over a list: which row is current, and where it goes when
// the list changes underneath it.
//
// A factory rather than a store, because four surfaces each want their own
// (journal, reports, holdings, imports rules) and a module singleton would make
// them fight. Only the INDEX MATH is shared — scrolling is not, because the
// journal's list is virtualized and its target row is usually not in the DOM,
// while the other three can just call `scrollIntoView`. Pushing pitch, overscan,
// sticky-header headroom and card-vs-table mode into a "generic" primitive that
// one of four callers needs is how you get an abstraction with a journal-shaped
// hole in it.
//
// There is deliberately NO `$effect` in here. The index is `$derived`, and
// `fallback` is written only by the mutators — so this adds no ordering hazard
// to a component that already documents one, and cannot produce the
// self-feeding shape `routes/effectLatch.test.ts` exists to catch.

export type CursorKey = string | number;

export interface ListCursor<T> {
    /** Position in the current list, or -1 when nothing is cursored or the list is empty. */
    readonly index: number;
    /** The cursored item, or null. */
    readonly item: T | null;
    /** The key the cursor is anchored to, or null. */
    readonly key: CursorKey | null;
    /** Relative move, clamped. A move on an unset cursor lands on the first row. */
    move(delta: number): void;
    /** Absolute move, clamped. */
    to(index: number): void;
    first(): void;
    last(): void;
    /**
     * Adopt whatever key now sits at the current position.
     *
     * Needed after any write the cursor initiated: `Transaction.index` is a
     * "stable id within a fetch" and the engine reassigns it, so a delete
     * renumbers every later transaction. Re-anchoring by position means `x`
     * leaves the cursor on the next transaction, which is what you want.
     */
    reanchor(): void;
    clear(): void;
}

export function listCursor<T>(items: () => readonly T[], keyOf: (item: T, at: number) => CursorKey): ListCursor<T> {
    let key = $state<CursorKey | null>(null);
    /** Where the cursor was, for when its key disappears. */
    let fallback = $state(0);

    const index = $derived.by(() => {
        const list = items();
        if (key === null || list.length === 0) return -1;
        // `fallback` is where `set` last put the cursor, and `set` writes it in
        // the same breath as `key` — so on a list that has not changed under us
        // it already IS the answer, and checking it is one comparison instead of
        // a scan. This matters because `index` is read on every render and the
        // scan is O(n) over the FILTERED journal: holding `j` down on an
        // all-time view walked 150k rows per keystroke to rediscover a position
        // the cursor had just set itself.
        //
        // Falling through is not a slow path so much as the correct one for the
        // case this is really about — the list changed, so the key must be
        // hunted for or given up on.
        if (fallback < list.length && keyOf(list[fallback], fallback) === key) return fallback;
        for (let at = 0; at < list.length; at += 1) {
            if (keyOf(list[at], at) === key) return at;
        }
        // The key is gone (deleted, or filtered out). Hold the POSITION, which
        // is vim's `dd` behaviour and what a keyboard user expects.
        return Math.min(fallback, list.length - 1);
    });

    function set(at: number): void {
        const list = items();
        if (list.length === 0) {
            // Keep the key: widening the filter again should bring the cursor
            // back to the row the user was on.
            fallback = 0;
            return;
        }
        const clamped = Math.max(0, Math.min(at, list.length - 1));
        fallback = clamped;
        key = keyOf(list[clamped], clamped);
    }

    return {
        get index(): number {
            return index;
        },
        get item(): T | null {
            const list = items();
            return index >= 0 && index < list.length ? list[index] : null;
        },
        get key(): CursorKey | null {
            return key;
        },
        move(delta: number): void {
            // An unset cursor starts at the first row for a downward move and
            // the first row for an upward one too — anything else means `k` on a
            // fresh list silently does nothing.
            set(index === -1 ? 0 : index + delta);
        },
        to(at: number): void {
            set(at);
        },
        first(): void {
            set(0);
        },
        last(): void {
            set(items().length - 1);
        },
        reanchor(): void {
            if (key !== null) set(index === -1 ? fallback : index);
        },
        clear(): void {
            key = null;
            fallback = 0;
        },
    };
}
