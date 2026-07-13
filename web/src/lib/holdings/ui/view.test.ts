import {describe, expect, it} from "vitest";
import {dec, formatDec, type Dec} from "$lib/domain/money";
import type {Holding} from "$lib/holdings/types";
import {amt, txn, usd} from "$lib/holdings/test-helpers";
import {EM_DASH, formatGainPct, formatShares, PIE_OTHER, pieSlices, stockAccounts} from "./view";

/** Priced holding with marketValue in whole dollars (other fields irrelevant to the pie). */
function holding(symbol: string, marketValueDollars: number | null): Holding {
    const marketValue = marketValueDollars === null ? null : dec(BigInt(marketValueDollars) * 100n, 2);
    return {symbol, name: `${symbol} Inc.`, accounts: [], shares: dec(1n, 0), basis: null, price: null, marketValue, gain: null, gainPct: null};
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
});
