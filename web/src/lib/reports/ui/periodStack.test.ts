// The Net Worth and Cash Flow charts draw exactly their tables: these tests
// hold the model to the rows `ReportTable` renders (`compressPeriodRows`),
// figure for figure, and pin the rules for sides, colours and commodity.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {decodePeriodReport} from "$lib/api/nativeDecode";
import {dec, maAdd, maIsZero, maNeg, type MixedAmount} from "$lib/domain/money";
import {DEFAULT_PALETTE, SLOT_COUNT} from "$lib/format/palette";
import {amountIn} from "$lib/projections/projectionView";
import type {PeriodReport, PeriodRow, PeriodRowKind} from "../types";
import {compressPeriodRows} from "./displayRows";
import {periodStack, splitBySign, stackCommodity, stackSegments, type StackSegment} from "./periodStack";

/** `{$: 12.34}`-style amounts from whole dollars, exact. */
const usd = (n: number): MixedAmount => (n === 0 ? new Map() : new Map([["$", dec(Math.round(n * 100), 2)]]));
const eur = (n: number): MixedAmount => (n === 0 ? new Map() : new Map([["EUR", dec(Math.round(n * 100), 2)]]));
const both = (dollars: number, euros: number): MixedAmount => maAdd(usd(dollars), eur(euros));

function row(account: string, values: MixedAmount[], kind?: PeriodRowKind): PeriodRow {
    const out: PeriodRow = {account, depth: account.split(":").length, values};
    if (kind !== undefined) out.kind = kind;
    return out;
}
const dollars = (account: string, values: number[], kind?: PeriodRowKind): PeriodRow => row(account, values.map(usd), kind);

function report(rows: PeriodRow[], totals: number[], buckets = totals.map((_, i) => `202${i}`)): PeriodReport {
    return {buckets, rows, totals: totals.map(usd)};
}

const golden = (name: string): PeriodReport =>
    decodePeriodReport(JSON.parse(readFileSync(new URL(`../../../../../fixtures/native/v1/${name}.json`, import.meta.url), "utf8")));

/** The sample journal's own reports (fixtures/native/v1, depth 2). */
const SAMPLE_CASH_FLOW = golden("cashflow");
const SAMPLE_NET_WORTH = golden("networth");

/**
 * A sample-like deeper cash flow: a parent with its own postings
 * (`assets:bank`), a single-child chain the table collapses
 * (`assets:broker:cash`), an account that changes sign, and a second commodity.
 */
const CASH_FLOW: PeriodReport = {
    buckets: ["2026-01", "2026-02", "2026-03"],
    rows: [
        row("assets", [both(50, 20), usd(-30), usd(20)]),
        row("assets:bank", [both(60, 20), usd(-20), usd(20)]),
        dollars("assets:bank:checking", [80, -60, 10]),
        dollars("assets:bank:savings", [-30, 30, 10]),
        row("assets:bank:wise", [eur(20), new Map(), new Map()]),
        dollars("assets:broker", [-10, -10, 0]),
        dollars("assets:broker:cash", [-10, -10, 0]),
    ],
    totals: [both(50, 20), usd(-30), usd(20)],
};

/** A deeper net worth: `assets` has postings of its own, and the mortgage is a collapsed chain. */
const NET_WORTH: PeriodReport = report(
    [
        dollars("assets", [1000, 1100, 1200], "asset"),
        dollars("assets:bank", [300, 350, 400], "asset"),
        dollars("assets:bank:checking", [200, 250, 300], "asset"),
        dollars("assets:bank:savings", [100, 100, 100], "asset"),
        dollars("assets:home", [650, 700, 750], "asset"),
        dollars("liabilities", [-500, -480, -460], "liability"),
        dollars("liabilities:mortgage", [-500, -480, -460], "liability"),
    ],
    [500, 620, 740],
    ["2024", "2025", "2026"]
);

/** Two roots, one with its own postings beside a child. */
const ROOTS: PeriodReport = report([dollars("assets", [100, -40]), dollars("assets:checking", [70, -40]), dollars("cash", [-5, 5])], [95, -35]);

