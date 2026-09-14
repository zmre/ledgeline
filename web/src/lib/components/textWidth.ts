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

// Per font size, because a second caller at a different size arrived (the
// Balances list renders account names at `text-sm`, 14px) and one shared
// context would have measured its labels a sixth narrow — the exact silent
// wrongness the note above says is worse than not measuring at all. The size is
// the key for the contexts AND for the memoized widths; sharing the width cache
// across sizes would be the same bug wearing a hat.
//
// A missing entry = not yet attempted, `null` = attempted and unavailable.
const contexts = new Map<number, CanvasRenderingContext2D | null>();
const widths = new Map<number, Map<string, number>>();

// Account names are a bounded set in any real journal, but a pathological
// import should not be able to grow this without limit.
const CACHE_LIMIT = 4096;

function contextFor(fontSizePx: number): CanvasRenderingContext2D | null {
    const existing = contexts.get(fontSizePx);
    if (existing !== undefined) return existing;
    let context: CanvasRenderingContext2D | null = null;
    contexts.set(fontSizePx, context);
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
        `${style.fontStyle || "normal"} ${style.fontWeight || "400"} ${fontSizePx}px ${families}`,
        `${fontSizePx}px ${families.split(",")[0].trim()}`,
    ]) {
        measured.font = candidate;
        if (measured.font.includes(`${fontSizePx}px`)) {
            context = measured;
            contexts.set(fontSizePx, context);
            return context;
        }
    }
    return context;
}

/**
 * A measurer for text rendered at `fontSizePx` in the document's body font, or
 * `null` on an engine that cannot measure. Callers must handle `null` — see the
 * note above on why it is not a guess.
 *
 * Pass the size the text is ACTUALLY rendered at. A measurer borrowed from a
 * different size answers confidently and wrongly, and the caller has no way to
 * tell.
 */
export function textMeasurer(fontSizePx: number): MeasureText | null {
    const ctx = contextFor(fontSizePx);
    if (ctx === null) return null;
    let cache = widths.get(fontSizePx);
    if (cache === undefined) {
        cache = new Map();
        widths.set(fontSizePx, cache);
    }
    const sized = cache;
    return (text: string): number => {
        const cached = sized.get(text);
        if (cached !== undefined) return cached;
        const width = ctx.measureText(text).width;
        if (sized.size >= CACHE_LIMIT) sized.clear();
        sized.set(text, width);
        return width;
    };
}

/** A measurer for text rendered in a journal account chip (daisyUI `badge-sm`, 12px). */
export function chipMeasurer(): MeasureText | null {
    return textMeasurer(CHIP_FONT_SIZE_PX);
}

/** Testing seam: drop every memoized context and width so a fake can be installed. */
export function resetChipMeasurer(): void {
    contexts.clear();
    widths.clear();
}
