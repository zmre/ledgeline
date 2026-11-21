// Every theme the app ships: the families a user picks from, the variants that
// `<html data-theme>` names, and the colours a picker shows as a preview.
//
// The theme CSS (`src/themes/<family>.css`) is the source of truth for colours;
// the swatches here are a copy so a picker can paint previews without mounting
// each theme, and `catalog.test.ts` fails if any swatch drifts from its CSS.
//
// A `theme` pref is either a FAMILY id, which follows the OS light/dark setting
// when the family has both variants, or a VARIANT id, which pins one. A
// dark-only family's id is also its only variant's id (`dracula`), so it is one
// pref either way. `resolveTheme` (`./resolve.ts`) turns a pref into a variant.

export type Scheme = "light" | "dark";

/** Every variant id: the values `<html data-theme>` takes, one daisyUI theme each. */
export const VARIANT_IDS = [
    "default-light",
    "default-dark",
    "github-light",
    "github-dark",
    "catppuccin-latte",
    "catppuccin-mocha",
    "gruvbox-light",
    "gruvbox-dark",
    "tokyo-night-day",
    "tokyo-night-night",
    "solarized-light",
    "solarized-dark",
    "one-dark-pro",
    "dracula",
    "monokai",
    "nord",
] as const;
export type VariantId = (typeof VARIANT_IDS)[number];

/** Every family id, in picker order. */
export const FAMILY_IDS = ["default", "github", "catppuccin", "gruvbox", "tokyo-night", "solarized", "one-dark-pro", "dracula", "monokai", "nord"] as const;
export type FamilyId = (typeof FAMILY_IDS)[number];

/** A saved `theme` pref: a family id (follows the OS when it has both variants) or a variant id (pinned). */
export type ThemePref = FamilyId | VariantId;

/** The pref used when none is saved or the saved one is unknown: the default family, following the OS. */
export const DEFAULT_THEME_PREF: ThemePref = "default";

/** The variant shown when nothing else is known (no pref, no OS answer): today's daisyUI dark. */
export const FALLBACK_VARIANT: VariantId = "default-dark";

/** The colours a picker paints for one variant. Each is a CSS colour exactly as its theme file declares it. */
export interface ThemeSwatch {
    readonly base100: string;
    readonly baseContent: string;
    readonly primary: string;
    readonly secondary: string;
    readonly accent: string;
    /** The first four chart slots, `--chart-1` … `--chart-4`. */
    readonly chart: readonly string[];
}

export interface ThemeVariant {
    readonly id: VariantId;
    readonly label: string;
    readonly family: FamilyId;
    readonly scheme: Scheme;
    readonly swatch: ThemeSwatch;
}

export interface ThemeFamily {
    readonly id: FamilyId;
    readonly label: string;
    /** A family with both follows the OS under its family id; a dark-only family has just `dark`. */
    readonly variants: Readonly<{light?: VariantId; dark?: VariantId}>;
}

const family = (id: FamilyId, label: string, variants: ThemeFamily["variants"]): ThemeFamily => Object.freeze({id, label, variants: Object.freeze(variants)});

export const THEME_FAMILIES: readonly ThemeFamily[] = Object.freeze([
    family("default", "Default", {light: "default-light", dark: "default-dark"}),
    family("github", "GitHub", {light: "github-light", dark: "github-dark"}),
    family("catppuccin", "Catppuccin", {light: "catppuccin-latte", dark: "catppuccin-mocha"}),
    family("gruvbox", "Gruvbox", {light: "gruvbox-light", dark: "gruvbox-dark"}),
    family("tokyo-night", "Tokyo Night", {light: "tokyo-night-day", dark: "tokyo-night-night"}),
    family("solarized", "Solarized", {light: "solarized-light", dark: "solarized-dark"}),
    family("one-dark-pro", "One Dark Pro", {dark: "one-dark-pro"}),
    family("dracula", "Dracula", {dark: "dracula"}),
    family("monokai", "Monokai", {dark: "monokai"}),
    family("nord", "Nord", {dark: "nord"}),
]);

