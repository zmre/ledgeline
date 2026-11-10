// The balance line's display decisions (plan 36 amendment A1). The numbers are
// the engine's — crates/ledgeline-core/tests/running_balance.rs pins them
// against `hledger reg`; this file pins only how they are labelled.

import {describe, expect, it} from "vitest";
import {balanceSeries} from "#lib/testing/balanceSeriesFixture.js";
import {balanceLabels, balanceValues, failureMarkers, markerExtent, periodLines, periodTarget, trendNote, windowNote} from "./balanceTrend";

describe("UNIT balanceTrend", () => {
    it("labels weeks by their last day, as the holdings chart does", () => {
        expect(balanceLabels(balanceSeries())).toEqual(["Jan 4", "Jan 11", "Jan 18", "Jan 25", "Feb 1", "Feb 3"]);
        const monthly = balanceSeries({
            interval: "monthly",
            points: [{bucket: "2026-01", label: "Jan 2026", end: "2026-01-31", balance: {m: "1", p: 0}}],
        });
        expect(balanceLabels(monthly)).toEqual(["Jan 2026"]);
    });

    it("names the commodity and the window, with the year said once", () => {
        expect(trendNote(balanceSeries())).toBe("$ · Jan 1 – Feb 3, 2026");
        expect(windowNote("2025-12-01", "2026-03-01")).toBe("Dec 1, 2025 – Mar 1, 2026");
        expect(windowNote("2026-03-01", "2026-03-01")).toBe("Mar 1, 2026");
    });

    it("plots each period's balance as written", () => {
        expect(balanceValues(balanceSeries())).toEqual([6000, 5984.25, 5922.25, 5402.25, 5402.25, 5402.25]);
    });

    it("tells the tooltip the date, and for a failure what was asserted, found and the difference", () => {
        const series = balanceSeries();
        expect(periodLines(series, 0)).toEqual(["at end of 2026-01-04"]);
        expect(periodLines(series, 4)).toEqual(["at end of 2026-02-01", "✗ 2026-01-31 assets:bank:checking = expected $300.00, found $220.25, off by $-79.75"]);
        expect(periodLines(series, 99)).toEqual([]);
    });

    it("marks only failing periods, with the gap to where the line should be", () => {
        const markers = failureMarkers(balanceSeries());
        expect(markers).toHaveLength(1);
        expect(markers[0]).toMatchObject({index: 4, balance: 5402.25, expected: 5482});
        expect(markers[0].title).toContain("expected $300.00");
        expect(markerExtent(markers)).toEqual([5482]);
    });

    it("draws one marker for a period with several failures, to the worst gap", () => {
        const series = balanceSeries();
        const wire = (m: number) => ({m: String(m), p: 2});
        const failure = (expected: number) => ({
            tindex: 11,
            date: "2026-01-31",
            account: "assets:bank:checking",
            op: "=",
            asserted: wire(0),
            calculated: wire(0),
            expected: wire(expected),
        });
        const crowded = balanceSeries({
            points: [{bucket: "2026-W05", label: "W05 2026", end: "2026-02-01", balance: wire(540_225), failures: [failure(541_000), failure(500_000)]}],
        });
        const markers = failureMarkers(crowded);
        expect(markers).toHaveLength(1);
        expect(markers[0].expected).toBe(5000);
        expect(markers[0].title.split("\n")).toHaveLength(2);
        expect(series.points).toHaveLength(6);
    });

    it("scrolls to the period's newest transaction, or nowhere when nothing posted", () => {
        const series = balanceSeries();
        expect(periodTarget(series, 1)).toBe(16);
        expect(periodTarget(series, 5)).toBeNull();
        expect(periodTarget(series, 42)).toBeNull();
    });
});
