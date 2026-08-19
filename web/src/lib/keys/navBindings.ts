// The global layer: help, and `g`-prefixed route jumps.
//
// The only file in `lib/keys/` that imports `$app/*` — keeping the router out of
// `keymap.svelte.ts` is what lets the core be tested in the node project with no
// SvelteKit around it.
//
// Plain `.ts`, not `.svelte.ts`: it READS runes-backed state (`page`,
// `rulesStore`) but declares none, so the naming rule does not apply.

import {goto} from "$app/navigation";
import {resolve} from "$app/paths";
import {page} from "$app/state";
import {rulesStore} from "$lib/imports/rulesStore.svelte";
import {keymap} from "./keymap.svelte";
import {PRIORITY, type Layer} from "./types";

type Route = "/" | "/holdings" | "/reports" | "/imports";

/**
 * No `svelte/no-navigation-without-resolve` disable is needed anywhere in this
 * feature: routes go through `goto(resolve(...))` verbatim, and the
 * query-string TABS are page state written directly, never a navigation.
 */
function to(path: Route): () => void {
    return () => {
        const target = resolve(path);
        // Same guard as ProblemsDrawer.jumpTo: re-navigating to where you
        // already are resets scroll for nothing.
        if (page.url.pathname !== target) void goto(target);
    };
}

/**
 * `g j` for the journal rather than `g g`, because `g g` is vim's "top of
 * document" and binding it to a route would fight muscle memory hard. `g g` and
 * `G` stay reserved for whichever surface owns the current scroll container.
 */
export function globalLayer(): Layer {
    return {
        id: "global",
        priority: PRIORITY.page,
        bindings: [
            {keys: "?", label: "Show keyboard shortcuts", group: "Global", run: () => keymap.toggleHelp()},
            {keys: "g j", label: "Go to Journal", group: "Navigation", run: to("/")},
            {keys: "g h", label: "Go to Holdings", group: "Navigation", run: to("/holdings")},
            {keys: "g r", label: "Go to Reports", group: "Navigation", run: to("/reports")},
            {
                keys: "g i",
                label: "Go to Imports",
                group: "Navigation",
                run: to("/imports"),
                // Mirrors the nav item, which is hidden on an engine with no
                // `/api/rules` route. A hidden destination should not be reachable.
                enabled: () => rulesStore.available,
            },
        ],
    };
}
