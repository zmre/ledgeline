// The invoicer's app-wide layer (PLAN 5k, Y3): help, search, New invoice (`n`),
// New deposit invoice (`d`; Shift+N opens the whole New menu) and
// the `g` go-to chords, one per sidebar section. Registered once by the root
// layout. Plain `.ts`: it reads runes-backed state (`page`, `pageSearch`)
// but declares none.
//
// `g g` and `G` stay free for lists (first and last row), as in Ledgeline.
//
// Escape leaves a field (Y5): single-letter keys are off while typing, so
// without a way out of a field a keyboard user would have to Tab to a
// button before Shift+S could send. It is the lowest layer's, so anything
// that means something by Escape claims it first: an open list in a
// combobox, the search box (clear, then leave), a dialog (cancel), the
// customer form (Cancel). In the template's code, CodeMirror keeps focus:
// there Escape is the first half of Escape-then-Tab (D-decisions).

import {goto} from "$app/navigation";
import {resolve} from "$app/paths";
import {page} from "$app/state";
import {keymap} from "#ui/keys/keymap.svelte.js";
import {PRIORITY, type Layer} from "#ui/keys/types.js";
import {pageSearch} from "./pageSearch.svelte.js";

type Section = "/" | "/invoices" | "/customers" | "/catalog" | "/snippets" | "/templates" | "/new";

/** Go to a page (with `search`, a query such as `?deposit=1`); already there, nothing. */
function to(path: Section, search = ""): () => void {
    return () => {
        const target = resolve(path);
        if (page.url.pathname === target && page.url.search === search) return;
        // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
        void goto(`${target}${search}`);
    };
}

/** Blur the focused field, unless it is CodeMirror's. */
function leaveField(): void {
    const focused = document.activeElement;
    if (!(focused instanceof HTMLElement) || focused === document.body) return;
    if (focused.closest(".cm-editor") !== null) return;
    focused.blur();
}

export function appLayer(): Layer {
    return {
        id: "invoicer-app",
        priority: PRIORITY.page,
        bindings: [
            {keys: "?", label: "Show keyboard shortcuts", group: "Global", run: () => keymap.toggleHelp()},
            {keys: "/", label: "Search this list", group: "Global", enabled: () => pageSearch.present, run: () => pageSearch.focus()},
            {keys: "n", label: "New invoice", group: "Global", run: to("/new")},
            {keys: "d", label: "New deposit invoice", group: "Global", run: to("/new", "?deposit=1")},
            {keys: "Escape", label: "Leave the field", group: "Global", fields: true, run: leaveField},
            {keys: "g d", label: "Go to Dashboard", group: "Go to", run: to("/")},
            {keys: "g i", label: "Go to Invoices", group: "Go to", run: to("/invoices")},
            {keys: "g c", label: "Go to Customers", group: "Go to", run: to("/customers")},
            {keys: "g k", label: "Go to Catalog", group: "Go to", run: to("/catalog")},
            {keys: "g s", label: "Go to Snippets", group: "Go to", run: to("/snippets")},
            {keys: "g t", label: "Go to Templates", group: "Go to", run: to("/templates")},
        ],
    };
}
