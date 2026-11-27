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
    /** The physical key (`"KeyK"`, `"Digit0"`, `"Equal"`). Only consulted for the layout fallbacks in `chordToken`. */
    code?: string;
}

/**
 * True on macOS, where `mod+` means Cmd. Read once at module load: the platform
 * does not change mid-session, and reading it per keystroke would make the
 * matcher untestable without stubbing globals.
 *
 * `navigator` is guarded because this module is imported by the node `unit`
 * project, which has no DOM.
 */
export const IS_MAC = typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent);

/** Modifier prefixes in canonical order, so `"ctrl+shift+Tab"` has exactly one spelling. */
const MODIFIER_ORDER = ["ctrl", "alt", "meta", "shift"] as const;

/**
 * The canonical token for one keystroke.
 *
 * Shift is deliberately NOT emitted for single printable characters: the browser
 * already folded it into `event.key`, so Shift+/ arrives as `"?"` and emitting
 * `"shift+?"` would mean no binding could ever spell it naturally. Named keys
 * (Tab, Enter, ArrowUp…) are more than one character, so they keep `shift+`.
 *
 * The one printable exception is a digit read from the physical key (see
 * `physicalDigit`): there Shift did not pick the character, it is part of the
 * chord, so `Ctrl+Shift+1` is `ctrl+shift+1` and not `ctrl+!`.
 */
export function chordToken(event: KeyEventLike): string {
    const digit = physicalDigit(event);
    const key = digit ?? commandKey(event);
    const keepShift = digit !== undefined || key.length !== 1;
    const parts: string[] = [];
    if (event.ctrlKey) parts.push("ctrl");
    if (event.altKey) parts.push("alt");
    if (event.metaKey) parts.push("meta");
    if (event.shiftKey && keepShift) parts.push("shift");
    parts.push(key);
    return parts.join("+");
}

/**
 * The digit on the physical key, for a Ctrl/Cmd chord that also holds Shift or
 * Alt; otherwise undefined.
 *
 * Those are the chords whose character is not the digit on any layout: Shift+1
 * types `!` (US, German, Polish), Option+1 types `¡` on a Mac, and German
 * Shift+3 types `§`. Reading `code` (`Digit1`) is the only way `ctrl+shift+1`
 * or `alt+meta+1` (⌥⌘1) can be spelled and matched everywhere, and it is what
 * desktop menus do for their own digit shortcuts. A Ctrl/Cmd digit chord
 * WITHOUT Shift or Alt still goes through `commandKey`, so an AZERTY Ctrl+-
 * (which sits on `Digit6`) stays `ctrl+-` and the zoom keys are untouched.
 * Ctrl+Alt without Cmd is left alone too: on Windows and many Linux layouts
 * that IS AltGr, which types `{` on a German 7, and a binding must not eat it.
 * The top row only: numpad codes are left alone, since Shift turns those into
 * navigation keys on Windows.
 */
function physicalDigit(event: KeyEventLike): string | undefined {
    if (!(event.ctrlKey || event.metaKey) || !(event.shiftKey || event.altKey)) return undefined;
    if (event.ctrlKey && event.altKey && !event.metaKey) return undefined;
    return /^Digit([0-9])$/.exec(event.code ?? "")?.[1];
}

/** Does a written step keep `shift+` on this printable key? Only a digit under Ctrl/Cmd (see `physicalDigit`). */
function keepsShift(mods: ReadonlySet<string>, key: string): boolean {
    return /^[0-9]$/.test(key) && (mods.has("ctrl") || mods.has("meta"));
}

/**
 * The key a Ctrl/Cmd chord names, on any keyboard layout.
 *
 * `event.key` is right for every Latin layout: a German `+` key arrives as
 * `"+"`, an AZERTY `-` as `"-"`, and that is the character the user reads off
 * the keycap. A non-Latin layout is the exception — Cmd+0 on AZERTY's unshifted
 * row is `"à"`, Ctrl+K on a Russian layout is `"л"` — and no binding can be
 * spelled in those, so for a Ctrl/Cmd chord whose character is not ASCII the
 * physical key (`code`) stands in, read as on a US keyboard. That is what
 * browsers and desktop menus do for their own shortcuts.
 */
function commandKey(event: KeyEventLike): string {
    const {key, code} = event;
    if (isAltLetter(event)) return /^Key([A-Z])$/.exec(code ?? "")?.[1] ?? key;
    if (!(event.ctrlKey || event.metaKey) || key.length !== 1 || key.charCodeAt(0) < 0x80 || code === undefined) return key;
    const letter = /^Key([A-Z])$/.exec(code);
    if (letter !== null) return event.shiftKey ? letter[1] : letter[1].toLowerCase();
    const digit = /^(?:Digit|Numpad)([0-9])$/.exec(code);
    if (digit !== null) return digit[1];
    return US_PUNCTUATION[code] ?? key;
}