const variant = (id: VariantId, label: string, familyId: FamilyId, scheme: Scheme, swatch: ThemeSwatch): ThemeVariant =>
    Object.freeze({id, label, family: familyId, scheme, swatch: Object.freeze({...swatch, chart: Object.freeze([...swatch.chart])})});

// Swatch values are copied from src/themes/*.css (tested). daisyUI's default
// themes declare oklch(); the others use their upstream hex.
export const THEME_VARIANTS: readonly ThemeVariant[] = Object.freeze([
    variant("default-light", "Default Light", "default", "light", {
        base100: "oklch(100% 0 0)",
        baseContent: "oklch(21% 0.006 285.885)",
        primary: "oklch(45% 0.24 277.023)",
        secondary: "oklch(65% 0.241 354.308)",
        accent: "oklch(77% 0.152 181.912)",
        chart: ["#422ad5", "#16a773", "#0088ff", "#00ab30"],
    }),
    variant("default-dark", "Default Dark", "default", "dark", {
        base100: "oklch(25.33% 0.016 252.42)",
        baseContent: "oklch(97.807% 0.029 256.847)",
        primary: "oklch(58% 0.233 277.117)",
        secondary: "oklch(65% 0.241 354.308)",
        accent: "oklch(77% 0.152 181.912)",
        chart: ["#615dff", "#00c184", "#f02bc9", "#ff942e"],
    }),
    variant("github-light", "GitHub Light", "github", "light", {
        base100: "#ffffff",
        baseContent: "#1f2328",
        primary: "#0969da",
        secondary: "#bf3989",
        accent: "#bc4c00",
        chart: ["#0969da", "#2da44e", "#8250df", "#bc4c00"],
    }),
    variant("github-dark", "GitHub Dark", "github", "dark", {
        base100: "#0d1117",
        baseContent: "#e6edf3",
        primary: "#58a6ff",
        secondary: "#a371f7",
        accent: "#ffa657",
        chart: ["#58a6ff", "#3fb950", "#d2a8ff", "#f85149"],
    }),
    variant("catppuccin-latte", "Catppuccin Latte", "catppuccin", "light", {
        base100: "#eff1f5",
        baseContent: "#4c4f69",
        primary: "#8839ef",
        secondary: "#e64553",
        accent: "#179299",
        chart: ["#8839ef", "#e64553", "#1393cb", "#d20f39"],
    }),
    variant("catppuccin-mocha", "Catppuccin Mocha", "catppuccin", "dark", {
        base100: "#1e1e2e",
        baseContent: "#cdd6f4",
        primary: "#cba6f7",
        secondary: "#eba0ac",
        accent: "#f9e2af",
        chart: ["#cba6f7", "#f5e0dc", "#f38ba8", "#f9e2af"],
    }),
    variant("gruvbox-light", "Gruvbox Light", "gruvbox", "light", {
        base100: "#fbf1c7",
        baseContent: "#3c3836",
        primary: "#af3a03",
        secondary: "#9d0006",
        accent: "#b57614",
        chart: ["#af3a03", "#619563", "#9d0006", "#8f8e03"],
    }),
    variant("gruvbox-dark", "Gruvbox Dark", "gruvbox", "dark", {
        base100: "#282828",
        baseContent: "#ebdbb2",
        primary: "#fe8019",
        secondary: "#fb4934",
        accent: "#fabd2f",
        chart: ["#fe8019", "#d73128", "#8ec07c", "#d65d0e"],
    }),
    variant("tokyo-night-day", "Tokyo Night Day", "tokyo-night", "light", {
        base100: "#e1e2e7",
        baseContent: "#3760bf",
        primary: "#2e7de9",
        secondary: "#7847bd",
        accent: "#b15c00",
        chart: ["#2e7de9", "#d20065", "#007197", "#587539"],
    }),
    variant("tokyo-night-night", "Tokyo Night Night", "tokyo-night", "dark", {
        base100: "#1a1b26",
        baseContent: "#c0caf5",
        primary: "#7aa2f7",
        secondary: "#9d7cd8",
        accent: "#ff9e64",
        chart: ["#7aa2f7", "#9ece6a", "#9d7cd8", "#e0af68"],
    }),
    variant("solarized-light", "Solarized Light", "solarized", "light", {
        base100: "#fdf6e3",
        baseContent: "#586e75",
        primary: "#268bd2",
        secondary: "#d33682",
        accent: "#b58900",
        chart: ["#268bd2", "#dc322f", "#a0045c", "#b1860e"],
    }),
    variant("solarized-dark", "Solarized Dark", "solarized", "dark", {
        base100: "#002b36",
        baseContent: "#93a1a1",
        primary: "#268bd2",
        secondary: "#d33682",
        accent: "#b58900",
        chart: ["#268bd2", "#859900", "#6c71c4", "#b58900"],
    }),
    variant("one-dark-pro", "One Dark Pro", "one-dark-pro", "dark", {
        base100: "#282c34",
        baseContent: "#abb2bf",
        primary: "#61afef",
        secondary: "#e06c75",
        accent: "#e5c07b",
        chart: ["#61afef", "#98c379", "#ac5fc3", "#e5c07b"],
    }),
    variant("dracula", "Dracula", "dracula", "dark", {
        base100: "#282a36",
        baseContent: "#f8f8f2",
        primary: "#ff79c6",
        secondary: "#bd93f9",
        accent: "#ffb86c",
        chart: ["#ff79c6", "#f1fa8c", "#4faec2", "#50fa7b"],
    }),
    variant("monokai", "Monokai", "monokai", "dark", {
        base100: "#272822",
        baseContent: "#f8f8f2",
        primary: "#66d9ef",
        secondary: "#ae81ff",
        accent: "#fd971f",
        chart: ["#66d9ef", "#f92672", "#e6db74", "#1a9fb4"],
    }),
    variant("nord", "Nord", "nord", "dark", {
        base100: "#2e3440",
        baseContent: "#d8dee9",
        primary: "#88c0d0",
        secondary: "#d08770",
        accent: "#ebcb8b",
        chart: ["#88c0d0", "#bf616a", "#a3be8c", "#5e81ac"],
    }),
]);

