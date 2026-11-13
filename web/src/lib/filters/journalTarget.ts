// "Show me this in the journal" — one description of that intent, shared by
// every surface that offers it.
//
// There were two mechanisms doing this. `BudgetTable.openInJournal` calls
// `filters.replace` then `goto`; `SubscriptionsBox.journalLink` builds an
// `<a href>`. Both are needed — a link must stay a real link so middle-click and
// open-in-new-tab work — so what is shared here is the STRING, not the
// navigation. `openJournal.ts` holds the imperative half.
//
// Pure, with no `$app/*` imports, so it is a node test.

import {defaultFilter, type DatePreset, type JournalFilter, type RunningPin} from "#lib/stores/filters.svelte.js";
import {filterToSearch} from "#lib/filters/urlCodec.js";
import type {ISODate} from "#lib/domain/types.js";

export interface JournalTarget {
    /** Account subtree roots to filter to. */
    accounts?: readonly string[];
    /** Free text, matched against the transaction haystack. */
    query?: string;
    from?: ISODate | null;
    to?: ISODate | null;
    /** A live preset (`"all"`, `"ytd"`, …) instead of frozen dates. */
    preset?: DatePreset | null;
    /**
     * Arrive with this account's running balance pinned (plan 36): what a
     * report or holdings drill-down into ONE account adds, so the Balance
     * column comes with the rows. `commodity` omitted: the engine's default.
     */
    pin?: {account: string; commodity?: string | null};
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
        pin: target.pin === undefined ? null : ({account: target.pin.account, commodity: target.pin.commodity ?? null} satisfies RunningPin),
    };
}

/** The target as a query string for the journal route, without the leading `?`. */
export function journalSearch(target: JournalTarget): string {
    return filterToSearch(targetToFilter(target), defaultFilter());
}
