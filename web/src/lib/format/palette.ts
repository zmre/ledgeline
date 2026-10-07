// The ONE chart palette: its shape, its fallback values, and the pure rules
// over it. The live, theme-reactive copy is `chartColors` in
// `./chartColors.svelte.ts`; components read that, never a literal.
//
// ── WHERE THE COLOURS LIVE ───────────────────────────────────────────────────
//
// The source of truth is CSS: `--chart-1` … `--chart-N`, `--chart-other`,
// `--chart-in`, `--chart-out` and `--chart-net`, declared per daisyUI theme in
// `src/app.css`. A theme defines as many categorical `--chart-N` slots as it
// likes, contiguous from 1: `resolvePalette` reads `--chart-1`, `--chart-2`, …
// up to the first empty one, and every chart sizes itself to that count
// (`palette.categorical.length`), folding to `other` only past the last slot.
// Charts cannot be handed `var(--chart-1)` — SVG presentation attributes set
// from JS, canvas, and any colour maths (layerchart's scales, opacity mixing)
// need a concrete colour string — so `chartColors` resolves the tokens with
// `getComputedStyle` and re-resolves when `<html data-theme>` changes.
// `DEFAULT_PALETTE` below is the fallback for when no stylesheet is there to
// ask (unit tests, jsdom, SSR) or the theme defines no categorical slots at
// all; `palette.test.ts` parses `app.css` and fails if the dark block and this
// object disagree.
//
// ── ADDING A THEME ───────────────────────────────────────────────────────────
//
//   1. Add the daisyUI theme to the `@plugin "daisyui"` list in `app.css`.
//   2. Add a `[data-theme="<name>"] { --chart-*: … }` block beside the dark one:
//      any number of `--chart-N` slots (no gaps — reading stops at the first
//      missing one) plus the four named tokens. A missing named token silently
//      falls back to the DARK value here, which will be wrong on a light surface.
//   3. Validate the slots, in order, against that theme's `--color-base-100`
//      with the dataviz skill's validator before shipping them.
//
// ── THE DARK VALUES AND WHY ──────────────────────────────────────────────────
//
// Fourteen hues generated from daisyUI dark's primary (#615dff) and warning
// (#fcb700) as anchors, so the charts carry the same vibrancy as the buttons
// and badges around them, then REORDERED so neighbours differ in both hue and
// lightness. Validated against daisy dark `--color-base-100` #1d232a:
//
//   worst ADJACENT CVD ΔE 12.6 · worst adjacent normal-vision ΔE 30.7 · every
//   slot ≥ 3:1 contrast on the surface
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
    /** Categorical slots `--chart-1` … `--chart-N`, fixed order; its length is the chart's slot count. */
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

/**
 * The most `--chart-N` slots `resolvePalette` will read — a safety cap on the
 * probe, far past what any validated palette holds.
 */
export const MAX_SLOTS = 32;

/** The folded tail's label. One literal: `series.ts` and `holdings/ui/view.ts` both alias this. */
export const OTHER_LABEL = "(other)";

/** The CSS custom property for categorical slot `i` (0-based): `--chart-{i+1}`. */
export const slotToken = (i: number): string => `--chart-${i + 1}`;

/** The CSS custom property behind each named palette entry, as declared in `app.css`. */
export const CHART_TOKENS = {
    other: "--chart-other",
    flowIn: "--chart-in",
    flowOut: "--chart-out",
    flowNet: "--chart-net",
} as const;

/** daisyUI dark's chart tokens — the fallback when CSS is absent. Must match `app.css` (tested). */
export const DEFAULT_PALETTE: ChartPalette = Object.freeze({
    categorical: Object.freeze([
        "#615dff", //  1 indigo   = daisy primary
        "#00c184", //  2 green
        "#f02bc9", //  3 magenta
        "#ff942e", //  4 orange
        "#b947ea", //  5 violet
        "#a3c500", //  6 lime
        "#ff4279", //  7 rose
        "#00b5cc", //  8 cyan
        "#ff6c53", //  9 coral
        "#0088ff", // 10 blue
        "#00c83a", // 11 emerald
        "#00a3ff", // 12 sky
        "#ff21a2", // 13 pink
        "#fcb700", // 14 amber    = daisy warning
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
 * lookup such as `getComputedStyle(el).getPropertyValue`).
 *
 * Categorical slots are read `--chart-1`, `--chart-2`, … up to the first that
 * reads back empty (at most `MAX_SLOTS`), so a theme sets its own slot count. A
 * theme that defines none keeps `fallback`'s slots. Each named token that reads
 * back empty keeps its `fallback` value, so a theme that omits one degrades to
 * a working colour rather than to an invisible mark.
 */
export function resolvePalette(read: (token: string) => string, fallback: ChartPalette = DEFAULT_PALETTE): ChartPalette {
    const pick = (token: string, backup: string): string => read(token).trim() || backup;
    const slots: string[] = [];
    for (let i = 0; i < MAX_SLOTS; i++) {
        const color = read(slotToken(i)).trim();
        if (color === "") break;
        slots.push(color);
    }
    return Object.freeze({
        categorical: Object.freeze(slots.length > 0 ? slots : [...fallback.categorical]),
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
