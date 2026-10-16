import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import type {PeriodReport} from "#lib/reports/types.js";
import {periodTable} from "./periodTable";

const usd = (cents: number): MixedAmount => new Map([["$", dec(cents, 2)]]);

const CASH_FLOW: PeriodReport = {
    buckets: ["2026-06", "2026-07"],
    rows: [{account: "assets", depth: 1, values: [usd(-5000), usd(10050)], total: usd(5050)}],
    totals: [usd(-5000), usd(10050)],
    grandTotal: usd(5050),
};

describe("UNIT reports/ui/periodTable", () => {
    it("adds a Total column after the buckets when the report carries totals", () => {
        const table = periodTable(CASH_FLOW);
        expect(table.headers).toEqual(["Jun 2026", "Jul 2026", "Total"]);
        expect(table.cells(CASH_FLOW.rows[0])).toEqual([usd(-5000), usd(10050), usd(5050)]);
        expect(table.net).toEqual([usd(-5000), usd(10050), usd(5050)]);
    });

    it("prints a single bucket as ONE column headed Total, not the same figure twice", () => {
        const single: PeriodReport = {
            buckets: ["2025-08-01..2026-07-08"],
            rows: [{account: "assets", depth: 1, values: [usd(5050)], total: usd(5050)}],
            totals: [usd(5050)],
            grandTotal: usd(5050),
        };
        const table = periodTable(single);
        expect(table.headers).toEqual(["Total"]);
        expect(table.cells(single.rows[0])).toEqual([usd(5050)]);
        expect(table.net).toEqual([usd(5050)]);
    });

    it("leaves a report without totals — net worth — exactly as it was", () => {
        const worth: PeriodReport = {
            buckets: ["2025", "2026"],
            rows: [{account: "assets", depth: 1, values: [usd(100), usd(200)], kind: "asset"}],
            totals: [usd(100), usd(200)],
        };
        const table = periodTable(worth);
        expect(table.headers).toEqual(["2025", "2026"]);
        expect(table.cells(worth.rows[0])).toEqual([usd(100), usd(200)]);
        expect(table.net).toEqual(worth.totals);
        // One snapshot column is still a snapshot, never relabelled "Total".
        expect(periodTable({...worth, buckets: ["2026"], totals: [usd(200)]}).headers).toEqual(["2026"]);
    });
});
