import {describe, expect, it} from "vitest";
import type {Amount, AmountStyle} from "./types";
import {add, cmp, dec, formatAmount, formatDec, isZero, maAdd, maIsZero, maNeg, mul, neg, sub, toNumber, type Dec, type MixedAmount} from "./money";

const style = (overrides: Partial<AmountStyle> = {}): AmountStyle => ({
    side: "L",
    spaced: false,
    precision: 2,
    decimalPoint: ".",
    digitGroups: null,
    ...overrides,
});

describe("UNIT money", () => {
    describe("dec", () => {
        it("builds from a safe-integer number mantissa", () => {
            expect(dec(123456, 2)).toEqual({m: 123456n, p: 2});
        });

        it("builds from a bigint mantissa beyond Number.MAX_SAFE_INTEGER", () => {
            expect(dec(90071992547409920n, 4)).toEqual({m: 90071992547409920n, p: 4});
        });

        it("throws on a non-safe-integer number mantissa (never silently degrades)", () => {
            expect(() => dec(2 ** 53, 2)).toThrow(RangeError);
            expect(() => dec(1.5, 2)).toThrow(RangeError);
        });

        it("throws on invalid decimal places", () => {
            expect(() => dec(1, -1)).toThrow(RangeError);
            expect(() => dec(1, 0.5)).toThrow(RangeError);
        });
    });

    describe("add/sub alignment", () => {
        it("rescales the lower-p operand without rounding", () => {
            // 1.5 + 0.25 = 1.75 at p=2
            expect(add(dec(15, 1), dec(25, 2))).toEqual({m: 175n, p: 2});
        });

        it("is symmetric in operand order", () => {
            expect(add(dec(25, 2), dec(15, 1))).toEqual({m: 175n, p: 2});
        });

        it("round-trips: (a + b) - b compares equal to a", () => {
            const a = dec(12345, 2); // 123.45
            const b = dec(678901, 4); // 67.8901
            const roundTrip = sub(add(a, b), b);
            expect(cmp(roundTrip, a)).toBe(0);
            expect(roundTrip).toEqual({m: 1234500n, p: 4}); // exact, only rescaled
        });

        it("accumulates float-hostile values exactly (0.1 + 0.2 = 0.3)", () => {
            expect(add(dec(1, 1), dec(2, 1))).toEqual({m: 3n, p: 1});
        });

        it("handles negative operands", () => {
            expect(sub(dec(100, 2), dec(250, 2))).toEqual({m: -150n, p: 2});
        });
    });

    describe("neg", () => {
        it("negates the mantissa only", () => {
            expect(neg(dec(-42, 3))).toEqual({m: 42n, p: 3});
        });
    });

    describe("mul", () => {
        it("adds precisions exactly: 3 shares @ $228.50", () => {
            expect(mul(dec(3, 0), dec(22850, 2))).toEqual({m: 68550n, p: 2});
        });

        it("keeps full precision: 0.1 * 0.2 = 0.02 at p=2", () => {
            expect(mul(dec(1, 1), dec(2, 1))).toEqual({m: 2n, p: 2});
        });

        it("price conversion: 45.00 EUR * 1.08 $/EUR = 48.6000 at p=4", () => {
            expect(mul(dec(4500, 2), dec(108, 2))).toEqual({m: 486000n, p: 4});
        });
    });

    describe("cmp / isZero", () => {
        it("compares across different precisions", () => {
            expect(cmp(dec(15, 1), dec(1500, 3))).toBe(0);
            expect(cmp(dec(15, 1), dec(1501, 3))).toBe(-1);
            expect(cmp(dec(-1, 0), dec(-2, 0))).toBe(1);
        });

        it("isZero ignores precision", () => {
            expect(isZero(dec(0, 5))).toBe(true);
            expect(isZero(dec(1, 5))).toBe(false);
        });
    });

    describe("toNumber (display only)", () => {
        it("converts to a float", () => {
            expect(toNumber(dec(123456, 2))).toBe(1234.56);
            expect(toNumber(dec(-5, 1))).toBe(-0.5);
        });
    });

    describe("MixedAmount", () => {
        it("maAdd sums per commodity and drops zero entries", () => {
            const a: MixedAmount = new Map([
                ["$", dec(100, 2)],
                ["EUR", dec(50, 2)],
            ]);
            const b: MixedAmount = new Map([
                ["$", dec(-100, 2)],
                ["AAPL", dec(3, 0)],
            ]);
            const sum = maAdd(a, b);
            expect(sum.has("$")).toBe(false); // cancelled to zero → dropped
            expect(sum.get("EUR")).toEqual({m: 50n, p: 2});
            expect(sum.get("AAPL")).toEqual({m: 3n, p: 0});
        });

        it("maAdd does not mutate its operands", () => {
            const a: MixedAmount = new Map([["$", dec(1, 0)]]);
            const b: MixedAmount = new Map([["$", dec(2, 0)]]);
            maAdd(a, b);
            expect(a.get("$")).toEqual({m: 1n, p: 0});
            expect(b.get("$")).toEqual({m: 2n, p: 0});
        });

        it("maNeg negates every commodity", () => {
            const negated = maNeg(new Map([["$", dec(150, 2)]]));
            expect(negated.get("$")).toEqual({m: -150n, p: 2});
        });

        it("maIsZero: empty and all-zero maps are zero", () => {
            expect(maIsZero(new Map())).toBe(true);
            expect(maIsZero(new Map([["$", dec(0, 2)]]))).toBe(true);
            expect(maIsZero(new Map([["$", dec(1, 2)]]))).toBe(false);
        });
    });

    describe("formatDec", () => {
        it("pads up to the style precision", () => {
            expect(formatDec(dec(15, 1), style())).toBe("1.50"); // 1.5 shown at precision 2
            expect(formatDec(dec(5, 0), style())).toBe("5.00");
        });

        it("caps display at two decimal places regardless of style/Dec precision", () => {
            expect(formatDec(dec(195000, 4), style({precision: 4}))).toBe("19.50"); // 19.5000 shares
            expect(formatDec(dec(12345, 3), style({precision: 3}))).toBe("12.35"); // rounds, half away from zero
            expect(formatDec(dec(15, 1), style({precision: 1}))).toBe("1.5"); // lower precisions untouched
        });

        it("rounds half away from zero at the style precision", () => {
            expect(formatDec(dec(1005, 3), style())).toBe("1.01");
            expect(formatDec(dec(-1005, 3), style())).toBe("-1.01");
            expect(formatDec(dec(1004, 3), style())).toBe("1.00");
        });

        it("precision 0 renders no decimal point", () => {
            expect(formatDec(dec(1499, 3), style({precision: 0}))).toBe("1");
            expect(formatDec(dec(15, 1), style({precision: 0}))).toBe("2");
        });

        it("groups digits with the last group size repeating", () => {
            const grouped = style({digitGroups: [",", [3]]});
            expect(formatDec(dec(123456789, 2), grouped)).toBe("1,234,567.89");
            expect(formatDec(dec(-123456789, 2), grouped)).toBe("-1,234,567.89");
            expect(formatDec(dec(12345, 2), grouped)).toBe("123.45"); // no group needed
        });

        it("supports Indian-style [3, 2] grouping", () => {
            const indian = style({digitGroups: [",", [3, 2]], precision: 0});
            expect(formatDec(dec(12345678, 0), indian)).toBe("1,23,45,678");
        });

        it("formats comma-decimal with dot groups (European style)", () => {
            const eur = style({decimalPoint: ",", digitGroups: [".", [3]]});
            expect(formatDec(dec(500000, 2), eur)).toBe("5.000,00");
        });

        it("formats zero and sub-one values with a leading zero", () => {
            expect(formatDec(dec(0, 2), style())).toBe("0.00");
            expect(formatDec(dec(5, 2), style())).toBe("0.05");
        });
    });

    describe("formatAmount", () => {
        const amt = (commodity: string, qty: Dec, s: AmountStyle): Amount => ({commodity, qty, style: s});

        it("left side, unspaced: $1,234.56", () => {
            expect(formatAmount(amt("$", dec(123456, 2), style({digitGroups: [",", [3]]})))).toBe("$1,234.56");
        });

        it("left side negative keeps hledger's $-87.20 shape", () => {
            expect(formatAmount(amt("$", dec(-8720, 2), style()))).toBe("$-87.20");
        });

        it("right side, spaced, comma decimal: 45,00 EUR", () => {
            expect(formatAmount(amt("EUR", dec(4500, 2), style({side: "R", spaced: true, decimalPoint: ","})))).toBe("45,00 EUR");
        });

        it("no commodity renders the bare number", () => {
            expect(formatAmount(amt("", dec(7, 0), style({precision: 0})))).toBe("7");
        });
    });
});
