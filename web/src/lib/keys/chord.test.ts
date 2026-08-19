// Canonical spelling of keystrokes. The load-bearing rule is that `event.key`
// already carries the shifted character, so bindings spell `?` and `G` directly
// and never `shift+/` or `shift+g`. Getting that backwards is the classic keymap
// bug and it fails silently — hence the tests.

import {describe, expect, it} from "vitest";
import {canonical, chordToken, formatKeys, isPrefixOf, matchesKeys, steps} from "./chord";

function press(key: string, mods: Partial<{ctrlKey: boolean; altKey: boolean; metaKey: boolean; shiftKey: boolean}> = {}) {
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
});
