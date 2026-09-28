// The ONE chart palette: its shape, its fallback values, and the pure rules
// over it. The live, theme-reactive copy is `chartColors` in
// `./chartColors.svelte.ts`; components read that, never a literal.
//
// ── WHERE THE COLOURS LIVE ───────────────────────────────────────────────────
//
// The source of truth is CSS: `--chart-1` … `--chart-8`, `--chart-other`,
// `--chart-in`, `--chart-out` and `--chart-net`, declared per daisyUI theme in
// `src/app.css`. Charts cannot be handed `var(--chart-1)` — SVG presentation
// attributes set from JS, canvas, and any colour maths (layerchart's scales,
// opacity mixing) need a concrete colour string — so `chartColors` resolves the
// tokens with `getComputedStyle` and re-resolves when `<html data-theme>`
// changes. `DEFAULT_PALETTE` below is the fallback for when no stylesheet is
// there to ask (unit tests, jsdom, SSR) or a token is missing; `palette.test.ts`
// parses `app.css` and fails if the dark block and this object disagree.
//
// ── ADDING A THEME ───────────────────────────────────────────────────────────
//
//   1. Add the daisyUI theme to the `@plugin "daisyui"` list in `app.css`.
//   2. Add a `[data-theme="<name>"] { --chart-*: … }` block beside the dark one,
//      all twelve tokens. A missing token silently falls back to the DARK value
//      here, which will be wrong on a light surface.
//   3. Validate the 8 slots, in order, against that theme's `--color-base-100`
//      with the dataviz skill's validator before shipping them.
//
// ── THE DARK VALUES AND WHY ──────────────────────────────────────────────────
//
// Built from daisyUI dark's own semantic hues (primary, warning, info, accent,
// secondary) plus four fills, so the charts carry the same vibrancy as the
// buttons and badges around them. The old palette sat inside the dataviz
// skill's dark lightness band and read as dull next to daisy's saturated UI;
// these are deliberately lifted ABOVE that band. Validated against daisy dark
// `--color-base-100` #1d232a:
//
//   worst ADJACENT CVD ΔE 15.2 · worst normal-vision ΔE 29.5 · every slot ≥ 3:1
//   contrast on the surface
//
// SLOT ORDER IS LOAD-BEARING. Adjacent slots are what share a pie edge or sit
// next to each other in a legend, so the order is what makes the palette
// colour-blind safe; reorder it and re-run the validator. Consumers still
// supply secondary encoding (always-on legends, slice gaps, tooltips) so
// identity is never colour-alone.
//
// The FLOW pair is a diverging encoding for sign (money in above the zero rule,
// out below), not two categorical slots. `in` is exactly daisy `success`
// (#00d390). `out` is NOT daisy `error` (#ff627d): success and error sit at
// nearly the same lightness and collapse under deuteranopia (CVD ΔE 5.1, below
// even the 6 floor). A deeper rose (#e44062) holds the error hue family and
// separates at CVD ΔE 13.3. `net` is `base-content`, the neutral midpoint, and
// the chart draws it over a surface-coloured halo so it is always read against
// the surface rather than a bar.

/** Resolved chart colours for one theme. Every value is a concrete CSS colour, never a `var(...)`. */
export interface ChartPalette {
    /** Categorical slots 1..8, fixed order. */
    readonly categorical: readonly string[];
    /** Muted tail colour for the folded `OTHER_LABEL` bucket — context, not a series identity. */
    readonly other: string;
    /** Money in: a bar above the zero rule. */
    readonly flowIn: string;
    /** Money out: a bar below the zero rule. */
    readonly flowOut: string;
    /** The net line drawn over the two. */
    readonly flowNet: string;
}

/** How many categorical slots every theme provides. Callers fold past this. */
export const SLOT_COUNT = 8;

/** The folded tail's label. One literal: `series.ts` and `holdings/ui/view.ts` both alias this. */
export const OTHER_LABEL = "(other)";

/** The CSS custom property behind each palette entry, as declared in `app.css`. */
export const CHART_TOKENS = {
    categorical: Array.from({length: SLOT_COUNT}, (_, i) => `--chart-${i + 1}`),
    other: "--chart-other",
    flowIn: "--chart-in",
    flowOut: "--chart-out",
    flowNet: "--chart-net",
} as const;

