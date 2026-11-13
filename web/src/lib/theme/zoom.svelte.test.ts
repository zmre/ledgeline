import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {rootPx} from "./scale";
import {
    currentScale,
    DEDUPE_MS,
    listenForMenuZoom,
    nextScale,
    resetZoomForTests,
    setScaleSink,
    zoom,
    zoomIn,
    zoomOut,
    zoomReset,
    ZOOM_EVENT,
} from "./zoom.svelte";

beforeEach(() => {
    vi.useFakeTimers();
    resetZoomForTests();
});
afterEach(() => {
    resetZoomForTests();
    vi.useRealTimers();
    document.documentElement.removeAttribute("style");
});

describe("COMPONENT zoom", () => {
    it("nextScale steps, and reset is actual size from anywhere", () => {
        expect(nextScale(100, "in")).toBe(110);
        expect(nextScale(100, "out")).toBe(90);
        expect(nextScale(175, "reset")).toBe(100);
        expect(nextScale(133, "in")).toBe(150);
    });

    it("by default applies the scale to the document and remembers it locally", () => {
        zoomIn();
        vi.advanceTimersByTime(DEDUPE_MS);
        zoomIn();

        expect(currentScale()).toBe(125);
        expect(document.documentElement.style.fontSize).toBe("125%");
        expect(rootPx()).toBe(20);

        zoomReset();
        expect(currentScale()).toBe(100);
        expect(document.documentElement.style.fontSize).toBe("100%");
    });

    it("hands new scales to a pluggable sink, stepping from the sink's own reading", () => {
        const saved: number[] = [];
        let stored = 150;
        setScaleSink(
            (percent) => {
                saved.push(percent);
                stored = percent;
            },
            () => stored
        );

        zoomOut();
        zoomOut();

        expect(saved).toEqual([125, 110]);
    });

    it("drops the other route's echo of the same press, inside the de-dupe window only", () => {
        const sink = vi.fn();
        setScaleSink(sink);

        expect(zoom("in", "keyboard")).toBe(true);
        expect(zoom("in", "menu")).toBe(false);
        expect(sink).toHaveBeenCalledTimes(1);

        vi.advanceTimersByTime(DEDUPE_MS);
        expect(zoom("in", "menu")).toBe(true);
        expect(sink).toHaveBeenCalledTimes(2);
    });

    it("never drops a repeat from the same route (holding the key), nor a different action", () => {
        const sink = vi.fn();
        setScaleSink(sink);

        zoom("in", "keyboard");
        zoom("in", "keyboard");
        zoom("out", "menu");

        expect(sink).toHaveBeenCalledTimes(3);
    });

    it("answers the desktop menu's ledgeline:zoom event, ignoring unknown details", () => {
        const sink = vi.fn();
        setScaleSink(sink, () => 100);
        const stop = listenForMenuZoom();

        window.dispatchEvent(new CustomEvent(ZOOM_EVENT, {detail: "out"}));
        window.dispatchEvent(new CustomEvent(ZOOM_EVENT, {detail: "sideways"}));
        window.dispatchEvent(new CustomEvent(ZOOM_EVENT));
        stop();
        vi.advanceTimersByTime(DEDUPE_MS);
        window.dispatchEvent(new CustomEvent(ZOOM_EVENT, {detail: "in"}));

        expect(sink.mock.calls).toEqual([[90]]);
    });

    it("zoomOut clamps at the smallest step", () => {
        const sink = vi.fn();
        setScaleSink(sink, () => 70);

        zoomOut();

        expect(sink).toHaveBeenCalledWith(70);
    });
});
