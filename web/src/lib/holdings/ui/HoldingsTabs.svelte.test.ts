// The Holdings tab strip, mounted.
//
// `params.test.ts` proves the vocabulary and `urlCodec.test.ts` proves the
// round-trip; neither says a click or a digit reaches either of them. The strip
// is also where the e2e convention lives (click by role, assert `aria-selected`),
// so it is worth asserting that convention holds here — in milliseconds, without
// a browser — rather than only in Playwright.

import {readFileSync} from "node:fs";
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

/** Press a key ON an element, the way a focused tab receives it (bubbles to Svelte's delegated listener). */
async function pressOn(el: HTMLElement, key: string): Promise<void> {
    el.dispatchEvent(new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true}));
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

    // The rest of the WAI-ARIA tabs pattern. role="tab" alone is a promise the
    // keyboard must keep: without the roving tabindex every tab is a Tab stop,
    // and without the arrow keys a screen-reader user is TOLD "tab, 1 of 2" and
    // then finds the advertised interaction missing.

    it("roves the tabindex: only the selected tab sits in the Tab order", async () => {
        render(HoldingsTabs, {tab: "stocks"});
        const stocks = screen.getByRole("tab", {name: "Stocks"});
        const other = screen.getByRole("tab", {name: "Other"});

        expect(stocks.getAttribute("tabindex")).toBe("0");
        expect(other.getAttribute("tabindex")).toBe("-1");

        other.click();
        await tick();
        expect(stocks.getAttribute("tabindex")).toBe("-1");
        expect(other.getAttribute("tabindex")).toBe("0");
    });

    it("moves selection AND focus with ArrowRight/ArrowLeft, wrapping at the ends", async () => {
        render(HoldingsTabs, {tab: "stocks"});
        const stocks = screen.getByRole("tab", {name: "Stocks"});
        const other = screen.getByRole("tab", {name: "Other"});

        stocks.focus();
        await pressOn(stocks, "ArrowRight");
        expect(selected()).toBe("Other");
        expect(document.activeElement).toBe(other); // focus follows, or the roving tabindex strands the keyboard

        // Right from the last tab wraps to the first…
        await pressOn(other, "ArrowRight");
        expect(selected()).toBe("Stocks");
        expect(document.activeElement).toBe(stocks);

        // …and Left wraps the other way.
        await pressOn(stocks, "ArrowLeft");
        expect(selected()).toBe("Other");
        expect(document.activeElement).toBe(other);
    });

    it("wires every tab to the switched panel by id", () => {
        render(HoldingsTabs, {tab: "stocks"});

        for (const t of TAB_ORDER) {
            const el = screen.getByRole("tab", {name: TAB_LABELS[t]});
            expect(el.id).toBe(`holdings-tab-${t}`);
            expect(el.getAttribute("aria-controls")).toBe("holdings-panel");
        }
    });

    it("…and the holdings page renders that panel as a tabpanel labelled by the active tab", () => {
        // Source-text, in the branchOrder.test.ts tradition: the panel half of
        // the pattern lives in the route, which mounts stores, fetches and URL
        // sync this suite has no business faking. The two halves meet as literal
        // strings — `holdings-panel`, `holdings-tab-{id}` — so text is the
        // honest seam to pin them at. Cwd-relative like alertStacking.test.ts
        // (vitest runs from web/): under jsdom, import.meta.url is not file://.
        const page = readFileSync("src/routes/holdings/+page.svelte", "utf8");

        expect(page).toContain('id="holdings-panel"');
        expect(page).toContain('role="tabpanel"');
        expect(page).toContain('aria-labelledby="holdings-tab-{holdingsTab.value}"');
    });
});
