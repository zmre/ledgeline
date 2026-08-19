// The store half of the keymap: registration tied to component lifetime, and
// the guard chain in `handle`. The decision logic itself is covered in
// `dispatch.test.ts` (node project); what needs a DOM is the lifecycle and the
// `event.target` inspection.

import {render} from "@testing-library/svelte";
import {flushSync} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import KeysProbe from "$lib/testing/fixtures/KeysProbe.svelte";
import {keymap} from "./keymap.svelte";
import {PRIORITY, type Layer} from "./types";

// Module-level runes state is shared across a test file, so a layer registered
// by a component that Testing Library has not yet unmounted would leak into the
// next test.
afterEach(() => keymap.reset());

function layer(id: string, keys: string, run: () => void, extra: Partial<Layer> = {}): Layer {
    return {id, bindings: [{keys, label: id, group: "Journal", run}], ...extra};
}

/** Dispatch a real keydown at the document, the way the window listener sees it. */
function press(key: string, target: EventTarget = document.body, init: KeyboardEventInit = {}): KeyboardEvent {
    const event = new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true, ...init});
    target.dispatchEvent(event);
    keymap.handle(event);
    return event;
}

describe("COMPONENT keymap registration", () => {
    it("registers a layer on mount and unregisters it on unmount", () => {
        let fired = 0;
        const view = render(KeysProbe, {props: {layerOf: () => layer("probe", "j", () => (fired += 1))}});

        press("j");
        expect(fired).toBe(1);

        view.unmount();
        // `$effect` cleanup is QUEUED, not synchronous, so without this the
        // layer is still registered when the next keystroke arrives. Harmless in
        // the app (an unmount and a keypress are frames apart) but it would make
        // this assertion test the flush schedule rather than the unregistration.
        flushSync();
        press("j");
        // Asserted as a COUNT, not as "nothing happened": with
        // `expect.requireAssertions` a test that merely unmounts and looks away
        // would pass while proving nothing.
        expect(fired).toBe(1);
    });

    it("lets a later, higher-priority layer shadow an earlier one", () => {
        let low = 0;
        let high = 0;
        render(KeysProbe, {props: {layerOf: () => layer("table", "j", () => (low += 1), {priority: PRIORITY.widget})}});
        render(KeysProbe, {props: {layerOf: () => layer("tree", "j", () => (high += 1), {priority: PRIORITY.transient})}});

        press("j");

        expect({low, high}).toEqual({low: 0, high: 1});
    });
});

describe("COMPONENT keymap guards", () => {
    it("does not fire while focus is in a text field", () => {
        // The whole basis of a non-modal keymap: typing `j` into the search box
        // must type a `j`.
        let fired = 0;
        render(KeysProbe, {props: {layerOf: () => layer("probe", "j", () => (fired += 1))}});
        const input = document.createElement("input");
        document.body.append(input);

        press("j", input);

        expect(fired).toBe(0);
        input.remove();
    });

    it("still fires when focus is on a checkbox, which does not swallow letters", () => {
        // The column menu is checkboxes; `j` has to keep working there.
        let fired = 0;
        render(KeysProbe, {props: {layerOf: () => layer("probe", "j", () => (fired += 1))}});
        const box = document.createElement("input");
        box.type = "checkbox";
        document.body.append(box);

        press("j", box);

        expect(fired).toBe(1);
        box.remove();
    });

    it("defers to a handler that already claimed the key", () => {
        // `<svelte:window>` is the bubble phase, so the four pre-existing
        // element-scoped handlers run first. `defaultPrevented` is how they say
        // "mine" without this codebase ever needing `stopPropagation`.
        let fired = 0;
        render(KeysProbe, {props: {layerOf: () => layer("probe", "Escape", () => (fired += 1))}});

        const event = new KeyboardEvent("keydown", {key: "Escape", cancelable: true});
        event.preventDefault();
        keymap.handle(event);

        expect(fired).toBe(0);
    });

    it("ignores a keystroke that is mid-IME-composition", () => {
        let fired = 0;
        render(KeysProbe, {props: {layerOf: () => layer("probe", "j", () => (fired += 1))}});

        keymap.handle(new KeyboardEvent("keydown", {key: "j", isComposing: true, cancelable: true}));

        expect(fired).toBe(0);
    });

    it("preventDefaults a key it claims, so `/` does not type into the field it focuses", () => {
        render(KeysProbe, {props: {layerOf: () => layer("probe", "/", () => {})}});

        expect(press("/").defaultPrevented).toBe(true);
    });
});

describe("COMPONENT keymap chords", () => {
    it("arms a prefix, shows it, then completes it", () => {
        let fired = 0;
        render(KeysProbe, {props: {layerOf: () => layer("probe", "g j", () => (fired += 1))}});

        press("g");
        expect(keymap.pending).toBe("g");

        press("j");
        expect({fired, pending: keymap.pending}).toEqual({fired: 1, pending: ""});
    });

    it("disarms when focus moves into a field mid-chord", () => {
        // Otherwise: press `g`, click the search box, type `j` — and the `j`
        // vanishes into a chord completed minutes later.
        render(KeysProbe, {props: {layerOf: () => layer("probe", "g j", () => {})}});
        const input = document.createElement("input");
        document.body.append(input);

        press("g");
        press("j", input);

        expect(keymap.pending).toBe("");
        input.remove();
    });

    it("disarms on Escape", () => {
        render(KeysProbe, {props: {layerOf: () => layer("probe", "g j", () => {})}});

        press("g");
        press("Escape");

        expect(keymap.pending).toBe("");
    });
});
