import {describe, expect, it} from "vitest";
import {dec, formatDec, type Dec} from "$lib/domain/money";
import type {Holding} from "$lib/holdings/types";
import {amt, txn, usd} from "$lib/holdings/test-helpers";
import {EM_DASH, formatGainPct, formatShares, PIE_OTHER, pieSlices, sortHoldings, stockAccounts, untotaledBasisCount, type SortKey} from "./view";

/** Priced holding with marketValue in whole dollars; `overrides` fills whichever other fields a test sorts on. */
function holding(symbol: string, marketValueDollars: number | null, overrides: Partial<Holding> = {}): Holding {
    const marketValue = marketValueDollars === null ? null : dec(BigInt(marketValueDollars) * 100n, 2);
    return {
        symbol,
        name: `${symbol} Inc.`,
        accounts: [],
        shares: dec(1n, 0),
        basis: null,
        firstBasisDate: null,
        price: null,
        marketValue,
        gain: null,
        gainPct: null,
        ...overrides,
    };
}

const fmt = (v: Dec): string => `$${formatDec(v, {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null})}`;

describe("UNIT holdings view helpers", () => {
    describe("stockAccounts", () => {
        it("returns sorted accounts that ever hold a non-currency commodity", () => {
            const txns = [
                txn(1, "2025-01-01", [
                    {account: "assets:broker:aapl", amounts: [amt("AAPL", 100n, 1)]},
                    {account: "assets:bank:checking", amounts: [usd(-100000n)]},
                ]),
                txn(2, "2025-02-01", [
                    {account: "assets:broker:vti", amounts: [amt("VTI", 50n, 1)]},
                    {account: "assets:bank:checking", amounts: [usd(-50000n)]},
                ]),
            ];
            expect(stockAccounts(txns)).toEqual(["assets:broker:aapl", "assets:broker:vti"]);
        });

        it("dedupes accounts and ignores currency-only postings (EUR is a currency)", () => {
            const txns = [
                txn(1, "2025-01-01", [{account: "assets:broker", amounts: [amt("AAPL", 10n, 0)]}]),
                txn(2, "2025-06-01", [{account: "assets:broker", amounts: [amt("AAPL", -10n, 0)]}]),
                txn(3, "2025-07-01", [{account: "assets:cash", amounts: [amt("EUR", 100n, 0)]}]),
            ];
            expect(stockAccounts(txns)).toEqual(["assets:broker"]);
        });
    });

    describe("pieSlices", () => {
        it("keeps one named slice per priced holding with % shares summing to 100", () => {
            const slices = pieSlices([holding("AAPL", 75), holding("VTI", 25)], fmt);
            expect(slices.map((s) => s.symbol)).toEqual(["AAPL", "VTI"]);
            expect(slices.map((s) => s.share)).toEqual([75, 25]);
            expect(slices[0].formatted).toBe("$75.00");
        });

        it("excludes unpriced holdings entirely", () => {
            const slices = pieSlices([holding("AAPL", 100), holding("GLD", null)], fmt);
            expect(slices.map((s) => s.symbol)).toEqual(["AAPL"]);
            expect(slices[0].share).toBe(100);
        });

        it("folds the tail beyond maxNamed into one PIE_OTHER bucket that sums the tail exactly", () => {
            const holdings = [10, 9, 8, 7].map((v, i) => holding(`S${i}`, v));
            const slices = pieSlices(holdings, fmt, 2);
            expect(slices.map((s) => s.symbol)).toEqual(["S0", "S1", PIE_OTHER]);
            expect(slices[2].value).toBe(15);
            expect(slices[2].formatted).toBe("$15.00");
            expect(slices.reduce((acc, s) => acc + s.share, 0)).toBeCloseTo(100);
        });

        it("returns no slices when nothing is priced", () => {
            expect(pieSlices([holding("GLD", null)], fmt)).toEqual([]);
        });
    });

    describe("formatShares", () => {
        it("caps display at 2 decimals and trims trailing zeros", () => {
            expect(formatShares(dec(195000n, 4))).toBe("19.5"); // 19.5000
            expect(formatShares(dec(170n, 1))).toBe("17"); // 17.0
            expect(formatShares(dec(45n, 1))).toBe("4.5");
            expect(formatShares(dec(123456n, 2))).toBe("1,234.56");
        });

        it("rounds (half away from zero) rather than truncating", () => {
            expect(formatShares(dec(19999n, 3))).toBe("20"); // 19.999 → 20.00 → 20
        });
    });

    describe("formatGainPct", () => {
        it("formats with explicit sign and one decimal, em-dash for null", () => {
            expect(formatGainPct(21.256)).toBe("+21.3%");
            expect(formatGainPct(-3.44)).toBe("-3.4%");
            expect(formatGainPct(0)).toBe("+0.0%");
            expect(formatGainPct(null)).toBe(EM_DASH);
        });
    });

    describe("untotaledBasisCount", () => {
        it("counts displayed holdings with a null (no recorded) basis, 0 when all known", () => {
            const known = holding("VTI", 100, {basis: dec(2000n, 0)});
            const tainted = holding("GLD", 180); // factory default basis is null
            const unpricedTainted = holding("SLV", null); // unpriced AND null basis
            expect(untotaledBasisCount([known, tainted])).toBe(1);
            expect(untotaledBasisCount([known, holding("AAPL", 50, {basis: dec(500n, 0)})])).toBe(0);
            expect(untotaledBasisCount([tainted, unpricedTainted])).toBe(2);
            expect(untotaledBasisCount([])).toBe(0);
        });
    });

    describe("sortHoldings", () => {
        const symbols = (holdings: readonly Holding[], key: SortKey, dir: "asc" | "desc"): string[] => sortHoldings(holdings, key, dir).map((h) => h.symbol);

        it("sorts Dec columns exactly via cmp across mixed precisions", () => {
            // 10.00 vs 9.5 vs 100: numeric order, not string/mantissa order.
            const rows = [
                holding("AAA", null, {basis: dec(1000n, 2)}),
                holding("BBB", null, {basis: dec(95n, 1)}),
                holding("CCC", null, {basis: dec(100n, 0)}),
            ];
            expect(symbols(rows, "basis", "asc")).toEqual(["BBB", "AAA", "CCC"]);
            expect(symbols(rows, "basis", "desc")).toEqual(["CCC", "AAA", "BBB"]);
        });

        it("keeps nulls last in BOTH directions, null ties broken by symbol asc", () => {
            const rows = [holding("NUL2", null), holding("AAA", 10), holding("NUL1", null), holding("BBB", 20)];
            expect(symbols(rows, "marketValue", "asc")).toEqual(["AAA", "BBB", "NUL1", "NUL2"]);
            expect(symbols(rows, "marketValue", "desc")).toEqual(["BBB", "AAA", "NUL1", "NUL2"]);
        });

        it("compares name and symbol case-insensitively", () => {
            const rows = [holding("ZZZ", null, {name: "apple"}), holding("MMM", null, {name: "Banana"}), holding("AAA", null, {name: "CHERRY"})];
            expect(symbols(rows, "name", "asc")).toEqual(["ZZZ", "MMM", "AAA"]);
            expect(symbols(rows, "name", "desc")).toEqual(["AAA", "MMM", "ZZZ"]);
        });

        it("sorts gainPct numerically and ISO dates lexically (chronological)", () => {
            const rows = [
                holding("AAA", null, {gainPct: -3.5, firstBasisDate: "2025-06-01"}),
                holding("BBB", null, {gainPct: 12, firstBasisDate: "2024-12-31"}),
                holding("CCC", null, {gainPct: 2, firstBasisDate: "2025-01-02"}),
            ];
            expect(symbols(rows, "gainPct", "desc")).toEqual(["BBB", "CCC", "AAA"]);
            expect(symbols(rows, "firstBasisDate", "asc")).toEqual(["BBB", "CCC", "AAA"]);
        });

        it("reads price and priceDate from the nested price field, null when unpriced", () => {
            const rows = [
                holding("AAA", null, {price: {qty: dec(500n, 2), date: "2025-03-01", source: "directive"}}),
                holding("BBB", null, {price: {qty: dec(100n, 2), date: "2025-04-01", source: "cost"}}),
                holding("CCC", null),
            ];
            expect(symbols(rows, "price", "desc")).toEqual(["AAA", "BBB", "CCC"]);
            expect(symbols(rows, "priceDate", "asc")).toEqual(["AAA", "BBB", "CCC"]);
        });

        it("breaks equal keys by symbol asc and never mutates the input", () => {
            const rows = [holding("BBB", 10), holding("AAA", 10), holding("CCC", 10)];
            expect(symbols(rows, "marketValue", "desc")).toEqual(["AAA", "BBB", "CCC"]);
            expect(rows.map((h) => h.symbol)).toEqual(["BBB", "AAA", "CCC"]); // input untouched
        });
    });
});