const minus = (a: MixedAmount, b: MixedAmount): MixedAmount => maAdd(a, maNeg(b));
const total = (amounts: readonly MixedAmount[]): MixedAmount => amounts.reduce((acc, a) => maAdd(acc, a), new Map() as MixedAmount);

/** The segments that make up `account`'s table figure: its own, and every descendant's. */
const under = (segments: readonly StackSegment[], account: string): StackSegment[] =>
    segments.filter((s) => s.account === account || s.account.startsWith(`${account}:`));

/**
 * The agreement contract, asserted exactly on `MixedAmount`s and then on the
 * numbers actually drawn:
 *
 *   - per bucket, the segments sum to the table's Net;
 *   - every displayed row's value is its own segment (a leaf), or the sum of
 *     its descendants' segments plus its own-postings segment (a parent);
 *   - what is drawn per bucket sums to the net line, the net line is the
 *     table's Net in the charted commodity, and every table figure in that
 *     commodity is the sum of the drawn entities under it.
 */
function assertAgreement(r: PeriodReport): void {
    const display = compressPeriodRows(r.rows);
    const segments = stackSegments(display);

    r.buckets.forEach((_, b) => {
        expect(maIsZero(minus(total(segments.map((s) => s.values[b])), r.totals[b])), `bucket ${b} sums to Net`).toBe(true);
    });

    for (const shown of display) {
        const parts = under(segments, shown.row.account);
        const leaf = !display.some((other) => other.row.account.startsWith(`${shown.row.account}:`));
        if (leaf) {
            expect(parts.map((s) => [s.key, s.own])).toEqual([[shown.row.account, false]]);
            expect(parts[0].values).toBe(shown.row.values);
        }
        r.buckets.forEach((_, b) => {
            expect(maIsZero(minus(total(parts.map((s) => s.values[b])), shown.row.values[b])), `${shown.row.account} bucket ${b}`).toBe(true);
        });
    }

    const stack = periodStack(r, DEFAULT_PALETTE, "$");
    r.buckets.forEach((_, b) => {
        expect(stack.net[b]).toBe(amountIn(r.totals[b], stack.commodity));
        const drawn = stack.series.reduce((sum, s) => sum + (s.values[b] ?? 0), 0);
        expect(drawn).toBeCloseTo(stack.net[b], 6);
        for (const shown of display) {
            const keys = new Set(under(segments, shown.row.account).map((s) => s.key));
            const charted = stack.entities.filter((e) => keys.has(e.key)).reduce((sum, e) => sum + e.values[b], 0);
            expect(charted).toBeCloseTo(amountIn(shown.row.values[b], stack.commodity), 6);
        }
    });
}

describe("UNIT periodStack — the chart agrees with the table", () => {
    it.each([
        ["the sample journal's cash flow", SAMPLE_CASH_FLOW],
        ["the sample journal's net worth", SAMPLE_NET_WORTH],
        ["a deeper cash flow with own postings, a collapsed chain and a second commodity", CASH_FLOW],
        ["a deeper net worth with own postings and a collapsed chain", NET_WORTH],
        ["two roots, one with its own postings", ROOTS],
    ])("%s", (_, r) => {
        assertAgreement(r);
    });

    it("stacks the sample cash flow's displayed leaves under the table's labels", () => {
        const stack = periodStack(SAMPLE_CASH_FLOW, DEFAULT_PALETTE, "$");
        expect(stack.omitted).toEqual(["EUR"]);
        expect(stack.entities.map((e) => [e.key, e.label])).toEqual([
            ["assets:bank", "bank"],
            ["assets:broker", "broker"],
        ]);
    });

    it("stacks every sample net-worth row the table shows, assets then liabilities", () => {
        const stack = periodStack(SAMPLE_NET_WORTH, DEFAULT_PALETTE, "$");
        expect(stack.entities.map((e) => [e.key, e.label, e.side])).toEqual([
            ["assets:bank", "bank", "up"],
            ["assets:broker", "broker", "up"],
            ["assets:property", "property", "up"],
            ["assets:vehicles", "vehicles", "up"],
            ["liabilities:cc", "cc", "down"],
            ["liabilities:mortgage", "mortgage", "down"],
        ]);
    });
});

