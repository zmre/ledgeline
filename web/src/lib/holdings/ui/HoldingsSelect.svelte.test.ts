// The scope bar's Holdings dropdown (plan 47 D6), mounted: its options, its
// checkboxes, and — because the page is meant to be driven from the keyboard —
// the whole keyboard path, open to tick. Then ScopeBar, against the real scope
// store, to pin that a tick lands in the scope and shows as a removable chip.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#ui/keys/keymap.svelte.js";
import {defaultScope, holdingsScope, holdingsTab} from "#lib/stores/holdings.svelte.js";
import HoldingsSelect from "./HoldingsSelect.svelte";
import ScopeBar from "./ScopeBar.svelte";
import type {HoldingOption} from "./selection";

const OPTIONS: HoldingOption[] = [
    {key: "VTI", label: "VTI", detail: "Vanguard Total Stock Market"},
    {key: "AAPL", label: "AAPL", detail: "Apple Inc."},
    {key: "GLD", label: "GLD", detail: null},
];

const boxes = (): HTMLInputElement[] => [...screen.getByTestId("holdings-select").querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
const search = (): HTMLInputElement => screen.getByTestId("holdings-select-search") as HTMLInputElement;
const details = (): HTMLDetailsElement => screen.getByTestId("holdings-select") as HTMLDetailsElement;

function mount(selected: ReadonlySet<string> = new Set()) {
    const onToggle = vi.fn();
    const onClear = vi.fn();
    render(HoldingsSelect, {options: OPTIONS, selected, onToggle, onClear, openKey: "F"});
    return {onToggle, onClear};
}

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

afterEach(() => {
    keymap.reset();
});

describe("COMPONENT HoldingsSelect", () => {
    it("lists the tab's rows, ticked as selected, with a count on the button", () => {
        mount(new Set(["AAPL"]));

        expect(boxes().map((box) => box.checked)).toEqual([false, true, false]);
        expect(screen.getByTestId("holdings-select-count").textContent).toBe("1");
        expect(screen.getByTestId("holdings-select").textContent).toContain("Apple Inc.");
    });

    it("ticking a box toggles that row", async () => {
        const {onToggle} = mount();
        await fireEvent.click(screen.getByTestId("holdings-select-GLD"));

        expect(onToggle).toHaveBeenCalledWith("GLD");
    });

    it("its key opens it with the search focused", async () => {
        mount();
        await press("F");

        expect(details().open).toBe(true);
        expect(document.activeElement).toBe(search());
        expect(keymap.help.some((section) => section.group === "Holdings")).toBe(true);
    });

    it("typing narrows the list, and Enter ticks the first match", async () => {
        const {onToggle} = mount();
        await fireEvent.input(search(), {target: {value: "aap"}});

        expect(boxes()).toHaveLength(1);
        await fireEvent.keyDown(search(), {key: "Enter"});
        expect(onToggle).toHaveBeenCalledWith("AAPL");
    });

    it("↓ enters the list from the search box, ↓/↑ walk it, ↑ off the top returns", async () => {
        mount();
        await fireEvent.keyDown(search(), {key: "ArrowDown"});
        expect(document.activeElement).toBe(boxes()[0]);

        await fireEvent.keyDown(boxes()[0], {key: "ArrowDown"});
        expect(document.activeElement).toBe(boxes()[1]);

        await fireEvent.keyDown(boxes()[1], {key: "ArrowUp"});
        await fireEvent.keyDown(boxes()[0], {key: "ArrowUp"});
        expect(document.activeElement).toBe(search());
    });

    it("offers Clear only while something is selected", async () => {
        mount();
        expect(screen.queryByRole("button", {name: "Clear"})).toBeNull();
    });

    it("Clear drops the whole filter", async () => {
        const {onClear} = mount(new Set(["VTI"]));
        await fireEvent.click(screen.getByRole("button", {name: "Clear"}));

        expect(onClear).toHaveBeenCalled();
    });

    it("says so when nothing matches", async () => {
        mount();
        await fireEvent.input(search(), {target: {value: "zzz"}});

        expect(screen.getByTestId("holdings-select").textContent).toContain("No matching holdings");
    });
});

describe("COMPONENT ScopeBar holdings filter", () => {
    beforeEach(() => {
        holdingsTab.value = "stocks";
        holdingsScope.replace(defaultScope());
    });

    it("a tick lands in the scope and shows as a chip that removes it", async () => {
        render(ScopeBar, {accountNames: [], holdingOptions: OPTIONS});
        await fireEvent.click(screen.getByTestId("holdings-select-VTI"));

        expect([...holdingsScope.value.holdings]).toEqual(["VTI"]);
        expect(screen.getByTestId("holdings-filter-chips").textContent).toContain("VTI");

        await fireEvent.click(screen.getByRole("button", {name: "Stop charting VTI"}));
        expect(holdingsScope.value.holdings.size).toBe(0);
        expect(screen.queryByTestId("holdings-filter-chips")).toBeNull();
    });
});
