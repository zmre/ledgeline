// The generic combobox's matching (components/Combobox.svelte): plain
// case-insensitive substrings, since its lists are names (customers,
// templates, catalog items, field values), not colon paths like accounts.

/** One choice the combobox offers. */
export interface ComboOption {
    /** What choosing it writes into the field. */
    value: string;
    /** What the list shows. */
    label: string;
    /** A second, quieter line of text (also matched). */
    detail?: string;
}

/** The popup's length: enough to scan, short of a wall. */
export const COMBO_LIMIT = 50;

/** The options `text` matches: case-insensitive substring over label, detail and value; all of them for empty text. */
export function matchOptions(text: string, options: readonly ComboOption[], limit = COMBO_LIMIT): ComboOption[] {
    const query = text.trim().toLowerCase();
    const found =
        query === ""
            ? options
            : options.filter(
                  (o) => o.label.toLowerCase().includes(query) || (o.detail ?? "").toLowerCase().includes(query) || o.value.toLowerCase().includes(query)
              );
    return found.slice(0, limit);
}
