// SEC-12 regression suite. The bug: `decodeURIComponent` throws `URIError` on a
// malformed percent escape, and both query-string codecs call it during
// `onMount`, so `/?acct=%` took down the entire page mount.
//
// Every "malformed" case below is asserted TWICE: once to pin that the raw
// builtin really does throw (so the test proves the bug existed and would catch
// a regression to the unguarded call), and once that `safeDecode` does not.

import {describe, expect, it} from "vitest";
import {safeDecode} from "./safeDecode";

// Inputs that make the raw builtin throw. `%E0` is a truncated UTF-8 lead byte:
// well-formed hex, but not a complete code point.
const MALFORMED = ["%", "%zz", "%E0", "%C3%28", "%%", "100%", "a%", "%FF"];

describe("UNIT url/safeDecode", () => {
    describe("malformed percent escapes", () => {
        it.each(MALFORMED)("decodeURIComponent(%o) throws URIError — the bug being guarded", (input) => {
            expect(() => decodeURIComponent(input)).toThrow(URIError);
        });

        it.each(MALFORMED)("safeDecode(%o) does not throw and returns the segment unchanged", (input) => {
            expect(() => safeDecode(input)).not.toThrow();
            expect(safeDecode(input)).toBe(input);
        });
    });

    describe("well-formed input is decoded normally", () => {
        it.each([
            ["", ""], // empty string: valid, decodes to itself
            ["plain", "plain"],
            ["a%20b", "a b"],
            ["%2F", "/"],
            ["%C3%A9", "é"],
            ["assets%3Abank%3Achecking", "assets:bank:checking"],
            ["expenses%2Cfood", "expenses,food"],
            ["%F0%9F%92%B0", "💰"], // 4-byte UTF-8 (astral plane)
            ["%25", "%"], // an ENCODED percent still round-trips
        ])("safeDecode(%o) === %o", (input, expected) => {
            expect(safeDecode(input)).toBe(expected);
        });
    });

    it("is idempotent on already-decoded text with no escapes", () => {
        expect(safeDecode("assets:bank")).toBe("assets:bank");
    });

    it("rethrows non-URIError failures instead of swallowing them", () => {
        // A hostile `toString` proves the catch is narrowed to URIError and is
        // not a blanket swallow that would hide real bugs.
        const boom = {
            toString() {
                throw new RangeError("not a URIError");
            },
        };
        expect(() => safeDecode(boom as unknown as string)).toThrow(RangeError);
    });
});
