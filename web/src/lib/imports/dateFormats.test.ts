import {describe, expect, it} from "vitest";
import {CUSTOM_OPTION, DATE_FORMATS, findDateFormat, REFERENCE, strftimeExample} from "./dateFormats";

// `date-format` is the setting that fails silently — `%m/%d/%Y` and `%d/%m/%Y`
// accept the same bytes and disagree about what they mean — so the example
// string a user reads to choose between them has to be right.

describe("UNIT dateFormats — the catalogue", () => {
    // The catalogue's examples are written out by hand so the picker is correct
    // even if the renderer is not; this turns that duplication into a mutual
    // check rather than two places to be wrong in.
    it("renders every catalogue example exactly as it is written down", () => {
        for (const option of DATE_FORMATS) {
            expect(strftimeExample(option.pattern), `pattern ${option.pattern}`).toBe(option.example);
        }
    });

    it("distinguishes month-first from day-first, which is the whole point", () => {
        expect(strftimeExample("%m/%d/%Y")).toBe("01/15/2026");
        expect(strftimeExample("%d/%m/%Y")).toBe("15/01/2026");
        // The reference day is past 12 on purpose: with a day of 03 both
        // patterns would render the same string and the control would be a lie.
        expect(Number(REFERENCE.day)).toBeGreaterThan(12);
    });

    it("finds a catalogue entry by pattern and reports nothing for anything else", () => {
        expect(findDateFormat("%Y-%m-%d")?.label).toBe("ISO — year first");
        expect(findDateFormat("%d %m %Y")).toBeNull();
        expect(findDateFormat("")).toBeNull();
    });

    it("keeps the custom sentinel out of the pattern space", () => {
        expect(findDateFormat(CUSTOM_OPTION)).toBeNull();
        expect(DATE_FORMATS.some((option) => option.pattern === CUSTOM_OPTION)).toBe(false);
    });

    it("has no duplicate patterns", () => {
        expect(new Set(DATE_FORMATS.map((option) => option.pattern)).size).toBe(DATE_FORMATS.length);
    });
});

describe("UNIT dateFormats — strftimeExample", () => {
    it("expands the specifiers a bank CSV actually uses", () => {
        expect(strftimeExample("%Y")).toBe("2026");
        expect(strftimeExample("%y")).toBe("26");
        expect(strftimeExample("%m")).toBe("01");
        expect(strftimeExample("%d")).toBe("15");
        expect(strftimeExample("%b")).toBe("Jan");
        expect(strftimeExample("%B")).toBe("January");
        expect(strftimeExample("%a")).toBe("Thu");
        expect(strftimeExample("%A")).toBe("Thursday");
        expect(strftimeExample("%H:%M:%S")).toBe("13:45:07");
        expect(strftimeExample("%F")).toBe("2026-01-15");
        expect(strftimeExample("%D")).toBe("01/15/26");
        expect(strftimeExample("%j")).toBe("015");
    });

    it("keeps literal text between specifiers", () => {
        expect(strftimeExample("on %d %B %Y at %H:%M")).toBe("on 15 January 2026 at 13:45");
        expect(strftimeExample("no specifiers here")).toBe("no specifiers here");
        expect(strftimeExample("")).toBe("");
    });

    it("applies the `%-` and `%_` padding flags", () => {
        expect(strftimeExample("%-m/%-d/%Y")).toBe("1/15/2026");
        expect(strftimeExample("%-j")).toBe("15");
        expect(strftimeExample("%_m")).toBe(" 1");
        // `%0` asks for the padding that is already there.
        expect(strftimeExample("%0m")).toBe("01");
    });

    it("renders `%%` as one literal percent", () => {
        expect(strftimeExample("100%%")).toBe("100%");
        expect(strftimeExample("%%Y")).toBe("%Y");
    });

    // An unknown specifier is echoed rather than guessed at: an example with a
    // raw `%q` in it says "Ledgeline cannot preview this piece", where a
    // plausible substitution would be a confident lie about the user's data.
    it("echoes a specifier it does not know, verbatim", () => {
        expect(strftimeExample("%q")).toBe("%q");
        expect(strftimeExample("%Y-%q-%d")).toBe("2026-%q-15");
        expect(strftimeExample("%-q")).toBe("%-q");
    });

    it("echoes a trailing lone `%` rather than swallowing it", () => {
        expect(strftimeExample("%Y%")).toBe("2026%");
        expect(strftimeExample("%")).toBe("%");
    });

    it("is a pure function of its arguments — no clock, no zone, no locale", () => {
        const alternate = {...REFERENCE, year: "1999", month: "12", day: "31", monthAbbrev: "Dec"};
        expect(strftimeExample("%d-%b-%Y", alternate)).toBe("31-Dec-1999");
        // The default reference is untouched by that call.
        expect(strftimeExample("%d-%b-%Y")).toBe("15-Jan-2026");
    });
});
