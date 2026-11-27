// A list's row cursor (PLAN 5k, Y6): `j`/`k` move it, `g g`/`G` jump to the
// ends, Enter or `o` opens the row, Escape drops the cursor. The cursored row
// carries `data-cursor` (the list draws its ring from `cursor.index`), is
// scrolled into view, and its link (or button) takes focus, so Tab goes on
// from there and a screen reader reads the row it is on.
//
// The index math is packages/ui's `listCursor`, keyed by each item's own key
// so a live reload or a re-sort keeps the cursor on the same row.
//
// Enter is scoped to the list (`within`): on a control in it (a sort header,
// another row's link) it does what that control does, and on the cursored
// row it opens it. Outside the list Enter is the focused control's own.

import {tick} from "svelte";
import {registerKeys} from "#ui/keys/keymap.svelte.js";
import {PRIORITY, type Binding} from "#ui/keys/types.js";
import {listCursor, type ListCursor} from "#ui/ui/listCursor.svelte.js";
import type {InvoicerKeyGroup} from "./groups.js";

export interface RowKeysOptions<T> {
    /** The layer's id. */
    id: string;
    group: InvoicerKeyGroup;
    /** What a row is, for the sheet: "invoice" reads "Next invoice". */
    noun: string;
    items: () => readonly T[];
    keyOf: (item: T) => string;
    /** The list's element: the cursored row is found, and Enter scoped, inside it. */
    root: () => HTMLElement | null;
    /**
     * How a row opens (a getter, since the list's prop can come and go).
     * Without one, the cursored row's link (`a[data-row-link]`) is clicked.
     */
    open?: () => ((item: T) => void) | undefined;
    /** More bindings for the list's layer (sort keys). */
    extra?: Binding[];
}

/** The control a cursored row hands focus to. */
const ROW_CONTROL = "a[data-row-link], button";

/**
 * After the ring moves: the cursored row in `root` into view and its
 * control focused. For a list's own moves beyond `j`/`k` (a tree's ←/→).
 */
export function revealCursored(root: () => HTMLElement | null): void {
    void tick().then(() => {
        const row = root()?.querySelector<HTMLElement>("[data-cursor]") ?? null;
        if (row === null) return;
        row.scrollIntoView?.({block: "nearest"});
        (row.querySelector<HTMLElement>(ROW_CONTROL) ?? row).focus({preventScroll: true});
    });
}

export function rowKeys<T>(options: RowKeysOptions<T>): ListCursor<T> {
    const {id, group, noun, items, keyOf, root} = options;
    const cursor = listCursor<T>(items, (item) => keyOf(item));

    function cursoredRow(): HTMLElement | null {
        return root()?.querySelector<HTMLElement>("[data-cursor]") ?? null;
    }

    const go = (move: () => void) => (): void => {
        move();
        revealCursored(root);
    };

    function openCursored(): void {
        const item = cursor.item;
        if (item === null) return;
        const open = options.open?.();
        if (open !== undefined) open(item);
        else cursoredRow()?.querySelector<HTMLAnchorElement>("a[data-row-link]")?.click();
    }

    /** Enter: the focused control's own action, else the cursored row. */
    function enter(): void {
        const focused = document.activeElement;
        const row = cursoredRow();
        const own = focused instanceof HTMLElement && focused.matches("a[href], button") && !(row !== null && row.contains(focused));
        if (own) focused.click();
        else openCursored();
    }

    const some = (): boolean => items().length > 0;
    const cursored = (): boolean => cursor.item !== null;

    registerKeys({
        id,
        priority: PRIORITY.widget,
        bindings: [
            {keys: "j", label: `Next ${noun}`, group, enabled: some, run: go(() => cursor.move(1))},
            {keys: "k", label: `Previous ${noun}`, group, enabled: some, run: go(() => cursor.move(-1))},
            {keys: "g g", label: `First ${noun}`, group, enabled: some, run: go(() => cursor.first())},
            {keys: "G", label: `Last ${noun}`, group, enabled: some, run: go(() => cursor.last())},
            {keys: "Enter", label: `Open the ${noun}`, group, enabled: cursored, within: root, run: enter},
            {keys: "o", label: `Open the ${noun}`, group, enabled: cursored, run: openCursored},
            {keys: "Escape", label: "Clear the row cursor", group, enabled: cursored, run: () => cursor.clear()},
            ...(options.extra ?? []),
        ],
    });

    return cursor;
}

/** The classes a cursored row wears: a ring that survives the row's own hover tint. */
export const CURSOR_ROW = "outline-2 -outline-offset-2 outline-primary";
