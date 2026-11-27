// Text colours against their backgrounds, in every theme variant, for both
// apps: body text (base-content) on base-100, -200 and -300; the muted-text
// token (`--color-muted`, `text-muted`, theme.css) on base-100 and -200; and
// primary and error as text (`text-primary-text`, `text-error-text`) on
// base-100 and -200. Each must clear WCAG 4.5:1,
// the floor for small text, so a theme edit that makes labels or links
// unreadable fails here.

import {readdirSync, readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {contrastRatio, daisyThemes, mixOklab} from "./themeCss";

const THEMES_DIR = fileURLToPath(new URL("../themes/", import.meta.url));
const THEMES = daisyThemes(
    readdirSync(THEMES_DIR)
        .filter((name) => name.endsWith(".css"))
        .map((name) => readFileSync(THEMES_DIR + name, "utf8"))
        .join("\n")
);
const THEME_CSS = readFileSync(fileURLToPath(new URL("../theme.css", import.meta.url)), "utf8");

const FLOOR = 4.5;

/** The themes theme.css gives full base-content as their muted colour. */
const FULL = new Set(
    (() => {
        const rule = /((?:\[data-theme="[\w-]+"\]\s*,?\s*)+)\{\s*--color-muted:\s*var\(--color-base-content\);/.exec(THEME_CSS);
        return rule === null ? [] : [...rule[1].matchAll(/\[data-theme="([\w-]+)"\]/g)].map(([, name]) => name);
    })()
);

/** The mix's share of base-content, as theme.css writes it. */
const SHARE = (() => {
    const mix = /--color-muted:\s*color-mix\(in oklab, var\(--color-base-content\) (\d+)%, var\(--color-base-100\)\)/.exec(THEME_CSS);
    if (mix === null) throw new Error("theme.css has no --color-muted mix");
    return Number(mix[1]) / 100;
})();

type Theme = (typeof THEMES)[number];
const token = (theme: Theme, name: string): string => theme.tokens.get(`--color-${name}`) ?? "";
const mixed = (theme: Theme): string => mixOklab(token(theme, "base-content"), SHARE, token(theme, "base-100"));
const muted = (theme: Theme): string => (FULL.has(theme.name) ? token(theme, "base-content") : mixed(theme));

/** The text-only shades theme.css sets per theme (`--primary-text`, `--error-text`), by theme name. */
const TEXT_SHADES: ReadonlyMap<string, ReadonlyMap<string, string>> = (() => {
    const out = new Map<string, Map<string, string>>();
    for (const rule of THEME_CSS.matchAll(/((?:(?::root:not\(\[data-theme\]\)|\[data-theme="[\w-]+"\])\s*,?\s*)+)\{([^}]*)\}/g)) {
        const shades = [...rule[2].matchAll(/--(primary|error)-text:\s*(#[0-9a-fA-F]{6});/g)];
        if (shades.length === 0) continue;
        for (const [, name] of rule[1].matchAll(/\[data-theme="([\w-]+)"\]/g)) {
            const map = out.get(name) ?? new Map<string, string>();
            for (const [, accent, value] of shades) map.set(accent, value);
            out.set(name, map);
        }
    }
    return out;
})();

/** An accent as text: the theme's text shade when theme.css sets one, else the fill colour. */
const accentText = (theme: Theme, accent: "primary" | "error"): string => TEXT_SHADES.get(theme.name)?.get(accent) ?? token(theme, accent);

describe("UNIT text contrast in every theme (WCAG 4.5:1)", () => {
    it("reads the muted token from theme.css", () => {
        expect(SHARE).toBeGreaterThan(0.5);
        expect(THEMES.length).toBeGreaterThanOrEqual(16);
    });

    it.each(THEMES.map((t) => [t.name, t] as const))("%s: body text on base-100, -200 and -300", (_name, theme) => {
        for (const bg of ["base-100", "base-200", "base-300"]) {
            expect(contrastRatio(token(theme, "base-content"), token(theme, bg)), bg).toBeGreaterThanOrEqual(FLOOR);
        }
    });

    it.each(THEMES.map((t) => [t.name, t] as const))("%s: muted text on base-100 and -200", (_name, theme) => {
        for (const bg of ["base-100", "base-200"]) {
            expect(contrastRatio(muted(theme), token(theme, bg)), bg).toBeGreaterThanOrEqual(FLOOR);
        }
    });

    it("gives full body text as muted to exactly the themes whose mix is too faint", () => {
        const needing = THEMES.filter((theme) => ["base-100", "base-200"].some((bg) => contrastRatio(mixed(theme), token(theme, bg)) < FLOOR)).map(
            (t) => t.name
        );
        expect([...FULL].sort()).toEqual(needing.sort());
    });

    it.each(THEMES.map((t) => [t.name, t] as const))(
        "%s: primary and error as text (text-primary-text, text-error-text) on base-100 and -200",
        (_name, theme) => {
            for (const accent of ["primary", "error"] as const) {
                for (const bg of ["base-100", "base-200"]) {
                    expect(contrastRatio(accentText(theme, accent), token(theme, bg)), `${accent} on ${bg}`).toBeGreaterThanOrEqual(FLOOR);
                }
            }
        }
    );

    it("sets a text shade only where the fill colour is too faint as text", () => {
        expect(TEXT_SHADES.size).toBeGreaterThan(0);
        for (const [name, shades] of TEXT_SHADES) {
            const theme = THEMES.find((t) => t.name === name);
            expect(theme, name).toBeDefined();
            if (theme === undefined) continue;
            for (const accent of shades.keys()) {
                const raw = Math.min(...["base-100", "base-200"].map((bg) => contrastRatio(token(theme, accent), token(theme, bg))));
                expect(raw, `${name} ${accent}`).toBeLessThan(FLOOR);
            }
        }
    });
});
