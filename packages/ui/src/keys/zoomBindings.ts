// The interface-scale keys (plan 39), shared by Ledgeline and the invoicer:
// Cmd/Ctrl `+` `=` `-` `0`. Each app puts them in its global layer under its
// own help section ("Global" in both today).
//
// `always`: like the browser's own zoom they work from inside a field and over
// an overlay. The keymap `preventDefault`s them, which is what stops the
// browser zooming the page as well (and, in the macOS desktop app, what stops
// the View menu's identical key equivalent firing a second step; the menu's
// half arrives through `listenForMenuZoom`). Cmd+= is the unshifted key on a
// US layout; Cmd++ is what a layout with a dedicated `+` key (German, numpad)
// sends, so both zoom in and only one is listed.

import {zoomIn, zoomOut, zoomReset} from "../theme/zoom.svelte.js";
import type {Binding, KeyGroup} from "./types.js";

/** The four zoom bindings, under `group` on the `?` sheet. */
export function zoomBindings(group: KeyGroup): Binding[] {
    return [
        {keys: "mod++", label: "Zoom in", group, run: zoomIn, always: true},
        {keys: "mod+=", label: "Zoom in", group, run: zoomIn, always: true, help: false},
        {keys: "mod+-", label: "Zoom out", group, run: zoomOut, always: true},
        {keys: "mod+0", label: "Actual size", group, run: zoomReset, always: true},
    ];
}
