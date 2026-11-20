// Every theme file's daisyUI theme, read from the stylesheets and checked as
// data: the files are all imported (default first), each declares every daisyUI
// token with default's shape, the default themes equal the installed daisyUI's
// own, and every text colour clears its WCAG contrast floor, so a theme edit
// that breaks legibility fails here rather than on screen. The chart tokens
// beside each theme are checked by the app that reads them (Ledgeline:
// web/src/lib/format/palette.test.ts).

import {readdirSync, readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import daisyThemes from "daisyui/theme/object";
import {describe, expect, it} from "vitest";
import {VARIANT_IDS} from "./catalog";
import {contrastRatio, daisyThemes as parseDaisyThemes} from "./themeCss";

const THEMES_DIR = fileURLToPath(new URL("../themes/", import.meta.url));
const THEME_STYLESHEET = readFileSync(fileURLToPath(new URL("../theme.css", import.meta.url)), "utf8");
const THEME_FILES = readdirSync(THEMES_DIR).filter((name) => name.endsWith(".css"));
const THEME_CSS = THEME_FILES.map((name) => readFileSync(THEMES_DIR + name, "utf8")).join("\n");
const THEMES = parseDaisyThemes(THEME_CSS);
const byName = (name: string) => THEMES.find((t) => t.name === name);

const ROLES = ["primary", "secondary", "accent", "neutral", "info", "success", "warning", "error"] as const;
/** Same order as daisyUI's own theme objects. */
const TOKEN_NAMES = Object.keys(daisyThemes.dark).filter((key) => key.startsWith("--"));
const SHAPE_TOKENS = TOKEN_NAMES.filter((key) => !key.startsWith("--color-"));
/** The two daisyUI copies: their colours are daisy's, so its -content contrasts are daisy's too. */
const DAISY_COPIES: Record<string, Readonly<Record<string, string>>> = {
    "default-dark": {...daisyThemes.dark},
    "default-light": {...daisyThemes.light},
};
/**
 * Variants whose upstream body text sits under 7:1 on its background, kept as
 * published (see the comment in each theme file). They still clear 4.5:1.
 */
const BODY_TEXT_EXCEPTIONS = new Set(["tokyo-night-day", "solarized-light", "solarized-dark", "one-dark-pro"]);

describe("UNIT theme files", () => {
    it("are all imported by theme.css, default.css first so its :where(:root) fallback is in place", () => {
        const imports = [...THEME_STYLESHEET.matchAll(/@import "\.\/themes\/([\w-]+\.css)";/g)].map(([, file]) => file);
        expect([...imports].sort()).toEqual([...THEME_FILES].sort());
        expect(imports[0]).toBe("default.css");
    });

    it("registers no built-in daisyUI theme", () => {
        expect(THEME_STYLESHEET).toMatch(/@plugin "daisyui" \{\s*themes: false;\s*\}/);
    });

    it("declare exactly the catalog's variants, each once", () => {
        expect(THEMES.map((t) => t.name).sort()).toEqual([...VARIANT_IDS].sort());
    });

    it("make default-dark, and only it, the default", () => {
        expect(THEMES.filter((t) => t.isDefault).map((t) => t.name)).toEqual(["default-dark"]);
    });

    it.each(THEMES.map((t) => [t.name, t] as const))("%s declares every daisyUI token, with default's shape", (_name, theme) => {
        expect(["light", "dark"]).toContain(theme.scheme);
        expect([...theme.tokens.keys()]).toEqual(TOKEN_NAMES);
        for (const key of SHAPE_TOKENS) expect(theme.tokens.get(key), key).toBe(daisyThemes.dark[key as keyof typeof daisyThemes.dark]);
    });

    it.each(Object.entries(DAISY_COPIES))("%s is daisyUI's built-in theme, value for value", (name, builtin) => {
        const theme = byName(name);
        expect(theme?.scheme).toBe(builtin["color-scheme"]);
        expect(Object.fromEntries(theme?.tokens ?? [])).toEqual(Object.fromEntries(Object.entries(builtin).filter(([key]) => key.startsWith("--"))));
    });
});

describe("UNIT theme contrast (WCAG)", () => {
    it("measures the way WCAG does", () => {
        expect(contrastRatio("#ffffff", "#000000")).toBeCloseTo(21, 5);
        expect(contrastRatio("#777777", "#ffffff")).toBeCloseTo(4.48, 2);
        expect(contrastRatio("oklch(100% 0 0)", "#ffffff")).toBeCloseTo(1, 3);
        expect(contrastRatio("oklch(0% 0 0)", "#ffffff")).toBeCloseTo(21, 3);
    });

    it.each(THEMES.map((t) => [t.name, t] as const))("%s: base-content on base-100 is ≥ 7:1 (≥ 4.5:1 for a documented exception)", (name, theme) => {
        const ratio = contrastRatio(theme.tokens.get("--color-base-content") ?? "", theme.tokens.get("--color-base-100") ?? "");
        expect(ratio).toBeGreaterThanOrEqual(BODY_TEXT_EXCEPTIONS.has(name) ? 4.5 : 7);
    });

    it.each(THEMES.filter((t) => !(t.name in DAISY_COPIES)).map((t) => [t.name, t] as const))(
        "%s: every -content colour is ≥ 4.5:1 on its role",
        (_name, theme) => {
            for (const role of ROLES) {
                const ratio = contrastRatio(theme.tokens.get(`--color-${role}-content`) ?? "", theme.tokens.get(`--color-${role}`) ?? "");
                expect(ratio, role).toBeGreaterThanOrEqual(4.5);
            }
        }
    );
});
