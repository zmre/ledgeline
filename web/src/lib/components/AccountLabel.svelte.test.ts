// Mounting the account label.
//
// jsdom has no layout engine, so the half of this component that is CSS — which
// end the ellipsis eats — cannot be asserted here and is not tried. What CAN
// rot silently, and is what this pins, is the OTHER half: an abbreviation is a
// visual convenience, and the moment it becomes the only string in the DOM the
// name stops being searchable by find-in-page, stops being readable by a screen
// reader, and stops being copyable. The full name has to be present whether or
// not the visible text is short enough, and the tooltip has to keep saying it.

import {render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import AccountLabel from "./AccountLabel.svelte";
import {resetChipMeasurer} from "./textWidth";

/**
 * Give jsdom a 2D canvas it does not have, in a monospaced 6px font.
 *
 * This does NOT make jsdom able to lay anything out — it cannot, and no test
 * here claims otherwise. What it does is let the MEASURED code path run at all,
 * so the plumbing between `maxWidth` and the string that ends up in the DOM is
 * exercised rather than skipped. Whether 6px is anyone's real font is beside the
 * point; that the label asks, and honours the answer, is the part that broke.
 */
const PX_PER_CHAR = 6;

function withMeasurableFont(): void {
    resetChipMeasurer();
    const context = {font: "", measureText: (text: string) => ({width: text.length * PX_PER_CHAR})};
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(context as never);
}

afterEach(() => {
    vi.restoreAllMocks();
    resetChipMeasurer();
});

/** The visible reading: everything the eye gets, with the sr-only copy removed. */
function visible(container: HTMLElement): string {
    const clone = container.cloneNode(true) as HTMLElement;
    for (const hidden of clone.querySelectorAll(".sr-only")) hidden.remove();
    return clone.textContent?.trim() ?? "";
}

describe("COMPONENT AccountLabel", () => {
    it("shows a name that fits verbatim, with no second copy of it", () => {
        const {container} = render(AccountLabel, {name: "expenses:auto:maintenance"});

        expect(visible(container)).toBe("expenses:auto:maintenance");
        // No abbreviation means no aria-hidden/sr-only split to go stale.
        expect(container.querySelector("[aria-hidden='true']")).toBeNull();
        expect(container.querySelector(".sr-only")).toBeNull();
    });

    it("abbreviates the ancestors of a name that does not fit, never the leaf", () => {
        const {container} = render(AccountLabel, {name: "assets:morganstanley:pw-roth-ira:cash"});

        expect(visible(container)).toBe("ass:mor:pw-roth-ira:cash");
    });

    it("still hands assistive technology the whole name", () => {
        const {container} = render(AccountLabel, {name: "assets:morganstanley:pw-roth-ira:cash"});

        // The shortened text is hidden from AT, and the real name is in the
        // accessibility tree beside it — the same split RenameList makes.
        expect(container.querySelector("[aria-hidden='true']")?.textContent).toBe("ass:mor:pw-roth-ira:cash");
        expect(screen.getByText("assets:morganstanley:pw-roth-ira:cash")).toBeDefined();
    });

    it("keeps the full name in the tooltip, abbreviated or not", () => {
        const long = render(AccountLabel, {name: "assets:morganstanley:pw-roth-ira:cash"});
        expect(long.container.querySelector("[title]")?.getAttribute("title")).toBe("assets:morganstanley:pw-roth-ira:cash");

        const short = render(AccountLabel, {name: "assets:bank:checking"});
        expect(short.container.querySelector("[title]")?.getAttribute("title")).toBe("assets:bank:checking");
    });

    it("REGRESSION: shows a long name whole when the measured chip has room for it", () => {
        // The reported bug. `assets:morganstanley:pw-roth-ira:cash` is 37
        // characters, so the old fixed budget of 30 abbreviated it in EVERY
        // chip at EVERY window size. At 6px a character it is 222px wide, and
        // a chip with 400px of room was showing 24 characters in it.
        withMeasurableFont();
        const name = "assets:morganstanley:pw-roth-ira:cash";
        const {container} = render(AccountLabel, {name, maxWidth: 400});

        expect(visible(container)).toBe(name);
        // Not abbreviated at all, so there is no aria-hidden/sr-only split.
        expect(container.querySelector("[aria-hidden='true']")).toBeNull();
    });

    it("still abbreviates once the measured chip genuinely runs out", () => {
        withMeasurableFont();
        const name = "assets:morganstanley:pw-roth-ira:cash";
        const {container} = render(AccountLabel, {name, maxWidth: 150});

        expect(visible(container)).toBe("ass:mor:pw-roth-ira:cash");
        // …and the whole name is still there for AT, find-in-page and copy.
        expect(screen.getByText(name)).toBeDefined();
        expect(container.querySelector("[title]")?.getAttribute("title")).toBe(name);
    });

    it("widens what it shows as the chip widens, rather than holding one budget", () => {
        withMeasurableFont();
        const name = "expenses:household:repairs:plumbing";
        const shown = [60, 120, 180, 240].map((maxWidth) => visible(render(AccountLabel, {name, maxWidth}).container));

        // Monotone: more room is never less name.
        expect(shown).toEqual(["e:h:r:plumbing", "exp:hou:rep:plumbing", "exp:household:repairs:plumbing", name]);
    });

    it("falls back to the character budget on an engine that cannot measure", () => {
        // Plain jsdom: no 2D canvas, so `maxWidth` cannot be honoured. The label
        // must degrade to the coarse budget rather than to zero or to a guess.
        resetChipMeasurer();
        const {container} = render(AccountLabel, {name: "assets:morganstanley:pw-roth-ira:cash", maxWidth: 4000});

        expect(visible(container)).toBe("ass:mor:pw-roth-ira:cash");
    });

    it("lets a narrower caller ask for a smaller budget", () => {
        const {container} = render(AccountLabel, {name: "expenses:auto:maintenance", budget: 16});

        expect(visible(container)).toBe("e:a:maintenance");
        expect(screen.getByText("expenses:auto:maintenance")).toBeDefined();
    });

    it("lets a caller supply its own tooltip", () => {
        // The journal chips are edit buttons and say so; the name is still in it.
        const {container} = render(AccountLabel, {name: "expenses:auto:maintenance", title: "Edit category · expenses:auto:maintenance"});

        expect(container.querySelector("[title]")?.getAttribute("title")).toBe("Edit category · expenses:auto:maintenance");
    });

    it("turns the ellipsis around by direction, not by cutting the string itself", () => {
        // The leaf survives at ANY width because the CSS eats the left edge —
        // `dir="rtl"` is what moves it there, and `text-left` is what keeps the
        // text where it was. Losing either silently restores the old bug, and
        // jsdom cannot see the result, so the cause is what gets pinned.
        const {container} = render(AccountLabel, {name: "expenses:auto:maintenance"});
        const label = container.querySelector("span");

        expect(label?.getAttribute("dir")).toBe("rtl");
        expect(label?.className).toContain("truncate");
        expect(label?.className).toContain("text-left");
        // …and the name itself is isolated, so an RTL line cannot reorder it.
        expect(label?.querySelector("bdi")).not.toBeNull();
    });
});
