// Text measurement, and the ways it is allowed to fail.
//
// Named `.svelte.test.ts` purely to route it to the `components` vitest project,
// which is the one with a jsdom `document`; there is no component here. See
// vite.config.ts for the split.
//
// The theme of every case below is that measuring WRONG is worse than not
// measuring. A wrong number is spent silently: labels get fitted to a chip
// wider than the real one and the browser clips them, which looks exactly like
// the bug this whole change was meant to remove.

import {afterEach, describe, expect, it, vi} from "vitest";
import {chipMeasurer, resetChipMeasurer} from "./textWidth";

function withCanvas(context: unknown): void {
    resetChipMeasurer();
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(context as never);
}

afterEach(() => {
    vi.restoreAllMocks();
    resetChipMeasurer();
});

describe("UNIT textWidth", () => {
    it("declines to measure on an engine with no 2D canvas", () => {
        // Real jsdom, and any locked-down webview.
        withCanvas(null);

        expect(chipMeasurer()).toBeNull();
    });

    it("REGRESSION: declines to measure when the font string was not accepted", () => {
        // Assigning an unparseable font to a canvas is a SILENT no-op — the
        // context keeps its default `10px sans-serif` and goes on answering
        // about a sixth narrow, forever. Reading the value back is the only way
        // to notice, and refusing is the only safe response.
        const context = {measureText: (text: string) => ({width: text.length * 5})};
        Object.defineProperty(context, "font", {get: () => "10px sans-serif", set: () => {}});
        withCanvas(context);

        expect(chipMeasurer()).toBeNull();
    });

    it("measures once the font has taken", () => {
        const context = {font: "", measureText: (text: string) => ({width: text.length * 6})};
        withCanvas(context);
        const measure = chipMeasurer();

        expect(measure).not.toBeNull();
        expect(measure?.("abcd")).toBe(24);
    });

    it("asks the canvas once per distinct string", () => {
        // Account names repeat constantly down a journal, and this runs while a
        // virtualized list scrolls.
        let calls = 0;
        const context = {
            font: "",
            measureText: (text: string) => {
                calls += 1;
                return {width: text.length * 6};
            },
        };
        withCanvas(context);
        const measure = chipMeasurer();
        measure?.("expenses:auto:maintenance");
        measure?.("expenses:auto:maintenance");
        measure?.("expenses:auto:maintenance");

        expect(calls).toBe(1);
    });
});
