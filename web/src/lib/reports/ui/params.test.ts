import {describe, expect, it} from "vitest";
import {defaultReportParams, MAX_COUNT, paramsToSearch, searchToParams, TAB_CONTROLS, TAB_ORDER, type ReportParams} from "./params";

const DFLT = defaultReportParams("2026-07-08");

describe("UNIT reports/ui/params", () => {
    describe("defaultReportParams", () => {
        it("uses today for point-in-time dates and the calendar year for the P&L range", () => {
            expect(DFLT).toEqual({
                tab: "bs",
                asOf: "2026-07-08",
                from: "2026-01-01",
                to: "2026-12-31",
                end: "2026-07-08",
                interval: "monthly",
                count: 12,
                depth: 2,
            });
        });
    });

    describe("paramsToSearch", () => {
        it("writes only the active tab's params, in full", () => {
            expect(paramsToSearch(DFLT)).toBe("tab=bs&asof=2026-07-08&depth=2");
            expect(paramsToSearch({...DFLT, tab: "is"})).toBe("tab=is&from=2026-01-01&to=2026-12-31&depth=2");
            expect(paramsToSearch({...DFLT, tab: "cf"})).toBe("tab=cf&end=2026-07-08&interval=monthly&count=12&depth=2");
            expect(paramsToSearch({...DFLT, tab: "nw"})).toBe("tab=nw&end=2026-07-08&interval=monthly&count=12&depth=2");
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

        it("clamps count and depth to sane ranges", () => {
            const parsed = searchToParams(`tab=cf&count=${MAX_COUNT + 500}&depth=0`, DFLT);
            expect(parsed.count).toBe(MAX_COUNT);
            expect(parsed.depth).toBe(1);
        });
    });
});
