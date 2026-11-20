import {describe, expect, it} from "vitest";
import {appearanceCache, followsSystem} from "./appearance.svelte.js";

describe("appearanceCache", () => {
    it("records the variant each OS scheme shows for a following family", () => {
        expect(appearanceCache("catppuccin", 125)).toEqual({light: "catppuccin-latte", dark: "catppuccin-mocha", scale: 125});
    });

    it("records the same variant twice for a pinned variant", () => {
        expect(appearanceCache("github-light", 100)).toEqual({light: "github-light", dark: "github-light", scale: 100});
    });

    it("keeps a dark-only theme dark under a light OS", () => {
        expect(appearanceCache("one-dark-pro", 100).light).toBe("one-dark-pro");
    });

    it("falls back to the default family for an unknown pref", () => {
        expect(appearanceCache("no-such-theme", 100)).toEqual({light: "default-light", dark: "default-dark", scale: 100});
    });
});

describe("followsSystem", () => {
    it.each([
        ["default", true],
        ["solarized", true],
        ["solarized-dark", false],
        ["dracula", false],
        ["nonsense", false],
    ])("%s → %s", (pref, expected) => {
        expect(followsSystem(pref)).toBe(expected);
    });
});
