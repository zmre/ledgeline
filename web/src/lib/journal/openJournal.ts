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
import {journalSearch, targetToFilter, type JournalTarget} from "$lib/filters/journalTarget";
import {filters} from "$lib/stores/filters.svelte";

export async function openJournal(target: JournalTarget): Promise<void> {
    filters.replace(targetToFilter(target));
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve("/") IS the route id; the query string is appended
    await goto(`${resolve("/")}?${journalSearch(target)}`);
}
