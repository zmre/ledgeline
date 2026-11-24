// "Override block" (invoicer PLAN 4d), mounted: what a template changes,
// what it can still override, and a block the base doesn't have.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import BlockPanel from "./BlockPanel.svelte";

const blocks = [
    {name: "settings", about: "before anything is written."},
    {name: "theme", about: "the CSS for the sheet."},
    {name: "footer", about: "a closing line, empty here."},
];

describe("COMPONENT BlockPanel", () => {
    it("lists the overridden blocks and offers the rest with what each is for", async () => {
        const onOverride = vi.fn();
        const onShow = vi.fn();
        render(BlockPanel, {blocks, defined: ["theme", "nope"], extendsBase: true, onOverride, onShow});
        const overridden = screen.getAllByTestId("block-overridden");
        expect(overridden.map((b) => b.textContent?.trim())).toEqual(["theme"]);
        expect(screen.getByTestId("block-unknown").textContent).toContain("not in the base");
        const choice = screen.getByTestId("block-choice") as HTMLSelectElement;
        expect([...choice.options].map((o) => o.value)).toEqual(["settings", "footer"]);
        expect(screen.getByTestId("block-about").textContent).toBe("before anything is written.");
        await fireEvent.change(choice, {target: {value: "footer"}});
        expect(screen.getByTestId("block-about").textContent).toBe("a closing line, empty here.");
        await fireEvent.click(screen.getByRole("button", {name: "Override block"}));
        expect(onOverride).toHaveBeenCalledWith("footer");
        await fireEvent.click(overridden[0]);
        expect(onShow).toHaveBeenCalledWith("theme");
    });

    it("says when nothing is overridden yet, and offers nothing when read only", () => {
        render(BlockPanel, {blocks, defined: [], extendsBase: true, readonly: true, onOverride: vi.fn(), onShow: vi.fn()});
        expect(screen.getByTestId("block-panel-none")).toBeDefined();
        expect(screen.queryByTestId("block-choice")).toBeNull();
    });

    it("says a whole page has no blocks to override", () => {
        render(BlockPanel, {blocks: [], defined: [], extendsBase: false, onOverride: vi.fn(), onShow: vi.fn()});
        expect(screen.getByTestId("block-panel-whole").textContent).toContain("A whole page");
        expect(screen.queryByRole("button", {name: "Override block"})).toBeNull();
    });
});
