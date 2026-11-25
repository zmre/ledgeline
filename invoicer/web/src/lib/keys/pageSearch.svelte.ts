// The current page's search box, for `/` (PLAN 5k, Y5). A list's filter bar
// marks its search input with `use:searchKeys`; the app layer's `/` focuses
// whichever is mounted, and is off (and off the `?` sheet) where none is.
//
// In the box, Escape clears what is typed (sending the empty search at
// once), and a second Escape leaves the box, so `j`/`k` drive the list
// again. The box owns Escape: it prevents the default (a search field's own
// clear, which would not reach the list's URL) and the keymap then leaves the
// key alone.

import type {ActionReturn} from "svelte/action";

let current = $state<HTMLInputElement | null>(null);

export const pageSearch = {
    /** A search box is mounted. Reactive: the `/` binding's `enabled`. */
    get present(): boolean {
        return current !== null;
    },
    /** Focus the box and select its text, so typing replaces it. */
    focus(): void {
        current?.focus();
        current?.select();
    },
};

export interface SearchKeysOptions {
    /** Empty the search and send it. */
    clear: () => void;
}

export function searchKeys(node: HTMLInputElement, options: SearchKeysOptions): ActionReturn<SearchKeysOptions> {
    let clear = options.clear;
    current = node;

    function onKeydown(event: KeyboardEvent): void {
        if (event.key !== "Escape" || event.defaultPrevented) return;
        event.preventDefault();
        if (node.value !== "") clear();
        else node.blur();
    }

    node.addEventListener("keydown", onKeydown);
    return {
        update(next: SearchKeysOptions): void {
            clear = next.clear;
        },
        destroy(): void {
            node.removeEventListener("keydown", onKeydown);
            if (current === node) current = null;
        },
    };
}
