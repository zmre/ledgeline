// Which months' recognition group rows are expanded (plan 45 D8): page
// state the journal page sends with every rows/summary/position read
// (`expand=`), and the table toggles (▸/▾, `l`, Enter on a group row).
//
// Seeded once from a URL `expand=` (Top Transactions' Q7 lines and the
// "Posted N entries" toast link to `journalMonthHref`), and cleared when the
// filter changes — a different list, as the table's transfer expansion is.

const MONTH = /^\d{4}-\d{2}$/;

let months = $state.raw<readonly string[]>([]);
/** The filter the months belong to (`expandKeyOf`), or null before the first `follow`. */
let forKey: string | null = null;

/** The part of a journal filter that makes it a different list. */
export function expandKeyOf(filter: {from: string | null; to: string | null; accounts: ReadonlySet<string>; query: string}): string {
    return JSON.stringify([filter.from, filter.to, [...filter.accounts].sort(), filter.query]);
}

/** The valid months in `items`, once each, sorted. */
function unique(items: readonly string[]): string[] {
    return items.filter((month, at) => MONTH.test(month) && items.indexOf(month) === at).sort();
}

/** The months in `raw` (`2026-11,2026-10`), valid and deduplicated, sorted. */
export function parseExpand(raw: string | null | undefined): string[] {
    if (raw === null || raw === undefined) return [];
    return unique(raw.split(",").map((part) => part.trim()));
}

export const recognitionExpand = {
    /** The expanded months, sorted. */
    get months(): readonly string[] {
        return months;
    },
    has(month: string): boolean {
        return months.includes(month);
    },
    /** Expand `month` if it is collapsed, else collapse it. */
    toggle(month: string): void {
        if (!MONTH.test(month)) return;
        months = months.includes(month) ? months.filter((m) => m !== month) : [...months, month].sort();
    },
    /** Replace the set. */
    set(next: readonly string[]): void {
        months = unique(next);
    },
    /** The URL's seed: `next`, belonging to the filter keyed `key` (so the first `follow` keeps it). */
    seed(next: readonly string[], key: string): void {
        this.set(next);
        forKey = key;
    },
    /** The filter is now `key`: a different one than the months belong to clears them. */
    follow(key: string): void {
        if (forKey !== null && forKey !== key) this.clear();
        forKey = key;
    },
    clear(): void {
        if (months.length > 0) months = [];
    },
};

/** Test seam. */
export function resetRecognitionExpand(): void {
    months = [];
    forKey = null;
}
