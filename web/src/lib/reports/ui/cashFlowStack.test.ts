// The Cash Flow chart draws exactly the table: these tests hold the model to
// the rows `ReportTable` renders (`compressPeriodRows`), figure for figure.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {decodePeriodReport} from "$lib/api/nativeDecode";
import {dec, maAdd, maIsZero, maNeg, type MixedAmount} from "$lib/domain/money";
import {DEFAULT_PALETTE, SLOT_COUNT} from "$lib/format/palette";
import {amountIn} from "$lib/projections/projectionView";
import type {PeriodReport, PeriodRow} from "../types";
import {cashFlowSegments, cashFlowStack, type FlowSegment} from "./cashFlowStack";
import {compressPeriodRows} from "./displayRows";

/** `{$: 12.34}`-style amounts from whole dollars, exact. */
const usd = (n: number): MixedAmount => (n === 0 ? new Map() : new Map([["$", dec(Math.round(n * 100), 2)]]));
const eur = (n: number): MixedAmount => (n === 0 ? new Map() : new Map([["EUR", dec(Math.round(n * 100), 2)]]));
const both = (dollars: number, euros: number): MixedAmount => maAdd(usd(dollars), eur(euros));

const row = (account: string, values: MixedAmount[]): PeriodRow => ({account, depth: account.split(":").length, values});
const dollars = (account: string, values: number[]): PeriodRow => row(account, values.map(usd));

/** The sample journal's own cash flow (fixtures/native/v1, depth 2, $ and EUR). */
const SAMPLE = decodePeriodReport(JSON.parse(readFileSync(new URL("../../../../../fixtures/native/v1/cashflow.json", import.meta.url), "utf8")));

/**
 * A sample-like deeper report: a parent with its own postings (`assets:bank`),
 * a single-child chain the table collapses (`assets:broker:cash`), an account
 * that changes sign, and a second commodity.
 */
