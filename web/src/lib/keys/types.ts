// The keymap's vocabulary: the shape of a binding, the shape of a layer, and
// the two ordering conventions everything else reads. Pure types and constants
// — no runes, no DOM — so `dispatch.ts` and its tests stay in the fast `unit`
// vitest project rather than needing jsdom.

/**
 * Help-sheet sections, rendered in this order. A binding with no home here has
 * no home in the app.
 *
 * The middle four mirror the nav bar's own order (Journal, Holdings, Reports,
 * Imports), so a reader scanning `?` finds the section where they expect the
 * page to be. "Holdings" arrived with plans/14: the holdings table's row cursor
 * had been borrowing "Journal" while it was the page's only keyboard surface,
 * and once the tab strip added a second one, that would have filed one feature
 * under two headings.
 */
export type KeyGroup = "Global" | "Navigation" | "Journal" | "Holdings" | "Reports" | "Imports" | "Filters";

export const GROUP_ORDER: readonly KeyGroup[] = ["Global", "Navigation", "Journal", "Holdings", "Reports", "Imports", "Filters"];

/**
 * Layer priorities. Higher wins; registration order breaks ties (later beats
 * earlier). These four are the whole ladder — resist inventing a fifth, because
 * the moment priorities are arbitrary numbers nobody can predict which `j` runs.
 */
export const PRIORITY = {
    /** A route's own bindings (report tab digits, say). */
    page: 0,
    /** A widget on the page: the journal table, the rules list. */
    widget: 10,
    /** A transient that is OPEN: a dropdown, the account tree. */
    transient: 20,
    /** An overlay that owns the screen. Always paired with `modal: true`. */
    overlay: 100,
} as const;

export interface Binding {
    /**
     * The keystroke in canonical spelling: `"j"`, `"?"`, `"G"`, `"g j"` (a
     * chord — SPACE separates the steps), `"mod+k"`.
     *
     * Case is significant for single printable characters, and that is the
     * whole shift story: `event.key` already carries the shifted character, so
     * `?` is `"?"` and never `shift+/`, and `G` is `"G"` and never `shift+g`.
     * `shift+` is only ever written on a NAMED key (`shift+Tab`). Getting this
     * wrong is the classic keymap bug; the matcher has no other shift rule.
     */
    keys: string;
    /** Imperative, sentence case. This IS the help sheet's row text — there is nowhere else to write one. */
    label: string;
    group: KeyGroup;
    run: () => void;
    /**
     * Read at dispatch AND while rendering help, so a disabled binding neither
     * fires nor appears (and does not arm its chord prefix). Must be a pure read
     * of runes state: the help sheet calls it inside a `$derived`.
     */
    enabled?: () => boolean;
}

export interface Layer {
    /** Debug handle and help-ordering key. */
    id: string;
    /** See `PRIORITY`. Defaults to `PRIORITY.page`. */
    priority?: number;
    /** Stop the search here: nothing below sees the event, matched or not. Overlays only. */
    modal?: boolean;
    bindings: Binding[];
}

/** A layer with the registration sequence the store stamps on it, so ties resolve deterministically. */
export interface RegisteredLayer extends Layer {
    seq: number;
}

/** A binding that survived resolution, tagged with the layer it came from (for debugging and help grouping). */
export interface ResolvedBinding extends Binding {
    layerId: string;
}

/**
 * `g` is reserved as a chord prefix app-wide. `g g` and `G` are deliberately NOT
 * bound globally: they mean "top"/"bottom" of whichever surface owns the current
 * scroll container, so they belong to that surface's layer.
 */
export const CHORD_PREFIXES: readonly string[] = ["g"];

/**
 * How long a half-typed chord stays armed. Vim has no timeout, but a UI that
 * silently eats the next keystroke forever after a stray `g` is hostile —
 * deliberate but self-healing.
 */
export const CHORD_TIMEOUT_MS = 1200;
