// Canonical spelling of keystrokes. The load-bearing rule is that `event.key`
// already carries the shifted character, so bindings spell `?` and `G` directly
// and never `shift+/` or `shift+g`. Getting that backwards is the classic keymap
// bug and it fails silently — hence the tests.

import {describe, expect, it} from "vitest";
import {canonical, chordToken, formatKeys, IS_MAC, isAltLetter, MODIFIER_JOINER, isPrefixOf, matchesKeys, steps} from "./chord";

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

describe("UNIT Alt+Shift+letter (the invoicer's bill-grid row keys)", () => {
    it("reads macOS's Option layer from the physical key, upper case", () => {
        // Option+Shift+S types "Í" and Option+Shift+I "ˆ" on a US Mac.
        expect(chordToken(press("Í", {altKey: true, shiftKey: true, code: "KeyS"}))).toBe("alt+S");
        expect(chordToken(press("ˆ", {altKey: true, shiftKey: true, code: "KeyI"}))).toBe("alt+I");
        expect(chordToken(press("Dead", {altKey: true, shiftKey: true, code: "KeyE"}))).toBe("alt+E");
        // Windows and Linux send the letter already.
        expect(chordToken(press("S", {altKey: true, shiftKey: true, code: "KeyS"}))).toBe("alt+S");
        expect(matchesKeys("alt+S", chordToken(press("Í", {altKey: true, shiftKey: true, code: "KeyS"})))).toBe(true);
    });

    it("leaves a plain Option+letter typing what it types", () => {
        expect(chordToken(press("ß", {altKey: true, code: "KeyS"}))).toBe("alt+ß");
        expect(isAltLetter(press("Dead", {altKey: true, code: "KeyE"}))).toBe(false);
        expect(isAltLetter(press("Í", {altKey: true, shiftKey: true, metaKey: true, code: "KeyS"}))).toBe(false);
    });

    it("says Shift when a modifier chord names a capital letter", () => {
        const [step] = formatKeys("alt+S");
        expect(step.text.split(MODIFIER_JOINER).slice(1)).toEqual(["Shift", "S"]);
        expect(formatKeys("G")).toEqual([{text: "G"}]);
    });
});

// Plan 46 (K, T9): the slot keys are ⌥⌘digit on macOS and Ctrl+Shift+digit
// elsewhere. Neither modifier set leaves the digit in `event.key` — Option
// and Shift both pick another character, and which one depends on the
// layout — so these chords are read from the physical key. Each row is a
// real press: the character that layout's `event.key` carries, and `code`.
describe("UNIT Ctrl/Cmd digit chords read the physical key (plan 46 K)", () => {
    const macOptionCmd: [layout: string, key: string, code: string, digit: string][] = [
        ["US", "¡", "Digit1", "1"],
        ["US", "™", "Digit2", "2"],
        ["US", "º", "Digit0", "0"],
        // Some WebKit builds report the plain digit while Cmd is held.
        ["US (Cmd keeps the digit)", "1", "Digit1", "1"],
        ["German", "¡", "Digit1", "1"],
        // German Option+7 is `|`: ASCII, so the old non-ASCII fallback missed it.
        ["German", "|", "Digit7", "7"],
        ["German", "≠", "Digit0", "0"],
        ["Polish Pro", "¡", "Digit1", "1"],
        ["Polish Pro", "º", "Digit0", "0"],
    ];
    it.each(macOptionCmd)("macOS ⌥⌘ on %s (%s, %s) is alt+meta+digit", (_layout, key, code, digit) => {
        const token = chordToken(press(key, {altKey: true, metaKey: true, code}));
        expect(token).toBe(`alt+meta+${digit}`);
        expect(matchesKeys(`meta+alt+${digit}`, token)).toBe(true);
    });

    const ctrlShift: [layout: string, key: string, code: string, digit: string][] = [
        ["US", "!", "Digit1", "1"],
        ["US", "&", "Digit7", "7"],
        ["US", ")", "Digit0", "0"],
        ["German", "!", "Digit1", "1"],
        ["German", "/", "Digit7", "7"],
        ["German", "=", "Digit0", "0"],
        // Non-ASCII: the old fallback read the digit but dropped the Shift.
        ["German", "§", "Digit3", "3"],
        ["Polish programmer", "!", "Digit1", "1"],
        ["Polish programmer", ")", "Digit0", "0"],
        // AZERTY's top row types digits WITH Shift: already right, still right.
        ["French AZERTY", "1", "Digit1", "1"],
    ];
    it.each(ctrlShift)("Linux/Windows Ctrl+Shift on %s (%s, %s) is ctrl+shift+digit", (_layout, key, code, digit) => {
        const token = chordToken(press(key, {ctrlKey: true, shiftKey: true, code}));
        expect(token).toBe(`ctrl+shift+${digit}`);
        expect(matchesKeys(`ctrl+shift+${digit}`, token)).toBe(true);
        // …and never the plain Ctrl+digit binding (browser tab switching).
        expect(matchesKeys(`ctrl+${digit}`, token)).toBe(false);
    });

    it("keeps shift+ on a written Ctrl/Cmd digit, and drops it elsewhere as before", () => {
        expect(canonical("ctrl+shift+1")).toBe("ctrl+shift+1");
        expect(canonical("shift+ctrl+1")).toBe("ctrl+shift+1");
        expect(canonical("meta+alt+1")).toBe("alt+meta+1");
        expect(canonical("mod+shift+1", true)).toBe("meta+shift+1");
        expect(canonical("mod+shift+1", false)).toBe("ctrl+shift+1");
        expect(canonical("shift+1")).toBe("1");
        expect(canonical("shift+?")).toBe("?");
    });

    it("does not let ⌥⌘1 and Ctrl+Shift+1 answer each other's binding", () => {
        expect(matchesKeys("ctrl+shift+1", chordToken(press("¡", {altKey: true, metaKey: true, code: "Digit1"})))).toBe(false);
        expect(matchesKeys("meta+alt+1", chordToken(press("!", {ctrlKey: true, shiftKey: true, code: "Digit1"})))).toBe(false);
    });

    it("leaves AltGr (Ctrl+Alt on Windows/Linux) typing its character", () => {
        // German AltGr+7 is `{`: a digit binding must never swallow it.
        expect(chordToken(press("{", {ctrlKey: true, altKey: true, code: "Digit7"}))).toBe("ctrl+alt+{");
    });

    it("leaves numpad digits alone", () => {
        expect(chordToken(press("End", {ctrlKey: true, shiftKey: true, code: "Numpad1"}))).toBe("ctrl+shift+End");
    });
});

