// The dark chart palette exists twice: as `--chart-*` tokens in `app.css` (the
// source of truth the running app reads) and as `DEFAULT_PALETTE` (the
// fallback for jsdom, SSR, and a missing token). This pins them together.

import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {CHART_TOKENS, DEFAULT_PALETTE, resolvePalette, samePalette, unknownColor} from "./palette";

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
        const all = [...CHART_TOKENS.categorical, CHART_TOKENS.other, CHART_TOKENS.flowIn, CHART_TOKENS.flowOut, CHART_TOKENS.flowNet];
        for (const token of all) expect(dark.get(token), token).toMatch(/^#[0-9a-f]{6}$/);
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
        const read = (token: string): string => (token === "--chart-3" ? " #123456 " : token === "--chart-out" ? "rgb(1 2 3)" : "");
        const palette = resolvePalette(read);
        expect(palette.categorical[2]).toBe("#123456");
        expect(palette.flowOut).toBe("rgb(1 2 3)");
    });

    it("falls back per token when a value is empty", () => {
        expect(resolvePalette(() => "")).toEqual(DEFAULT_PALETTE);
        const palette = resolvePalette((token) => (token === "--chart-1" ? "#000000" : ""));
        expect(palette.categorical.slice(1)).toEqual(DEFAULT_PALETTE.categorical.slice(1));
        expect(palette.other).toBe(DEFAULT_PALETTE.other);
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
