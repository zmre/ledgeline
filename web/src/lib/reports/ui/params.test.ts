import {describe, expect, it} from "vitest";
import {
    budgetRedirect,
    defaultReportParams,
    MAX_COUNT,
    paramsToSearch,
    rangeKeys,
    searchToParams,
    TAB_CONTROLS,
    TAB_DEFAULTS,
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
                // Cash flow's own range: the first of the month eleven months
                // back through today — the twelve monthly columns the old End +
                // 12 Periods default drew.
                cfFrom: "2025-08-01",
                cfTo: "2026-07-08",
                end: "2026-07-08",
                interval: "monthly",
                count: 12,
                // Shared by cf and nw. Neither statement reads it: both tabs
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
            expect(paramsToSearch({...DFLT, tab: "is", interval: "total"})).toBe("tab=is&from=2026-01-01&to=2026-12-31&interval=total");
            // Cash flow writes ITS OWN range as from/to, and no end or count.
            expect(paramsToSearch({...DFLT, tab: "cf"})).toBe("tab=cf&from=2025-08-01&to=2026-07-08&interval=monthly&depth=3");
            expect(paramsToSearch({...DFLT, tab: "nw"})).toBe("tab=nw&end=2026-07-08&interval=monthly&count=12&depth=3");
            // Subscriptions scan a fixed trailing window, so the tab is all there is to restore.
            expect(paramsToSearch({...DFLT, tab: "subs"})).toBe("tab=subs");
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
                    cfFrom: "2024-10-15",
                    cfTo: "2025-03-31",
                    end: "2025-12-31",
                    interval: "quarterly",
                    count: 8,
                    depth: 3,
                };
                const parsed = searchToParams(paramsToSearch(params), DFLT);
                const config = TAB_CONTROLS[tab];
                expect(parsed.tab).toBe(tab);
                if (config.asOf) expect(parsed.asOf).toBe(params.asOf);
                const keys = rangeKeys(tab);
                if (config.range) expect([parsed[keys.from], parsed[keys.to]]).toEqual([params[keys.from], params[keys.to]]);
                if (config.end) expect(parsed.end).toBe(params.end);
                if (config.intervals.length > 0) expect(parsed.interval).toBe(params.interval);
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
            expect(paramsToSearch(parsed)).toBe("tab=is&from=2025-01-01&to=2025-06-30&interval=total");
        });

        it("shows a depth slider on the period tabs and on neither statement", () => {
            // The control is driven off this config, so this IS the assertion
            // that the slider is gone from the P&L (`ReportControls` renders it
            // `{#if config.depth}`).
            expect(TAB_CONTROLS.bs.depth).toBe(false);
            expect(TAB_CONTROLS.is.depth).toBe(false);
            expect(TAB_CONTROLS.is.range).toBe(true);
            expect([TAB_CONTROLS.cf.depth, TAB_CONTROLS.nw.depth]).toEqual([true, true]);
        });

        it("reads a P&L link with no interval as the Total statement it always showed", () => {
            // The Interval control is new on this tab (plans/24); every older P&L
            // link has no `interval`, and what it showed was one column against
            // the prior period — not the shared default's monthly columns.
            expect(searchToParams("tab=is&from=2025-01-01&to=2025-06-30", DFLT).interval).toBe("total");
            expect(searchToParams("tab=is&from=2025-01-01&to=2025-06-30&interval=quarterly", DFLT).interval).toBe("quarterly");
            expect(searchToParams("tab=is&interval=hourly", DFLT).interval).toBe("total");
        });

        it("never gives net worth a Total, which it does not offer", () => {
            // Summing point-in-time snapshots means nothing, so `total` on the nw
            // tab falls back to the default exactly as an unknown value would.
            expect(searchToParams("tab=nw&interval=total", DFLT).interval).toBe(DFLT.interval);
            expect(searchToParams("tab=nw&interval=yearly", DFLT).interval).toBe("yearly");
        });

        it("translates a cash-flow link from before From/To into the range it drew", () => {
            // `end` + `count` (+ interval) named the last N buckets ending with
            // the one containing `end`; the same columns are the range from the
            // oldest bucket's first day through `end`.
            const monthly = searchToParams("tab=cf&end=2026-07-08&interval=monthly&count=12&depth=2", DFLT);
            expect([monthly.cfFrom, monthly.cfTo, monthly.interval, monthly.depth]).toEqual(["2025-08-01", "2026-07-08", "monthly", 2]);
            const quarterly = searchToParams("tab=cf&end=2026-05-20&interval=quarterly&count=4", DFLT);
            expect([quarterly.cfFrom, quarterly.cfTo]).toEqual(["2025-07-01", "2026-05-20"]);
            // A count alone still counts back from the default end.
            const counted = searchToParams("tab=cf&count=3", DFLT);
            expect([counted.cfFrom, counted.cfTo]).toEqual(["2026-05-01", "2026-07-08"]);
            // And the next URL mirror writes the new shape.
            expect(paramsToSearch(monthly)).toBe("tab=cf&from=2025-08-01&to=2026-07-08&interval=monthly&depth=2");
        });

        it("keeps the P&L's range and cash flow's apart", () => {
            // from/to name the ACTIVE tab's range, so a cash-flow link cannot
            // overwrite the P&L's calendar year, or the reverse.
            const cf = searchToParams("tab=cf&from=2026-02-01&to=2026-03-31&interval=total", DFLT);
            expect([cf.cfFrom, cf.cfTo, cf.interval]).toEqual(["2026-02-01", "2026-03-31", "total"]);
            expect([cf.from, cf.to]).toEqual([DFLT.from, DFLT.to]);
            const is = searchToParams("tab=is&from=2026-02-01&to=2026-03-31", DFLT);
            expect([is.from, is.to]).toEqual(["2026-02-01", "2026-03-31"]);
            expect([is.cfFrom, is.cfTo]).toEqual([DFLT.cfFrom, DFLT.cfTo]);
        });

        it("offers Total on the ranged reports only, first, and opens the P&L on it", () => {
            expect(TAB_CONTROLS.is.intervals).toEqual(["total", "monthly", "quarterly", "yearly"]);
            expect(TAB_CONTROLS.cf.intervals).toEqual(["total", "monthly", "quarterly", "yearly"]);
            expect(TAB_CONTROLS.nw.intervals).toEqual(["monthly", "quarterly", "yearly"]);
            expect([TAB_DEFAULTS.is.interval, TAB_DEFAULTS.cf.interval, TAB_DEFAULTS.nw.interval]).toEqual(["total", "monthly", "yearly"]);
            // Cash flow traded End + Periods for From/To; net worth kept both.
            expect([TAB_CONTROLS.cf.range, TAB_CONTROLS.cf.end, TAB_CONTROLS.cf.count]).toEqual([true, false, false]);
            expect([TAB_CONTROLS.nw.range, TAB_CONTROLS.nw.end, TAB_CONTROLS.nw.count]).toEqual([false, true, true]);
        });

        it("clamps count and depth to sane ranges", () => {
            const parsed = searchToParams(`tab=cf&count=${MAX_COUNT + 500}&depth=0`, DFLT);
            expect(parsed.count).toBe(MAX_COUNT);
            expect(parsed.depth).toBe(1);
        });
    });
});

