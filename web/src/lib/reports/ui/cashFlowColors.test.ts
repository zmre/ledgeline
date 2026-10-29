// The cash flow chart's colours are the statement's heading colours (plans/31,
// A1), and the theme's three of them read apart on the chart (A3).
//
// `ACCENT` is a static literal inside `CashFlowStatementView.svelte` (Tailwind's
// scanner needs the class names spelled out), so it is read here from the
// component's source, the way `palette.test.ts` reads `app.css`.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {DEFAULT_PALETTE, pinColor} from "#lib/format/palette.js";
import type {CfSectionKind} from "../types";
import {CF_EXCHANGE_PIN, CF_PIN} from "./cashFlowColors";

/** Every section kind: `CF_PIN` is a total `Record` over them, so its keys are the list. */
const CF_SECTION_KINDS = Object.keys(CF_PIN) as CfSectionKind[];

const VIEW = readFileSync(new URL("./CashFlowStatementView.svelte", import.meta.url), "utf8");

/** Each section kind's heading colour in `ACCENT`, e.g. `operating` → `info` from `text-info`. */
function accentColours(source: string): Map<string, string> {
    const block = /const ACCENT[^=]*=\s*\{([\s\S]*?)\};/.exec(source)?.[1] ?? "";
    return new Map(
        [...block.matchAll(/(\w+):\s*\{\s*text:\s*"text-([\w-]+)",\s*rule:\s*"border-([\w-]+)\/\d+"/g)].map(([, kind, text, rule]) => [
            kind,
            text === rule ? text : `${text}≠${rule}`,
        ])
    );
}

describe("UNIT cash flow colours", () => {
    it("pins each activity to the daisyUI colour its statement heading uses", () => {
        const accent = accentColours(VIEW);
        expect([...accent.keys()].sort()).toEqual([...CF_SECTION_KINDS].sort());
        for (const kind of CF_SECTION_KINDS) expect(CF_PIN[kind], kind).toBe(accent.get(kind));
    });

    it("draws the effect of exchange rates in the muted grey", () => {
        expect(pinColor(DEFAULT_PALETTE, CF_EXCHANGE_PIN)).toBe(DEFAULT_PALETTE.other);
    });
});

// ── DISTINGUISHABILITY (A3) ──────────────────────────────────────────────────
//
// CIEDE2000 between the three activity colours, the net-change line and the
// exchange grey, on daisy dark (`DEFAULT_PALETTE`, itself pinned to daisyUI).
// Measured when written (daisyUI 5.7.50): info/secondary 65.2, info/accent 28.2,
// info/net 26.1, info/other 24.2, secondary/accent 68.1, secondary/net 39.3,
// secondary/other 28.7, accent/net 27.3, accent/other 29.6, net/other 29.7. The
// weakest is info vs other; the plan's weak pair, info vs accent (blue vs teal),
// is 28.2. A daisyUI update that pulls two of them together fails here.

const MIN_DELTA_E = 20;

type Lab = readonly [number, number, number];

const toLinear = (v: number): number => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);

/** Linear sRGB from `#rrggbb` or `oklch(L[%] C h)`, the two syntaxes the palette holds. Not clamped to the gamut. */
function linearRgb(color: string): [number, number, number] {
    const hex = /^#([0-9a-f]{6})$/i.exec(color.trim());
    if (hex !== null) {
        const [r, g, b] = [0, 2, 4].map((i) => toLinear(parseInt(hex[1].slice(i, i + 2), 16) / 255));
        return [r, g, b];
    }
    const m = /^oklch\(\s*([\d.]+)(%?)\s+([\d.]+)\s+([\d.]+)\s*\)$/.exec(color.trim());
    if (m === null) throw new Error(`cannot parse colour ${color}`);
    const L = Number(m[1]) / (m[2] === "%" ? 100 : 1);
    const hue = (Number(m[4]) * Math.PI) / 180;
    const a = Number(m[3]) * Math.cos(hue);
    const b = Number(m[3]) * Math.sin(hue);
    const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const mid = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
    return [
        4.0767416621 * l - 3.3077115913 * mid + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * mid - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * mid + 1.707614701 * s,
    ];
}

