import {describe, expect, it} from "vitest";
import {dec, toNumber} from "../domain/money";
import {DEFAULT_PALETTE, OTHER_LABEL} from "../format/palette";

const SLOTS = DEFAULT_PALETTE.categorical.length;
import {availableDimensions, categorySlices, UNCLASSIFIED_LABEL, type CategorySlice} from "./categories";
import type {Breakdown, CategoryWeight, HoldingsProfiles, SymbolProfile} from "./profileTypes";
import type {Holding} from "./types";

function holding(symbol: string, dollars: number | null): Holding {
    return {
        symbol,
        name: symbol,
        accounts: [],
        shares: dec(1n, 0),
        basis: null,
        firstBasisDate: null,
        price: null,
        marketValue: dollars === null ? null : dec(BigInt(Math.round(dollars * 100)), 2),
        gain: null,
        gainPct: null,
    };
}

const EMPTY: Breakdown = {assetClass: [], sector: [], industry: [], securityType: [], category: [], risk: []};

function profile(symbol: string, breakdown: Partial<Record<keyof Breakdown, CategoryWeight[]>>): SymbolProfile {
    return {symbol, breakdown: {...EMPTY, ...breakdown}};
}

function profiles(...list: SymbolProfile[]): HoldingsProfiles {
    return {yahoo: "ok", profiles: new Map(list.map((p) => [p.symbol, p]))};
}

const w = (label: string, weight: number): CategoryWeight => ({label, weight});

/** label → value rounded to cents, in slice order. */
const summary = (slices: CategorySlice[]): [string, number][] => slices.map((s) => [s.label, Math.round(s.value * 100) / 100]);

const total = (slices: CategorySlice[]): number => slices.reduce((sum, s) => sum + s.value, 0);

