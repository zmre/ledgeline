import {describe, expect, it} from "vitest";
import {isCurrency} from "./commodities";

describe("UNIT holdings/commodities isCurrency", () => {
    it.each(["USD", "EUR", "GBP", "JPY", "CHF", "$", "€", "£", "¥", "US$", "C$", "A$", "HK$", "NZ$", "S$"])("classifies %s as a currency", (commodity) => {
        expect(isCurrency(commodity)).toBe(true);
    });

    it.each(["AAPL", "VTI", "GLD", "BRK.B", "usd", "eur", "", "ZZZ"])("classifies %s as a stock (not a currency)", (commodity) => {
        expect(isCurrency(commodity)).toBe(false);
    });
});
