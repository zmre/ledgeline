// The invoicer's muted-text token (`--color-muted`, `text-muted` in
// src/app.css): base-content mixed 70% into base-100 in OKLab, or full
// base-content in the themes where that mix is too faint. Checked as data
// against every theme variant, as packages/ui's themeFiles.test.ts checks the
// themes themselves: muted text must clear WCAG 4.5:1 on base-100 and on
// base-200 (the customer card's header band) in every variant, except where
// the theme's own body text is under 4.5:1 on base-200 (BASE_200_EXCEPTIONS):
// there muted is the body text, as legible as that theme allows.

import {readdirSync, readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {contrastRatio, daisyThemes} from "#ui/theme/themeCss.js";

const THEMES_DIR = fileURLToPath(new URL("../../../../packages/ui/src/themes/", import.meta.url));
const THEMES = daisyThemes(
    readdirSync(THEMES_DIR)
        .filter((name) => name.endsWith(".css"))
        .map((name) => readFileSync(THEMES_DIR + name, "utf8"))
        .join("\n")
);
const APP_CSS = readFileSync(fileURLToPath(new URL("../app.css", import.meta.url)), "utf8");

/** The themes app.css gives full base-content as their muted colour. */
function fullContrastThemes(css: string): string[] {
    const rule = /((?:\[data-theme="[\w-]+"\]\s*,?\s*)+)\{\s*--color-muted:\s*var\(--color-base-content\);/.exec(css);
    return rule === null ? [] : [...rule[1].matchAll(/\[data-theme="([\w-]+)"\]/g)].map(([, name]) => name);
}

/** The mix's share of base-content, as app.css writes it. */
function mixShare(css: string): number {
    const mix = /--color-muted:\s*color-mix\(in oklab, var\(--color-base-content\) (\d+)%, var\(--color-base-100\)\)/.exec(css);
    if (mix === null) throw new Error("app.css has no --color-muted mix");
    return Number(mix[1]) / 100;
}

type Lab = [number, number, number];

const toLinear = (c: number): number => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

/** A theme colour (`#rrggbb` or `oklch(L C H)`) in OKLab. */
function oklab(color: string): Lab {
    const value = color.trim().toLowerCase();
    const lch = /^oklch\(\s*([\d.]+)(%?)\s+([\d.]+)\s+([\d.]+)\s*\)$/.exec(value);
    if (lch !== null) {
        const l = lch[2] === "%" ? Number(lch[1]) / 100 : Number(lch[1]);
        const h = (Number(lch[4]) * Math.PI) / 180;
        return [l, Number(lch[3]) * Math.cos(h), Number(lch[3]) * Math.sin(h)];
    }
    const hex = /^#([0-9a-f]{6})$/.exec(value);
    if (hex === null) throw new Error(`unsupported colour: ${color}`);
    const n = Number.parseInt(hex[1], 16);
    const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((x) => toLinear(x / 255));
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    return [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ];
}

/** `color-mix(in oklab, a share, b)`, as an `oklch()` string. */
function mix(a: string, share: number, b: string): string {
    const [x, y] = [oklab(a), oklab(b)];
    const [l, aa, bb] = x.map((v, i) => v * share + y[i] * (1 - share));
    const c = Math.hypot(aa, bb);
    const h = ((Math.atan2(bb, aa) * 180) / Math.PI + 360) % 360;
    return `oklch(${l} ${c} ${h})`;
}

/** Themes whose own base-content is under 4.5:1 on base-200 (as published). */
const BASE_200_EXCEPTIONS = new Set(["solarized-light", "tokyo-night-day"]);

const SHARE = mixShare(APP_CSS);
const FULL = new Set(fullContrastThemes(APP_CSS));

/** The muted colour a theme gets. */
function muted(theme: (typeof THEMES)[number]): string {
    const content = theme.tokens.get("--color-base-content") ?? "";
    return FULL.has(theme.name) ? content : mix(content, SHARE, theme.tokens.get("--color-base-100") ?? "");
}

/** The themes where the plain mix is under 4.5:1 on base-100 or base-200: those app.css must override. */
function themesNeedingFullContrast(): string[] {
    return THEMES.filter((theme) => {
        const mixed = mix(theme.tokens.get("--color-base-content") ?? "", SHARE, theme.tokens.get("--color-base-100") ?? "");
        const backgrounds = BASE_200_EXCEPTIONS.has(theme.name) ? ["--color-base-100"] : ["--color-base-100", "--color-base-200"];
        return backgrounds.some((bg) => contrastRatio(mixed, theme.tokens.get(bg) ?? "") < 4.5);
    }).map((theme) => theme.name);
}

describe("UNIT muted text token", () => {
    it("is read from app.css", () => {
        expect(SHARE).toBeGreaterThan(0.5);
        expect(THEMES.length).toBeGreaterThanOrEqual(16);
    });

    it.each(THEMES.map((t) => [t.name, t] as const))("%s: muted text is ≥ 4.5:1 on base-100 and base-200", (name, theme) => {
        const content = theme.tokens.get("--color-base-content") ?? "";
        const base200 = theme.tokens.get("--color-base-200") ?? "";
        expect(contrastRatio(muted(theme), theme.tokens.get("--color-base-100") ?? "")).toBeGreaterThanOrEqual(4.5);
        if (BASE_200_EXCEPTIONS.has(name)) {
            expect(contrastRatio(content, base200)).toBeLessThan(4.5);
            expect(muted(theme)).toBe(content);
        } else {
            expect(contrastRatio(muted(theme), base200)).toBeGreaterThanOrEqual(4.5);
        }
    });

    it("overrides exactly the themes that need it", () => {
        expect([...FULL].sort()).toEqual(themesNeedingFullContrast().sort());
    });
});
