import {describe, expect, it} from "vitest";
import {moveItem} from "./reorder";

// Reordering is the one operation on this screen whose bug is invisible: a rules
// file whose `if` blocks silently swapped still imports, just into the wrong
// accounts, because later matches win. So the arithmetic gets its own suite.

describe("UNIT reorder — moveItem", () => {
    const list = ["a", "b", "c", "d"] as const;

    it("moves an element forwards and backwards", () => {
        expect(moveItem(list, 0, 2)).toEqual(["b", "c", "a", "d"]);
        expect(moveItem(list, 3, 1)).toEqual(["a", "d", "b", "c"]);
    });

    it("treats adjacent moves as a swap, in both directions", () => {
        expect(moveItem(list, 1, 2)).toEqual(["a", "c", "b", "d"]);
        expect(moveItem(list, 2, 1)).toEqual(["a", "c", "b", "d"]);
    });

    it("is its own inverse for any pair", () => {
        for (let from = 0; from < list.length; from += 1) {
            for (let to = 0; to < list.length; to += 1) {
                expect(moveItem(moveItem(list, from, to), to, from)).toEqual([...list]);
            }
        }
    });

    it("is a no-op when from === to", () => {
        expect(moveItem(list, 2, 2)).toEqual([...list]);
    });

    // A disabled ↑/↓ that gets clicked anyway (a stale render, a keyboard
    // repeat) must do nothing at all rather than corrupt the document.
    it("returns an unchanged copy for out-of-range indices", () => {
        for (const [from, to] of [
            [-1, 0],
            [0, -1],
            [4, 0],
            [0, 4],
            [0, 99],
            [-5, -5],
        ]) {
            expect(moveItem(list, from as number, to as number)).toEqual([...list]);
        }
    });

    it("refuses non-integer indices rather than splicing at a fractional position", () => {
        expect(moveItem(list, 0.5, 2)).toEqual([...list]);
        expect(moveItem(list, 0, Number.NaN)).toEqual([...list]);
    });

    it("never mutates its input", () => {
        const source = ["a", "b", "c"];
        const moved = moveItem(source, 0, 2);
        expect(source).toEqual(["a", "b", "c"]);
        expect(moved).not.toBe(source);
    });

    it("handles the degenerate lengths", () => {
        expect(moveItem([], 0, 0)).toEqual([]);
        expect(moveItem(["only"], 0, 0)).toEqual(["only"]);
    });

    it("preserves element identity, not just equality", () => {
        const a = {id: 1};
        const b = {id: 2};
        expect(moveItem([a, b], 0, 1)[1]).toBe(a);
    });
});
