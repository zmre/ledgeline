// The keymap's vocabulary: the shape of a binding, the shape of a layer, and
// the ordering conventions everything else reads. Pure types and constants
// — no runes, no DOM — so `dispatch.ts` and its tests stay in the fast `unit`
// vitest project rather than needing jsdom.

/**
 * An app's help-sheet sections, by name, for the type checker: an app adds
 * them by augmenting this interface (Ledgeline's `#lib/keys/groups.ts`), and
 * a binding's `group` must then be one of them, so a typo'd group is a type
 * error rather than a stray section at the bottom of the `?` sheet. Left
 * empty (the invoicer, this package's own tests), a group is any string.
 */
// eslint-disable-next-line @typescript-eslint/no-empty-object-type -- filled by declaration merging
export interface KeyGroupNames {}

/**
 * A help-sheet section. Each app names its own ("Journal", "Invoices") and
 * hands the keymap their order with `keymap.setGroupOrder` (see
 * `helpSections`): the engine is shared by Ledgeline and the invoicer
 * (invoicer PLAN J13), and their sections are not, so neither app's
 * vocabulary leaks into the other's (see `KeyGroupNames`).
 */
export type KeyGroup = keyof KeyGroupNames extends never ? string : keyof KeyGroupNames & string;

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
     * The one exception: a digit under Ctrl/Cmd is read from the physical key,
     * so `"ctrl+shift+1"` and `"meta+alt+1"` keep their Shift/Alt and match on
     * every layout (see `chordToken`).
     *
     * When `macKeys` is set, this is the spelling for every platform but macOS.
     */
    keys: string;
    /**
     * The spelling on macOS, when it is not just `keys` with `mod+` meaning
     * Cmd — a chord that needs different modifiers per platform because the
     * natural one is taken. The keymap resolves it once (`keysFor`), so the
     * dispatcher, the shadowing rule and the `?` sheet all see only the
     * current platform's keys. Plan 46's slot toggle, for instance:
     *
     *     {keys: "ctrl+shift+1", macKeys: "meta+alt+1", label: "Toggle slot 1", group: "Time", run}
     *
     * is ⌥⌘1 on a Mac (⌘⇧1 would collide with the screenshot keys) and
     * Ctrl+Shift+1 on Linux and Windows (Ctrl+Alt is AltGr there). Neither sets
     * `fields`, so both stay off while typing, where Ctrl+Shift+arrows extend
     * the selection.
     */
    macKeys?: string;
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
    /**
     * Fires EVERYWHERE, the way the browser's own zoom does: while focus is in
     * a field, and under a `modal` overlay that blinds every other binding
     * below it. For Ctrl/Cmd chords that type nothing and that no field or
     * overlay could mean anything else by — the zoom keys, and nothing else so
     * far. Never a chord sequence: a field gets no `g` prefix.
     */
    always?: boolean;
    /**
     * Fires while focus is in a field too, but, unlike `always`, a `modal`
     * layer above still blinds it: an editor's Cmd/Alt chord that must work
     * from the cell being typed in (the invoicer's bill-grid row keys), yet
     * not from inside a dialog over the editor. Never a chord sequence.
     */
    fields?: boolean;
    /**
     * Only when the key's target is inside this element (Alt+↑ moves a row
     * only from within the grid that owns it). Read at dispatch alone: the
     * `?` sheet lists the binding whatever has focus, since the sheet itself
     * takes focus when it opens. Null (not mounted) means nowhere.
     */
    within?: () => Scope | null;
    /**
     * `false` keeps an alternate spelling off the `?` sheet (Cmd+= beside the
     * listed Cmd++). It still fires. Default: listed.
     */
    help?: boolean;
}

/** What `Binding.within` returns. An `Element` satisfies it; so does a test's object literal. */
export interface Scope {
    contains(node: unknown): boolean;
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

/**
 * A binding that survived resolution, tagged with the layer it came from (for
 * debugging and help grouping). Its `keys` are already the current platform's
 * (`macKeys` applied), so nothing downstream needs to know about platforms.
 */
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
