// The `app` container's width, for the few decisions JS has to make the same
// way the stylesheet's `md:` does (plan 42).
//
// The breakpoints are CONTAINER queries on <body> (app.css, plan 39), so
// `matchMedia` cannot ask them; the container's inline size is <body>'s
// content box, which a ResizeObserver reports exactly. Compared in rem at the
// live root size, so a zoom moves the answer the way it moves the CSS.
//
// Null until measured (the first frame, and jsdom, which has no
// ResizeObserver): callers treat "unknown" as the desktop layout, so nothing
// changes for tests or for a browser that cannot say.

import {rootPx} from "#lib/theme/scale.js";

/** The `md` breakpoint, in rem (app.css: `@custom-variant md`). */
export const MD_REM = 48;

let width = $state<number | null>(null);
let observer: ResizeObserver | null = null;

function observe(): void {
    if (observer !== null || typeof document === "undefined" || typeof ResizeObserver === "undefined") return;
    observer = new ResizeObserver((entries) => {
        const box = entries[entries.length - 1]?.contentBoxSize?.[0];
        width = box?.inlineSize ?? entries[entries.length - 1]?.contentRect.width ?? null;
    });
    observer.observe(document.body);
}

/** Whether a container `widthPx` wide is below `md` at a root of `root` px. Null width = unknown = not narrow. */
export function isBelowMd(widthPx: number | null, root: number): boolean {
    return widthPx !== null && widthPx < MD_REM * root;
}

/** Whether the app is laid out below `md` right now (reactive). False until measured. */
export function belowMd(): boolean {
    observe();
    return isBelowMd(width, rootPx());
}