describe("UNIT regressions: existing bindings resolve as before (plan 46 K)", () => {
    const cases: [binding: string, pressed: ReturnType<typeof press>][] = [
        ["?", press("?", {shiftKey: true, code: "Slash"})],
        ["G", press("G", {shiftKey: true, code: "KeyG"})],
        ["ctrl+d", press("d", {ctrlKey: true, code: "KeyD"})],
        ["ctrl+meta+k", press("k", {ctrlKey: true, metaKey: true, code: "KeyK"})],
        ["alt+S", press("Í", {altKey: true, shiftKey: true, code: "KeyS"})],
        ["alt+ArrowUp", press("ArrowUp", {altKey: true, code: "ArrowUp"})],
        ["ctrl+shift+ArrowDown", press("ArrowDown", {ctrlKey: true, shiftKey: true, code: "ArrowDown"})],
        // The zoom keys on both platforms' modifiers, US and German/AZERTY.
        ["ctrl++", press("+", {ctrlKey: true, shiftKey: true, code: "Equal"})],
        ["meta++", press("+", {metaKey: true, code: "BracketRight"})],
        ["ctrl+=", press("=", {ctrlKey: true, code: "Equal"})],
        ["ctrl+-", press("-", {ctrlKey: true, code: "Minus"})],
        ["ctrl+-", press("-", {ctrlKey: true, code: "Digit6"})],
        ["meta+0", press("0", {metaKey: true, code: "Digit0"})],
        ["meta+0", press("à", {metaKey: true, code: "Digit0"})],
        ["ctrl+0", press("0", {ctrlKey: true, code: "Digit0"})],
    ];
    it.each(cases)("%s still matches", (binding, pressed) => {
        expect(matchesKeys(binding, chordToken(pressed))).toBe(true);
    });

    it("mod+Enter and mod+shift+Enter resolve per platform as before", () => {
        expect(canonical("mod+Enter", true)).toBe("meta+Enter");
        expect(canonical("mod+Enter", false)).toBe("ctrl+Enter");
        expect(canonical("mod+shift+Enter", true)).toBe("meta+shift+Enter");
        expect(matchesKeys("mod+0", chordToken(press("0", {[IS_MAC ? "metaKey" : "ctrlKey"]: true, code: "Digit0"})))).toBe(true);
    });

    it("a g chord still splits into two plain steps", () => {
        expect(steps("g j", true)).toEqual(["g", "j"]);
        expect(steps("g j", false)).toEqual(["g", "j"]);
    });
});

describe("UNIT formatKeys per platform (plan 46 K)", () => {
    it("renders the slot chord with each platform's glyphs and joiner", () => {
        expect(formatKeys("meta+alt+1", true)).toEqual([{text: "⌥-⌘-1"}]);
        expect(formatKeys("ctrl+shift+1", false)).toEqual([{text: "Ctrl+Shift+1"}]);
        expect(formatKeys("ctrl+shift+ArrowDown", false)).toEqual([{text: "Ctrl+Shift+↓"}]);
    });

    it("renders existing bindings as before", () => {
        expect(formatKeys("mod+shift+Enter", true)).toEqual([{text: "⌘-Shift-Enter"}]);
        expect(formatKeys("mod+shift+Enter", false)).toEqual([{text: "Ctrl+Shift+Enter"}]);
        expect(formatKeys("alt+S", true)).toEqual([{text: "⌥-Shift-S"}]);
        expect(formatKeys("alt+S", false)).toEqual([{text: "Alt+Shift+S"}]);
        expect(formatKeys("mod++", true)).toEqual([{text: "⌘-+"}]);
        expect(formatKeys("?", true)).toEqual([{text: "?"}]);
    });
});
