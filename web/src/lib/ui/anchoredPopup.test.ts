// Pure positioning arithmetic. It lives apart from the component precisely so it
// can be tested: jsdom reports every rect as zero, so a component test could not
// tell a correct placement from a broken one.

import {describe, expect, it} from "vitest";
import {menuPosition, popupPosition} from "./anchoredPopup";

const VIEWPORT = {width: 1024, height: 768};
const rect = (top: number, height = 32, left = 100, width = 240) => ({top, bottom: top + height, left, width});

describe("UNIT popupPosition", () => {
    it("sits just below the input when there is room", () => {
        const placed = popupPosition(rect(100), VIEWPORT, 224);

        expect(placed.below).toBe(true);
        expect(placed.top).toBe(134);
    });

    it("matches the input's width and left edge", () => {
        const placed = popupPosition(rect(100), VIEWPORT, 224);

        expect({left: placed.left, width: placed.width}).toEqual({left: 100, width: 240});
    });

    it("flips above when the input is near the bottom", () => {
        // The case that matters in the transaction popup: the last posting row
        // sits low, and a popup below it would be off screen.
        const placed = popupPosition(rect(720), VIEWPORT, 224);

        expect(placed.below).toBe(false);
        expect(placed.top).toBeLessThan(720);
    });

    it("stays below when neither side has much room, preferring the roomier one", () => {
        // A short viewport. Flip-flopping between sides as the list resizes is
        // worse than being cramped, so the choice is made on which side is bigger.
        const placed = popupPosition(rect(40, 32), {width: 1024, height: 200}, 224);

        expect(placed.below).toBe(true);
    });

    it("never proposes a negative height", () => {
        const placed = popupPosition(rect(0, 0), {width: 1024, height: 10}, 224);

        expect(placed.maxHeight).toBeGreaterThanOrEqual(0);
    });

    it("clamps a right-edge input back inside the viewport", () => {
        const placed = popupPosition(rect(100, 32, 900, 240), VIEWPORT, 224);

        expect(placed.left + placed.width).toBeLessThanOrEqual(VIEWPORT.width);
    });

    it("narrows a popup wider than the viewport", () => {
        const placed = popupPosition(rect(100, 32, 0, 2000), {width: 320, height: 768}, 224);

        expect(placed.width).toBeLessThanOrEqual(320);
        expect(placed.left).toBeGreaterThanOrEqual(0);
    });

    it("caps maxHeight to the room actually available", () => {
        // The popup scrolls internally rather than overflowing the screen.
        const placed = popupPosition(rect(600), VIEWPORT, 400);

        expect(placed.maxHeight).toBeLessThanOrEqual(VIEWPORT.height - 632);
    });
});

describe("UNIT menuPosition", () => {
    /** A row menu's trigger: the 24px `⋯` button in a table's last column. */
    const TRIGGER = rect(300, 24, 880, 24);
    const MENU = {width: 208, height: 124};

    it("takes its OWN width, not the trigger's", () => {
        // The whole reason this is not `popupPosition`: a 24px trigger would
        // otherwise give a 24px menu.
        const placed = menuPosition(TRIGGER, VIEWPORT, MENU);

        expect(placed.width).toBe(208);
    });

    it("aligns its RIGHT edge to the trigger's", () => {
        // daisyUI's `dropdown-end`, as arithmetic. Left-aligning a menu on a
        // trigger this close to the right edge would push it off screen and
        // then clamp it away from the row it belongs to.
        const placed = menuPosition(TRIGGER, VIEWPORT, MENU);

        expect(placed.left + placed.width).toBe(TRIGGER.left + TRIGGER.width);
    });

    it("sits just below the trigger when there is room", () => {
        const placed = menuPosition(TRIGGER, VIEWPORT, MENU);

        expect(placed.below).toBe(true);
        expect(placed.top).toBe(326);
    });

    it("flips above for a row at the bottom of a long table", () => {
        // THE case that made the old menu invisible: the last rows of a section
        // opened downwards into clipped space.
        const placed = menuPosition(rect(730, 24, 880, 24), VIEWPORT, MENU);

        expect(placed.below).toBe(false);
        expect(placed.top).toBeLessThan(730);
    });

    it("never starts left of the viewport margin", () => {
        // A narrow window, where right-aligning would put the menu off the left
        // edge instead.
        const placed = menuPosition(rect(300, 24, 40, 24), {width: 200, height: 768}, MENU);

        expect(placed.left).toBeGreaterThanOrEqual(8);
        expect(placed.left + placed.width).toBeLessThanOrEqual(200);
    });
});
