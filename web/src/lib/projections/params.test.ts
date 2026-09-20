import {describe, expect, it} from "vitest";
import {MAX_COUNT} from "$lib/reports/ui/params";
import {
    DEFAULT_PROJECTION_COUNT,
    defaultProjectionParams,
    projectionParamsToSearch,
    PROJECTION_TABS,
    searchToProjectionParams,
    type ProjectionParams,
} from "./params";

describe("UNIT projections params — the URL codec", () => {
    const dflt = defaultProjectionParams();

    it("opens on net income, monthly, over two years", () => {
        expect(dflt).toEqual({tab: "net", interval: "monthly", count: DEFAULT_PROJECTION_COUNT, depth: 2});
        // The horizon a runway question is asked over, and inside MAX_COUNT.
        expect(DEFAULT_PROJECTION_COUNT).toBe(24);
        expect(DEFAULT_PROJECTION_COUNT).toBeLessThanOrEqual(MAX_COUNT);
    });

    it("round-trips every tab", () => {
        for (const tab of PROJECTION_TABS) {
            const params: ProjectionParams = {tab, interval: "quarterly", count: 8, depth: 4};
            expect(searchToProjectionParams(projectionParamsToSearch(params), dflt)).toEqual(params);
        }
    });

    it("round-trips through a search string with a leading ?", () => {
        const params: ProjectionParams = {tab: "cash", interval: "yearly", count: 5, depth: 1};
        expect(searchToProjectionParams(`?${projectionParamsToSearch(params)}`, dflt)).toEqual(params);
    });

    it("writes every param in full, so a shared link reproduces the same charts", () => {
        // Not only the non-defaults: the defaults are relative to a codebase that
        // can change them, and a link saved today must still open the same page.
        expect(projectionParamsToSearch(dflt)).toBe("tab=net&interval=monthly&count=24&depth=2");
    });

    it("falls back per field rather than wholesale, so one bad param does not discard the rest", () => {
        expect(searchToProjectionParams("tab=nonsense&interval=weekly&count=6&depth=3", dflt)).toEqual({
            tab: "net",
            interval: "monthly",
            count: 6,
            depth: 3,
        });
    });

    it("an unknown tab falls back rather than rendering an empty box with no way out", () => {
        expect(searchToProjectionParams("tab=runway", dflt).tab).toBe("net");
    });

    it("clamps count into [1, MAX_COUNT] and depth into [1, 99]", () => {
        expect(searchToProjectionParams(`count=0&depth=0`, dflt)).toMatchObject({count: 1, depth: 1});
        expect(searchToProjectionParams(`count=${MAX_COUNT + 500}&depth=500`, dflt)).toMatchObject({count: MAX_COUNT, depth: 99});
    });

    it("a malformed number is the default, not NaN", () => {
        expect(searchToProjectionParams("count=lots&depth=-3", dflt)).toMatchObject({count: dflt.count, depth: dflt.depth});
    });

    it("an empty search is the defaults", () => {
        expect(searchToProjectionParams("", dflt)).toEqual(dflt);
    });

    it("never carries the scenario — a query string is the one part of a page that gets pasted into chat windows", () => {
        const search = projectionParamsToSearch({tab: "cash", interval: "monthly", count: 24, depth: 2});
        expect([...new URLSearchParams(search).keys()].sort()).toEqual(["count", "depth", "interval", "tab"]);
    });
});
