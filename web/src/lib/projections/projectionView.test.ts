import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {PeriodReport} from "$lib/reports/types";
import {
    amountIn,
    assetContributions,
    assetRowsByGroup,
    balanceNumbers,
    chartCommodity,
    flowSeries,
    journalBalanceFor,
    otherCommodities,
    periodsPhrase,
    runwayStatement,
    seriesOf,
} from "./projectionView";
import type {AssetRow, Projection} from "./types";

const money = (entries: [string, number][]): MixedAmount => new Map(entries.map(([c, n]) => [c, dec(n, 0)]));
const usd = (n: number): MixedAmount => money([["$", n]]);

/**
 * A net-income report in CASH-FLOW ORIENTATION, depth-clamped: `expenses` and
 * `income` are the depth-1 rows and their children repeat the same money one
 * level down, which is why summing every row would double it.
 */
const NET_INCOME: PeriodReport = {
    buckets: ["2026-08", "2026-09"],
    rows: [
        {account: "expenses", depth: 1, values: [usd(-4200), usd(-4200)]},
        {account: "expenses:rent", depth: 2, values: [usd(-4200), usd(-4200)]},
        {account: "income", depth: 1, values: [usd(9000), usd(9000)]},
        {account: "income:salary", depth: 2, values: [usd(9000), usd(9000)]},
    ],
    totals: [usd(4800), usd(4800)],
};

function projection(overrides: Partial<Projection> = {}): Projection {
    return {
        buckets: ["2026-08", "2026-09"],
        start: "2026-08-01",
        netIncome: NET_INCOME,
        cash: {opening: usd(10000), values: [usd(14800), usd(19600)]},
        netWorth: {opening: usd(50000), values: [usd(54800), usd(59600)]},
        runway: null,
        assets: [],
        warnings: [],
        ...overrides,
    };
}

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

describe("UNIT projectionView — the net-worth growth breakdown", () => {
    it("lists the biggest contributor first, and keeps a row that grew nothing", () => {
        const p = projection({
            assets: [assetRow("a", "assets:savings", 1000, 0), assetRow("b", "assets:brokerage", 500000, 35000), assetRow("c", "assets:house", 640000, 19200)],
        });
        expect(assetContributions(p, "$")).toEqual([
            {account: "assets:brokerage", opening: 500000, growth: 35000},
            {account: "assets:house", opening: 640000, growth: 19200},
            // Kept, not filtered: a $0 line is the answer to "why did nothing
            // happen", and dropping it sends the reader back to the table.
            {account: "assets:savings", opening: 1000, growth: 0},
        ]);
    });

    it("SUMS two rules that name one account, rather than listing it twice", () => {
        const p = projection({assets: [assetRow("a", "assets:brokerage", 1000, 70), assetRow("b", "assets:brokerage", 1000, 30)]});
        expect(assetContributions(p, "$")).toEqual([{account: "assets:brokerage", opening: 2000, growth: 100}]);
    });

    it("reports the balance the row COMPOUNDED, which an override replaces", () => {
        const p = projection({assets: [assetRow("h", "assets:house", 512300, 19200, 640000)]});
        expect(assetContributions(p, "$")[0].opening).toBe(640000);
    });

    it("is empty for a scenario with no asset rows", () => {
        expect(assetContributions(projection(), "$")).toEqual([]);
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
        const empty = projection({
            netIncome: {buckets: [], rows: [], totals: []},
            cash: {opening: new Map(), values: []},
            netWorth: {opening: new Map(), values: []},
        });
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

describe("UNIT projectionView — the flow chart's three columns", () => {
    it("splits the DEPTH-1 rows by sign, into magnitudes", () => {
        // Depth-1 only: summing every row would count the rent twice, once under
        // `expenses`. `PeriodFlowChart` owns the sign, so both come back positive.
        expect(flowSeries(NET_INCOME, "$")).toEqual({inflows: [9000, 9000], outflows: [4200, 4200], net: [4800, 4800]});
    });

    it("does NOT flip the sign again — revenue already arrives positive", () => {
        const {inflows, outflows} = flowSeries(NET_INCOME, "$");
        expect(inflows.every((n) => n > 0)).toBe(true);
        expect(outflows.every((n) => n > 0)).toBe(true);
    });

    it("takes `net` from the engine's totals rather than recomputing it from the two columns", () => {
        // A report whose total includes something in neither column must chart
        // the total it states, not the difference this function can see.
        const odd: PeriodReport = {...NET_INCOME, totals: [usd(1), usd(2)]};
        expect(flowSeries(odd, "$").net).toEqual([1, 2]);
    });

    it("an empty report charts nothing rather than throwing", () => {
        expect(flowSeries({buckets: [], rows: [], totals: []}, "$")).toEqual({inflows: [], outflows: [], net: []});
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
