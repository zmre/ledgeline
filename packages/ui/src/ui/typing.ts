/**
 * Marks a subtree that owns the keyboard without being a field: the attribute
 * an app's global keymap treats as "typing here", so a single-key binding
 * (`j`, `/`) does not fire while, say, a combobox's popup has the arrow keys.
 * Ledgeline's keymap (`web/src/lib/keys/target.ts`) reads it; shared
 * components such as `AccountInput` set it.
 */
export const TYPING_ATTRIBUTE = "data-keys-typing";