describe("UNIT categorySlices", () => {
    it("splits a fund proportionally and the parts sum to the holding's value", () => {
        const slices = categorySlices(
            [holding("BAL", 1000)],
            profiles(profile("BAL", {sector: [w("Technology", 0.3), w("Healthcare", 0.3), w("Bonds", 0.4)]})),
            "sector",
            SLOTS
        );
        expect(summary(slices)).toEqual([
            ["Bonds", 400],
            ["Healthcare", 300],
            ["Technology", 300],
        ]);
        expect(total(slices)).toBeCloseTo(1000, 9);
        expect(slices.map((s) => Math.round(s.share * 1e6) / 1e6)).toEqual([40, 30, 30]);
    });

    it("sums the same category across holdings", () => {
        const slices = categorySlices(
            [holding("AAPL", 600), holding("VTI", 400)],
            profiles(profile("AAPL", {sector: [w("Technology", 1)]}), profile("VTI", {sector: [w("Technology", 0.5), w("Energy", 0.5)]})),
            "sector",
            SLOTS
        );
        expect(summary(slices)).toEqual([
            ["Technology", 800],
            ["Energy", 200],
        ]);
        expect(slices[0].holdings).toBe(2);
    });

    it("puts unknown value in a distinct, last, slot-less unclassified slice", () => {
        const slices = categorySlices(
            [holding("AAPL", 100), holding("PRIVATE", 300), holding("HALF", 200)],
            profiles(profile("AAPL", {industry: [w("Consumer Electronics", 1)]}), profile("HALF", {industry: [w("Chips", 0.5)]})),
            "industry",
            SLOTS
        );
        // Equal values tie-break by label.
        expect(summary(slices)).toEqual([
            ["Chips", 100],
            ["Consumer Electronics", 100],
            [UNCLASSIFIED_LABEL, 400],
        ]);
        const last = slices[slices.length - 1];
        expect(last.kind).toBe("unclassified");
        expect(last.slot).toBeNull();
        expect(last.holdings).toBe(2);
        expect(total(slices)).toBeCloseTo(600, 9);
    });

    it("treats no profiles at all as one unclassified disc", () => {
        const slices = categorySlices([holding("A", 10), holding("B", 20)], null, "sector", SLOTS);
        expect(summary(slices)).toEqual([[UNCLASSIFIED_LABEL, 30]]);
        expect(slices[0].share).toBe(100);
    });

    it("ignores float noise in weights that should sum to one", () => {
        const slices = categorySlices([holding("A", 100)], profiles(profile("A", {sector: [w("X", 0.3333333), w("Y", 0.6666667)]})), "sector", SLOTS);
        expect(slices.map((s) => s.kind)).toEqual(["named", "named"]);
    });

    it("caps weights that overshoot one rather than inventing negative unclassified value", () => {
        const slices = categorySlices([holding("A", 100)], profiles(profile("A", {sector: [w("X", 0.7), w("Y", 0.5)]})), "sector", SLOTS);
        expect(slices.some((s) => s.kind === "unclassified")).toBe(false);
    });

    it("skips unpriced and non-positive holdings, as the by-holding pie does", () => {
        const slices = categorySlices(
            [holding("A", null), holding("B", -50), holding("C", 0), holding("D", 10)],
            profiles(profile("D", {sector: [w("X", 1)]})),
            "sector",
            SLOTS
        );
        expect(summary(slices)).toEqual([["X", 10]]);
    });

    it("merges labels that differ only in case, keeping the first spelling", () => {
        const slices = categorySlices(
            [holding("A", 10), holding("B", 5)],
            profiles(profile("A", {assetClass: [w("real estate", 1)]}), profile("B", {assetClass: [w("Real Estate", 1)]})),
            "assetClass",
            SLOTS
        );
        expect(summary(slices)).toEqual([["real estate", 15]]);
    });

    it("folds categories past the eighth into (other), never a ninth colour", () => {
        const sectors = Array.from({length: 10}, (_, i) => w(`S${i}`, 0.1));
        const slices = categorySlices([holding("A", 1000)], profiles(profile("A", {sector: sectors})), "sector", 8);
        expect(slices).toHaveLength(9);
        const other = slices[8];
        expect(other.kind).toBe("other");
        expect(other.label).toBe(OTHER_LABEL);
        expect(other.value).toBeCloseTo(200, 9);
        expect(other.slot).toBeNull();
        expect(new Set(slices.slice(0, 8).map((s) => s.slot))).toEqual(new Set([0, 1, 2, 3, 4, 5, 6, 7]));
    });

    it("orders the fold before unclassified, and both after the named slices", () => {
        const sectors = Array.from({length: 3}, (_, i) => w(`S${i}`, 0.2));
        const slices = categorySlices([holding("A", 100)], profiles(profile("A", {sector: sectors})), "sector", 2);
        expect(slices.map((s) => s.kind)).toEqual(["named", "named", "other", "unclassified"]);
    });

    it("pins asset classes to fixed colour slots whatever their rank", () => {
        const slices = categorySlices(
            [holding("BND", 900), holding("VTI", 100)],
            profiles(profile("BND", {assetClass: [w("Bonds", 1)]}), profile("VTI", {assetClass: [w("Equity", 1)]})),
            "assetClass",
            SLOTS
        );
        expect(slices.map((s) => [s.label, s.slot])).toEqual([
            ["Bonds", 1],
            ["Equity", 0],
        ]);
        // An unpinned class takes the lowest slot no class is pinned to, so it
        // never borrows the Bonds colour just because there are no bonds.
        const withRealEstate = categorySlices(
            [holding("R", 50), holding("VTI", 10)],
            profiles(profile("R", {assetClass: [w("Real estate", 1)]}), profile("VTI", {assetClass: [w("Equity", 1)]})),
            "assetClass",
            SLOTS
        );
        expect(withRealEstate.map((s) => [s.label, s.slot])).toEqual([
            ["Real estate", 5],
            ["Equity", 0],
        ]);
    });

    it("lends absent pinned slots out only once the unpinned ones run out", () => {
        // Five slots are pinned to asset classes this holding has none of, so
        // SLOTS - 5 unpinned ones are free; two more classes than that borrow
        // the first two pinned slots.
        const free = SLOTS - 5;
        const labels = Array.from({length: free + 2}, (_, i) => String.fromCharCode(65 + i));
        const classes = labels.map((label) => w(label, 1 / labels.length));
        const slices = categorySlices([holding("X", 100)], profiles(profile("X", {assetClass: classes})), "assetClass", SLOTS);
        expect(slices.map((s) => s.slot)).toEqual([...Array.from({length: free}, (_, i) => 5 + i), 0, 1]);
    });

    it("ranks open-ended dimensions: the largest slice takes slot 0", () => {
        const slices = categorySlices(
            [holding("A", 10), holding("B", 90)],
            profiles(profile("A", {sector: [w("Energy", 1)]}), profile("B", {sector: [w("Technology", 1)]})),
            "sector",
            SLOTS
        );
        expect(slices.map((s) => [s.label, s.slot])).toEqual([
            ["Technology", 0],
            ["Energy", 1],
        ]);
    });

    it("gives every slice a unique key, even a category named like a bucket", () => {
        const slices = categorySlices([holding("A", 10), holding("B", 10)], profiles(profile("A", {category: [w(OTHER_LABEL, 1)]})), "category", SLOTS);
        const keys = slices.map((s) => s.key);
        expect(new Set(keys).size).toBe(keys.length);
    });

    it("carries a cents-rounded Dec for formatting", () => {
        const [slice] = categorySlices([holding("A", 123.456)], profiles(profile("A", {sector: [w("X", 1)]})), "sector", SLOTS);
        expect(toNumber(slice.amount)).toBe(123.46);
    });

    it("is empty for no chartable holdings", () => {
        expect(categorySlices([], null, "sector", SLOTS)).toEqual([]);
    });
});

describe("UNIT availableDimensions", () => {
    it("offers only dimensions some shown holding has data for, in canonical order", () => {
        const data = profiles(profile("A", {sector: [w("X", 1)], risk: [w("High", 1)]}), profile("B", {assetClass: [w("Equity", 1)]}));
        expect(availableDimensions([holding("A", 1), holding("B", 1)], data)).toEqual(["assetClass", "sector", "risk"]);
    });

    it("ignores profiles for holdings not shown", () => {
        const data = profiles(profile("HIDDEN", {sector: [w("X", 1)]}));
        expect(availableDimensions([holding("A", 1)], data)).toEqual([]);
    });

    it("offers nothing before profiles have loaded", () => {
        expect(availableDimensions([holding("A", 1)], null)).toEqual([]);
    });
});