/** CIE L*a*b* (D65 white). */
function labOf(color: string): Lab {
    const [r, g, b] = linearRgb(color);
    const x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    const y = 0.2126729 * r + 0.7151522 * g + 0.072175 * b;
    const z = (0.0193339 * r + 0.119192 * g + 0.9503041 * b) / 1.08883;
    const f = (t: number): number => (t > 216 / 24389 ? Math.cbrt(t) : ((24389 / 27) * t + 16) / 116);
    return [116 * f(y) - 16, 500 * (f(x) - f(y)), 200 * (f(y) - f(z))];
}

/** CIEDE2000 (Sharma, Wu & Dalal 2005). */
function ciede2000([L1, a1, b1]: Lab, [L2, a2, b2]: Lab): number {
    const rad = Math.PI / 180;
    const c7 = ((Math.hypot(a1, b1) + Math.hypot(a2, b2)) / 2) ** 7;
    const g = 0.5 * (1 - Math.sqrt(c7 / (c7 + 25 ** 7)));
    const [ap1, ap2] = [(1 + g) * a1, (1 + g) * a2];
    const [cp1, cp2] = [Math.hypot(ap1, b1), Math.hypot(ap2, b2)];
    const hueOf = (b: number, a: number): number => (b === 0 && a === 0 ? 0 : (Math.atan2(b, a) / rad + 360) % 360);
    const [hp1, hp2] = [hueOf(b1, ap1), hueOf(b2, ap2)];
    const chromatic = cp1 * cp2 !== 0;
    const span = hp2 - hp1;
    const dh = !chromatic ? 0 : Math.abs(span) <= 180 ? span : span > 180 ? span - 360 : span + 360;
    const dL = L2 - L1;
    const dC = cp2 - cp1;
    const dH = 2 * Math.sqrt(cp1 * cp2) * Math.sin((dh / 2) * rad);
    const lBar = (L1 + L2) / 2;
    const cBar = (cp1 + cp2) / 2;
    const sumH = hp1 + hp2;
    const hBar = !chromatic ? sumH : Math.abs(hp1 - hp2) <= 180 ? sumH / 2 : sumH < 360 ? (sumH + 360) / 2 : (sumH - 360) / 2;
    const t =
        1 -
        0.17 * Math.cos((hBar - 30) * rad) +
        0.24 * Math.cos(2 * hBar * rad) +
        0.32 * Math.cos((3 * hBar + 6) * rad) -
        0.2 * Math.cos((4 * hBar - 63) * rad);
    const sL = 1 + (0.015 * (lBar - 50) ** 2) / Math.sqrt(20 + (lBar - 50) ** 2);
    const sC = 1 + 0.045 * cBar;
    const sH = 1 + 0.015 * cBar * t;
    const cBar7 = cBar ** 7;
    const rT = -2 * Math.sqrt(cBar7 / (cBar7 + 25 ** 7)) * Math.sin(60 * Math.exp(-(((hBar - 275) / 25) ** 2)) * rad);
    return Math.sqrt((dL / sL) ** 2 + (dC / sC) ** 2 + (dH / sH) ** 2 + rT * (dC / sC) * (dH / sH));
}

describe("UNIT cash flow colours — distinguishable on daisy dark", () => {
    it("computes CIEDE2000 as published (Sharma et al. test pairs)", () => {
        expect(ciede2000([50, 2.6772, -79.7751], [50, 0, -82.7485])).toBeCloseTo(2.0425, 4);
        expect(ciede2000([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387])).toBeCloseTo(1.2644, 4);
        expect(ciede2000([50, 2.5, 0], [73, 25, -18])).toBeCloseTo(27.1492, 4);
        expect(labOf("oklch(100% 0 0)")[0]).toBeCloseTo(100, 3);
        expect(labOf("#ffffff")[0]).toBeCloseTo(100, 3);
    });

    it(`keeps every activity, the net line and the exchange grey at least ΔE ${MIN_DELTA_E} apart`, () => {
        const colours: [string, string][] = [
            ...CF_SECTION_KINDS.map((kind): [string, string] => [kind, pinColor(DEFAULT_PALETTE, CF_PIN[kind])]),
            ["net change", DEFAULT_PALETTE.flowNet],
            ["exchange", pinColor(DEFAULT_PALETTE, CF_EXCHANGE_PIN)],
        ];
        for (const [i, [nameA, a]] of colours.entries()) {
            for (const [nameB, b] of colours.slice(i + 1)) {
                expect(ciede2000(labOf(a), labOf(b)), `${nameA} vs ${nameB}`).toBeGreaterThanOrEqual(MIN_DELTA_E);
            }
        }
    });
});
