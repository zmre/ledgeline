import {describe, expect, it} from "vitest";
import {datePattern} from "./datePattern";

describe("datePattern", () => {
    it.each([
        ["en-US", "m/d/y"],
        ["en-GB", "d/m/y"],
        ["de-DE", "d.m.y"],
        ["sv-SE", "y-m-d"],
        ["ja-JP", "y/m/d"],
    ])("follows %s's own field order and separators", (locale, expected) => {
        expect(datePattern(locale)).toBe(expected);
    });

    it("carries no bidi marks into the hint", () => {
        expect(datePattern("he-IL")).not.toMatch(/[\u200E\u200F\u061C]/);
    });

    it("always names all three fields, whatever the locale", () => {
        for (const locale of ["fr-FR", "ko-KR", "hu-HU", "zh-CN", "nl-NL"]) {
            expect([...datePattern(locale)].filter((c) => "ymd".includes(c)).sort()).toEqual(["d", "m", "y"]);
        }
    });
});
