// Keyboard-shortcut hints follow the pointer, not the platform (plan 42).
//
// A "(v)" at the end of a tooltip, or a `<kbd>` beside a button, is noise on
// a device you drive with a finger: there is no `v` to press. The test is the
// one CSS uses for the `<kbd>` side (app.css): `any-pointer: fine` — a mouse
// or trackpad anywhere. An iPad with a trackpad gets its hints back; a phone
// never shows them.
//
// `withKeys` is the pure decision; `hint` is the reactive one the templates
// call. Use `hint` at every call site that appends a shortcut to a label, so
// the stripping is one rule and not a conditional per tooltip.

import {MediaQuery} from "svelte/reactivity";

/** `label (keys)` when hints show, else `label` alone. */
export function withKeys(label: string, keys: string, show: boolean): string {
    return show ? `${label} (${keys})` : label;
}

// Module scope, like every MediaQuery: one listener for the whole app. Svelte
// reads `fallback` (true: a desktop browser) where there is no `matchMedia`.
const finePointer = typeof window === "undefined" ? null : new MediaQuery("any-pointer: fine", true);

/** Whether keyboard hints show: a fine pointer (mouse, trackpad) is attached. Reactive. */
export function keyHintsShown(): boolean {
    return finePointer?.current ?? true;
}

/** `label (keys)` on a device with a fine pointer, `label` alone on a touch-only one. Reactive. */
export function hint(label: string, keys: string): string {
    return withKeys(label, keys, keyHintsShown());
}
