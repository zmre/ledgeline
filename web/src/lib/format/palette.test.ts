// The dark chart palette exists twice: as `--chart-*` tokens in `app.css` (the
// source of truth the running app reads) and as `DEFAULT_PALETTE` (the
// fallback for jsdom, SSR, and a missing token). This pins them together.

import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {CHART_TOKENS, DEFAULT_PALETTE, MAX_SLOTS, resolvePalette, samePalette, slotToken, unknownColor} from "./palette";

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
