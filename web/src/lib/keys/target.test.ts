// The single guard that makes a non-modal keymap possible: if this is wrong,
// either `j` types nothing in the search box or `j` navigates while you are
// naming a payee.
//
// `TargetLike` is structural precisely so these are object literals in the node
// project rather than jsdom mounts.

import {describe, expect, it} from "vitest";
import {isTypingTarget, TYPING_ATTRIBUTE, type TargetLike} from "./target";

function target(tagName: string, extra: Partial<TargetLike> = {}): TargetLike {
    return {tagName, closest: () => null, ...extra};
}

describe("UNIT isTypingTarget", () => {
    it("is false for nothing focused", () => {
        expect(isTypingTarget(null)).toBe(false);
    });

    it("is true for the input types that swallow letters", () => {
        for (const type of ["text", "search", "email", "password", "number", "date"]) {
            expect(isTypingTarget(target("INPUT", {type}))).toBe(true);
        }
    });

    it("is true for an input with no type at all, which defaults to text", () => {
        expect(isTypingTarget(target("INPUT"))).toBe(true);
    });

    it("is FALSE for inputs that do not take letters", () => {
        // The column menu is checkboxes and the holdings scope bar is buttons.
        // `j` has to keep working while one of those has focus, or clicking a
        // filter silently kills navigation until you click elsewhere.
        for (const type of ["checkbox", "radio", "button", "submit", "reset", "file", "color", "range"]) {
            expect(isTypingTarget(target("INPUT", {type}))).toBe(false);
        }
    });

    it("is true for textarea and select", () => {
        // <select> counts because native type-ahead is real: "dec" jumps to December.
        expect(isTypingTarget(target("TEXTAREA"))).toBe(true);
        expect(isTypingTarget(target("SELECT"))).toBe(true);
    });

    it("is true inside a contenteditable, including for a descendant", () => {
        // `isContentEditable` is inherited, unlike the attribute.
        expect(isTypingTarget(target("SPAN", {isContentEditable: true}))).toBe(true);
    });

    it("is false for ordinary elements", () => {
        expect(isTypingTarget(target("BUTTON"))).toBe(false);
        expect(isTypingTarget(target("TR"))).toBe(false);
        expect(isTypingTarget(target("BODY"))).toBe(false);
    });

    it("is true anywhere under a data-keys-typing ancestor", () => {
        // The account combobox's opt-in: its popup is a <ul> of <li>, neither of
        // which is a field, but arrow keys there belong to the combobox.
        const marked = target("LI", {closest: (selector) => (selector === `[${TYPING_ATTRIBUTE}]` ? {} : null)});

        expect(isTypingTarget(marked)).toBe(true);
    });
});
