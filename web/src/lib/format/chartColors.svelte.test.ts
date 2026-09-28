// The live palette: resolves the `--chart-*` tokens, falls back when they are
// absent, and re-resolves when `<html data-theme>` changes. Runs in jsdom for
// `MutationObserver`; the token lookup is injected so the test does not depend
// on jsdom's partial `getComputedStyle`.

import {flushSync} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {chartColors, createChartColors, type ChartColors} from "./chartColors.svelte";
import {DEFAULT_PALETTE} from "./palette";

const THEMES: Record<string, Record<string, string>> = {
    dark: {"--chart-1": "#111111", "--chart-other": "#222222", "--chart-in": "#333333"},
    other: {"--chart-1": "#aaaaaa", "--chart-other": "#bbbbbb", "--chart-in": "#cccccc"},
};

/** Microtask turn, which is when a `MutationObserver` callback runs. */
const observed = (): Promise<void> => new Promise((resolve) => queueMicrotask(resolve));

describe("COMPONENT chartColors", () => {
    let root: HTMLElement;
    let colors: ChartColors | null = null;

    const build = (): ChartColors => {
        root = document.createElement("div");
        root.setAttribute("data-theme", "dark");
        colors = createChartColors({root, read: (token) => THEMES[root.getAttribute("data-theme") ?? ""]?.[token] ?? ""});
        return colors;
    };

    afterEach(() => colors?.destroy());

    it("uses the tokens that are present", () => {
        const c = build();
        expect(c.colorAt(0)).toBe("#111111");
        expect(c.other).toBe("#222222");
        expect(c.flowIn).toBe("#333333");
    });

    it("falls back to DEFAULT_PALETTE for tokens that are empty", () => {
        const c = build();
        expect(c.categorical.slice(1)).toEqual(DEFAULT_PALETTE.categorical.slice(1));
        expect(c.flowOut).toBe(DEFAULT_PALETTE.flowOut);
        expect(c.flowNet).toBe(DEFAULT_PALETTE.flowNet);
    });

    it("folds past the last slot to `other`, never cycling", () => {
        const c = build();
        expect(c.colorAt(DEFAULT_PALETTE.categorical.length)).toBe(c.other);
        expect(c.colorAt(99)).toBe(c.other);
    });

    it("re-resolves when data-theme changes, and a reader sees the change", async () => {
        const c = build();
        const seen: string[] = [];
        const stop = $effect.root(() => {
            $effect(() => {
                seen.push(c.colorAt(0));
            });
        });
        flushSync();

        root.setAttribute("data-theme", "other");
        await observed();
        flushSync();

        expect(c.colorAt(0)).toBe("#aaaaaa");
        expect(c.other).toBe("#bbbbbb");
        expect(seen).toEqual(["#111111", "#aaaaaa"]);
        stop();
    });

    it("ignores other attribute changes and stops watching once destroyed", async () => {
        const c = build();
        const before = c.current;
        root.setAttribute("class", "whatever");
        await observed();
        expect(c.current).toBe(before);

        c.destroy();
        root.setAttribute("data-theme", "other");
        await observed();
        expect(c.colorAt(0)).toBe("#111111");
    });

    it("the app instance resolves to the dark defaults when no stylesheet is loaded", () => {
        // jsdom loads no app.css, so every token is empty: this is the SSR/test path.
        expect(chartColors.current).toEqual(DEFAULT_PALETTE);
    });
});
