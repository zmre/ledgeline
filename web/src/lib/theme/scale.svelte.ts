// The live size of one rem in CSS px, as runes state.
//
// Geometry code that encodes rendered text or row sizes (the journal's row
// pitch, the chip measurer's font size, chart paddings) reads `rootPx()` inside
// a `$derived`, so it re-derives when the interface scale changes. Only
// `applyScale` in `scale.ts` writes it. Import from `scale.ts`, which
// re-exports everything here; this file exists only because runes need the
// `.svelte.ts` suffix.

/** CSS px per rem at 100 %: the browser default every rem in the stylesheet was designed against. */
export const BASE_ROOT_PX = 16;

/**
 * The root's computed font size in px, or null when nothing can say: no
 * document (tests, server rendering), or an engine without a cascade that
 * resolves lengths (jsdom hands back the specified "125%", not a size).
 */
export function measureRootPx(root?: HTMLElement): number | null {
    if (typeof document === "undefined" || typeof getComputedStyle === "undefined") return null;
    const size = getComputedStyle(root ?? document.documentElement).fontSize;
    const measured = size.endsWith("px") ? Number.parseFloat(size) : Number.NaN;
    return Number.isFinite(measured) && measured > 0 ? measured : null;
}

let px = $state(measureRootPx() ?? BASE_ROOT_PX);

/** CSS px per rem right now. Reactive: read it inside a `$derived` and the derivation follows the scale. */
export function rootPx(): number {
    return px;
}

/** Written by `applyScale` only (and tests). */
export function setRootPx(next: number): void {
    px = next;
}