const DEEP: PeriodReport = {
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

/** Two roots, and a root that has its own postings beside a child. */
const ROOTS: PeriodReport = {
    buckets: ["2026-Q1", "2026-Q2"],
    rows: [dollars("assets", [100, -40]), dollars("assets:checking", [70, -40]), dollars("cash", [-5, 5])],
    totals: [usd(95), usd(-35)],
};

const minus = (a: MixedAmount, b: MixedAmount): MixedAmount => maAdd(a, maNeg(b));
const total = (amounts: readonly MixedAmount[]): MixedAmount => amounts.reduce((acc, a) => maAdd(acc, a), new Map() as MixedAmount);

/** The segments that make up `account`'s table figure: its own, and every descendant's. */
const under = (segments: readonly FlowSegment[], account: string): FlowSegment[] =>
    segments.filter((s) => s.account === account || s.account.startsWith(`${account}:`));

/**
 * The agreement contract, asserted exactly on `MixedAmount`s and then on the
 * numbers actually drawn:
 *
 *   - per bucket, the segments sum to the table's Net;
 *   - every displayed row's value is its own segment (a leaf), or the sum of
 *     its descendants' segments plus its own-postings segment (a parent);
 *   - what is drawn per bucket sums to the net line, and the net line is the
 *     table's Net in the charted commodity.
 */
function assertAgreement(report: PeriodReport): void {
    const display = compressPeriodRows(report.rows);
    const segments = cashFlowSegments(display);

    report.buckets.forEach((_, b) => {
        expect(maIsZero(minus(total(segments.map((s) => s.values[b])), report.totals[b])), `bucket ${b} sums to Net`).toBe(true);
    });

    for (const shown of display) {
        const parts = under(segments, shown.row.account);
        const leaf = !display.some((other) => other.row.account.startsWith(`${shown.row.account}:`));
        if (leaf) {
            expect(parts.map((s) => [s.key, s.own])).toEqual([[shown.row.account, false]]);
            expect(parts[0].values).toBe(shown.row.values);
        }
        report.buckets.forEach((_, b) => {
            expect(maIsZero(minus(total(parts.map((s) => s.values[b])), shown.row.values[b])), `${shown.row.account} bucket ${b}`).toBe(true);
        });
    }

    const stack = cashFlowStack(report, DEFAULT_PALETTE, "$");
    report.buckets.forEach((_, b) => {
        expect(stack.net[b]).toBe(amountIn(report.totals[b], stack.commodity));
        const drawn = stack.series.reduce((sum, s) => sum + (s.values[b] ?? 0), 0);
        expect(drawn).toBeCloseTo(stack.net[b], 9);
        // Every table figure, in the charted commodity, is the sum of the drawn
        // entities under it.
        for (const shown of display) {
            const keys = new Set(under(segments, shown.row.account).map((s) => s.key));
            const charted = stack.entities.filter((e) => keys.has(e.key)).reduce((sum, e) => sum + e.values[b], 0);
            expect(charted).toBeCloseTo(amountIn(shown.row.values[b], stack.commodity), 9);
        }
    });
}

describe("UNIT cashFlowStack — the chart agrees with the table", () => {
    it.each([
        ["the sample journal's cash flow", SAMPLE],
        ["a deeper report with own postings, a collapsed chain and a second commodity", DEEP],
        ["two roots, one with its own postings", ROOTS],
    ])("%s", (_, report) => {
        assertAgreement(report);
    });

    it("stacks the sample's displayed leaves under the table's labels", () => {
        const stack = cashFlowStack(SAMPLE, DEFAULT_PALETTE, "$");
        expect(stack.commodity).toBe("$");
        expect(stack.omitted).toEqual(["EUR"]);
        expect(stack.entities.map((e) => [e.key, e.label])).toEqual([
            ["assets:bank", "bank"],
            ["assets:broker", "broker"],
        ]);
    });
});

describe("UNIT cashFlowSegments — the displayed tree's parts", () => {
    it("draws a parent's own postings as a segment labelled like the parent, at its place", () => {
        const segments = cashFlowSegments(compressPeriodRows(DEEP.rows));
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

    it("uses the collapsed chain's label, as the table does", () => {
        const labels = compressPeriodRows(DEEP.rows).map((d) => d.label);
        expect(labels).toContain("broker:cash");
        expect(cashFlowSegments(compressPeriodRows(DEEP.rows)).map((s) => s.label)).toContain("broker:cash");
    });

    it("adds no own segment where the children explain the parent", () => {
        const segments = cashFlowSegments(compressPeriodRows(DEEP.rows));
        expect(segments.some((s) => s.key === "assets (own)")).toBe(false);
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
        expect(cashFlowSegments(compressPeriodRows(rows)).map((s) => s.label)).toEqual(["assets:bank:cash", "other", "assets:broker:cash", "float"]);
    });
});

describe("UNIT cashFlowStack — accounts, colours, commodity", () => {
    /** Eleven cash accounts, more than the palette has slots. */
    const WIDE: PeriodReport = (() => {
        const leaves = Array.from({length: 11}, (_, i) => dollars(`assets:a${String(i).padStart(2, "0")}`, [i + 1, -(i + 1)]));
        const sum = (b: number) => leaves.reduce((acc, r) => acc + (b === 0 ? 1 : -1) * (Number(r.account.slice(-2)) + 1), 0);
        return {buckets: ["2026-01", "2026-02"], rows: [dollars("assets", [sum(0), sum(1)]), ...leaves], totals: [usd(sum(0)), usd(sum(1))]};
    })();

    it("folds nothing: every table account past the palette is still its own entity", () => {
        const stack = cashFlowStack(WIDE, DEFAULT_PALETTE, "$");
        expect(stack.entities.map((e) => e.label)).toEqual(
            compressPeriodRows(WIDE.rows)
                .slice(1)
                .map((d) => d.label)
        );
        assertAgreement(WIDE);
    });

    it("colours the first SLOT_COUNT in table order and the rest `other`, never cycling", () => {
        const colors = cashFlowStack(WIDE, DEFAULT_PALETTE, "$").entities.map((e) => e.color);
        expect(colors.slice(0, SLOT_COUNT)).toEqual(DEFAULT_PALETTE.categorical);
        expect(colors.slice(SLOT_COUNT)).toEqual(Array(11 - SLOT_COUNT).fill(DEFAULT_PALETTE.other));
    });

    it("keeps legend order and colour per account whatever the amounts", () => {
        const flipped: PeriodReport = {...DEEP, rows: DEEP.rows.map((r) => ({...r, values: r.values.map(maNeg)})), totals: DEEP.totals.map(maNeg)};
        const colour = (report: PeriodReport) => cashFlowStack(report, DEFAULT_PALETTE, "$").entities.map((e) => [e.key, e.color]);
        expect(colour(flipped)).toEqual(colour(DEEP));
    });

    it("leaves out a segment with nothing in the charted commodity, and names that commodity", () => {
        const stack = cashFlowStack(DEEP, DEFAULT_PALETTE, "$");
        expect(stack.omitted).toEqual(["EUR"]);
        expect(stack.entities.map((e) => e.key)).not.toContain("assets:bank:wise");
    });

    it("splits an account that changes sign into halves of one entity", () => {
        const stack = cashFlowStack(DEEP, DEFAULT_PALETTE, "$");
        expect(stack.series.filter((s) => s.entity === "assets:bank:savings").map((s) => s.values)).toEqual([
            [0, 30, 10],
            [-30, 0, 0],
        ]);
    });
});
