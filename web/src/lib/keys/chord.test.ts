// Canonical spelling of keystrokes. The load-bearing rule is that `event.key`
// already carries the shifted character, so bindings spell `?` and `G` directly
// and never `shift+/` or `shift+g`. Getting that backwards is the classic keymap
// bug and it fails silently — hence the tests.

import {describe, expect, it} from "vitest";
import {canonical, chordToken, formatKeys, MODIFIER_JOINER, isPrefixOf, matchesKeys, steps} from "./chord";

function press(key: string, mods: Partial<{ctrlKey: boolean; altKey: boolean; metaKey: boolean; shiftKey: boolean; code: string}> = {}) {
    return {key, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, ...mods};
}

describe("UNIT chordToken", () => {
    it("does not emit shift+ for a printable character", () => {
        // Shift+/ arrives as "?" with shiftKey true. Emitting "shift+?" would
        // mean no binding could ever spell it naturally.
        expect(chordToken(press("?", {shiftKey: true}))).toBe("?");
        expect(chordToken(press("G", {shiftKey: true}))).toBe("G");
    });

    it("does emit shift+ for a named key", () => {
        expect(chordToken(press("Tab", {shiftKey: true}))).toBe("shift+Tab");
    });

    it("emits modifiers in canonical order", () => {
        expect(chordToken(press("d", {ctrlKey: true}))).toBe("ctrl+d");
        expect(chordToken(press("k", {ctrlKey: true, metaKey: true}))).toBe("ctrl+meta+k");
    });
});

describe("UNIT steps and canonical", () => {
    it("splits a chord on spaces, not on characters", () => {
        expect(steps("g j")).toEqual(["g", "j"]);
        expect(steps("gj")).toEqual(["gj"]);
    });

    it("treats G and g as different bindings", () => {
        expect(canonical("G")).not.toBe(canonical("g"));
    });

    it("parses mod+shift+Enter, and a Cmd/Ctrl+Shift+Enter press matches it (plan 32 D16)", () => {
        // `mod` resolves per platform; `shift+` survives because Enter is a named key.
        const spelled = canonical("mod+shift+Enter");
        expect(["meta+shift+Enter", "ctrl+shift+Enter"]).toContain(spelled);
        const pressed = spelled.startsWith("meta") ? press("Enter", {metaKey: true, shiftKey: true}) : press("Enter", {ctrlKey: true, shiftKey: true});
        expect(matchesKeys("mod+shift+Enter", chordToken(pressed))).toBe(true);
        // …and plain mod+Enter stays a different binding.
        expect(canonical("mod+Enter")).not.toBe(spelled);
    });

    it("drops a hand-written shift+ from a printable key so it can still match", () => {
        // Nothing produces "shift+?" at runtime, so a binding spelled that way
        // would be dead. Normalize rather than let it fail silently.
        expect(canonical("shift+?")).toBe("?");
    });
});

describe("UNIT matchesKeys and isPrefixOf", () => {
    it("matches a full sequence", () => {
        expect(matchesKeys("g j", "g j")).toBe(true);
        expect(matchesKeys("g j", "g")).toBe(false);
    });

    it("treats a live prefix as strict", () => {
        expect(isPrefixOf("g j", "g")).toBe(true);
        // A complete sequence is not a prefix of itself, or it would arm forever.
        expect(isPrefixOf("g j", "g j")).toBe(false);
        // And "g" must not count as a prefix of an unrelated single key.
        expect(isPrefixOf("j", "g")).toBe(false);
    });
});

describe("UNIT formatKeys", () => {
    it("renders one token per chord step, so the sheet can print them adjacent", () => {
        expect(formatKeys("g j").map((t) => t.text)).toEqual(["g", "j"]);
    });

    it("renders arrows and named keys as symbols", () => {
        expect(formatKeys("ArrowDown")[0].text).toBe("↓");
        expect(formatKeys("Escape")[0].text).toBe("Esc");
    });

    it("separates a chord's modifiers and key, so ⌘⇧↵ never reads as one word", () => {
        expect(formatKeys("mod+shift+Enter")[0].text.split(MODIFIER_JOINER)).toHaveLength(3);
        expect(MODIFIER_JOINER).not.toBe("");
    });
});

describe("UNIT the + key and keyboard layouts (zoom, plan 39)", () => {
    it("spells the + key itself, which a split on + alone cannot", () => {
        expect(canonical("ctrl++")).toBe("ctrl++");
        expect(canonical("shift+ctrl++")).toBe("ctrl++");
        expect(canonical("+")).toBe("+");
        expect(matchesKeys("ctrl++", chordToken(press("+", {ctrlKey: true, shiftKey: true})))).toBe(true);
        expect(formatKeys("ctrl++")).toEqual([{text: `Ctrl${MODIFIER_JOINER}+`}]);
    });

    it("leaves - = and 0 chords alone", () => {
        expect(canonical("ctrl+-")).toBe("ctrl+-");
        expect(chordToken(press("-", {ctrlKey: true}))).toBe("ctrl+-");
        expect(chordToken(press("=", {metaKey: true}))).toBe("meta+=");
        expect(chordToken(press("0", {ctrlKey: true, code: "Digit0"}))).toBe("ctrl+0");
    });

    it("trusts event.key on a Latin layout, even where the physical key differs", () => {
        // German: `+` has its own key (US `]`); AZERTY: `-` sits on Digit6.
        expect(chordToken(press("+", {metaKey: true, code: "BracketRight"}))).toBe("meta++");
        expect(chordToken(press("-", {ctrlKey: true, code: "Digit6"}))).toBe("ctrl+-");
    });

    it("falls back to the physical key for a Ctrl/Cmd chord on a non-Latin character", () => {
        expect(chordToken(press("à", {metaKey: true, code: "Digit0"}))).toBe("meta+0");
        expect(chordToken(press("л", {ctrlKey: true, code: "KeyK"}))).toBe("ctrl+k");
        expect(chordToken(press("Л", {ctrlKey: true, shiftKey: true, code: "KeyK"}))).toBe("ctrl+K");
        expect(chordToken(press("ß", {ctrlKey: true, code: "Minus"}))).toBe("ctrl+-");
    });

    it("never rewrites a character typed without Ctrl/Cmd", () => {
        expect(chordToken(press("à", {code: "Digit0"}))).toBe("à");
        expect(chordToken(press("à", {altKey: true, code: "Digit0"}))).toBe("alt+à");
    });
});
