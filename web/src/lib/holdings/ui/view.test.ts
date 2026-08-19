import {describe, expect, it} from "vitest";
import {dec, formatDec, type Dec, type MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import type {Holding, OtherHolding} from "$lib/holdings/types";
import {
    EM_DASH,
    formatGainPct,
    formatHeldCommodities,
    formatShares,
    formatUnitsWith,
    partitionShortPositions,
    PIE_OTHER,
    pieSlices,
    shortPositionNote,
    sortHoldings,
    sortOtherHoldings,
    untotaledBasisCount,
    type OtherSortKey,
    type SortKey,
} from "./view";

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
        it("keeps the quantity's own precision and trims trailing zeros", () => {
            expect(formatShares(dec(195000n, 4))).toBe("19.5"); // 19.5000
            expect(formatShares(dec(170n, 1))).toBe("17"); // 17.0
            expect(formatShares(dec(45n, 1))).toBe("4.5");
            expect(formatShares(dec(123456n, 2))).toBe("1,234.56");
        });

        // FE-6: a share count is not money. Under the 2-place MONEY cap these
        // read "0" and "1" next to a real dollar market value.
        it("does not round fractional units to cents", () => {
            expect(formatShares(dec(123456n, 8))).toBe("0.00123456"); // 0.00123456 BTC, was "0"
            expect(formatShares(dec(100123456n, 8))).toBe("1.00123456"); // was "1"
            expect(formatShares(dec(19999n, 3))).toBe("19.999"); // 19.999 units are not 20 units
        });

        it("caps at 8 places, rounding half away from zero, and keeps a sub-satoshi dust row readable", () => {
            expect(formatShares(dec(1999999995n, 10))).toBe("0.2"); // 0.1999999995 → 0.20000000 → trimmed
            expect(formatShares(dec(123456789n, 10))).toBe("0.01234568"); // 0.0123456789 rounds up at place 8
            expect(formatShares(dec(1n, 12))).toBe("0"); // below 1e-8: the short zero beats a row of zeros
        });
    });

    describe("formatGainPct", () => {
        it("formats with explicit sign and one decimal, em-dash for null", () => {
            expect(formatGainPct(21.256)).toBe("+21.3%");
            expect(formatGainPct(null)).toBe(EM_DASH);
        });

        // DRY-6: this was a second implementation of the insights dashboard's
        // `fmtSignedPct`, and the two disagreed on both of these. It now IS
        // that function, so these pin the single canonical rendering.
        it("uses the typographic minus U+2212, not an ASCII hyphen", () => {
            expect(formatGainPct(-3.44)).toBe("−3.4%");
            expect(formatGainPct(-3.44)).not.toBe("-3.4%");
        });

        it("leaves zero unsigned — '+0.0%' claimed a gain that did not happen", () => {
            expect(formatGainPct(0)).toBe("0.0%");
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

    describe("partitionShortPositions", () => {
        it("hides only the net-negative rows, preserving engine order in both halves", () => {
            const vti = holding("VTI", 5282, {shares: dec(17n, 0)});
            const tsla = holding("TSLA", -630, {shares: dec(-2n, 0)});
            const gld = holding("GLD", null, {shares: dec(5n, 0)});
            const {shown, hidden} = partitionShortPositions([vti, tsla, gld]);
            expect(shown.map((h) => h.symbol)).toEqual(["VTI", "GLD"]);
            expect(hidden.map((h) => h.symbol)).toEqual(["TSLA"]);
        });

        it("compares the exact mantissa, so a sub-unit short is still short", () => {
            // −0.5 sh: a float-free sign test (0.5 rounds to 0 under a 2dp display cap).
            const {shown, hidden} = partitionShortPositions([holding("FRC", -12, {shares: dec(-5n, 1)})]);
            expect(shown).toEqual([]);
            expect(hidden.map((h) => h.symbol)).toEqual(["FRC"]);
        });

        it("keeps everything when nothing is short", () => {
            const rows = [holding("VTI", 100, {shares: dec(1n, 0)}), holding("AAPL", 50, {shares: dec(2n, 0)})];
            expect(partitionShortPositions(rows).shown).toHaveLength(2);
            expect(partitionShortPositions(rows).hidden).toEqual([]);
            expect(partitionShortPositions([]).hidden).toEqual([]);
        });
    });

    describe("shortPositionNote", () => {
        it("is null when nothing is hidden, so the note never renders", () => {
            expect(shortPositionNote([], fmt)).toBeNull();
        });

        it("names the symbol and the exact value the totals still carry (singular)", () => {
            const note = shortPositionNote([holding("TSLA", -630, {shares: dec(-2n, 0)})], fmt);
            expect(note).toBe(
                "1 short position is hidden (TSLA): net shares are negative, so the opening purchase was likely never recorded. " +
                    "Its market value ($-630.00) is still counted in the totals above."
            );
        });

        it("pluralizes and sums exactly across several shorts", () => {
            const note = shortPositionNote([holding("TSLA", -630, {shares: dec(-2n, 0)}), holding("SHT", -70, {shares: dec(-1n, 0)})], fmt);
            expect(note).toContain("2 short positions are hidden (TSLA, SHT)");
            expect(note).toContain("the opening purchases were likely never recorded");
            expect(note).toContain("Their market value ($-700.00) is still counted in the totals above.");
        });

        it("drops the value clause when no hidden row is priced (it contributes nothing either way)", () => {
            const note = shortPositionNote([holding("SHT", null, {shares: dec(-1n, 0)})], fmt);
            expect(note).toContain("1 short position is hidden (SHT)");
            expect(note).toContain("No price is known for it, so it adds nothing to the totals.");
            expect(note).not.toContain("market value");
        });

        it("counts only the priced shorts toward the stated value", () => {
            const note = shortPositionNote([holding("TSLA", -630, {shares: dec(-2n, 0)}), holding("SHT", null, {shares: dec(-1n, 0)})], fmt);
            expect(note).toContain("Their market value ($-630.00) is still counted in the totals above.");
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

// --- Other holdings (plans/14) ---------------------------------------------

/** Units as a journal writes them: `1 HOUSE` (symbol on the right, spaced, no grouping). */
const UNIT_STYLE: AmountStyle = {side: "R", spaced: true, precision: 0, decimalPoint: ".", digitGroups: null};
const formatUnits = formatUnitsWith(() => UNIT_STYLE);

const mixed = (entries: [string, Dec][]): MixedAmount => new Map(entries);

/** Other holding with a value in whole dollars; `overrides` fills whichever other fields a test sorts on. */
function other(account: string, valueDollars: number | null, overrides: Partial<OtherHolding> = {}): OtherHolding {
    return {
        account,
        name: account.split(":").pop() ?? account,
        commodities: mixed([["$", dec(0n, 2)]]),
        value: valueDollars === null ? null : dec(BigInt(valueDollars) * 100n, 2),
        cost: null,
        change: null,
        changePct: null,
        ...overrides,
    };
}

describe("UNIT other-holdings view helpers", () => {
    describe("formatHeldCommodities", () => {
        it("prints a non-base commodity as written", () => {
            expect(formatHeldCommodities(mixed([["HOUSE", dec(1n, 0)]]), "$", formatUnits)).toBe("1 HOUSE");
        });

        it("is blank when the only commodity IS the base — the Value column already says it", () => {
            expect(formatHeldCommodities(mixed([["$", dec(1800000n, 2)]]), "$", formatUnits)).toBe("");
        });

        it("is blank for an empty amount, not the string 'undefined'", () => {
            expect(formatHeldCommodities(mixed([]), "$", formatUnits)).toBe("");
        });

        it("prints the base alongside a real unit rather than hiding half the balance", () => {
            const held = mixed([
                ["HOUSE", dec(1n, 0)],
                ["$", dec(500n, 2)],
            ]);
            expect(formatHeldCommodities(held, "$", formatUnits)).toBe("1 HOUSE, 5 $");
        });

        it("follows the report's base, not a hardcoded dollar", () => {
            expect(formatHeldCommodities(mixed([["€", dec(100n, 2)]]), "€", formatUnits)).toBe("");
            expect(formatHeldCommodities(mixed([["€", dec(100n, 2)]]), "$", formatUnits)).toBe("1 €");
        });
    });

    describe("formatUnitsWith", () => {
        it("formats past the 2-place money cap — a unit count is not cents", () => {
            const precise = formatUnitsWith(() => ({...UNIT_STYLE, precision: 8}));
            // Under MAX_DISPLAY_DECIMALS this read "0 BTC" beside a real dollar value.
            expect(precise("BTC", dec(123456n, 8))).toBe("0.00123456 BTC");
        });

        it("asks the caller for a style per commodity", () => {
            const asked: string[] = [];
            const format = formatUnitsWith((commodity) => {
                asked.push(commodity);
                return UNIT_STYLE;
            });
            format("HOUSE", dec(1n, 0));
            expect(asked).toEqual(["HOUSE"]);
        });
    });

    describe("sortOtherHoldings", () => {
        const accounts = (rows: OtherHolding[], key: OtherSortKey, dir: "asc" | "desc"): string[] => sortOtherHoldings(rows, key, dir).map((h) => h.account);

        it("sorts Dec columns exactly, in both directions", () => {
            const rows = [other("a:one", 10), other("a:two", 30), other("a:three", 20)];
            expect(accounts(rows, "value", "desc")).toEqual(["a:two", "a:three", "a:one"]);
            expect(accounts(rows, "value", "asc")).toEqual(["a:one", "a:three", "a:two"]);
        });

        it("keeps nulls last in BOTH directions, null ties broken by account asc", () => {
            const rows = [other("z:nul", null), other("a:aaa", 10), other("b:nul", null), other("c:bbb", 20)];
            expect(accounts(rows, "value", "asc")).toEqual(["a:aaa", "c:bbb", "b:nul", "z:nul"]);
            expect(accounts(rows, "value", "desc")).toEqual(["c:bbb", "a:aaa", "b:nul", "z:nul"]);
        });

        it("compares name and account case-insensitively", () => {
            const rows = [other("z:c", null, {name: "apple"}), other("m:b", null, {name: "Banana"}), other("a:a", null, {name: "CHERRY"})];
            expect(accounts(rows, "name", "asc")).toEqual(["z:c", "m:b", "a:a"]);
            expect(accounts(rows, "account", "asc")).toEqual(["a:a", "m:b", "z:c"]);
        });

        it("sorts changePct numerically", () => {
            const rows = [other("a", null, {changePct: -3.5}), other("b", null, {changePct: 12}), other("c", null, {changePct: 2})];
            expect(accounts(rows, "changePct", "desc")).toEqual(["b", "c", "a"]);
        });

        it("breaks equal keys by ACCOUNT — two assets may share a name: tag — and never mutates the input", () => {
            const rows = [other("b:home", 10, {name: "Home"}), other("a:home", 10, {name: "Home"}), other("c:home", 10, {name: "Home"})];
            expect(accounts(rows, "value", "desc")).toEqual(["a:home", "b:home", "c:home"]);
            expect(rows.map((h) => h.account)).toEqual(["b:home", "a:home", "c:home"]); // input untouched
        });
    });
});
