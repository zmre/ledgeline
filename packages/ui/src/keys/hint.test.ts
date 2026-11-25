// Shortcut suffixes in tooltips follow the pointer (plan 42): "(v)" on a
// device with a mouse or trackpad, nothing on a touch-only one.

import {describe, expect, it} from "vitest";
import {hint, keyHintsShown, withKeys} from "./hint.svelte";

describe("withKeys", () => {
    it("appends the keys in parentheses when hints show", () => {
        expect(withKeys("Calendar view", "v", true)).toBe("Calendar view (v)");
        expect(withKeys("Done", "⌘↵", true)).toBe("Done (⌘↵)");
    });

    it("leaves the label alone when they do not", () => {
        expect(withKeys("Calendar view", "v", false)).toBe("Calendar view");
    });
});

describe("hint", () => {
    it("shows the keys where nothing can say there is no fine pointer (no window: a desktop default)", () => {
        expect(keyHintsShown()).toBe(true);
        expect(hint("Find Matches", "m")).toBe("Find Matches (m)");
    });
});
