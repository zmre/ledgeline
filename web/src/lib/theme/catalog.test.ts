// The catalog against the theme stylesheets: every variant it lists is a
// daisyUI theme in src/themes/, with the scheme it claims, and every swatch
// colour is the value that theme file declares — the CSS is the source of
// truth and the swatches are a tested copy.

import {readdirSync, readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {
    DEFAULT_THEME_PREF,
    FALLBACK_VARIANT,
    FAMILY_IDS,
    isFamilyId,
    isThemePref,
    isVariantId,
    THEME_FAMILIES,
    THEME_PREF_IDS,
    THEME_VARIANTS,
    themeFamily,
    themeVariant,
    VARIANT_IDS,
} from "./catalog";
import {daisyThemes, tokensFor} from "./themeCss";

const THEMES_DIR = fileURLToPath(new URL("../../themes/", import.meta.url));
const THEME_CSS = readdirSync(THEMES_DIR)
    .filter((name) => name.endsWith(".css"))
    .map((name) => readFileSync(THEMES_DIR + name, "utf8"))
    .join("\n");
const CSS_THEMES = new Map(daisyThemes(THEME_CSS).map((t) => [t.name, t]));

describe("UNIT theme catalog", () => {
    it("lists each variant once, in VARIANT_IDS order", () => {
        expect(THEME_VARIANTS.map((v) => v.id)).toEqual([...VARIANT_IDS]);
    });

    it("lists each family once, in FAMILY_IDS order", () => {
        expect(THEME_FAMILIES.map((f) => f.id)).toEqual([...FAMILY_IDS]);
    });

    it("has 22 pref ids: the six follow-the-OS families, then all sixteen variants", () => {
        expect(THEME_PREF_IDS).toEqual([
            "default",
            "github",
            "catppuccin",
            "gruvbox",
            "tokyo-night",
            "solarized",
            "default-light",
            "default-dark",
            "github-light",
            "github-dark",
            "catppuccin-latte",
            "catppuccin-mocha",
            "gruvbox-light",
            "gruvbox-dark",
            "tokyo-night-day",
            "tokyo-night-night",
            "solarized-light",
            "solarized-dark",
            "one-dark-pro",
            "dracula",
            "monokai",
            "nord",
        ]);
        expect(new Set(THEME_PREF_IDS).size).toBe(22);
        for (const id of THEME_PREF_IDS) expect(isThemePref(id), id).toBe(true);
    });

    it("links families and variants both ways, each variant's scheme matching its slot", () => {
        for (const family of THEME_FAMILIES) {
            for (const scheme of ["light", "dark"] as const) {
                const id = family.variants[scheme];
                if (id === undefined) continue;
                expect(themeVariant(id)?.family, id).toBe(family.id);
                expect(themeVariant(id)?.scheme, id).toBe(scheme);
            }
        }
        for (const variant of THEME_VARIANTS) {
            expect(themeFamily(variant.family)?.variants[variant.scheme], variant.id).toBe(variant.id);
        }
    });

    it("names a dark-only family by its only variant's id", () => {
        for (const family of THEME_FAMILIES.filter((f) => f.variants.light === undefined)) expect(family.variants.dark).toBe(family.id);
    });

    it("defaults to the default family, falling back to default-dark", () => {
        expect(DEFAULT_THEME_PREF).toBe("default");
        expect(FALLBACK_VARIANT).toBe("default-dark");
    });

    it("tells ids apart", () => {
        expect(isVariantId("nord")).toBe(true);
        expect(isFamilyId("nord")).toBe(true);
        expect(isVariantId("github")).toBe(false);
        expect(isFamilyId("github-dark")).toBe(false);
        for (const id of ["", "dark", "light", "GitHub", "solarized-", "constructor", "__proto__"]) expect(isThemePref(id), id).toBe(false);
        expect(themeVariant("dark")).toBeUndefined();
        expect(themeFamily("dark")).toBeUndefined();
    });

    it("has a unique, non-empty label for every family and variant", () => {
        const labels = [...THEME_FAMILIES.map((f) => f.label)];
        expect(labels.every((l) => l.trim() !== "")).toBe(true);
        expect(new Set(labels).size).toBe(labels.length);
        const variantLabels = THEME_VARIANTS.map((v) => v.label);
        expect(new Set(variantLabels).size).toBe(variantLabels.length);
    });
});

describe("UNIT theme catalog against the CSS", () => {
    it.each(THEME_VARIANTS.map((v) => [v.id, v] as const))("%s is a CSS theme with the same scheme and swatch colours", (id, variant) => {
        const theme = CSS_THEMES.get(id);
        expect(theme, id).toBeDefined();
        expect(theme?.scheme).toBe(variant.scheme);
        const tokens = theme?.tokens ?? new Map<string, string>();
        expect({
            base100: variant.swatch.base100,
            baseContent: variant.swatch.baseContent,
            primary: variant.swatch.primary,
            secondary: variant.swatch.secondary,
            accent: variant.swatch.accent,
        }).toEqual({
            base100: tokens.get("--color-base-100"),
            baseContent: tokens.get("--color-base-content"),
            primary: tokens.get("--color-primary"),
            secondary: tokens.get("--color-secondary"),
            accent: tokens.get("--color-accent"),
        });
        const chart = tokensFor(THEME_CSS, `[data-theme="${id}"]`);
        expect(variant.swatch.chart).toEqual([1, 2, 3, 4].map((n) => chart.get(`--chart-${n}`)));
    });
});

// Three copies of the theme id list must agree: this catalog (what the SPA can
// render), the prefs wire (`THEME_IDS`, what the store accepts) and the engine's
// `THEMES` (what `ui-prefs.toml` / `ledgeline.toml` validate against).
describe("theme ids across the wire", () => {
    it("THEME_PREF_IDS is the prefs THEME_IDS set", async () => {
        const {THEME_IDS} = await import("#lib/settings/uiPrefs.js");
        expect([...THEME_PREF_IDS].sort()).toEqual([...THEME_IDS].sort());
    });

    it("THEME_PREF_IDS is the engine's THEMES set", () => {
        const rust = readFileSync(fileURLToPath(new URL("../../../../crates/ledgeline-server/src/ui_prefs.rs", import.meta.url)), "utf8");
        const block = /const THEMES: &\[&str\] = &\[([^\]]*)\];/.exec(rust)?.[1] ?? "";
        const ids = [...block.matchAll(/"([a-z-]+)"/g)].map((m) => m[1]);
        expect(ids.sort()).toEqual([...THEME_PREF_IDS].sort());
    });
});
