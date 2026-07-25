import {describe, expect, test} from "vitest";
import {activeInsightsPreset, defaultReportParams, insightsPresetRange} from "./params";

const NOW = "2026-07-24";

describe("insights comparison presets", () => {
    test("year-over-year is a trailing 24 complete months", () => {
        // 24 months ending with the last COMPLETE month (June 2026); the engine
        // splits this into two clean 12-month halves.
        expect(insightsPresetRange("yoy", NOW)).toEqual({start: "2024-07-01", end: "2026-06-30"});
    });

    test("shorter presets span the right number of complete months", () => {
        expect(insightsPresetRange("hoh", NOW)).toEqual({start: "2025-07-01", end: "2026-06-30"}); // 12 mo
        expect(insightsPresetRange("qoq", NOW)).toEqual({start: "2026-01-01", end: "2026-06-30"}); // 6 mo
        expect(insightsPresetRange("mom", NOW)).toEqual({start: "2026-05-01", end: "2026-06-30"}); // 2 mo
    });

    test("activeInsightsPreset recognises a preset span and flags custom otherwise", () => {
        expect(activeInsightsPreset("2024-07-01", "2026-06-30", NOW)).toBe("yoy");
        expect(activeInsightsPreset("2026-05-01", "2026-06-30", NOW)).toBe("mom");
        expect(activeInsightsPreset("2024-07-01", "2026-06-15", NOW)).toBe("custom");
    });

    test("Insights is the default landing tab, seeded with the year-over-year span", () => {
        const params = defaultReportParams(NOW);
        expect(params.tab).toBe("insights");
        expect(params.insStart).toBe("2024-07-01");
        expect(params.insEnd).toBe("2026-06-30");
    });
});
