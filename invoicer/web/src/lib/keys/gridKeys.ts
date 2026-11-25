// A grid's row-moving keys (H16, folded into the keymap in PLAN 5k, Y7):
// Alt+↑ / Alt+↓ move the row holding focus, from any cell (`fields`), and
// only from inside the grid (`within`): Alt+↑ in a textarea elsewhere keeps
// its own meaning. A dialog over the page blinds them (they are not
// `always`).

import type {Binding} from "#ui/keys/types.js";
import type {InvoicerKeyGroup} from "./groups.js";

/** The focused row of `grid` and the column of the focused cell, or null. */
export function focusedCell(grid: HTMLElement | null): {row: number; column: number} | null {
    const focused = document.activeElement;
    if (grid === null || !(focused instanceof HTMLElement) || !grid.contains(focused)) return null;
    const row = focused.closest<HTMLElement>("tr[data-row]");
    if (row === null) return null;
    const cell = focused.closest<HTMLElement>("[data-col]");
    return {row: Number(row.dataset.row), column: Number(cell?.dataset.col ?? 0)};
}

export interface MoveKeysOptions {
    group: InvoicerKeyGroup;
    /** The grid's body (`tbody`), whose rows carry `data-row` and cells `data-col`. */
    grid: () => HTMLElement | null;
    readonly: () => boolean;
    /** Move row `from` to `to`, refocusing column `column`. */
    move: (from: number, to: number, column: number) => void;
}

export function moveRowKeys(options: MoveKeysOptions): Binding[] {
    const {group, grid, readonly, move} = options;
    const step = (delta: number) => (): void => {
        const at = focusedCell(grid());
        if (at !== null) move(at.row, at.row + delta, at.column);
    };
    const editable = (): boolean => !readonly();
    return [
        {keys: "alt+ArrowUp", label: "Move the row up", group, fields: true, within: grid, enabled: editable, run: step(-1)},
        {keys: "alt+ArrowDown", label: "Move the row down", group, fields: true, within: grid, enabled: editable, run: step(1)},
    ];
}
