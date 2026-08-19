// "Show me this in the journal" — one description of that intent, shared by
// every surface that offers it.
//
// There were two mechanisms doing this. `BudgetSummary.openInJournal` calls
// `filters.replace` then `goto`; `SubscriptionsBox.journalLink` builds an
// `<a href>`. Both are needed — a link must stay a real link so middle-click and
// open-in-new-tab work — so what is shared here is the STRING, not the
// navigation. `openJournal.ts` holds the imperative half.
//
// Pure, with no `$app/*` imports, so it is a node test.

import {defaultFilter, type DatePreset, type JournalFilter} from "$lib/stores/filters.svelte";
import {filterToSearch} from "$lib/filters/urlCodec";
import type {ISODate} from "$lib/domain/types";

export interface JournalTarget {
    /** Account subtree roots to filter to. */
    accounts?: readonly string[];
    /** Free text, matched against the transaction haystack. */
    query?: string;
    from?: ISODate | null;
    to?: ISODate | null;
    /** A live preset (`"all"`, `"ytd"`, …) instead of frozen dates. */
    preset?: DatePreset | null;
}

/** The target as a filter the journal store can be handed directly. */
export function targetToFilter(target: JournalTarget): JournalFilter {
    const base = defaultFilter();
    return {
        from: target.from ?? null,
        to: target.to ?? null,
        accounts: new Set(target.accounts ?? []),
        query: target.query ?? "",
        preset: target.preset ?? (target.from === undefined && target.to === undefined ? base.preset : null),
    };
}

/** The target as a query string for the journal route, without the leading `?`. */
export function journalSearch(target: JournalTarget): string {
    return filterToSearch(targetToFilter(target), defaultFilter());
}
