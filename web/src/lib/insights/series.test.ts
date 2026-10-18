// Display helpers for the insights charts. The chart DATA is the engine's now
// (crates/ledgeline-core/src/view/series.rs, pinned by
// tests/journal_view_golden.rs against goldens captured from the functions that
// used to live in series.ts); what is left here formats its numbers.

import {describe, expect, it} from "vitest";
import type {AmountStyle} from "#lib/domain/types.js";
import {formatChartValue, formatCompactChartValue} from "./series";

const USD_STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const EUR_STYLE: AmountStyle = {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]};

describe("UNIT formatChartValue", () => {
    it("renders a chart number back in the commodity's style", () => {
        expect(formatChartValue(1234.5, "$", USD_STYLE)).toBe("$1,234.50");
        expect(formatChartValue(-2_500_000, "EUR", EUR_STYLE)).toBe("-2.500.000,00 EUR");
    });

    it("falls back to plain text out of the exact range", () => {
        expect(formatChartValue(1e300, "$", USD_STYLE)).toBe("1e+300 $");
    });
});

describe("UNIT formatCompactChartValue", () => {
    it("abbreviates by magnitude with ~1 decimal (K/M/B/T)", () => {
        expect(formatCompactChartValue(1234, "$", USD_STYLE)).toBe("$1.2K");
        expect(formatCompactChartValue(5_269_875, "$", USD_STYLE)).toBe("$5.3M");
        expect(formatCompactChartValue(3_400_000, "$", USD_STYLE)).toBe("$3.4M");
        expect(formatCompactChartValue(1_000_000_000, "$", USD_STYLE)).toBe("$1.0B");
        expect(formatCompactChartValue(2_500_000_000_000, "$", USD_STYLE)).toBe("$2.5T");
    });

    it("zero and sub-thousand amounts render plainly, no suffix", () => {
        expect(formatCompactChartValue(0, "$", USD_STYLE)).toBe("$0");
        expect(formatCompactChartValue(500, "$", USD_STYLE)).toBe("$500");
        expect(formatCompactChartValue(999, "$", USD_STYLE)).toBe("$999");
        expect(formatCompactChartValue(1000, "$", USD_STYLE)).toBe("$1.0K");
    });

    it("promotes at unit boundaries instead of printing 1000.0K", () => {
        expect(formatCompactChartValue(999_999, "$", USD_STYLE)).toBe("$1.0M");
        expect(formatCompactChartValue(999_999_999, "$", USD_STYLE)).toBe("$1.0B");
    });

    it("keeps sign and commodity placement consistent with formatAmount", () => {
        expect(formatCompactChartValue(-1234, "$", USD_STYLE)).toBe("$-1.2K");
        const eurStyle = EUR_STYLE; // side R, spaced, comma decimal
        expect(formatCompactChartValue(1234, "EUR", eurStyle)).toBe("1,2K EUR");
        expect(formatCompactChartValue(-2_500_000, "EUR", eurStyle)).toBe("-2,5M EUR");
        expect(formatCompactChartValue(0, "EUR", eurStyle)).toBe("0 EUR");
    });
});
