// What the header's Refresh button actually refreshes.
//
// It used to be exactly one line — `journal.refresh({force: true})` — which is a
// far smaller promise than a circular-arrow icon in the app bar makes. The
// Imports screen alone reads four MORE server resources, each behind its own
// `ensure…` dedupe key, and the button re-read none of them:
//
//   - the rules INDEX, which the layout prefetches at startup so the nav item
//     knows whether to exist. Its key is therefore set before the user has ever
//     opened Imports, and `ensureIndex` has returned early ever since;
//   - the OPEN rules document and its CSV preview, latched in `EditRulesPanel`
//     on `${nonce}|${url}|${id}` — a key that cannot change while you sit on the
//     file you are editing, so that document was read once and never again;
//   - the alias listing, whose key is set the first time the Aliases tab is
//     opened, which is usually AFTER startup and after whatever the user changed
//     on disk. That timing accident is the whole of "reload does reload the
//     aliases but not the rules file": neither was reloaded, the aliases had
//     simply never been loaded before;
//   - the import capabilities probe, which is what decides whether the New
//     Transactions screen is usable at all;
//   - the Holdings tab's report and trend, and the price status beside them.
//     Both are keyed on `(nonce, url[, scope])`, none of which a press moves,
//     and they are the surface most likely to be stale: the prices under them
//     change from outside this screen — this app's own "Update prices" button,
//     or the user's own script against the same journal.
//
// A refresh that renews some of what is on screen and not the rest is worse than
// one that renews nothing, because it teaches the user to trust it.
//
// # "Everything" means REFRESH_TARGETS, and nothing else
//
// The list below IS the definition. A resource missing from it is a resource the
// button silently leaves stale, so adding one to the app means adding its name
// here and a case to `reload`. `refreshAll.test.ts` presses the button against a
// stubbed engine and asserts on the routes that go out, so a target added to the
// type without a case fails the type check and a route left out fails the test.
//
// # Never `ensure…`
//
// Every case in `reload` calls a `reload…` / `open` / `refresh({force: true})`
// entry point. The `ensureIndex` / `ensureListing` / `ensureCapabilities`
// wrappers exist to dedupe a prefetch against a page visit, and return early
// when their (nonce, url) key is unchanged — which it always is here, because
// pressing refresh reconnects to nothing. Routing an explicit refresh through
// one of those is how you write a button that does nothing.

import {aliasStore} from "$lib/imports/aliasStore.svelte";
import {importStore} from "$lib/imports/importStore.svelte";
import {openRules, rulesStore} from "$lib/imports/rulesStore.svelte";
import {holdingsData, holdingsScope, otherHoldingsData} from "./holdings.svelte";
import {journal} from "./journal.svelte";
import {pricesStore} from "./prices.svelte";
import {settings} from "./settings.svelte";

/** Every server resource a global refresh re-reads. See the note above before editing. */
export const REFRESH_TARGETS = ["journal", "importCapabilities", "rulesIndex", "openRules", "aliases", "prices", "holdings"] as const;

export type RefreshTargetName = (typeof REFRESH_TARGETS)[number];

/** As much of what is on screen as the plan needs to know about. */
export interface RefreshState {
    /** The rules document the editor has open, or null when it has none. */
    readonly openRulesId: string | null;
    /** Resources an unsaved form on screen is an edit OF — re-reading one discards that edit. */
    readonly unsaved: readonly RefreshTargetName[];
}

/**
 * Which targets a refresh in this state will actually re-read.
 *
 * Two exclusions, both narrow on purpose:
 *
 *   - `openRules` when nothing is open, because `rulesStore.open` needs an id
 *     and there is no sensible one to invent;
 *   - anything a form on screen is an unsaved edit of. Re-reading it would
 *     replace the user's typing with the file's bytes and say nothing, which is
 *     silent data loss dressed up as a refresh. The stale-base case already has
 *     an answer that is not silent: a save against a changed file 409s, and both
 *     editors offer "Reload and discard my changes" on that.
 */
export function refreshPlan(state: RefreshState): RefreshTargetName[] {
    return REFRESH_TARGETS.filter((name) => {
        if (state.unsaved.includes(name)) return false;
        return name !== "openRules" || state.openRulesId !== null;
    });
}

/**
 * Surfaces holding unsaved user input, by the resource they are an edit of.
 *
 * A plain module `Set`, deliberately not `$state`: it is written from an
 * `$effect` in each editor and read only inside a click handler, so making it
 * reactive would buy nothing and add a signal that an effect both reads and
 * writes — the shape that froze the whole app once already (see
 * `routes/effectLatch.test.ts`).
 */
const unsaved = new Set<RefreshTargetName>();

/** Declare (or withdraw) an editor's claim that it holds unsaved edits of `name`. */
export function holdUnsavedEdits(name: RefreshTargetName, held: boolean): void {
    if (held) unsaved.add(name);
    else unsaved.delete(name);
}

/** What the stores say is on screen right now. */
export function currentRefreshState(): RefreshState {
    // `openRules.query` is the question the held document answers — one value
    // with the payload (FE-1), so it cannot drift from what is actually open.
    return {openRulesId: openRules.query?.id ?? null, unsaved: [...unsaved]};
}

async function reload(name: RefreshTargetName, serverUrl: string, state: RefreshState): Promise<void> {
    switch (name) {
        case "journal":
            return journal.refresh({force: true});
        case "importCapabilities":
            return importStore.reloadCapabilities(serverUrl);
        case "rulesIndex":
            return rulesStore.reloadIndex(serverUrl);
        case "openRules":
            // Guarded by `refreshPlan`; the check is here too because a null id
            // would otherwise become the string "null" in a URL.
            if (state.openRulesId !== null) await rulesStore.open(serverUrl, state.openRulesId);
            return;
        case "aliases":
            return aliasStore.reload(serverUrl);
        case "prices":
            // Which symbols need a quote, and where prices already live. Its
            // own `ensureStatus` key is set the first time /holdings is
            // opened and cannot change again while the app is running — the
            // exact shape of the aliases bug described above.
            return pricesStore.reloadStatus(serverUrl);
        case "holdings":
            // The report and trend behind the Stocks table. Keyed on
            // `(url, nonce, scope)` in `holdings/+page.svelte`, and a refresh
            // moves none of the three, so nothing else re-reads it — while
            // it is the surface most likely to have gone stale, since a price
            // update (this app's own button, or the user's own script) changes
            // every market value on it. The Other tab only once it has been
            // opened; see `pricesStore.afterWrite`.
            await Promise.all([
                holdingsData.load(serverUrl, holdingsScope.value),
                otherHoldingsData.report === null ? Promise.resolve() : otherHoldingsData.load(serverUrl, holdingsScope.value),
            ]);
            return;
    }
}

/**
 * Re-read everything on screen, and answer with the targets that failed.
 *
 * Concurrent and `allSettled`: one dead route must not strand the other four,
 * and each resource already owns its own error state — the surfaces show it.
 * Nothing here throws, so the caller may `void` it.
 */
export async function refreshEverything(): Promise<readonly RefreshTargetName[]> {
    const serverUrl = settings.serverUrl;
    if (serverUrl === null) return [];
    const state = currentRefreshState();
    const plan = refreshPlan(state);
    const settled = await Promise.allSettled(plan.map((name) => reload(name, serverUrl, state)));
    return plan.filter((_, at) => settled[at].status === "rejected");
}