describe("UNIT stackSegments — the displayed tree's parts", () => {
    it("stacks the leaves, never a parent and its child together", () => {
        const segments = stackSegments(compressPeriodRows([dollars("assets", [300]), dollars("assets:bank", [100]), dollars("assets:broker", [200])]));
        expect(segments.map((s) => s.key)).toEqual(["assets:bank", "assets:broker"]);
    });

    it("draws a parent's own postings as a segment labelled like the parent, at its place", () => {
        const segments = stackSegments(compressPeriodRows(CASH_FLOW.rows));
        expect(segments.map((s) => [s.key, s.label])).toEqual([
            ["assets:bank (own)", "bank"],
            ["assets:bank:checking", "checking"],
            ["assets:bank:savings", "savings"],
            ["assets:bank:wise", "wise"],
            ["assets:broker:cash", "broker:cash"],
        ]);
        // 60 − 80 − (−30) = 10; the EUR is all wise's.
        expect(segments[0].values).toEqual([usd(10), usd(10), usd(0)]);
    });

    it("uses a collapsed chain's label, as the table does", () => {
        expect(compressPeriodRows(CASH_FLOW.rows).map((d) => d.label)).toContain("broker:cash");
        expect(stackSegments(compressPeriodRows(CASH_FLOW.rows)).map((s) => s.label)).toContain("broker:cash");
    });

    it("adds no own segment where the children explain the parent", () => {
        expect(stackSegments(compressPeriodRows(CASH_FLOW.rows)).some((s) => s.key === "assets (own)")).toBe(false);
    });

    it("carries the engine's side onto a leaf and onto an own segment", () => {
        const segments = stackSegments(compressPeriodRows(NET_WORTH.rows));
        expect(segments.map((s) => [s.key, s.kind])).toEqual([
            ["assets (own)", "asset"],
            ["assets:bank:checking", "asset"],
            ["assets:bank:savings", "asset"],
            ["assets:home", "asset"],
            ["liabilities:mortgage", "liability"],
        ]);
    });

    it("falls back to the full account only where two segments would read alike", () => {
        const rows = [
            dollars("assets", [30]),
            dollars("assets:bank", [10]),
            dollars("assets:bank:cash", [10]),
            dollars("assets:bank:other", [0]),
            dollars("assets:broker", [20]),
            dollars("assets:broker:cash", [15]),
            dollars("assets:broker:float", [5]),
        ];
        expect(stackSegments(compressPeriodRows(rows)).map((s) => s.label)).toEqual(["assets:bank:cash", "other", "assets:broker:cash", "float"]);
    });
});

