// Top gainers / losers, mounted — for plan 47's click-to-filter: a symbol is a
// button that charts only that holding, and the chosen ones are highlighted.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {dec, type Dec} from "#lib/domain/money.js";
import type {Holding, HoldingsReport} from "#lib/holdings/types.js";
import GainersLosers from "./GainersLosers.svelte";

const format = (qty: Dec): string => `$${(Number(qty.m) / 10 ** qty.p).toFixed(2)}`;

function holding(symbol: string, gainPct: number): Holding {
    return {
        symbol,
        name: `${symbol} Inc.`,
        accounts: ["assets:broker"],
        shares: dec(1n, 0),
        basis: dec(10000n, 2),
        firstBasisDate: null,
        price: null,
        marketValue: dec(10000n + BigInt(gainPct * 100), 2),
        gain: dec(BigInt(gainPct * 100), 2),
        gainPct,
    };
}

const UP = holding("VTI", 12);
const DOWN = holding("AAPL", -4);

const REPORT: HoldingsReport = {
    asOf: "2026-11-26",
    base: "$",
    holdings: [UP, DOWN],
    accounts: [],
    totals: {marketValue: dec(0n, 0), basis: null, gain: null, gainPct: null},
    topGainers: [UP],
    topLosers: [DOWN],
    warnings: [],
    liveAt: null,
};

describe("COMPONENT GainersLosers click-to-filter (plan 47 D7)", () => {
    it("a symbol charts only that holding", async () => {
        const onSelect = vi.fn();
        render(GainersLosers, {report: REPORT, format, onSelect});
        await fireEvent.click(screen.getByTestId("top-losers-AAPL"));

        expect(onSelect).toHaveBeenCalledWith("AAPL");
    });

    it("marks the chosen symbols, pressed and highlighted", () => {
        render(GainersLosers, {report: REPORT, format, onSelect: vi.fn(), selected: new Set(["VTI"])});

        expect(screen.getByTestId("top-gainers-VTI").getAttribute("aria-pressed")).toBe("true");
        expect(screen.getByTestId("top-losers-AAPL").getAttribute("aria-pressed")).toBe("false");
        expect(screen.getByTestId("top-gainers-VTI").closest("li")?.getAttribute("data-selected")).toBe("true");
    });
});
