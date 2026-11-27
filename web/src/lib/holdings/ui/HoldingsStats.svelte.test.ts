// The stat tiles: a windowed Gain % says it is money-weighted (so it is not
// mistaken for the time-weighted Performance line); the all-time figure, a gain
// over cost, does not.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {dec, type Dec} from "#lib/domain/money.js";
import type {HoldingsReport} from "#lib/holdings/types.js";
import HoldingsStats from "./HoldingsStats.svelte";

const format = (qty: Dec): string => `$${(Number(qty.m) / 10 ** qty.p).toFixed(2)}`;

const totals: HoldingsReport["totals"] = {
    marketValue: dec(110000n, 2),
    basis: dec(100000n, 2),
    gain: dec(10000n, 2),
    gainPct: 10,
};

describe("COMPONENT HoldingsStats", () => {
    it("marks a windowed Gain % as money-weighted, and explains it", () => {
        render(HoldingsStats, {totals, holdings: [], format, gainPeriod: "12mo"});
        const note = screen.getByTestId("gain-pct-note");
        expect(note.textContent).toBe("money-weighted");
        expect(note.parentElement?.getAttribute("data-tip")).toContain("time-weighted");
    });

    it("leaves the all-time unrealized gain % unmarked", () => {
        render(HoldingsStats, {totals, holdings: [], format, gainPeriod: "all"});
        expect(screen.queryByTestId("gain-pct-note")).toBeNull();
    });
});
