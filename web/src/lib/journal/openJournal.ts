// The imperative half of a journal drill-down: set the filter, then navigate.
//
// Split from `filters/journalTarget.ts` so the target→search arithmetic stays
// testable in the node project without SvelteKit's router around it.
//
// `filters.replace` AND the query string, belt and braces: the journal route's
// URL restore is a one-shot `onMount`, so navigating to a route that is already
// mounted would not re-read the search. The store write is what actually takes
// effect there; the query string is what makes the destination linkable and
// survive a reload.

import {goto} from "$app/navigation";
import {resolve} from "$app/paths";
import {endOfMonth} from "#lib/accrual/model.js";
import {journalSearch, targetToFilter, type JournalTarget} from "#lib/filters/journalTarget.js";
import {expandKeyOf, recognitionExpand} from "#lib/journal/recognitionExpand.svelte.js";
import {filters} from "#lib/stores/filters.svelte.js";

export async function openJournal(target: JournalTarget): Promise<void> {
    filters.replace(targetToFilter(target));
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve("/") IS the route id; the query string is appended
    await goto(`${resolve("/")}?${journalSearch(target)}`);
}

/**
 * Plan 45 Q7 (and the "Posted N entries" toast): the journal on month `month`
 * (`YYYY-MM`) with its recognition group expanded. Like `openJournal`, the
 * store and the URL both: the journal page may already be mounted (the
 * Insights panel lives on it), where only the store write takes effect.
 */
export async function openRecognitionMonth(month: string): Promise<void> {
    const target: JournalTarget = {from: `${month}-01`, to: endOfMonth(month)};
    filters.replace(targetToFilter(target));
    recognitionExpand.seed([month], expandKeyOf(filters.value));
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve("/") IS the route id; the query string is appended
    await goto(`${resolve("/")}?${journalSearch(target)}&expand=${month}`);
}
