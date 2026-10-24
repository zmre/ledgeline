import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import {projectionFixture} from "#lib/testing/projectionFixture.js";
import {
    amountIn,
    assetRowsByGroup,
    balanceNumbers,
    chartCommodity,
    journalBalanceFor,
    otherCommodities,
    periodsPhrase,
    runwayStatement,
    seriesOf,
} from "./projectionView";
import type {AssetRow, Projection} from "./types";

const money = (entries: [string, number][]): MixedAmount => new Map(entries.map(([c, n]) => [c, dec(n, 0)]));
const usd = (n: number): MixedAmount => money([["$", n]]);

/** The shared two-plus-two projection: $10,000 of cash building $4,800 a month. */
const projection = (overrides: Partial<Projection> = {}): Projection => projectionFixture(overrides);

/** One attributed asset row, as `Projection.assets` carries it. */
const assetRow = (group: string, account: string, held: number, grew: number, compounded = held): AssetRow => ({
    group,
    account,
    journalOpening: usd(held),
    opening: usd(compounded),
    growth: usd(grew),
});

describe("UNIT projectionView — an asset row's journal balance", () => {
    const rows = assetRowsByGroup(projection({assets: [assetRow("brok", "assets:brokerage", 512300, 35861)]}));

    it("is keyed by the row's GROUP, which is what the table has on the row", () => {
        expect([...rows.keys()]).toEqual(["brok"]);
        expect(journalBalanceFor(rows, "brok", "assets:brokerage", "$")).toEqual(dec(512300, 0));
    });

    it("is a real ZERO for an account the journal holds nothing in — that is an answer", () => {
        expect(journalBalanceFor(rows, "brok", "assets:brokerage", "EUR")).toEqual({m: 0n, p: 0});
    });

    it("is NULL for a row nothing has been projected for", () => {
        // Before the first run, and for a row the engine declined to model:
        // there is no entry either way, and null is "no answer", not zero.
        expect(journalBalanceFor(new Map(), "brok", "assets:brokerage", "$")).toBeNull();
        expect(journalBalanceFor(rows, "house", "assets:house", "$")).toBeNull();
    });

    it("REFUSES an entry computed for a different account", () => {
        // The stale case, and the reason the account travels with the figure:
        // a user who has just retyped this row's account would otherwise be
        // shown the previous account's balance as if it were the ledger's
        // answer about the new one.
        expect(journalBalanceFor(rows, "brok", "assets:house", "$")).toBeNull();
        // Whitespace is not a different account, though.
        expect(journalBalanceFor(rows, "brok", "  assets:brokerage  ", "$")).toEqual(dec(512300, 0));
    });

    it("is empty for a projection that has not arrived at all", () => {
        expect(assetRowsByGroup(null).size).toBe(0);
    });
});

describe("UNIT projectionView — reading one commodity out of a mixed amount", () => {
    it("an amount that holds none of the charted commodity is zero, not a gap", () => {
        expect(amountIn(money([["EUR", 5]]), "$")).toBe(0);
        expect(seriesOf([usd(1), money([["EUR", 2]])], "$")).toEqual([1, 0]);
    });

    it("charts the commodity the projection carries the most figures in", () => {
        expect(chartCommodity(projection(), "EUR")).toBe("$");
    });

    it("falls back only when the projection names no commodity at all", () => {
        // Only the runway's cash is charted, so only it is counted.
        const empty = projection({cash: {opening: new Map(), values: []}});
        expect(chartCommodity(empty, "£")).toBe("£");
    });

    it("names what charting one commodity leaves out, rather than dropping it silently", () => {
        const mixed = projection({
            cash: {
                opening: money([
                    ["$", 10],
                    ["EUR", 3],
                ]),
                values: [usd(1), usd(2)],
            },
        });
        expect(otherCommodities(mixed, "$")).toEqual(["EUR"]);
        expect(otherCommodities(projection(), "$")).toEqual([]);
    });

    it("an opening balance is a real figure, kept beside the closing ones", () => {
        expect(balanceNumbers(projection().cash, "$")).toEqual({opening: 10000, values: [14800, 19600]});
    });
});

describe("UNIT projectionView — the runway, in words", () => {
    const format = (n: number) => `$${n.toFixed(2)}`;

    it("names the crossing bucket and how far out it is", () => {
        const crossing = projection({runway: {bucket: 1, label: "Mar 2028", date: "2028-03-31", periods: 18}});
        const statement = runwayStatement(crossing, "monthly", "$", format);
        expect(statement.tone).toBe("warn");
        expect(statement.headline).toBe("Cash turns negative in Mar 2028 — 18 months.");
        expect(statement.detail).toBe("That is the first period closing below zero, on 2028-03-31.");
    });

    it("counts in the interval's own noun", () => {
        const crossing = projection({runway: {bucket: 1, label: "2029", date: "2029-12-31", periods: 3}});
        expect(runwayStatement(crossing, "yearly", "$", format).headline).toBe("Cash turns negative in 2029 — 3 years.");
        expect(periodsPhrase(1, "quarterly")).toBe("1 quarter");
        expect(periodsPhrase(4, "quarterly")).toBe("4 quarters");
    });

    it("says so plainly when it never crosses, and gives the average build instead", () => {
        const statement = runwayStatement(projection(), "monthly", "$", format);
        expect(statement.tone).toBe("ok");
        expect(statement.headline).toBe("Cash never turns negative over the 2 months projected.");
        // (19600 - 10000) / 2 buckets.
        expect(statement.detail).toBe("It builds by $4800.00 a month on average, ending at $19600.00.");
    });

    it("a falling but still-positive balance says it is drawing down", () => {
        const falling = projection({cash: {opening: usd(10000), values: [usd(8000), usd(6000)]}});
        expect(runwayStatement(falling, "monthly", "$", format).detail).toBe("It draws down by $2000.00 a month on average, ending at $6000.00.");
    });

    it("a flat balance says it ends where it started, rather than claiming a build of zero", () => {
        const flat = projection({cash: {opening: usd(10000), values: [usd(10000), usd(10000)]}});
        expect(runwayStatement(flat, "monthly", "$", format).detail).toBe("It ends where it started, at $10000.00.");
    });

    it("an empty window makes no claim at all", () => {
        const nothing = projection({buckets: [], cash: {opening: usd(0), values: []}});
        expect(runwayStatement(nothing, "monthly", "$", format)).toEqual({tone: "none", headline: "Nothing is projected over this window.", detail: null});
    });
});
