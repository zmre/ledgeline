// The holdings filter's pure rules (plan 47 D6–D9): what a click does to the
// selection, and when two scopes are the same request. No Svelte runtime —
// node-testable, like the codec beside it.

import type {HoldingsScope} from "#lib/holdings/types.js";

/** Whether two sets hold exactly the same members. */
export function sameMembers(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
    if (a.size !== b.size) return false;
    for (const key of a) if (!b.has(key)) return false;
    return true;
}

/**
 * The selection after a click that names `keys` (a table row, a holding wedge,
 * a category wedge's every holding — D7). It becomes exactly `keys`, unless it
 * already WAS exactly `keys`, in which case it clears: the same click undoes
 * itself, which is what makes click-to-filter safe to reach for.
 */
export function toggledSelection(current: ReadonlySet<string>, keys: readonly string[]): ReadonlySet<string> {
    const next = new Set(keys);
    return sameMembers(current, next) ? new Set<string>() : next;
}

/** Whether two scopes ask the engine the same question — every field, sets by membership. */
export function sameScope(a: HoldingsScope, b: HoldingsScope): boolean {
    return (
        a.asOf === b.asOf && a.mode === b.mode && a.gainPeriod === b.gainPeriod && sameMembers(a.accounts, b.accounts) && sameMembers(a.holdings, b.holdings)
    );
}

/** One row of the Holdings dropdown: what is stored in the filter, and what the reader sees. */
export interface HoldingOption {
    key: string;
    label: string;
    /** A second, muted line: the symbol's full name, or the account path. */
    detail: string | null;
}

/**
 * The dropdown's options — the current tab's report rows — filtered by a
 * free-text search over key and label (case-insensitive). Selected keys the
 * report no longer lists (a URL from another scope) are kept, at the end, so
 * the dropdown always "lists exactly what is selected" (D7) and each can still
 * be unticked.
 */
export function holdingOptions(rows: readonly HoldingOption[], selected: ReadonlySet<string>, search: string): HoldingOption[] {
    const known = new Set(rows.map((row) => row.key));
    const orphans: HoldingOption[] = [...selected]
        .filter((key) => !known.has(key))
        .sort()
        .map((key) => ({key, label: key, detail: null}));
    const needle = search.trim().toLowerCase();
    const all = [...rows, ...orphans];
    if (needle === "") return all;
    return all.filter((row) => row.key.toLowerCase().includes(needle) || row.label.toLowerCase().includes(needle));
}

/** The holdings filter as the wire takes it: comma-joined, sorted (equal filters, equal URLs), undefined when empty (every row). */
export function filterParam(holdings: ReadonlySet<string>): string | undefined {
    return holdings.size === 0 ? undefined : [...holdings].sort().join(",");
}
