// The dark chart palette exists twice: as `--chart-*` tokens in `app.css` (the
// source of truth the running app reads) and as `DEFAULT_PALETTE` (the
// fallback for jsdom, SSR, and a missing token). This pins them together, and
// pins `DEFAULT_PALETTE.theme` to the installed daisyUI's dark theme the same way.

import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import daisyThemes from "daisyui/theme/object";
import {describe, expect, it} from "vitest";
import {CHART_TOKENS, DEFAULT_PALETTE, MAX_SLOTS, pinColor, resolvePalette, samePalette, slotToken, THEME_TOKENS, unknownColor} from "./palette";

const APP_CSS = readFileSync(fileURLToPath(new URL("../../app.css", import.meta.url)), "utf8");

/** The custom properties declared in the rule whose selector list includes `selector`. */
function tokensFor(css: string, selector: string): Map<string, string> {
    const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");
    for (const [, selectors, body] of withoutComments.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
        if (!selectors.split(",").some((s) => s.trim() === selector)) continue;
        return new Map([...body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim().toLowerCase()]));
    }
    return new Map();
}

describe("UNIT app.css chart tokens", () => {
    const dark = tokensFor(APP_CSS, '[data-theme="dark"]');

    it("declares every chart token for the dark theme", () => {
        const all = [CHART_TOKENS.other, CHART_TOKENS.flowIn, CHART_TOKENS.flowOut, CHART_TOKENS.flowNet];
        for (const token of all) expect(dark.get(token), token).toMatch(/^#[0-9a-f]{6}$/);
    });

    it("numbers its categorical slots contiguously from --chart-1, every one a colour", () => {
        const slots = [...dark.keys()].filter((token) => /^--chart-\d+$/.test(token));
        expect(slots.length).toBeGreaterThan(0);
        expect(slots.map((token) => token)).toEqual(slots.map((_, i) => slotToken(i)));
        for (const token of slots) expect(dark.get(token), token).toMatch(/^#[0-9a-f]{6}$/);
    });

    it("matches DEFAULT_PALETTE exactly, so the fallback cannot drift from the theme", () => {
        const fromCss = resolvePalette((token) => dark.get(token) ?? "");
        expect(fromCss).toEqual(DEFAULT_PALETTE);
    });

    it("makes dark the :root default too", () => {
        expect(tokensFor(APP_CSS, ":root")).toEqual(dark);
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
