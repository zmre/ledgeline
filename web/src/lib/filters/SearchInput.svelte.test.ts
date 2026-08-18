// `/` focuses the search box, and Escape hands the keyboard back WITHOUT
// clearing. That second half is a deliberate product decision, not an
// oversight, so it gets a test — otherwise the "obvious improvement" of
// clear-on-Escape lands later and silently eats people's queries.

import {render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it} from "vitest";
import {keymap} from "$lib/keys/keymap.svelte";
import {filters} from "$lib/stores/filters.svelte";
import SearchInput from "./SearchInput.svelte";

afterEach(() => {
    keymap.reset();
    filters.setQuery("");
});

function press(key: string, target: EventTarget = document.body): void {
    const event = new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true});
    target.dispatchEvent(event);
    keymap.handle(event);
}

describe("COMPONENT SearchInput", () => {
    it("registers `/` and focuses the field", () => {
        render(SearchInput);

        press("/");

        expect(document.activeElement).toBe(screen.getByLabelText("Search transactions"));
    });

    it("advertises `/` in the help sheet", () => {
        // The binding is registered by this component, so the help row exists
        // only where a search box does — which is why Reports and Imports get no
        // `/` without anyone maintaining a list of which pages have one.
        render(SearchInput);

        expect(keymap.active.some((binding) => binding.keys === "/" && binding.label === "Search transactions")).toBe(true);
    });

    it("does not register `/` once unmounted", () => {
        const view = render(SearchInput);
        view.unmount();

        expect(keymap.active.filter((binding) => binding.keys === "/")).toHaveLength(0);
    });

    it("blurs on Escape but keeps the typed query", () => {
        filters.setQuery("plumber");
        render(SearchInput);
        const field = screen.getByLabelText("Search transactions");
        press("/");

        press("Escape", field);

        expect(document.activeElement).not.toBe(field);
        // The point of the test: Escape is "give me the keyboard back", not
        // "throw away what I typed".
        expect(filters.value.query).toBe("plumber");
    });
});
