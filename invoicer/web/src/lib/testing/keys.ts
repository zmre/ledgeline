// The keymap in a component test (PLAN 5k): the app's root layout routes
// every keydown on the window to `keymap.handle`; a mounted component has no
// layout, so a test that presses keys listens the same way, and resets the
// keymap's module state after each test (layers, the open help sheet, an
// armed chord) so one test's layers never answer the next one's keys.

import {fireEvent} from "@testing-library/svelte";
import {afterEach, beforeEach} from "vitest";
import {keymap} from "#ui/keys/keymap.svelte.js";

export function listenForKeys(): void {
    beforeEach(() => window.addEventListener("keydown", keymap.handle));
    afterEach(() => {
        window.removeEventListener("keydown", keymap.handle);
        keymap.reset();
    });
}

/** Press `key` on `target` (focused first, as a real keystroke's target is), the event bubbling to the window. */
export async function press(key: string, target: HTMLElement = document.body, init: KeyboardEventInit = {}): Promise<boolean> {
    if (target !== document.body) target.focus();
    return fireEvent.keyDown(target, {key, ...init});
}
