// The row menu's behaviour, minus anything positional — jsdom has no layout
// engine, so where the menu lands is `anchoredPopup.test.ts` (pure) plus
// Playwright. What is testable here is the portal, the dismissals and the keys.
//
// Every test below is a defect the `<details class="dropdown">` this replaced
// actually shipped: the menu was clipped by the table's `overflow-x-auto` and
// invisible for the rows nearest the bottom of a section, Escape did nothing
// (the header claimed `<details>` closed on it — that is `<dialog>`), clicking
// the trigger again did nothing, and `bg-base-100` made the menu the same
// colour as the page behind it.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {describe, expect, it} from "vitest";
import RowMenu from "./RowMenu.svelte";

const LABEL = "Row menu for expenses:housing";

function mount(items?: {label: string; onSelect: () => void; danger?: boolean}[]) {
    const chosen: string[] = [];
    const view = render(RowMenu, {
        props: {
            label: LABEL,
            items: items ?? [
                {label: "Add a step on 2027-04-01", onSelect: () => chosen.push("step")},
                {label: "Duplicate", onSelect: () => chosen.push("duplicate")},
                {label: "Delete row", onSelect: () => chosen.push("delete"), danger: true},
            ],
        },
    });
    return {view, chosen, trigger: screen.getByLabelText(LABEL) as HTMLButtonElement};
}

async function open(trigger: HTMLElement): Promise<HTMLElement> {
    await fireEvent.click(trigger);
    await tick();
    return screen.getByTestId("row-menu");
}

/** A real pointerdown on `target`, which is what the outside-click guard listens for. */
async function pointerDown(target: EventTarget): Promise<void> {
    target.dispatchEvent(new MouseEvent("pointerdown", {bubbles: true, cancelable: true}));
    await tick();
}

describe("COMPONENT RowMenu — the portal", () => {
    it("is shut until the trigger is pressed", () => {
        mount();

        expect(screen.queryByRole("menu")).toBeNull();
    });

    it("REGRESSION: portals the menu to <body>, out of the scroller that clipped it", async () => {
        // Load-bearing, not tidiness. Both Projections tables are wrapped in
        // `overflow-x-auto`, and a non-`visible` overflow on one axis computes
        // the other to `auto` — so an in-flow menu was clipped on BOTH axes and
        // the last rows of a section opened into nothing at all.
        const {trigger, view} = mount();

        const menu = await open(trigger);

        expect(menu.parentElement).toBe(document.body);
        expect(view.container.contains(menu)).toBe(false);
    });

    it("takes the portalled menu with it when it unmounts", async () => {
        // The other half of a portal: a node moved out of its own fragment must
        // still be cleaned up, or deleting a row leaks its menu into <body>.
        const {trigger, view} = mount();
        await open(trigger);

        view.unmount();
        await tick();

        expect(screen.queryByRole("menu")).toBeNull();
        expect(document.body.querySelector("[data-testid=row-menu]")).toBeNull();
    });

    it("says whether it is open, so the trigger is not a mystery button", async () => {
        const {trigger} = mount();
        expect(trigger.getAttribute("aria-expanded")).toBe("false");

        await open(trigger);

        expect(trigger.getAttribute("aria-expanded")).toBe("true");
    });
});

describe("COMPONENT RowMenu — dismissal", () => {
    it("REGRESSION: closes on Escape", async () => {
        // A native `<details>` does NOT close on Escape. `<dialog>` does, and
        // the comment that claimed otherwise is why an opened menu could not be
        // got rid of at all.
        const {trigger} = mount();
        await open(trigger);

        document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true}));
        await tick();

        expect(screen.queryByRole("menu")).toBeNull();
    });

    it("puts focus back on the trigger when it closes", async () => {
        // The menu is portalled to the end of <body>, so focus left where it
        // was would strand a keyboard user a whole document away from the row.
        const {trigger} = mount();
        await open(trigger);

        document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true}));
        await tick();

        expect(document.activeElement).toBe(trigger);
    });

    it("REGRESSION: closes on a click outside it", async () => {
        const {trigger} = mount();
        await open(trigger);

        await pointerDown(document.body);

        expect(screen.queryByRole("menu")).toBeNull();
    });

    it("REGRESSION: the trigger closes what it opened", async () => {
        // The subtle one. A generic outside-click guard does not know about the
        // trigger, so pointerdown would dismiss and the click behind it would
        // re-open — leaving `⋯` looking like it did nothing, for ever.
        const {trigger} = mount();
        await open(trigger);

        await pointerDown(trigger);
        await fireEvent.click(trigger);
        await tick();

        expect(screen.queryByRole("menu")).toBeNull();
    });

    it("stays open when the click is inside the menu itself", async () => {
        const {trigger} = mount();
        const menu = await open(trigger);

        await pointerDown(menu);

        expect(screen.queryByRole("menu")).not.toBeNull();
    });

    it("closes on a scroll anywhere, so it cannot drift off its row", async () => {
        // The menu is `position: fixed` against a row inside a scroller that
        // `<svelte:window onscroll>` never sees. Closing beats drifting.
        const {trigger} = mount();
        await open(trigger);

        document.body.dispatchEvent(new Event("scroll", {bubbles: false}));
        await tick();

        expect(screen.queryByRole("menu")).toBeNull();
    });
});

describe("COMPONENT RowMenu — the items", () => {
    it("renders real buttons, so a test finds them by role and name", async () => {
        const {trigger} = mount();
        const menu = await open(trigger);

        const items = within(menu).getAllByRole("menuitem");
        expect(items.map((i) => i.textContent?.trim())).toEqual(["Add a step on 2027-04-01", "Duplicate", "Delete row"]);
        expect(items.map((i) => i.tagName)).toEqual(["BUTTON", "BUTTON", "BUTTON"]);
    });

    it("runs the item and closes", async () => {
        const {trigger, chosen} = mount();
        const menu = await open(trigger);

        await fireEvent.click(within(menu).getByRole("menuitem", {name: "Delete row"}));
        await tick();

        expect(chosen).toEqual(["delete"]);
        expect(screen.queryByRole("menu")).toBeNull();
    });

    it("moves focus with the arrow keys, and wraps", async () => {
        // A portalled menu is nowhere near its trigger in tab order, so the
        // keyboard has to be given a way in and around.
        const {trigger} = mount();
        const menu = await open(trigger);
        const items = within(menu).getAllByRole("menuitem");
        // `dismissible`'s trap focuses the first item on open.
        expect(document.activeElement).toBe(items[0]);

        await fireEvent.keyDown(menu, {key: "ArrowDown"});
        expect(document.activeElement).toBe(items[1]);

        await fireEvent.keyDown(menu, {key: "ArrowUp"});
        await fireEvent.keyDown(menu, {key: "ArrowUp"});
        expect(document.activeElement).toBe(items[2]);
    });
});

describe("COMPONENT RowMenu — the surface", () => {
    it("REGRESSION: reads as a layer above the page, not a hole in it", async () => {
        // `bg-base-100` is what `+layout.svelte` paints the page, so the old
        // menu was exactly the colour of what it covered, with a shadow and no
        // border as the only hint it was there. This is the surface
        // `AccountInput`'s popup and `ColumnMenu` already use.
        const {trigger} = mount();
        const menu = await open(trigger);

        expect(menu.className).toContain("bg-base-200");
        expect(menu.className).not.toContain("bg-base-100");
        expect(menu.className).toContain("border-base-300");
    });
});
