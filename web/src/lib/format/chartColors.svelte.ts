// The live chart palette: `app.css`'s `--chart-*` tokens resolved to concrete
// colour strings, re-resolved whenever `<html data-theme>` changes.
//
// Why resolve at all, rather than hand charts `var(--chart-1)`: see the header
// of `./palette.ts`. Reading any getter here inside a component template or a
// `$derived` subscribes to it, so a theme switch recolours every chart live.

import {colorAt, DEFAULT_PALETTE, resolvePalette, samePalette, type ChartPalette} from "./palette";

/** Where the tokens are read from, and how. Injected in tests; the app uses the defaults. */
export interface ChartColorsSource {
    /** The element whose `data-theme` is watched and whose computed style is read. */
    root: HTMLElement | null;
    /** Look up one custom property; empty string when unset. */
    read: (token: string) => string;
}

/** The read-only, reactive palette every chart component uses. */
export interface ChartColors {
    /** The whole resolved palette — hand this to a pure module that takes a `ChartPalette`. */
    readonly current: ChartPalette;
    readonly categorical: readonly string[];
    readonly other: string;
    readonly flowIn: string;
    readonly flowOut: string;
    readonly flowNet: string;
    /** Slot `i`'s colour, folding to `other` past the last slot (never cycling). */
    colorAt(i: number): string;
    /** Re-read the tokens now. The attribute observer calls this; exposed for a caller that swaps stylesheets. */
    refresh(): void;
    /** Stop watching `data-theme`. Only a test needs this; the app instance lives as long as the page. */
    destroy(): void;
}

function browserSource(): ChartColorsSource {
    if (typeof document === "undefined" || typeof getComputedStyle === "undefined") return {root: null, read: () => ""};
    const root = document.documentElement;
    return {root, read: (token) => getComputedStyle(root).getPropertyValue(token)};
}

class ReactiveChartColors implements ChartColors {
    #current = $state.raw<ChartPalette>(DEFAULT_PALETTE);
    readonly #read: (token: string) => string;
    readonly #observer: MutationObserver | null;

    constructor({root, read}: ChartColorsSource) {
        this.#read = read;
        this.refresh();
        if (root === null || typeof MutationObserver === "undefined") {
            this.#observer = null;
            return;
        }
        this.#observer = new MutationObserver(() => this.refresh());
        this.#observer.observe(root, {attributes: true, attributeFilter: ["data-theme"]});
    }

    get current(): ChartPalette {
        return this.#current;
    }
    get categorical(): readonly string[] {
        return this.#current.categorical;
    }
    get other(): string {
        return this.#current.other;
    }
    get flowIn(): string {
        return this.#current.flowIn;
    }
    get flowOut(): string {
        return this.#current.flowOut;
    }
    get flowNet(): string {
        return this.#current.flowNet;
    }

    colorAt(i: number): string {
        return colorAt(this.#current, i);
    }

    refresh(): void {
        const next = resolvePalette(this.#read);
        if (!samePalette(next, this.#current)) this.#current = next;
    }

    destroy(): void {
        this.#observer?.disconnect();
    }
}

/** A palette bound to `source`. Tests build their own; the app uses `chartColors`. */
export function createChartColors(source: ChartColorsSource = browserSource()): ChartColors {
    return new ReactiveChartColors(source);
}

/** The app's palette, resolved from `document.documentElement`. Falls back to `DEFAULT_PALETTE` off-browser. */
export const chartColors: ChartColors = createChartColors();
