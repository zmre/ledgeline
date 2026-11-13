import {describe, expect, it} from "vitest";
import {THEME_FAMILIES, THEME_PREF_IDS, VARIANT_IDS, type VariantId} from "./catalog";
import {resolveTheme} from "./resolve";

describe("UNIT resolveTheme", () => {
    it("follows the OS for a family with both variants", () => {
        const cases: [string, VariantId, VariantId][] = [
            ["default", "default-light", "default-dark"],
            ["github", "github-light", "github-dark"],
            ["catppuccin", "catppuccin-latte", "catppuccin-mocha"],
            ["gruvbox", "gruvbox-light", "gruvbox-dark"],
            ["tokyo-night", "tokyo-night-day", "tokyo-night-night"],
            ["solarized", "solarized-light", "solarized-dark"],
        ];
        for (const [pref, light, dark] of cases) {
            expect(resolveTheme(pref, false), pref).toBe(light);
            expect(resolveTheme(pref, true), pref).toBe(dark);
        }
    });

    it("pins a variant id whatever the OS says", () => {
        for (const id of VARIANT_IDS) {
            expect(resolveTheme(id, false)).toBe(id);
            expect(resolveTheme(id, true)).toBe(id);
        }
    });

    it("keeps a dark-only family dark in light mode", () => {
        for (const id of ["one-dark-pro", "dracula", "monokai", "nord"]) expect(resolveTheme(id, false)).toBe(id);
    });

    it("resolves an unknown pref as `default`", () => {
        for (const pref of ["", "dark", "light", "GitHub", "no-such-theme", "__proto__", " default"]) {
            expect(resolveTheme(pref, false), pref).toBe("default-light");
            expect(resolveTheme(pref, true), pref).toBe("default-dark");
        }
    });

    it("turns every pref id, under either scheme, into a variant", () => {
        for (const pref of THEME_PREF_IDS) {
            for (const dark of [false, true]) expect(VARIANT_IDS).toContain(resolveTheme(pref, dark));
        }
    });

    it("gives every family's variants back for the matching scheme", () => {
        for (const family of THEME_FAMILIES) {
            if (family.variants.dark) expect(resolveTheme(family.id, true)).toBe(family.variants.dark);
            if (family.variants.light) expect(resolveTheme(family.id, false)).toBe(family.variants.light);
        }
    });
});
