// Durations read and written as the server does (parse_duration, TimeQty's
// Display), and the note check that warns before the server refuses.
import {describe, expect, it} from "vitest";
import {formatDuration, minutesText, noteWarning, parseDuration, roundMinutes} from "./duration.js";

describe("UNIT time durations", () => {
    it("reads every form the server takes", () => {
        expect(parseDuration("45m")).toBe(2700);
        expect(parseDuration("2h")).toBe(7200);
        expect(parseDuration("1h30")).toBe(5400);
        expect(parseDuration("1h 30m")).toBe(5400);
        expect(parseDuration("1h05m")).toBe(3900);
        expect(parseDuration("1.5")).toBe(5400);
        expect(parseDuration(".25")).toBe(900);
        expect(parseDuration("0:45")).toBe(2700);
        expect(parseDuration("90s")).toBe(90);
        expect(parseDuration("1.5h")).toBe(5400);
    });

    it("refuses what the server refuses", () => {
        for (const bad of ["", "soon", "-1h", "1:75", "1:2:3", "h", "1hx", "m"]) expect(parseDuration(bad), bad).toBeNull();
    });

    it("writes minutes half up, as the server's text", () => {
        expect(formatDuration(0)).toBe("0m");
        expect(formatDuration(2700)).toBe("45m");
        expect(formatDuration(7200)).toBe("2h");
        expect(formatDuration(3900)).toBe("1h 05m");
        expect(formatDuration(29)).toBe("0m");
        expect(formatDuration(30)).toBe("1m");
        expect(formatDuration(-900)).toBe("-15m");
        expect(roundMinutes(90)).toBe(2);
        expect(minutesText(5430)).toBe("91m");
    });

    it("round-trips its own text", () => {
        for (const minutes of [1, 45, 60, 65, 130, 600]) expect(parseDuration(formatDuration(minutes * 60))).toBe(minutes * 60);
    });

    it("warns about a tag, a semicolon or a line break in a note", () => {
        expect(noteWarning("call with Fred")).toBeNull();
        expect(noteWarning("re: K-1")?.offending).toBe("re:");
        expect(noteWarning("met at 10:30")?.offending).toBe("10:");
        expect(noteWarning("one; two")?.offending).toBe(";");
        expect(noteWarning("one\ntwo")?.offending).toBe("a line break");
        expect(noteWarning("see https://example.com")?.offending).toBe("https:");
    });
});
