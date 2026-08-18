// Turning a KeyboardEvent into a canonical token, and canonical tokens into
// something renderable. This file owns the shift rule (see `Binding.keys`) and
// the `mod+` platform split. Pure: `KeyEventLike` is structural, so an object
// literal satisfies it and every test here runs in the node project.

/** The bits of a KeyboardEvent this reads. `KeyboardEvent` satisfies it structurally. */
export interface KeyEventLike {
    key: string;
    ctrlKey: boolean;
    altKey: boolean;
    metaKey: boolean;
    shiftKey: boolean;
}

/**
 * True on macOS, where `mod+` means Cmd. Read once at module load: the platform
 * does not change mid-session, and reading it per keystroke would make the
 * matcher untestable without stubbing globals.
 *
 * `navigator` is guarded because this module is imported by the node `unit`
 * project, which has no DOM.
 */
const IS_MAC = typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent);

/** Modifier prefixes in canonical order, so `"ctrl+shift+Tab"` has exactly one spelling. */
const MODIFIER_ORDER = ["ctrl", "alt", "meta", "shift"] as const;

/**
 * The canonical token for one keystroke.
 *
 * Shift is deliberately NOT emitted for single printable characters: the browser
 * already folded it into `event.key`, so Shift+/ arrives as `"?"` and emitting
 * `"shift+?"` would mean no binding could ever spell it naturally. Named keys
 * (Tab, Enter, ArrowUp…) are more than one character, so they keep `shift+`.
 */
export function chordToken(event: KeyEventLike): string {
    const printable = event.key.length === 1;
    const parts: string[] = [];
    if (event.ctrlKey) parts.push("ctrl");
    if (event.altKey) parts.push("alt");
    if (event.metaKey) parts.push("meta");
    if (event.shiftKey && !printable) parts.push("shift");
    parts.push(event.key);
    return parts.join("+");
}

/** Normalize a written binding: resolve `mod+` and sort modifiers into canonical order. */
export function normalizeToken(token: string): string {
    const pieces = token.split("+");
    const key = pieces[pieces.length - 1];
    const mods = new Set(
        pieces.slice(0, -1).map((m) => {
            const lower = m.toLowerCase();
            return lower === "mod" ? (IS_MAC ? "meta" : "ctrl") : lower;
        })
    );
    // A printable key never carries `shift+` (see `chordToken`), so drop it here
    // too rather than letting a hand-written "shift+?" silently never match.
    if (key.length === 1) mods.delete("shift");
    return [...MODIFIER_ORDER.filter((m) => mods.has(m)), key].join("+");
}

/** Split a binding's `keys` into its canonical steps. `"g j"` → two steps; `"gj"` → one. */
export function steps(keys: string): string[] {
    return keys
        .split(" ")
        .filter((s) => s !== "")
        .map(normalizeToken);
}

/** The sequence a binding spells, as one space-joined canonical string. */
export function canonical(keys: string): string {
    return steps(keys).join(" ");
}

/** Does `sequence` (canonical, space-joined) exactly spell `keys`? */
export function matchesKeys(keys: string, sequence: string): boolean {
    return canonical(keys) === sequence;
}

/** Is `sequence` a STRICT prefix of `keys` — i.e. would another step complete it? */
export function isPrefixOf(keys: string, sequence: string): boolean {
    const full = canonical(keys);
    return full.length > sequence.length && full.startsWith(`${sequence} `);
}

/** One rendered chunk of a binding, for the help sheet. */
export interface KeyToken {
    /** What to print inside a `<kbd>`. */
    text: string;
}

const DISPLAY_NAMES: Record<string, string> = {
    ctrl: "Ctrl",
    alt: IS_MAC ? "⌥" : "Alt",
    meta: IS_MAC ? "⌘" : "Win",
    shift: "Shift",
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
    Enter: "Enter",
    Escape: "Esc",
    " ": "Space",
};

/**
 * Render `keys` for the help sheet: one token per chord STEP, each already
 * carrying its own modifiers. The sheet prints these adjacent with no `+`
 * between them, which is the standard "press these in sequence" form.
 */
export function formatKeys(keys: string): KeyToken[] {
    return steps(keys).map((step) => {
        const pieces = step.split("+");
        const text = pieces.map((p) => DISPLAY_NAMES[p] ?? p).join(IS_MAC ? "" : "+");
        return {text};
    });
}
