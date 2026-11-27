import {describe, expect, it} from "vitest";
import type {HoldingsScope} from "#lib/holdings/types.js";
import {filterParam, holdingOptions, sameMembers, sameScope, toggledSelection, type HoldingOption} from "./selection";

const scope = (overrides: Partial<HoldingsScope> = {}): HoldingsScope => ({
    accounts: new Set(["assets:broker"]),
    mode: "include",
    asOf: "2026-11-26",
    gainPeriod: "1mo",
    holdings: new Set(["VTI"]),
    ...overrides,
});

describe("UNIT holdings selection — click-to-filter (plan 47 D7)", () => {
    it("a click selects exactly what it names", () => {
        expect([...toggledSelection(new Set(), ["VTI"])]).toEqual(["VTI"]);
        expect([...toggledSelection(new Set(["AAPL"]), ["VTI"])]).toEqual(["VTI"]);
        // A category wedge: every holding with any share in it.
        expect([...toggledSelection(new Set(["AAPL"]), ["VTI", "VXUS"])].sort()).toEqual(["VTI", "VXUS"]);
    });

    it("the same click again clears the filter", () => {
        expect(toggledSelection(new Set(["VTI"]), ["VTI"]).size).toBe(0);
        expect(toggledSelection(new Set(["VXUS", "VTI"]), ["VTI", "VXUS"]).size).toBe(0);
    });

    it("compares sets by membership, not order", () => {
        expect(sameMembers(new Set(["a", "b"]), new Set(["b", "a"]))).toBe(true);
        expect(sameMembers(new Set(["a"]), new Set(["a", "b"]))).toBe(false);
    });
});

describe("UNIT holdings selection — same request", () => {
    it("is the same scope when every field matches, sets by membership", () => {
        expect(sameScope(scope(), scope({accounts: new Set(["assets:broker"]), holdings: new Set(["VTI"])}))).toBe(true);
    });

    it.each<[string, Partial<HoldingsScope>]>([
        ["asOf", {asOf: "2026-11-25"}],
        ["mode", {mode: "exclude"}],
        ["gain period", {gainPeriod: "1wk"}],
        ["accounts", {accounts: new Set()}],
        ["holdings", {holdings: new Set(["AAPL"])}],
    ])("differs when %s does", (_, change) => {
        expect(sameScope(scope(), scope(change))).toBe(false);
    });

    it("sends the filter sorted, and nothing when empty", () => {
        expect(filterParam(new Set(["VTI", "AAPL"]))).toBe("AAPL,VTI");
        expect(filterParam(new Set())).toBeUndefined();
    });
});

describe("UNIT holdings selection — the dropdown's options", () => {
    const rows: HoldingOption[] = [
        {key: "VTI", label: "VTI", detail: "Vanguard Total Stock Market"},
        {key: "AAPL", label: "AAPL", detail: "Apple Inc."},
    ];

    it("lists the report's rows in report order", () => {
        expect(holdingOptions(rows, new Set(), "").map((o) => o.key)).toEqual(["VTI", "AAPL"]);
    });

    it("keeps a selected key the report no longer lists, so it can still be unticked", () => {
        expect(holdingOptions(rows, new Set(["GONE", "AAPL"]), "").map((o) => o.key)).toEqual(["VTI", "AAPL", "GONE"]);
    });

    it("searches key and label, case-insensitively", () => {
        expect(holdingOptions(rows, new Set(), "aap").map((o) => o.key)).toEqual(["AAPL"]);
        expect(holdingOptions(rows, new Set(), "  vt ").map((o) => o.key)).toEqual(["VTI"]);
        expect(holdingOptions(rows, new Set(), "zzz")).toEqual([]);
    });
});
