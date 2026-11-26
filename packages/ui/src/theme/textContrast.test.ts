// Text colours against their backgrounds, in every theme variant, for both
// apps: body text (base-content) on base-100, -200 and -300; the muted-text
// token (`--color-muted`, `text-muted`, theme.css) on base-100 and -200; and
// primary and error used as text on base-100. Each must clear WCAG 4.5:1,
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

/**
 * Accent colours under 4.5:1 as text on base-100, as published (measured
 * 2026-11-26). They are kept: the default themes are daisyUI's own, value for
 * value (themeFiles.test.ts), and the others are each palette's canonical
 * blue or red, which also fill buttons and badges. So neither is used for
 * small text: links are `link` (base-content), overdue is a badge.
 */
const ACCENT_TEXT_EXCEPTIONS: Readonly<Record<string, readonly ("primary" | "error")[]>> = {
    "default-dark": ["primary"],
    "default-light": ["error"],
    "gruvbox-dark": ["error"],
    nord: ["error"],
    monokai: ["error"],
    "one-dark-pro": ["error"],
    "solarized-light": ["primary", "error"],
    "solarized-dark": ["primary", "error"],
    "tokyo-night-day": ["primary", "error"],
};

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

    it.each(THEMES.map((t) => [t.name, t] as const))("%s: primary and error as text on base-100, or a documented exception", (name, theme) => {
        for (const accent of ["primary", "error"] as const) {
            const ratio = contrastRatio(token(theme, accent), token(theme, "base-100"));
            const excepted = ACCENT_TEXT_EXCEPTIONS[name]?.includes(accent) ?? false;
            // An exception that no longer fails must come off the list.
            if (excepted) expect(ratio, accent).toBeLessThan(FLOOR);
            else expect(ratio, accent).toBeGreaterThanOrEqual(FLOOR);
        }
    });
});