describe("UNIT periodStack — sides", () => {
    it("places net-worth rows by the engine's kind, not by sign", () => {
        const rows = [
            dollars("assets", [-50, 100], "asset"),
            dollars("assets:checking", [-50, 100], "asset"), // overdrawn in the first bucket
            dollars("liabilities", [30, -200], "liability"),
            dollars("liabilities:cc", [30, -200], "liability"), // a refund in the first bucket
        ];
        const stack = periodStack(report(rows, [-20, -100]), DEFAULT_PALETTE, "$");
        expect(stack.entities.map((e) => [e.key, e.side])).toEqual([
            ["assets:checking", "up"],
            ["liabilities:cc", "down"],
        ]);
        // ... and the SIGN still decides where each bucket's segment is drawn:
        // the overdraft is a negative asset segment, the refund a positive
        // liability one.
        expect(stack.series.map((s) => [s.key, s.values])).toEqual([
            ["assets:checking|+", [0, 100]],
            ["assets:checking|-", [-50, 0]],
            ["liabilities:cc|+", [30, 0]],
            ["liabilities:cc|-", [0, -200]],
        ]);
    });

    it("places kind-less rows by the sign of their window total, in table order", () => {
        const rows = [dollars("assets:broker", [100, -400]), dollars("expenses:rent", [-300, -300]), dollars("income:salary", [500, 500])];
        const stack = periodStack(report(rows, [300, -200]), DEFAULT_PALETTE, "$");
        expect(stack.entities.map((e) => [e.key, e.side])).toEqual([
            ["assets:broker", "down"],
            ["expenses:rent", "down"],
            ["income:salary", "up"],
        ]);
    });

    it("places a mixed row by its sign too", () => {
        const stack = periodStack(report([dollars("assets", [-10], "mixed")], [-10], ["2026"]), DEFAULT_PALETTE, "$");
        expect(stack.entities[0].side).toBe("down");
    });

    it("groups classified rows up side first, each in table order, while colours keep table order", () => {
        const rows = [dollars("bank", [100], "asset"), dollars("cc", [-30], "liability"), dollars("house", [500], "asset")];
        const stack = periodStack(report(rows, [570], ["2026"]), DEFAULT_PALETTE, "$");
        expect(stack.entities.map((e) => [e.key, e.color])).toEqual([
            ["bank", DEFAULT_PALETTE.categorical[0]],
            ["house", DEFAULT_PALETTE.categorical[2]],
            ["cc", DEFAULT_PALETTE.categorical[1]],
        ]);
    });
});

describe("UNIT periodStack — splitting by sign", () => {
    it("gives an account that changes sign two uniformly signed halves sharing its colour", () => {
        const [plus, less] = splitBySign([{key: "a", label: "a", color: "#111", side: "up", values: [5, -3, 0, 2]}]);
        expect(plus).toEqual({key: "a|+", entity: "a", label: "a", color: "#111", values: [5, 0, 0, 2]});
        expect(less).toEqual({key: "a|-", entity: "a", label: "a", color: "#111", values: [0, -3, 0, 0]});
    });

    it("draws no empty half", () => {
        expect(splitBySign([{key: "a", label: "a", color: "#111", side: "up", values: [5, 1]}]).map((s) => s.key)).toEqual(["a|+"]);
    });

    it("splits a table account that changes sign into halves of one entity", () => {
        const stack = periodStack(CASH_FLOW, DEFAULT_PALETTE, "$");
        expect(stack.series.filter((s) => s.entity === "assets:bank:savings").map((s) => s.values)).toEqual([
            [0, 30, 10],
            [-30, 0, 0],
        ]);
    });
});

