// The column menu opens.
//
// That sentence is the whole point of this file. The menu was a daisyUI
// dropdown whose open state lived entirely in CSS — `.dropdown-content` is
// `display:none` until the wrapper matches `:focus-within` — so opening it
// depended on a `<button>` taking focus when clicked, which on macOS WebKit it
// does not. The control was unopenable by mouse in the WKWebView this app ships
// in, and no test could see it: jsdom has no CSS engine, so `:focus-within`
// resolves to nothing here and a CSS-only dropdown is untestable BY
// CONSTRUCTION.
//
// `<details>` moves that state into the DOM, where it can be asserted. Keeping
// these assertions honest therefore means keeping the mechanism honest — if
// someone converts this back to a focus-driven dropdown, the first test fails.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it} from "vitest";
import {settings} from "$lib/stores/settings.svelte";
import ColumnMenu from "./ColumnMenu.svelte";

/**
 * Let the DOM and Svelte both catch up.
 *
 * `<details>` fires `toggle` as a TASK, not a microtask, so `bind:open` has not
 * seen the click yet when `tick()` alone resolves — and the Escape listener is
 * attached by an `$effect` that depends on the value `bind:open` is about to
 * write. One macrotask, then one flush.
 */
async function settle(): Promise<void> {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await tick();
}

let columnsBefore: typeof settings.columns;

beforeEach(() => {
    columnsBefore = settings.columns;
});

afterEach(() => {
    settings.columns = columnsBefore;
});

describe("COMPONENT ColumnMenu", () => {
    it("REGRESSION: opens on a click, with no dependence on the trigger taking focus", () => {
        const {container} = render(ColumnMenu);
        const details = container.querySelector("details");
        const summary = container.querySelector("summary");

        expect(details?.open).toBe(false);
        summary?.click();
        expect(details?.open).toBe(true);
    });

    it("keeps its open state in the DOM rather than in a CSS pseudo-class", () => {
        // The mechanism, pinned. A `<div class="dropdown">` holding a `<button>`
        // renders identically here and is broken in the browser, so asserting
        // the visible result is not enough — the CAUSE has to be asserted.
        const {container} = render(ColumnMenu);

        expect(container.querySelector("details.dropdown")).not.toBeNull();
        expect(container.querySelector("details > summary")).not.toBeNull();
        // The old markup, which must not come back.
        expect(container.querySelector("div.dropdown > button")).toBeNull();
    });

    it("closes again on a second click and on Escape", async () => {
        const {container} = render(ColumnMenu);
        const details = container.querySelector("details");
        const summary = container.querySelector("summary");

        summary?.click();
        await settle();
        expect(details?.open).toBe(true);

        summary?.click();
        await settle();
        expect(details?.open).toBe(false);

        summary?.click();
        await settle();
        document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape"}));
        await settle();
        expect(details?.open).toBe(false);
    });

    it("still toggles the columns it is there to toggle", () => {
        render(ColumnMenu);
        const before = settings.columns.status;

        screen.getByRole("checkbox", {name: "Status"}).click();

        expect(settings.columns.status).toBe(!before);
    });

    it("offers every column", () => {
        render(ColumnMenu);

        for (const label of ["Date", "Status", "Description", "Accounts", "Amount"]) {
            expect(screen.getByRole("checkbox", {name: label})).toBeDefined();
        }
    });
});
