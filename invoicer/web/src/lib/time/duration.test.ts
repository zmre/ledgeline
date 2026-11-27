// Durations read and written as the server does (parse_duration, TimeQty's
// Display), and the banked totals Track shows.
import {describe, expect, it} from "vitest";
import {durationText, formatDuration, minutesText, parseDuration, roundMinutes} from "./duration.js";

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

    it("rounds each session to the minute, half up (T4)", () => {
        expect(roundMinutes(29)).toBe(0);
        expect(roundMinutes(30)).toBe(1);
        expect(roundMinutes(80)).toBe(1);
        expect(roundMinutes(90)).toBe(2);
        expect(roundMinutes(-5)).toBe(0);
        expect(roundMinutes(-90)).toBe(-2);
    });

    it("reads banked totals as 45m, 2h, 2h 05m, and nothing as a dash", () => {
        expect(durationText(0)).toBe("—");
        expect(durationText(29)).toBe("—");
        expect(durationText(-600)).toBe("—");
        expect(durationText(45 * 60)).toBe("45m");
        expect(durationText(2 * 3600)).toBe("2h");
        expect(durationText(2 * 3600 + 5 * 60)).toBe("2h 05m");
    });
});