describe("UNIT periodStack — every account, and its colour", () => {
    /** Eleven accounts, more than the palette has slots; assets and liabilities when `kinds`. */
    function wide(kinds: boolean): PeriodReport {
        const kindOf = (i: number): PeriodRowKind | undefined => (kinds ? (i < 7 ? "asset" : "liability") : undefined);
        const leaves = Array.from({length: 11}, (_, i) =>
            dollars(`${kindOf(i) === "liability" ? "liabilities" : "assets"}:a${String(i).padStart(2, "0")}`, [i + 1, -(i + 1)], kindOf(i))
        );
        const rootOf = (prefix: string, kind: PeriodRowKind | undefined) => {
            const kids = leaves.filter((r) => r.account.startsWith(prefix));
            const at = (b: number) => kids.reduce((s, r) => s + amountIn(r.values[b], "$"), 0);
            return dollars(prefix, [at(0), at(1)], kind);
        };
        const roots = kinds ? [rootOf("assets", "asset"), rootOf("liabilities", "liability")] : [rootOf("assets", undefined)];
        const rows = [...roots, ...leaves].sort((a, b) => (a.account < b.account ? -1 : 1));
        const net = (b: number) => roots.reduce((s, r) => s + amountIn(r.values[b], "$"), 0);
        return report(rows, [net(0), net(1)], ["2025", "2026"]);
    }

    it.each([
        ["a cash flow", false],
        ["a net worth", true],
    ])("folds nothing in %s: every table account past the palette is still its own entity", (_, kinds) => {
        const r = wide(kinds);
        const stack = periodStack(r, DEFAULT_PALETTE, "$");
        const leaves = compressPeriodRows(r.rows).filter((d) => d.indent > 0);
        expect(stack.entities.map((e) => e.label).sort()).toEqual(leaves.map((d) => d.label).sort());
        assertAgreement(r);
    });

    it.each([
        ["a cash flow", false],
        ["a net worth", true],
    ])("colours the first SLOT_COUNT in table order and the rest `other`, never cycling, in %s", (_, kinds) => {
        const r = wide(kinds);
        const byKey = new Map(periodStack(r, DEFAULT_PALETTE, "$").entities.map((e) => [e.key, e.color]));
        const inTableOrder = compressPeriodRows(r.rows)
            .filter((d) => d.indent > 0)
            .map((d) => byKey.get(d.row.account));
        expect(inTableOrder.slice(0, SLOT_COUNT)).toEqual(DEFAULT_PALETTE.categorical);
        expect(inTableOrder.slice(SLOT_COUNT)).toEqual(Array(11 - SLOT_COUNT).fill(DEFAULT_PALETTE.other));
    });

    it("keeps legend order and colour per account whatever the amounts", () => {
        const flipped: PeriodReport = {
            ...CASH_FLOW,
            rows: CASH_FLOW.rows.map((r) => ({...r, values: r.values.map(maNeg)})),
            totals: CASH_FLOW.totals.map(maNeg),
        };
        const colour = (r: PeriodReport) => periodStack(r, DEFAULT_PALETTE, "$").entities.map((e) => [e.key, e.color]);
        expect(colour(flipped)).toEqual(colour(CASH_FLOW));
    });

    it("keeps an account's colour identical in every bucket — one colour per entity, both halves", () => {
        const stack = periodStack(
            report([dollars("assets:bank", [100, -20, 300]), dollars("income:salary", [50, 50, 50])], [150, 30, 350]),
            DEFAULT_PALETTE,
            "$"
        );
        const bank = stack.series.filter((s) => s.entity === "assets:bank");
        expect(bank).toHaveLength(2);
        expect(new Set(bank.map((s) => s.color)).size).toBe(1);
    });

    it("draws nothing for an account that is zero throughout, and spends no colour on it", () => {
        const stack = periodStack(report([dollars("assets:a", [0, 0]), dollars("assets:b", [5, 5])], [5, 5]), DEFAULT_PALETTE, "$");
        expect(stack.entities.map((e) => [e.key, e.color])).toEqual([["assets:b", DEFAULT_PALETTE.categorical[0]]]);
    });
});

describe("UNIT periodStack — commodity", () => {
    it("charts the commodity carrying the most figures, and names the rest", () => {
        const rows: PeriodRow[] = [row("assets:checking", [usd(10), usd(20), usd(30)]), row("assets:wise", [eur(5), new Map(), new Map()])];
        const r: PeriodReport = {buckets: ["a", "b", "c"], rows, totals: [both(10, 5), usd(20), usd(30)]};
        expect(stackCommodity(r, "X")).toEqual({commodity: "$", omitted: ["EUR"]});
        const stack = periodStack(r, DEFAULT_PALETTE, "X");
        // wise holds only EUR: nothing to draw, so no entity — the note names EUR.
        expect(stack.entities.map((e) => e.key)).toEqual(["assets:checking"]);
        expect(stack.net).toEqual([10, 20, 30]);
    });

    it("falls back when the report holds no commodity at all", () => {
        expect(stackCommodity({buckets: ["a"], rows: [], totals: [new Map()]}, "$")).toEqual({commodity: "$", omitted: []});
    });
});

describe("UNIT periodStack — the net", () => {
    it("takes the net from the table's Net row, not from the parts", () => {
        const stack = periodStack(report([dollars("assets:a", [5, 5])], [7, 9]), DEFAULT_PALETTE, "$");
        expect(stack.net).toEqual([7, 9]);
    });
});
