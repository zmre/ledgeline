// `applyScale` against a real (jsdom) element, and the reactive root size it
// publishes. jsdom has no layout engine, so the px per rem falls back to the
// arithmetic one (16 × scale) — which is the browser's answer at the default
// 16px root, the one every rem in the stylesheet was designed against.

import {flushSync} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {applyScale, BASE_ROOT_PX, remToPx, rootPx} from "./scale";

afterEach(() => applyScale(100));

describe("COMPONENT applyScale", () => {
    it("sets the scale variable and the root font size inline", () => {
        const root = document.createElement("div");

        applyScale(125, root);

        expect(root.style.getPropertyValue("--ui-scale")).toBe("1.25");
        expect(root.style.fontSize).toBe("125%");
    });

    it("defaults to the document element", () => {
        applyScale(150);

        expect(document.documentElement.style.fontSize).toBe("150%");
        expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe("1.5");
    });

    it("publishes the live px per rem, so rem geometry doubles at 200 %", () => {
        applyScale(100);
        const at100 = remToPx(2.5);
        applyScale(200);

        expect(rootPx()).toBe(BASE_ROOT_PX * 2);
        expect(remToPx(2.5)).toBe(at100 * 2);
    });

    it("re-runs a derivation that reads the root size", () => {
        const seen: number[] = [];
        const stop = $effect.root(() => {
            const pitch = $derived(remToPx(2.5, rootPx()));
            $effect(() => {
                seen.push(pitch);
            });
        });
        flushSync();
        applyScale(70);
        flushSync();
        stop();

        expect(seen).toEqual([40, 28]);
    });
});
