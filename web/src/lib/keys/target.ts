// "Is the user typing?" — the single guard that makes a non-modal keymap
// possible. If this is wrong, either `j` types nothing in the search box or `j`
// navigates while you are naming a payee. There are no other modes, so this
// predicate carries the whole weight.
//
// Structurally typed on purpose: `HTMLElement` satisfies `TargetLike`, and so
// does a plain object literal, which is why every case here is a node test
// rather than a jsdom one.

export interface TargetLike {
    tagName: string;
    type?: string;
    isContentEditable?: boolean;
    closest(selectors: string): unknown;
}

/**
 * Input types that swallow letters. Everything else — checkbox, radio, button,
 * submit, reset, file, color, range — does not, and must not: the column menu is
 * checkboxes and the holdings scope bar is buttons, and `j` has to keep working
 * while one of those has focus.
 *
 * `date`/`time` are included because WebKit's segmented date field takes digits
 * and arrow keys per segment, and the transaction popup has two of them.
 */
const TYPING_TYPES = new Set(["text", "search", "url", "tel", "email", "password", "number", "date", "month", "week", "time", "datetime-local"]);

/**
 * Marks a subtree that owns the keyboard without being a field — the account
 * combobox puts this on its wrapper so its popup counts as "typing" too,
 * without this file needing to know the combobox exists.
 */
export const TYPING_ATTRIBUTE = "data-keys-typing";

export function isTypingTarget(target: TargetLike | null | undefined): boolean {
    if (target === null || target === undefined) return false;
    // Inherited, so this covers descendants of a contenteditable host that are
    // not themselves marked. `getAttribute` would miss those.
    if (target.isContentEditable === true) return true;
    const tag = target.tagName;
    // <select> counts: native type-ahead is real ("dec" jumps to December), and
    // both the scope bar and the transaction popup have one.
    if (tag === "TEXTAREA" || tag === "SELECT") return true;
    if (tag === "INPUT") return TYPING_TYPES.has(target.type ?? "text");
    return target.closest(`[${TYPING_ATTRIBUTE}]`) !== null;
}

/** Everything that can hold focus, for the focus trap in `dismissible.ts`. */
export const FOCUSABLE = [
    "a[href]",
    "button:not([disabled])",
    "input:not([disabled])",
    "select:not([disabled])",
    "textarea:not([disabled])",
    "summary",
    "[tabindex]:not([tabindex='-1'])",
].join(",");
