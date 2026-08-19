// The cursor's whole job is behaving sensibly when the list changes underneath
// it, and every one of those cases is pure logic over a plain array — no DOM is
// touched here at all.
//
// It is a `.svelte.test.ts` (and so runs in the jsdom project) only because it
// declares runes, which the Svelte compiler processes for `.svelte.*` files
// alone. Nothing here needs a document.

import {flushSync} from "svelte";
import {describe, expect, it} from "vitest";
import {listCursor, type ListCursor} from "./listCursor.svelte";

interface Row {
    id: number;
}

/**
 * Build a cursor over mutable state, inside an effect root so `$derived` works
 * outside a component. Returns a setter so a test can change the list under the
 * cursor, which is the interesting half.
 */
function harness(initial: number[]): {cursor: ListCursor<Row>; setIds: (ids: number[]) => void} {
    let rows = $state<Row[]>(initial.map((id) => ({id})));
    let cursor!: ListCursor<Row>;
    $effect.root(() => {
        cursor = listCursor<Row>(
            () => rows,
            (row) => row.id
        );
    });
    return {
        cursor,
        setIds: (ids) => {
            rows = ids.map((id) => ({id}));
            flushSync();
        },
    };
}

describe("UNIT listCursor", () => {
    it("starts unset", () => {
        const {cursor} = harness([1, 2, 3]);

        expect({index: cursor.index, item: cursor.item}).toEqual({index: -1, item: null});
    });

    it("lands on the first row for the first move, in either direction", () => {
        // `k` on a fresh list doing nothing at all reads as a broken keymap.
        const down = harness([1, 2, 3]);
        down.cursor.move(1);
        expect(down.cursor.index).toBe(0);

        const up = harness([1, 2, 3]);
        up.cursor.move(-1);
        expect(up.cursor.index).toBe(0);
    });

    it("moves and clamps at both ends", () => {
        const {cursor} = harness([1, 2, 3]);

        cursor.move(1);
        cursor.move(10);
        expect(cursor.index).toBe(2);

        cursor.move(-10);
        expect(cursor.index).toBe(0);
    });

    it("jumps to first and last", () => {
        const {cursor} = harness([1, 2, 3]);

        cursor.last();
        expect(cursor.item).toEqual({id: 3});

        cursor.first();
        expect(cursor.item).toEqual({id: 1});
    });

    it("follows the RECORD when the list is reordered", () => {
        // A passive refresh can re-sort; the cursor should stay on the row the
        // user is looking at, not on the position it happened to occupy.
        const {cursor, setIds} = harness([1, 2, 3]);
        cursor.to(2);
        expect(cursor.item).toEqual({id: 3});

        setIds([3, 1, 2]);

        expect({index: cursor.index, item: cursor.item}).toEqual({index: 0, item: {id: 3}});
    });

    it("holds the POSITION when the cursored row disappears", () => {
        // vim's `dd`: delete the row you are on and the cursor lands on what
        // took its place, not back at the top.
        const {cursor, setIds} = harness([1, 2, 3]);
        cursor.to(1);

        setIds([1, 3]);

        expect(cursor.item).toEqual({id: 3});
    });

    it("clamps to the last row when the list shrinks past the fallback", () => {
        const {cursor, setIds} = harness([1, 2, 3, 4, 5]);
        cursor.last();

        setIds([1, 2]);

        expect(cursor.index).toBe(1);
    });

    it("reports -1 on an empty list but remembers the row, so widening a filter restores it", () => {
        const {cursor, setIds} = harness([1, 2, 3]);
        cursor.to(1);

        setIds([]);
        expect({index: cursor.index, item: cursor.item}).toEqual({index: -1, item: null});

        setIds([1, 2, 3]);
        expect(cursor.item).toEqual({id: 2});
    });

    it("survives moves on an empty list", () => {
        const {cursor} = harness([]);

        expect(() => {
            cursor.move(1);
            cursor.last();
        }).not.toThrow();
        expect(cursor.index).toBe(-1);
    });

    it("reanchors to whatever now occupies the position", () => {
        // After a write, `Transaction.index` is reassigned by the engine — a
        // delete renumbers everything later. Re-anchoring by position is what
        // leaves the cursor on the next transaction rather than on a stale id.
        const {cursor, setIds} = harness([1, 2, 3]);
        cursor.to(1);

        setIds([1, 9, 3]);
        cursor.reanchor();
        setIds([1, 9, 3]);

        expect(cursor.item).toEqual({id: 9});
    });

    it("clears", () => {
        const {cursor} = harness([1, 2, 3]);
        cursor.to(1);

        cursor.clear();

        expect({index: cursor.index, key: cursor.key}).toEqual({index: -1, key: null});
    });
});
