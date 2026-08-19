// The Holdings tab strip, mounted.
//
// `params.test.ts` proves the vocabulary and `urlCodec.test.ts` proves the
// round-trip; neither says a click or a digit reaches either of them. The strip
// is also where the e2e convention lives (click by role, assert `aria-selected`),
// so it is worth asserting that convention holds here — in milliseconds, without
// a browser — rather than only in Playwright.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {TAB_LABELS, TAB_ORDER} from "$lib/holdings/params";
import {keymap} from "$lib/keys/keymap.svelte";
import HoldingsTabs from "./HoldingsTabs.svelte";

/** Press a key the way the app's window listener would. */
async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

const selected = (): string | null => screen.getByRole("tablist").querySelector('[aria-selected="true"]')?.textContent?.trim() ?? null;

afterEach(() => {
    keymap.reset();
});

describe("COMPONENT HoldingsTabs", () => {
    it("renders one tab per id, in TAB_ORDER, inside a labelled tablist", () => {
        render(HoldingsTabs, {tab: "stocks"});
        const list = screen.getByRole("tablist");

        expect(list.getAttribute("aria-label")).toBe("Holdings");
        expect([...list.querySelectorAll('[role="tab"]')].map((t) => t.textContent?.trim())).toEqual(TAB_ORDER.map((t) => TAB_LABELS[t]));
    });

    it("marks exactly the active tab, so the e2e aria-selected convention holds", () => {
        render(HoldingsTabs, {tab: "other"});

        expect(screen.getByRole("tablist").querySelectorAll('[aria-selected="true"]')).toHaveLength(1);
        expect(selected()).toBe("Other");
    });

    it("switches on click", async () => {
        render(HoldingsTabs, {tab: "stocks"});

        screen.getByRole("tab", {name: "Other"}).click();
        await tick();

        expect(selected()).toBe("Other");
    });

    it("binds one digit per tab, in order", async () => {
        render(HoldingsTabs, {tab: "stocks"});

        await press("2");
        expect(selected()).toBe("Other");

        await press("1");
        expect(selected()).toBe("Stocks");
    });

    it("files those digits under Holdings, not under Reports or Journal", async () => {
        render(HoldingsTabs, {tab: "stocks"});
        await tick();

        expect(keymap.help.map((section) => section.group)).toEqual(["Holdings"]);
    });
});
