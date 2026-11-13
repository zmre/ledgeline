// The zoom ACTION: Cmd/Ctrl `+` `-` `0` and the desktop View menu's Zoom In /
// Zoom Out / Actual Size all land here, so the step logic exists once.
//
// Two routes reach it:
//   - the keymap's global layer (`keys/navBindings.ts`), source "keyboard";
//   - a `ledgeline:zoom` CustomEvent on `window`, which the desktop shell
//     dispatches from its menu with `evaluate_script` — source "menu".
//
// ONE PRESS, ONE STEP. A menu accelerator and the page's own keydown could in
// principle both see the same keystroke. Per platform, what the shell relies
// on (crates/ledgeline-server/src/gui.rs has the long form):
//   - macOS: WKWebView offers a Cmd key-equivalent to the page FIRST; the
//     keymap `preventDefault`s it, so AppKit never walks the menu. The menu
//     fires only when clicked, or when the page did not take the key.
//   - Linux (WebKitGTK): with the menu bar shown, its GtkAccelGroup consumes
//     the key before the web view sees it; with the bar hidden (the default)
//     the accelerator is dead and only the page sees the key.
//   - Windows (WebView2): keystrokes go to the WebView2 browser process, and
//     wry does not forward them to the host's accelerator table, so the menu
//     is not expected to see a key the page has focus for. (wry's default
//     also turns WebView2's own Ctrl +/- page zoom off.)
// None of that is something a unit test can hold the platform to, so a tiny
// de-dupe backs it up: the SAME action from the OTHER source within
// `DEDUPE_MS` is taken to be the same press and dropped. Same-source repeats
// (holding the key down) are never dropped.

import {applyScale, DEFAULT_SCALE, stepScale} from "./scale";

export type ZoomAction = "in" | "out" | "reset";
export type ZoomSource = "keyboard" | "menu";

/** The CustomEvent the desktop menu dispatches on `window`; `detail` is a `ZoomAction`. */
export const ZOOM_EVENT = "ledgeline:zoom";

/** How long a press from one route shadows the same action from the other. Well under a key-repeat interval's worth of human intent. */
export const DEDUPE_MS = 150;

/** Where a new scale goes: persist it, apply it. */
export type ScaleSink = (percent: number) => void;

let local = $state(DEFAULT_SCALE);

function defaultSink(percent: number): void {
    local = percent;
    applyScale(percent);
}

let sink: ScaleSink = defaultSink;
let read: () => number = () => local;

/**
 * Route new scales somewhere other than the local default, and read the
 * current scale from there too.
 *
 * MERGE SEAM: wire to settings.uiScale — the integration package calls
 * `setScaleSink((p) => settings.setUiScale(p), () => settings.uiScale)` once
 * the prefs store has the key, and applies the persisted scale reactively
 * (`applyScale`) from there. Until then the default sink applies it and keeps
 * a local value, so the shortcuts work but nothing is saved.
 *
 * `null` restores the default (tests).
 */
export function setScaleSink(next: ScaleSink | null, current?: () => number): void {
    sink = next ?? defaultSink;
    read = next === null || current === undefined ? () => local : current;
}

/** The scale zoom steps from: the sink's reader, else the local value. */
export function currentScale(): number {
    return read();
}

let recent: {action: ZoomAction; source: ZoomSource} | null = null;
let recentTimer: ReturnType<typeof setTimeout> | undefined;

/** The scale `action` moves `current` to. */
export function nextScale(current: number, action: ZoomAction): number {
    return action === "reset" ? DEFAULT_SCALE : stepScale(current, action);
}

/**
 * Perform `action`, unless it is the other route's echo of a press just
 * handled (see the file header). Returns whether it ran.
 */
export function zoom(action: ZoomAction, source: ZoomSource = "keyboard"): boolean {
    if (recent !== null && recent.action === action && recent.source !== source) return false;
    recent = {action, source};
    if (recentTimer !== undefined) clearTimeout(recentTimer);
    // A timer, never `Date.now()` arithmetic: the e2e suite freezes `Date`
    // with `page.clock.setFixedTime` while leaving timers running (the same
    // reasoning as the keymap's chord timeout).
    recentTimer = setTimeout(() => {
        recent = null;
        recentTimer = undefined;
    }, DEDUPE_MS);
    sink(nextScale(currentScale(), action));
    return true;
}

export const zoomIn = (): boolean => zoom("in");
export const zoomOut = (): boolean => zoom("out");
export const zoomReset = (): boolean => zoom("reset");

function isZoomAction(value: unknown): value is ZoomAction {
    return value === "in" || value === "out" || value === "reset";
}

/**
 * Listen for the desktop menu's `ledgeline:zoom` on `target`. Returns the
 * teardown, so it slots straight into an `$effect`. Anything but a known
 * action in `detail` is ignored: the event is a public name on `window`.
 */
export function listenForMenuZoom(target: Pick<Window, "addEventListener" | "removeEventListener"> = window): () => void {
    const onZoom = (event: Event): void => {
        const detail = (event as CustomEvent<unknown>).detail;
        if (isZoomAction(detail)) zoom(detail, "menu");
    };
    target.addEventListener(ZOOM_EVENT, onZoom);
    return () => target.removeEventListener(ZOOM_EVENT, onZoom);
}

/** Test-only: forget the de-dupe window and the local scale. */
export function resetZoomForTests(): void {
    if (recentTimer !== undefined) clearTimeout(recentTimer);
    recent = null;
    recentTimer = undefined;
    local = DEFAULT_SCALE;
    setScaleSink(null);
}