/** daisyUI dark's chart tokens — the fallback when CSS is absent. Must match `app.css` (tested). */
export const DEFAULT_PALETTE: ChartPalette = Object.freeze({
    categorical: Object.freeze([
        "#6c72fb", // 1 indigo  oklch(0.62 0.20 277) ≈ daisy primary, lifted
        "#e7ad01", // 2 amber   oklch(0.78 0.16 84)  ≈ daisy warning
        "#00b3f2", // 3 sky     oklch(0.72 0.15 233) ≈ daisy info
        "#f6722b", // 4 orange  oklch(0.70 0.18 45)
        "#00c7b1", // 5 teal    oklch(0.74 0.13 181) ≈ daisy accent
        "#af6af2", // 6 violet  oklch(0.66 0.20 305)
        "#8ac738", // 7 lime    oklch(0.76 0.18 130)
        "#f74ca1", // 8 pink    oklch(0.68 0.22 354) ≈ daisy secondary
    ]),
    other: "#81878d",
    flowIn: "#00d390",
    flowOut: "#e44062",
    flowNet: "#ecf9ff",
});

/**
 * The colour for categorical slot `i` (0-based).
 *
 * Past the last slot this FOLDS to the muted tail colour rather than cycling
 * back to slot 1 — the dataviz non-negotiable. Callers that can produce more
 * groups than there are slots should be folding their data into an
 * `OTHER_LABEL` bucket before they get here; this is the backstop that keeps a
 * slip from silently painting two different series the same hue.
 */
export function colorAt(palette: ChartPalette, i: number): string {
    return palette.categorical[i] ?? palette.other;
}

/** How opaque the "unknown" wash is: see `unknownColor`. */
export const UNKNOWN_ALPHA = 0.45;

/**
 * The muted colour at partial opacity — for a bucket of value nothing has
 * classified, which a chart must show (it is real money) but must not dress as
 * a category. It sits beside the solid `other` of a folded tail, so it is the
 * same neutral, faded: "less known" rather than "different".
 *
 * `#rgb`/`#rrggbb` get a hex alpha (plain SVG `fill`); any other colour syntax
 * goes through `color-mix`, which every engine the app targets understands.
 */
export function unknownColor(palette: ChartPalette, alpha = UNKNOWN_ALPHA): string {
    const base = palette.other.trim();
    const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(base);
    const byte = Math.round(Math.min(1, Math.max(0, alpha)) * 255)
        .toString(16)
        .padStart(2, "0");
    if (hex !== null) {
        const digits = hex[1].length === 3 ? [...hex[1]].map((d) => d + d).join("") : hex[1];
        return `#${digits}${byte}`;
    }
    return `color-mix(in srgb, ${base} ${Math.round(alpha * 100)}%, transparent)`;
}

/**
 * Build a palette by reading each token through `read` (a custom-property
 * lookup such as `getComputedStyle(el).getPropertyValue`). Any token that
 * reads back empty keeps its `fallback` value, so a theme that omits one
 * degrades to a working colour rather than to an invisible mark.
 */
export function resolvePalette(read: (token: string) => string, fallback: ChartPalette = DEFAULT_PALETTE): ChartPalette {
    const pick = (token: string, backup: string): string => read(token).trim() || backup;
    return Object.freeze({
        categorical: Object.freeze(CHART_TOKENS.categorical.map((token, i) => pick(token, fallback.categorical[i]))),
        other: pick(CHART_TOKENS.other, fallback.other),
        flowIn: pick(CHART_TOKENS.flowIn, fallback.flowIn),
        flowOut: pick(CHART_TOKENS.flowOut, fallback.flowOut),
        flowNet: pick(CHART_TOKENS.flowNet, fallback.flowNet),
    });
}

/** Value equality, so a no-op re-resolve does not invalidate every chart. */
export function samePalette(a: ChartPalette, b: ChartPalette): boolean {
    return (
        a.other === b.other &&
        a.flowIn === b.flowIn &&
        a.flowOut === b.flowOut &&
        a.flowNet === b.flowNet &&
        a.categorical.length === b.categorical.length &&
        a.categorical.every((c, i) => c === b.categorical[i])
    );
}
