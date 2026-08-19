// How wide a string will be, asked before the browser has laid it out.
//
// `canvas.measureText` is the right instrument for this. It reads the same font
// metrics the layout engine will use, and — unlike putting the string in a probe
// element and reading `offsetWidth` — it neither reads nor invalidates layout,
// so asking it sixty times while rendering a virtualized list costs no reflow at
// all. Results are memoized per string on top of that, and account names repeat
// constantly down a journal, so in practice a scroll measures almost nothing.
//
// Everything here degrades to `null` rather than to a guess. jsdom has no 2D
// canvas (and no layout engine either), server rendering has no `document`, and
// a wrong number would be worse than none: callers fall back to the character
// budget in `accounts.ts`, which is imprecise but never claims otherwise.

import type {MeasureText} from "$lib/domain/accounts";

// The journal's account chips are daisyUI `badge-sm`, whose `font-size` is
// 0.75rem. There is no chip in the document at the time the first measurement is
// wanted, so this is read from the stylesheet's constant rather than from an
// element; the family still comes from the live document, since that is the part
// a theme or a user stylesheet actually changes.
const CHIP_FONT_SIZE_PX = 12;

// `undefined` = not yet attempted, `null` = attempted and unavailable.
let context: CanvasRenderingContext2D | null | undefined;
const widths = new Map<string, number>();

// Account names are a bounded set in any real journal, but a pathological
// import should not be able to grow this without limit.
const CACHE_LIMIT = 4096;

function chipContext(): CanvasRenderingContext2D | null {
    if (context !== undefined) return context;
    context = null;
    if (typeof document === "undefined" || document.body === null) return context;
    const canvas = document.createElement("canvas");
    const measured = canvas.getContext("2d");
    if (measured === null) return context;
    const style = getComputedStyle(document.body);
    const families = style.fontFamily || "sans-serif";
    // Assembled from longhands because the `font` shorthand is not reliably
    // serialized by every engine.
    //
    // Then CHECKED, which is the part that matters: assigning an unparseable
    // font string to a canvas is a SILENT NO-OP — the context keeps its default
    // `10px sans-serif` and goes on answering, about a sixth narrow, forever.
    // That failure is worse than not measuring, because every label would be
    // fitted to a chip wider than the real one and then clipped by CSS. Reading
    // the value back is the only way to know it took; if none of these parse,
    // measuring is abandoned rather than done wrong.
    for (const candidate of [
        `${style.fontStyle || "normal"} ${style.fontWeight || "400"} ${CHIP_FONT_SIZE_PX}px ${families}`,
        `${CHIP_FONT_SIZE_PX}px ${families.split(",")[0].trim()}`,
    ]) {
        measured.font = candidate;
        if (measured.font.includes(`${CHIP_FONT_SIZE_PX}px`)) {
            context = measured;
            return context;
        }
    }
    return context;
}

/**
 * A measurer for text rendered in a journal account chip, or `null` on an engine
 * that cannot measure. Callers must handle `null` — see the note above on why it
 * is not a guess.
 */
export function chipMeasurer(): MeasureText | null {
    const ctx = chipContext();
    if (ctx === null) return null;
    return (text: string): number => {
        const cached = widths.get(text);
        if (cached !== undefined) return cached;
        const width = ctx.measureText(text).width;
        if (widths.size >= CACHE_LIMIT) widths.clear();
        widths.set(text, width);
        return width;
    };
}

/** Testing seam: drop the memoized context and widths so a fake can be installed. */
export function resetChipMeasurer(): void {
    context = undefined;
    widths.clear();
}
