// Reading the theme stylesheets as data, and WCAG contrast over their colours.
// Pure (string in, values out) so the tests that pin themes, palettes and the
// catalog to the CSS share one parser; nothing in the running app needs it.

/** One `@plugin "daisyui/theme" { … }` block. */
export interface DaisyThemeBlock {
    readonly name: string;
    readonly scheme: string;
    readonly isDefault: boolean;
    /** Every `--*` custom property the block declares, in order. */
    readonly tokens: ReadonlyMap<string, string>;
}

/** Every `selectors { body }` rule with no nested braces, comments stripped. */
function rules(css: string): {selectors: string; body: string}[] {
    const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");
    return [...withoutComments.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, prelude, body]) => ({
        // A prelude can carry a preceding `@import …;` statement: the rule starts after the last `;`.
        selectors: prelude.slice(prelude.lastIndexOf(";") + 1).trim(),
        body,
    }));
}

const declarations = (body: string): [string, string][] => [...body.matchAll(/([\w-]+)\s*:\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim()]);

/** The custom properties declared in the rule whose selector list includes `selector`, values lower-cased. */
export function tokensFor(css: string, selector: string): Map<string, string> {
    const rule = rules(css).find((r) => r.selectors.split(",").some((s) => s.trim() === selector));
    if (rule === undefined) return new Map();
    return new Map(
        declarations(rule.body)
            .filter(([name]) => name.startsWith("--"))
            .map(([name, value]) => [name, value.toLowerCase()])
    );
}

/** Every `[data-theme="<id>"]` id that some rule's selector list names. */
export function dataThemeSelectors(css: string): string[] {
    return rules(css).flatMap((r) => [...r.selectors.matchAll(/\[data-theme="([^"]+)"\]/g)].map(([, id]) => id));
}

/** Every daisyUI custom theme the stylesheet declares. */
export function daisyThemes(css: string): DaisyThemeBlock[] {
    return rules(css)
        .filter((r) => r.selectors === '@plugin "daisyui/theme"')
        .map(({body}) => {
            const decls = declarations(body);
            const option = (key: string): string => decls.find(([name]) => name === key)?.[1].replace(/^"|"$/g, "") ?? "";
            return {
                name: option("name"),
                scheme: option("color-scheme"),
                isDefault: option("default") === "true",
                tokens: new Map(decls.filter(([name]) => name.startsWith("--"))),
            };
        });
}

// ── WCAG contrast ────────────────────────────────────────────────────────────

const srgbToLinear = (c: number): number => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

/** OKLCH → linear sRGB, clipped to the gamut (as a browser paints it). */
function oklchToLinear(l: number, c: number, hDeg: number): [number, number, number] {
    const h = (hDeg * Math.PI) / 180;
    const a = c * Math.cos(h);
    const b = c * Math.sin(h);
    const l3 = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m3 = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s3 = (l - 0.0894841775 * a - 1.291485548 * b) ** 3;
    const clip = (v: number): number => Math.min(1, Math.max(0, v));
    return [
        clip(4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3),
        clip(-1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3),
        clip(-0.0041960863 * l3 - 0.7034186147 * m3 + 1.707614701 * s3),
    ];
}

/** Linear sRGB channels of a `#rrggbb` or `oklch(L C H)` colour (the two syntaxes the theme files use). */
function linearRgb(color: string): [number, number, number] {
    const value = color.trim().toLowerCase();
    const hex = /^#([0-9a-f]{6})$/.exec(value);
    if (hex !== null) {
        const n = Number.parseInt(hex[1], 16);
        return [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((x) => srgbToLinear(x / 255)) as [number, number, number];
    }
    const oklch = /^oklch\(\s*([\d.]+)(%?)\s+([\d.]+)\s+([\d.]+)\s*\)$/.exec(value);
    if (oklch !== null) return oklchToLinear(oklch[2] === "%" ? Number(oklch[1]) / 100 : Number(oklch[1]), Number(oklch[3]), Number(oklch[4]));
    throw new Error(`unsupported colour syntax: ${color}`);
}

/** WCAG 2 relative luminance. */
export function relativeLuminance(color: string): number {
    const [r, g, b] = linearRgb(color);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2 contrast ratio between two colours, 1 … 21. */
export function contrastRatio(a: string, b: string): number {
    const [hi, lo] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
}
