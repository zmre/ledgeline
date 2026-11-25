// A sortable list's sort keys (PLAN 5k, Y6): `s` sorts by the next column
// (each starting its own way round, as a header click does) and `S` flips
// the order of the one shown. The list's `onSort` is the header click's, so
// the URL, history and server sort are unchanged; without it, no keys.

import type {Binding} from "#ui/keys/types.js";
import type {SortDir} from "#lib/listQuery.js";
import type {InvoicerKeyGroup} from "./groups.js";

export interface SortKeysOptions {
    group: InvoicerKeyGroup;
    /** The sortable columns, in header order. */
    columns: readonly string[];
    shown: () => {sort: string; dir: SortDir};
    /** The header click; a getter, as the prop can come and go. */
    onSort: () => ((column: string) => void) | undefined;
}

/** The column after `current` in header order, wrapping. */
export function nextColumn(columns: readonly string[], current: string): string {
    const at = columns.indexOf(current);
    return columns[(at + 1) % columns.length];
}

export function sortKeys(options: SortKeysOptions): Binding[] {
    const {group, columns, shown, onSort} = options;
    const sortable = (): boolean => onSort() !== undefined && columns.length > 0;
    return [
        {keys: "s", label: "Sort by the next column", group, enabled: sortable, run: () => onSort()?.(nextColumn(columns, shown().sort))},
        {keys: "S", label: "Reverse the sort", group, enabled: sortable, run: () => onSort()?.(shown().sort)},
    ];
}
