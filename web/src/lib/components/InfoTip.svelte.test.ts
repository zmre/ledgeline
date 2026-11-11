// The shared (i) help button: its accessible name and description, and that it
// opens on hover, focus and tap and closes on leave, blur and Escape; that the
// tooltip escapes its card (portalled to <body>); and, because jsdom has no
// layout engine, the placement arithmetic as the pure `tipPosition`.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {tipPosition} from "#lib/ui/anchoredPopup.js";
import InfoTip from "./InfoTip.svelte";

const HELP = {label: "DSO", title: "DSO — Days Sales Outstanding", text: "How many days customers take to pay you.", formula: "receivables ÷ revenue × days"};

function mount(props: Record<string, unknown> = HELP) {
    render(InfoTip, {props: props as typeof HELP});
    const button = screen.getByRole("button", {name: "About DSO"});
    const wrapper = screen.getByTestId("info-tip");
    const tip = screen.getByRole("tooltip", {hidden: true});
    // Open is both the wrapper's state and the tooltip actually being shown.
    const isOpen = () => {
        const open = wrapper.dataset.state === "open";
        expect(tip.classList.contains("hidden")).toBe(!open);
        return open;
    };
    return {button, isOpen};
}

describe("COMPONENT InfoTip", () => {
    it("labels the button 'About <label>' and describes it with the tooltip", () => {
        const {button} = mount();
        const tip = screen.getByRole("tooltip", {hidden: true});
        expect(button.getAttribute("aria-describedby")).toBe(tip.id);
        expect(tip.textContent).toContain("DSO — Days Sales Outstanding");
        expect(tip.textContent).toContain("How many days customers take to pay you.");
        expect(tip.textContent).toContain("receivables ÷ revenue × days");
    });

    it("leaves out the title and formula lines when not given", () => {
        mount({label: "DSO", text: "Just a sentence."});
        expect(screen.getByRole("tooltip", {hidden: true}).children).toHaveLength(1);
    });

    it("is closed until hovered, and closes when the pointer leaves", async () => {
        const {button, isOpen} = mount();
        expect(isOpen()).toBe(false);
        await fireEvent.mouseEnter(button);
        expect(isOpen()).toBe(true);
        await fireEvent.mouseLeave(button);
        expect(isOpen()).toBe(false);
    });

    it("opens on keyboard focus and closes on blur", async () => {
        const {button, isOpen} = mount();
        await fireEvent.focus(button);
        expect(isOpen()).toBe(true);
        await fireEvent.blur(button);
        expect(isOpen()).toBe(false);
    });

    it("stays open after a tap until focus leaves", async () => {
        const {button, isOpen} = mount();
        await fireEvent.click(button);
        await fireEvent.mouseLeave(button);
        expect(isOpen()).toBe(true);
        await fireEvent.blur(button);
        expect(isOpen()).toBe(false);
    });

    it("Escape hides it, even while hovered and focused, until it is hovered again", async () => {
        const {button, isOpen} = mount();
        await fireEvent.mouseEnter(button);
        await fireEvent.focus(button);
        await fireEvent.keyDown(button, {key: "Escape"});
        expect(isOpen()).toBe(false);
        await fireEvent.mouseLeave(button);
        await fireEvent.mouseEnter(button);
        expect(isOpen()).toBe(true);
    });

    it("portals the tooltip to <body>, out of any card that would clip or bury it", () => {
        const {container} = render(InfoTip, {props: HELP});
        const tip = screen.getByRole("tooltip", {hidden: true});
        expect(tip.parentElement).toBe(document.body);
        expect(container.contains(tip)).toBe(false);
        expect(tip.classList.contains("fixed")).toBe(true);
    });

    it("takes the portalled tooltip with it when it unmounts", () => {
        const {unmount} = render(InfoTip, {props: HELP});
        unmount();
        expect(document.querySelector('[role="tooltip"]')).toBeNull();
    });
});

describe("UNIT tipPosition", () => {
    const VIEWPORT = {width: 1000, height: 800};
    const SIZE = {width: 256, height: 100};
    // A 14px (i) icon at (x, y).
    const icon = (left: number, top: number) => ({top, bottom: top + 14, left, width: 14});

    it("bottom-start: below the icon, left edges aligned", () => {
        expect(tipPosition(icon(100, 200), VIEWPORT, SIZE, "bottom-start")).toEqual({top: 220, left: 100, side: "below"});
    });

    it("bottom-end: below the icon, right edges aligned", () => {
        expect(tipPosition(icon(600, 200), VIEWPORT, SIZE, "bottom-end")).toEqual({top: 220, left: 600 + 14 - 256, side: "below"});
    });

    it("flips above when the icon is too near the bottom for the whole tooltip", () => {
        expect(tipPosition(icon(100, 750), VIEWPORT, SIZE, "bottom-start")).toEqual({top: 750 - 6 - 100, left: 100, side: "above"});
    });

    it("stays below on a viewport too short for either side when below is roomier, clamped on screen", () => {
        const placed = tipPosition(icon(100, 10), {width: 1000, height: 90}, SIZE, "bottom-start");
        expect(placed.side).toBe("below");
        expect(placed.top).toBeGreaterThanOrEqual(8);
    });

    it("clamps a bottom-start tooltip at the right edge back inside the viewport", () => {
        const placed = tipPosition(icon(900, 200), VIEWPORT, SIZE, "bottom-start");
        expect(placed.left + SIZE.width).toBe(VIEWPORT.width - 8);
    });

    it("clamps a bottom-end tooltip at the left edge back inside the viewport", () => {
        expect(tipPosition(icon(20, 200), VIEWPORT, SIZE, "bottom-end").left).toBe(8);
    });

    it("right: beside the icon, vertically centred on it", () => {
        expect(tipPosition(icon(100, 300), VIEWPORT, SIZE, "right")).toEqual({top: 300 + 7 - 50, left: 120, side: "right"});
    });

    it("right: flips to the left near the right edge", () => {
        expect(tipPosition(icon(900, 300), VIEWPORT, SIZE, "right")).toEqual({top: 257, left: 900 - 6 - 256, side: "left"});
    });

    it("right: falls back to below when neither side has the width", () => {
        const placed = tipPosition(icon(150, 300), {width: 320, height: 800}, SIZE, "right");
        expect(placed.side).toBe("below");
        expect(placed.left).toBeGreaterThanOrEqual(8);
        expect(placed.left + SIZE.width).toBeLessThanOrEqual(320 - 8);
    });

    it("right: keeps a tooltip beside an icon at the very top on screen", () => {
        expect(tipPosition(icon(100, 0), VIEWPORT, SIZE, "right").top).toBe(8);
    });
});