describe("UNIT reports/ui/params — the Budget tab's old address", () => {
    it("forwards ?tab=budget, carrying the params the new route still honours", () => {
        // `tab` is dropped (there is no tab to name any more) and from/to/depth
        // survive, so the forwarded link shows the same budget the old one did.
        expect(budgetRedirect("?tab=budget&from=2026-01-01&to=2026-06-30&depth=2")).toBe("?from=2026-01-01&to=2026-06-30&depth=2");
    });

    it("forwards a bare ?tab=budget to no query at all", () => {
        // An empty string, not "?" — appending a lone question mark to the route
        // would put one in the address bar for nothing.
        expect(budgetRedirect("?tab=budget")).toBe("");
    });

    it("declines every URL that is not the budget tab", () => {
        // null, not "": the page distinguishes "forward this" from "forward this
        // with no query", and collapsing them would redirect every reports visit.
        expect(budgetRedirect("?tab=bs&asof=2026-07-08")).toBeNull();
        expect(budgetRedirect("")).toBeNull();
        expect(budgetRedirect("?from=2026-01-01")).toBeNull();
        // A tab that merely CONTAINS the word is not the budget tab.
        expect(budgetRedirect("?tab=budgetary")).toBeNull();
    });

    it("tolerates a missing leading ?", () => {
        expect(budgetRedirect("tab=budget&depth=3")).toBe("?depth=3");
    });
});
