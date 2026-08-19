import {describe, expect, it} from "vitest";
import {
    activeBudgetPreset,
    budgetPresetRange,
    defaultReportParams,
    MAX_COUNT,
    paramsToSearch,
    searchToParams,
    TAB_CONTROLS,
    TAB_ORDER,
    type ReportParams,
} from "./params";

const DFLT = defaultReportParams("2026-07-08");

describe("UNIT reports/ui/params", () => {
    describe("defaultReportParams", () => {
        it("uses today for point-in-time dates and the calendar year for the P&L range", () => {
            expect(DFLT).toEqual({
                // Insights is the landing tab, seeded with the year-over-year span
                // (24 complete months ending with the last full month).
                tab: "insights",
                asOf: "2026-07-08",
                from: "2026-01-01",
                to: "2026-12-31",
                end: "2026-07-08",
                interval: "monthly",
                count: 12,
                // Shared by cf/nw/budget. Neither statement reads it: both tabs
                // ask the engine for an unclamped report.
                depth: 3,
                insStart: "2024-07-01",
                insEnd: "2026-06-30",
            });
        });
    });

    describe("paramsToSearch", () => {
        it("writes only the active tab's params, in full", () => {
            expect(paramsToSearch(DFLT)).toBe("tab=insights&istart=2024-07-01&iend=2026-06-30");
            // No `depth` on bs or is: the slider is gone from both statements
            // and each requests an unclamped report, so the URL has nothing to
            // say about depth. The P&L keeps its RANGE — that is the one thing a
            // report about a period cannot infer.
            expect(paramsToSearch({...DFLT, tab: "bs"})).toBe("tab=bs&asof=2026-07-08");
            expect(paramsToSearch({...DFLT, tab: "is"})).toBe("tab=is&from=2026-01-01&to=2026-12-31");
            expect(paramsToSearch({...DFLT, tab: "cf"})).toBe("tab=cf&end=2026-07-08&interval=monthly&count=12&depth=3");
            expect(paramsToSearch({...DFLT, tab: "nw"})).toBe("tab=nw&end=2026-07-08&interval=monthly&count=12&depth=3");
            expect(paramsToSearch({...DFLT, tab: "budget"})).toBe("tab=budget&from=2026-01-01&to=2026-12-31&depth=3");
            // Subscriptions scan a fixed trailing window, so the tab is all there is to restore.
            expect(paramsToSearch({...DFLT, tab: "subs"})).toBe("tab=subs");
        });
    });

    describe("budget presets", () => {
        const NOW = "2026-07-21";

        it("resolves each preset to an inclusive range", () => {
            expect(budgetPresetRange("this-month", NOW)).toEqual({from: "2026-07-01", to: "2026-07-21"});
            expect(budgetPresetRange("last-month", NOW)).toEqual({from: "2026-06-01", to: "2026-06-30"});
            expect(budgetPresetRange("ytd", NOW)).toEqual({from: "2026-01-01", to: "2026-07-21"});
            expect(budgetPresetRange("this-year", NOW)).toEqual({from: "2026-01-01", to: "2026-12-31"});
            expect(budgetPresetRange("trailing-12", NOW)).toEqual({from: "2025-08-01", to: "2026-07-21"});
        });

        it("identifies the active preset, or 'custom' for an unmatched range", () => {
            expect(activeBudgetPreset("2026-01-01", "2026-07-21", NOW)).toBe("ytd");
            expect(activeBudgetPreset("2026-06-01", "2026-06-30", NOW)).toBe("last-month");
            expect(activeBudgetPreset("2020-03-01", "2020-04-15", NOW)).toBe("custom");
        });
    });

    describe("searchToParams", () => {
        it("round-trips every tab", () => {
            for (const tab of TAB_ORDER) {
                const params: ReportParams = {
                    ...DFLT,
                    tab,
                    asOf: "2025-03-31",
                    from: "2025-01-01",
                    to: "2025-06-30",
                    end: "2025-12-31",
                    interval: "quarterly",
                    count: 8,
                    depth: 3,
                };
                const parsed = searchToParams(paramsToSearch(params), DFLT);
                const config = TAB_CONTROLS[tab];
                expect(parsed.tab).toBe(tab);
                if (config.asOf) expect(parsed.asOf).toBe(params.asOf);
                if (config.range) expect([parsed.from, parsed.to]).toEqual([params.from, params.to]);
                if (config.end) expect(parsed.end).toBe(params.end);
                if (config.interval) expect(parsed.interval).toBe(params.interval);
                if (config.count) expect(parsed.count).toBe(params.count);
                if (config.depth) expect(parsed.depth).toBe(params.depth);
            }
        });

        it("falls back to defaults for absent params (leading ? tolerated)", () => {
            expect(searchToParams("?tab=nw", DFLT)).toEqual({...DFLT, tab: "nw"});
            expect(searchToParams("", DFLT)).toEqual(DFLT);
        });

        it("ignores malformed values", () => {
            const parsed = searchToParams("tab=bogus&asof=07/08/2026&interval=hourly&count=zero&depth=-3", DFLT);
            expect(parsed).toEqual(DFLT);
        });

        it("still loads a bookmarked ?tab=bs&depth=N from before the slider was removed", () => {
            // Decoding is deliberately NOT tab-gated, so a stale `depth` lands
            // in the shared field instead of erroring. The balance sheet ignores
            // it, and the next URL mirror drops it — a saved link keeps working.
            const parsed = searchToParams("tab=bs&asof=2026-07-08&depth=3", DFLT);
            expect(parsed.tab).toBe("bs");
            expect(parsed.asOf).toBe("2026-07-08");
            expect(parsed.depth).toBe(3);
            expect(paramsToSearch(parsed)).toBe("tab=bs&asof=2026-07-08");
        });

        it("still loads a bookmarked ?tab=is&depth=N, keeping the range it does honor", () => {
            // Same posture, one tab further on. The range is what the P&L
            // actually needs restoring, and it must survive the stale param
            // beside it rather than being dropped with it.
            const parsed = searchToParams("tab=is&from=2025-01-01&to=2025-06-30&depth=4", DFLT);
            expect(parsed.tab).toBe("is");
            expect([parsed.from, parsed.to]).toEqual(["2025-01-01", "2025-06-30"]);
            expect(parsed.depth).toBe(4);
            expect(paramsToSearch(parsed)).toBe("tab=is&from=2025-01-01&to=2025-06-30");
        });

        it("shows a depth slider on the period tabs and on neither statement", () => {
            // The control is driven off this config, so this IS the assertion
            // that the slider is gone from the P&L (`ReportControls` renders it
            // `{#if config.depth}`).
            expect(TAB_CONTROLS.bs.depth).toBe(false);
            expect(TAB_CONTROLS.is.depth).toBe(false);
            expect(TAB_CONTROLS.is.range).toBe(true);
            expect([TAB_CONTROLS.cf.depth, TAB_CONTROLS.nw.depth, TAB_CONTROLS.budget.depth]).toEqual([true, true, true]);
        });

        it("clamps count and depth to sane ranges", () => {
            const parsed = searchToParams(`tab=cf&count=${MAX_COUNT + 500}&depth=0`, DFLT);
            expect(parsed.count).toBe(MAX_COUNT);
            expect(parsed.depth).toBe(1);
        });
    });
});
