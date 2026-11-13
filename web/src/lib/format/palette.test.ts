// Every theme's chart block and daisyUI theme, read from the stylesheets and
// checked as data: each variant has a complete, contiguous chart block; the
// default-dark block equals `DEFAULT_PALETTE` (the fallback for jsdom, SSR and a
// missing token); the default themes equal the installed daisyUI's own; and
// every text colour clears its WCAG contrast floor, so a palette edit that
// breaks legibility fails here rather than on screen.

import {readdirSync, readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import daisyThemes from "daisyui/theme/object";
import {describe, expect, it} from "vitest";
import {VARIANT_IDS} from "../theme/catalog";
import {contrastRatio, daisyThemes as parseDaisyThemes, dataThemeSelectors, tokensFor} from "../theme/themeCss";
import {CHART_TOKENS, DEFAULT_PALETTE, MAX_SLOTS, pinColor, resolvePalette, samePalette, slotToken, THEME_TOKENS, unknownColor} from "./palette";

const THEMES_DIR = fileURLToPath(new URL("../../themes/", import.meta.url));
const APP_CSS = readFileSync(fileURLToPath(new URL("../../app.css", import.meta.url)), "utf8");
const THEME_FILES = readdirSync(THEMES_DIR).filter((name) => name.endsWith(".css"));
const THEME_CSS = THEME_FILES.map((name) => readFileSync(THEMES_DIR + name, "utf8")).join("\n");
const THEMES = parseDaisyThemes(THEME_CSS);
const byName = (name: string) => THEMES.find((t) => t.name === name);

const NAMED = [CHART_TOKENS.other, CHART_TOKENS.flowIn, CHART_TOKENS.flowOut, CHART_TOKENS.flowNet];
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
    it("are all imported by app.css, default.css first so its :where(:root) fallback is in place", () => {
        const imports = [...APP_CSS.matchAll(/@import "\.\/themes\/([\w-]+\.css)";/g)].map(([, file]) => file);
        expect([...imports].sort()).toEqual([...THEME_FILES].sort());
        expect(imports[0]).toBe("default.css");
    });

    it("registers no built-in daisyUI theme", () => {
        expect(APP_CSS).toMatch(/@plugin "daisyui" \{\s*themes: false;\s*\}/);
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

describe("UNIT theme chart tokens", () => {
    it("has a chart block for exactly the declared themes", () => {
        expect([...new Set(dataThemeSelectors(THEME_CSS))].sort()).toEqual(THEMES.map((t) => t.name).sort());
    });

    describe.each(THEMES.map((t) => [t.name, t] as const))("%s", (name, theme) => {
        const chart = tokensFor(THEME_CSS, `[data-theme="${name}"]`);
        const slots = [...chart.keys()].filter((token) => /^--chart-\d+$/.test(token));
        const base100 = theme.tokens.get("--color-base-100") ?? "";

        it("declares all four named tokens as colours", () => {
            for (const token of NAMED) expect(chart.get(token), token).toMatch(/^#[0-9a-f]{6}$/);
        });

        it("numbers its categorical slots contiguously from --chart-1, within MAX_SLOTS, every one a colour", () => {
            expect(slots.length).toBeGreaterThan(0);
            expect(slots.length).toBeLessThanOrEqual(MAX_SLOTS);
            expect(slots).toEqual(slots.map((_, i) => slotToken(i)));
            for (const token of slots) expect(chart.get(token), token).toMatch(/^#[0-9a-f]{6}$/);
        });

        it("keeps every slot and the in/out/other marks ≥ 3:1 on base-100", () => {
            for (const token of [...slots, CHART_TOKENS.other, CHART_TOKENS.flowIn, CHART_TOKENS.flowOut]) {
                expect(contrastRatio(chart.get(token) ?? "", base100), token).toBeGreaterThanOrEqual(3);
            }
        });

        it("draws the net line in base-content", () => {
            expect(contrastRatio(chart.get(CHART_TOKENS.flowNet) ?? "", theme.tokens.get("--color-base-content") ?? "")).toBeCloseTo(1, 1);
        });
    });

    it("default-dark matches DEFAULT_PALETTE exactly, so the fallback cannot drift from the theme", () => {
        const dark = tokensFor(THEME_CSS, '[data-theme="default-dark"]');
        const fromCss = resolvePalette((token) => dark.get(token) ?? byName("default-dark")?.tokens.get(token) ?? "");
        expect(fromCss).toEqual(DEFAULT_PALETTE);
    });

    it("makes default-dark the :root fallback too, at zero specificity", () => {
        expect(tokensFor(THEME_CSS, ":where(:root)")).toEqual(tokensFor(THEME_CSS, '[data-theme="default-dark"]'));
    });
});

describe("UNIT resolvePalette", () => {
    it("uses every token that is set", () => {
        const theme: Record<string, string> = {"--chart-1": "#000001", "--chart-2": "#000002", "--chart-3": " #123456 ", "--chart-out": "rgb(1 2 3)"};
        const palette = resolvePalette((token) => theme[token] ?? "");
        expect(palette.categorical).toEqual(["#000001", "#000002", "#123456"]);
        expect(palette.flowOut).toBe("rgb(1 2 3)");
    });

    it("falls back to the default slots only when the theme defines none", () => {
        expect(resolvePalette(() => "")).toEqual(DEFAULT_PALETTE);
        const onlyNamed = resolvePalette((token) => (token === "--chart-other" ? "#999999" : ""));
        expect(onlyNamed.categorical).toEqual(DEFAULT_PALETTE.categorical);
        expect(onlyNamed.other).toBe("#999999");
    });

    it("takes the theme's slot count, reading up to the first empty slot", () => {
        const theme = new Map([
            ["--chart-1", "#000001"],
            ["--chart-2", "#000002"],
            ["--chart-3", "#000003"],
            ["--chart-5", "#000005"], // after a gap: never read
        ]);
        const palette = resolvePalette((token) => theme.get(token) ?? "");
        expect(palette.categorical).toEqual(["#000001", "#000002", "#000003"]);
        expect(palette.other).toBe(DEFAULT_PALETTE.other);
    });

    it("reads more slots than the default holds when the theme defines them", () => {
        const count = DEFAULT_PALETTE.categorical.length + 3;
        const palette = resolvePalette((token) => {
            const n = Number(/^--chart-(\d+)$/.exec(token)?.[1] ?? 0);
            return n >= 1 && n <= count ? `#0000${n.toString(16).padStart(2, "0")}` : "";
        });
        expect(palette.categorical).toHaveLength(count);
    });

    it("stops probing at MAX_SLOTS", () => {
        expect(resolvePalette(() => "#abcdef").categorical).toHaveLength(MAX_SLOTS);
    });

    it("compares by value", () => {
        expect(
            samePalette(
                resolvePalette(() => ""),
                DEFAULT_PALETTE
            )
        ).toBe(true);
        expect(
            samePalette(
                resolvePalette((t) => (t === "--chart-net" ? "#fff" : "")),
                DEFAULT_PALETTE
            )
        ).toBe(false);
    });
});

describe("UNIT theme colours (pinned series)", () => {
    it("DEFAULT_PALETTE.theme is daisyUI dark's info, secondary and accent, so the fallback cannot drift from daisy", () => {
        const dark = daisyThemes.dark;
        expect(DEFAULT_PALETTE.theme).toEqual({
            info: dark[THEME_TOKENS.info],
            secondary: dark[THEME_TOKENS.secondary],
            accent: dark[THEME_TOKENS.accent],
        });
    });

    it("reads daisyUI's --color-* tokens", () => {
        const theme: Record<string, string> = {"--color-info": " oklch(1 0 0) ", "--color-secondary": "#222222", "--color-accent": "rgb(3 3 3)"};
        expect(resolvePalette((token) => theme[token] ?? "").theme).toEqual({info: "oklch(1 0 0)", secondary: "#222222", accent: "rgb(3 3 3)"});
    });

    it("falls back per token when one is empty", () => {
        const palette = resolvePalette((token) => (token === "--color-secondary" ? "#222222" : ""));
        expect(palette.theme).toEqual({...DEFAULT_PALETTE.theme, secondary: "#222222"});
    });

    it("counts a change in a theme colour as a different palette", () => {
        for (const token of Object.values(THEME_TOKENS)) {
            expect(
                samePalette(
                    resolvePalette((t) => (t === token ? "#010203" : "")),
                    DEFAULT_PALETTE
                ),
                token
            ).toBe(false);
        }
    });

    it("pinColor resolves a token name to its colour, and `other` to the muted grey", () => {
        expect(pinColor(DEFAULT_PALETTE, "info")).toBe(DEFAULT_PALETTE.theme.info);
        expect(pinColor(DEFAULT_PALETTE, "secondary")).toBe(DEFAULT_PALETTE.theme.secondary);
        expect(pinColor(DEFAULT_PALETTE, "accent")).toBe(DEFAULT_PALETTE.theme.accent);
        expect(pinColor(DEFAULT_PALETTE, "other")).toBe(DEFAULT_PALETTE.other);
    });
});

describe("UNIT unknownColor", () => {
    it("adds a hex alpha to a hex muted colour", () => {
        expect(unknownColor(DEFAULT_PALETTE)).toBe(`${DEFAULT_PALETTE.other}73`);
        expect(unknownColor({...DEFAULT_PALETTE, other: "#abc"}, 1)).toBe("#aabbccff");
    });

    it("falls back to color-mix for any other syntax", () => {
        expect(unknownColor({...DEFAULT_PALETTE, other: "oklch(0.6 0 0)"}, 0.5)).toBe("color-mix(in srgb, oklch(0.6 0 0) 50%, transparent)");
    });

    it("is never the solid muted colour itself", () => {
        expect(unknownColor(DEFAULT_PALETTE)).not.toBe(DEFAULT_PALETTE.other);
    });
});