/**
 * Alt with Shift on a letter, whose character is not ASCII: macOS's Option
 * layer (Option+Shift+S types "Í"), where no binding could be spelled in the
 * character. The physical letter stands in, upper case: Alt+Shift+S is
 * `alt+S` on every platform (Windows and Linux already send "S"). Only with
 * Shift, so a plain Option+letter (ß, a dead key for an accent) keeps typing
 * what it types; the keymap binds Alt+letter chords with Shift for that
 * reason.
 */
export function isAltLetter(event: KeyEventLike): boolean {
    const {key, code} = event;
    if (!event.altKey || !event.shiftKey || event.ctrlKey || event.metaKey || code === undefined || !/^Key[A-Z]$/.test(code)) return false;
    return key === "Dead" || (key.length === 1 && key.charCodeAt(0) >= 0x80);
}

const US_PUNCTUATION: Record<string, string> = {Equal: "=", Minus: "-", NumpadAdd: "+", NumpadSubtract: "-"};

/**
 * One written step split into its modifiers and its key. Splitting on `+` alone
 * cannot spell the `+` key itself (`"mod++"`), so a step that ends in `+` has
 * `+` for its key.
 */
function splitStep(token: string): {mods: string[]; key: string} {
    if (token.endsWith("+")) {
        const head = token.slice(0, -1);
        return {mods: head === "" ? [] : head.replace(/\+$/, "").split("+"), key: "+"};
    }
    const pieces = token.split("+");
    return {mods: pieces.slice(0, -1), key: pieces[pieces.length - 1]};
}

/**
 * Normalize a written binding: resolve `mod+` and sort modifiers into canonical
 * order. `mac` is the platform `mod+` resolves for; tests pass it explicitly.
 */
export function normalizeToken(token: string, mac: boolean = IS_MAC): string {
    const split = splitStep(token);
    const key = split.key;
    const mods = new Set(
        split.mods.map((m) => {
            const lower = m.toLowerCase();
            return lower === "mod" ? (mac ? "meta" : "ctrl") : lower;
        })
    );
    // A printable key never carries `shift+` (see `chordToken`), so drop it here
    // too rather than letting a hand-written "shift+?" silently never match.
    // A Ctrl/Cmd digit is read from the physical key, so its Shift is real.
    if (key.length === 1 && !keepsShift(mods, key)) mods.delete("shift");
    return [...MODIFIER_ORDER.filter((m) => mods.has(m)), key].join("+");
}

/** Split a binding's `keys` into its canonical steps. `"g j"` → two steps; `"gj"` → one. */
export function steps(keys: string, mac: boolean = IS_MAC): string[] {
    return keys
        .split(" ")
        .filter((s) => s !== "")
        .map((step) => normalizeToken(step, mac));
}

/** The sequence a binding spells, as one space-joined canonical string. */
export function canonical(keys: string, mac: boolean = IS_MAC): string {
    return steps(keys, mac).join(" ");
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
    shift: "Shift",
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
    Enter: "Enter",
    Escape: "Esc",
    " ": "Space",
};

/** The modifiers whose name differs by platform. */
const MAC_MODIFIERS: Record<string, string> = {alt: "⌥", meta: "⌘"};
const OTHER_MODIFIERS: Record<string, string> = {alt: "Alt", meta: "Win"};

function displayName(part: string, mac: boolean): string {
    return (mac ? MAC_MODIFIERS : OTHER_MODIFIERS)[part] ?? DISPLAY_NAMES[part] ?? part;
}

function modifierJoiner(mac: boolean): string {
    return mac ? "-" : "+";
}

/**
 * What sits between a chord's modifiers and its key: `⌘-Shift-Enter` on macOS,
 * `Ctrl+Shift+Enter` elsewhere (each platform's own convention). Never nothing:
 * run together, `⌘ShiftEnter` and `Ctrld` read as one word.
 */
export const MODIFIER_JOINER = modifierJoiner(IS_MAC);

/**
 * Render `keys` for the help sheet: one token per chord STEP, each already
 * carrying its own modifiers. The sheet prints these adjacent with no `+`
 * between them, which is the standard "press these in sequence" form. `mac`
 * picks the platform's glyphs and joiner (`⌥-⌘-1`, `Ctrl+Shift+1`).
 */
export function formatKeys(keys: string, mac: boolean = IS_MAC): KeyToken[] {
    return steps(keys, mac).map((step) => {
        const {mods, key} = splitStep(step);
        // A capital letter under a modifier is typed with Shift: say so, as
        // `⌥-S` would read as Option+S. (Alone, `G` is the key's own name.)
        const shifted = mods.length > 0 && /^[A-Z]$/.test(key) ? ["shift"] : [];
        const text = [...mods, ...shifted, key].map((p) => displayName(p, mac)).join(modifierJoiner(mac));
        return {text};
    });
}