/**
 * Every accepted `theme` pref, 22 in all: the six follow-the-OS family ids,
 * then the sixteen variant ids (which include the four dark-only family ids).
 * The server's `theme` Choice must accept exactly these.
 */
export const THEME_PREF_IDS: readonly ThemePref[] = Object.freeze([
    ...THEME_FAMILIES.filter((f) => f.variants.light !== undefined && f.variants.dark !== undefined).map((f) => f.id),
    ...VARIANT_IDS,
]);

const VARIANTS_BY_ID: ReadonlyMap<string, ThemeVariant> = new Map(THEME_VARIANTS.map((v) => [v.id, v]));
const FAMILIES_BY_ID: ReadonlyMap<string, ThemeFamily> = new Map(THEME_FAMILIES.map((f) => [f.id, f]));

export const isVariantId = (id: string): id is VariantId => VARIANTS_BY_ID.has(id);
export const isFamilyId = (id: string): id is FamilyId => FAMILIES_BY_ID.has(id);
export const isThemePref = (id: string): id is ThemePref => isVariantId(id) || isFamilyId(id);

/** The variant with this id, or `undefined`. */
export const themeVariant = (id: string): ThemeVariant | undefined => VARIANTS_BY_ID.get(id);

/** The family with this id, or `undefined`. */
export const themeFamily = (id: string): ThemeFamily | undefined => FAMILIES_BY_ID.get(id);
