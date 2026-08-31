import {describe, expect, it} from "vitest";
import {activeBudgetPreset, budgetParamsToSearch, budgetPresetRange, budgetSpan, defaultBudgetParams, searchToBudgetParams, type BudgetParams} from "./params";

const NOW = "2026-07-21";

describe("UNIT budget/params — presets", () => {
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

    it("opens on year to date", () => {
        expect(defaultBudgetParams(NOW)).toEqual({from: "2026-01-01", to: "2026-07-21", depth: 3});
    });
});

describe("UNIT budgetSpan — the bars' real span", () => {
    // The engine walks whole months back from `end`, so a mid-month `from` makes
    // the first bucket start on the 1st. Measured against a live engine: a bar
    // for from=2026-01-15 reported $720.00 while the journal link, filtered to
    // 2026-01-15, showed $20.00. The link now uses this span instead.
    it("snaps `from` to the first of its month, because the first bucket does", () => {
        expect(budgetSpan("2026-01-15", "2026-01-31")).toEqual({from: "2026-01-01", to: "2026-01-31", count: 1});
    });

    it("leaves an already-aligned from untouched", () => {
        expect(budgetSpan("2026-01-01", "2026-07-25")).toEqual({from: "2026-01-01", to: "2026-07-25", count: 7});
    });

    it("keeps `to` exactly as asked — the engine truncates the last bucket at `end`", () => {
        // Verified against the engine: end=2026-07-25 with a 2026-07-28 txn in the
        // journal reported $30.00, not $530.00. Only the START drifts.
        expect(budgetSpan("2026-07-01", "2026-07-25").to).toBe("2026-07-25");
    });
});

describe("UNIT budget/params — URL codec", () => {
    const DFLT: BudgetParams = defaultBudgetParams(NOW);

    it("round-trips every field", () => {
        const params: BudgetParams = {from: "2025-01-01", to: "2025-06-30", depth: 5};
        expect(searchToBudgetParams(budgetParamsToSearch(params), DFLT)).toEqual(params);
    });

    it("writes the full range, so a shared link survives the default moving on", () => {
        // The default is today-based. A link that leaned on it would show a
        // different budget tomorrow than the one whoever sent it was looking at.
        expect(budgetParamsToSearch(DFLT)).toBe("from=2026-01-01&to=2026-07-21&depth=3");
    });

    it("falls back to defaults for absent params (leading ? tolerated)", () => {
        expect(searchToBudgetParams("?from=2025-01-01", DFLT)).toEqual({...DFLT, from: "2025-01-01"});
        expect(searchToBudgetParams("", DFLT)).toEqual(DFLT);
    });

    it("ignores malformed values rather than erroring", () => {
        expect(searchToBudgetParams("from=07/08/2026&to=&depth=-3", DFLT)).toEqual(DFLT);
    });

    it("clamps depth to a sane range", () => {
        expect(searchToBudgetParams("depth=0", DFLT).depth).toBe(1);
        expect(searchToBudgetParams("depth=500", DFLT).depth).toBe(99);
    });

    it("accepts the params a ?tab=budget bookmark carried over", () => {
        // The reports page forwards `/reports?tab=budget&from=…&to=…&depth=…`
        // here with `tab` dropped, so the very query it sends must decode.
        expect(searchToBudgetParams("from=2026-01-01&to=2026-12-31&depth=3", DFLT)).toEqual({from: "2026-01-01", to: "2026-12-31", depth: 3});
    });
});
