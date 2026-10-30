import {describe, expect, it} from "vitest";
import {MAX_COUNT} from "#lib/reports/ui/params.js";
import {
    DEFAULT_PROJECTION_COUNT,
    defaultProjectionParams,
    projectionParamsToSearch,
    PROJECTION_TAB_LABELS,
    PROJECTION_TABS,
    searchToProjectionParams,
    type ProjectionParams,
} from "./params";

describe("UNIT projections params — the URL codec", () => {
    const dflt = defaultProjectionParams();

    it("opens on the runway, monthly, over two years with two periods of actuals", () => {
        expect(dflt).toEqual({tab: "cash", interval: "monthly", count: DEFAULT_PROJECTION_COUNT, lookback: 2, depth: 2, empty: false, show: "all"});
        // The horizon a runway question is asked over, and inside MAX_COUNT.
        expect(DEFAULT_PROJECTION_COUNT).toBe(24);
        expect(DEFAULT_PROJECTION_COUNT).toBeLessThanOrEqual(MAX_COUNT);
    });

    it("round-trips every tab", () => {
        for (const tab of PROJECTION_TABS) {
            const params: ProjectionParams = {tab, interval: "quarterly", count: 8, lookback: 3, depth: 4, empty: true, show: "off"};
            expect(searchToProjectionParams(projectionParamsToSearch(params), dflt)).toEqual(params);
        }
    });

    it("round-trips through a search string with a leading ?", () => {
        const params: ProjectionParams = {tab: "cash", interval: "yearly", count: 5, lookback: 0, depth: 1, empty: false, show: "notes"};
        expect(searchToProjectionParams(`?${projectionParamsToSearch(params)}`, dflt)).toEqual(params);
    });

    it("round-trips every account-type chip, and the old income/expense/asset values still parse", () => {
        for (const show of ["income", "expense", "asset", "cash", "liability", "equity", "gain", "conversion"] as const) {
            const params: ProjectionParams = {...dflt, show};
            expect(searchToProjectionParams(projectionParamsToSearch(params), dflt).show).toBe(show);
        }
    });

    it("writes every param in full, so a shared link reproduces the same charts", () => {
        // Not only the non-defaults: the defaults are relative to a codebase that
        // can change them, and a link saved today must still open the same page.
        expect(projectionParamsToSearch(dflt)).toBe("tab=cash&interval=monthly&count=24&lookback=2&depth=2&empty=0&show=all");
    });

    it("falls back per field rather than wholesale, so one bad param does not discard the rest", () => {
        expect(
            searchToProjectionParams("tab=nonsense&interval=weekly&count=6&lookback=4&depth=3&accounts=cumulative&empty=maybe&show=everything", dflt)
        ).toEqual({
            tab: "cash",
            interval: "monthly",
            count: 6,
            lookback: 4,
            depth: 3,
            empty: false,
            show: "all",
        });
    });

    it("an unknown tab falls back rather than rendering an empty box with no way out", () => {
        expect(searchToProjectionParams("tab=runway", dflt).tab).toBe("cash");
    });

    it("the readings plan 28 replaced fall back to the runway, so an old link still opens", () => {
        expect(searchToProjectionParams("tab=net", dflt).tab).toBe("cash");
        expect(searchToProjectionParams("tab=worth", dflt).tab).toBe("cash");
    });

    it("the strip is the runway, the statements in the Reports tab's order under its codes, then accounts and journal", () => {
        expect(PROJECTION_TABS).toEqual(["cash", "bs", "is", "cf", "accounts", "journal"]);
        expect(PROJECTION_TABS.map((tab) => PROJECTION_TAB_LABELS[tab])).toEqual([
            "Runway",
            "Balance sheet",
            "Income statement",
            "Cash flow",
            "Accounts",
            "Journal",
        ]);
    });

    it("reads the Accounts tab's two toggles", () => {
        // An `accounts=` from before the per-section readings is ignored.
        expect(searchToProjectionParams("tab=accounts&accounts=balance&empty=1", dflt)).toEqual({...dflt, tab: "accounts", empty: true});
        expect(searchToProjectionParams("tab=journal", dflt).tab).toBe("journal");
    });

    it("clamps lookback into [0, MAX_COUNT], and zero is a real answer", () => {
        expect(searchToProjectionParams("lookback=0", dflt).lookback).toBe(0);
        expect(searchToProjectionParams("lookback=-2", dflt).lookback).toBe(dflt.lookback);
        expect(searchToProjectionParams(`lookback=${MAX_COUNT + 9}`, dflt).lookback).toBe(MAX_COUNT);
        expect(searchToProjectionParams("lookback=lots", dflt).lookback).toBe(dflt.lookback);
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
        const search = projectionParamsToSearch(dflt);
        expect([...new URLSearchParams(search).keys()].sort()).toEqual(["count", "depth", "empty", "interval", "lookback", "show", "tab"]);
    });
});
