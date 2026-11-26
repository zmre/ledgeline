// The shared zoom keys (plan 39), as both apps register them: each step lands
// in the scale sink, from a field too, and the browser's own zoom is stopped.
import {render} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import KeysProbe from "#ui/testing/fixtures/KeysProbe.svelte";
import {keymap} from "./keymap.svelte";
import {zoomBindings} from "./zoomBindings";
import {DEDUPE_MS, resetZoomForTests, setScaleSink} from "../theme/zoom.svelte";

beforeEach(() => {
    vi.useFakeTimers();
    resetZoomForTests();
});
afterEach(() => {
    keymap.reset();
    resetZoomForTests();
    vi.useRealTimers();
    document.documentElement.removeAttribute("style");
});

function press(key: string, target: EventTarget = document.body): KeyboardEvent {
    const event = new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true, ctrlKey: true});
    target.dispatchEvent(event);
    keymap.handle(event);
    vi.advanceTimersByTime(DEDUPE_MS);
    return event;
}

describe("COMPONENT zoomBindings", () => {
    it("lists Zoom in, Zoom out and Actual size once each, under the app's section", () => {
        const bindings = zoomBindings("Global");
        expect(bindings.map((b) => b.keys)).toEqual(["mod++", "mod+=", "mod+-", "mod+0"]);
        expect(bindings.every((b) => b.group === "Global" && b.always === true)).toBe(true);
        expect(bindings.filter((b) => b.help !== false).map((b) => b.label)).toEqual(["Zoom in", "Zoom out", "Actual size"]);
    });

    it("steps the saved scale from anywhere, a field included, and keeps the browser from zooming", () => {
        const saved: number[] = [];
        let scale = 100;
        setScaleSink(
            (percent) => {
                scale = percent;
                saved.push(percent);
            },
            () => scale
        );
        render(KeysProbe, {props: {layerOf: () => ({id: "global", bindings: zoomBindings("Global")})}});
        const input = document.createElement("input");
        document.body.append(input);

        const first = press("=", input);
        press("+");
        press("-");
        press("0");
        input.remove();

        expect(saved).toEqual([110, 125, 110, 100]);
        expect(first.defaultPrevented).toBe(true);
    });
});
