// The period table's Total column (plans/24), mounted.
//
// Cash flow carries a Total per row and a grand total, and the table prints
// them as a last column; with ONE bucket (Interval = Total) that column would
// repeat the only one beside it, so the table prints just one, headed "Total".
// Net worth carries no totals and its table must be exactly what it was.

import {render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import type {PeriodReport} from "#lib/reports/types.js";
import ReportTable from "./ReportTable.svelte";

const STYLES: ReadonlyMap<string, AmountStyle> = new Map<string, AmountStyle>([
    ["$", {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}],
]);
const usd = (cents: number): MixedAmount => new Map([["$", dec(cents, 2)]]);

const CASH_FLOW: PeriodReport = {
    buckets: ["2026-06", "2026-07"],
    rows: [
        {account: "assets", depth: 1, values: [usd(-5000), usd(10050)], total: usd(5050)},
        {account: "assets:bank", depth: 2, values: [usd(-5000), usd(10050)], total: usd(5050)},
    ],
    totals: [usd(-5000), usd(10050)],
    grandTotal: usd(5050),
};

/** The header row's figure-column labels. */
const headers = (): string[] => [...document.querySelectorAll("thead td")].map((td) => td.textContent?.trim() ?? "");
/** One body row's figure cells, by its first cell's text. */
const cells = (label: string): string[] => {
    const tr = [...document.querySelectorAll("tbody tr")].find((row) => row.querySelector("th")?.textContent?.trim() === label);
    return [...(tr?.querySelectorAll("td") ?? [])].map((td) => td.textContent?.trim() ?? "");
};

afterEach(() => {
    keymap.reset();
});

describe("COMPONENT ReportTable — period Total column", () => {
    it("prints a Total column after the buckets when the report carries totals", () => {
        render(ReportTable, {report: CASH_FLOW, styles: STYLES});
        expect(headers()).toEqual(["Jun 2026", "Jul 2026", "Total"]);
        expect(cells("assets:bank")).toEqual(["$-50.00", "$100.50", "$50.50"]);
        expect(cells("Net")).toEqual(["$-50.00", "$100.50", "$50.50"]);
    });

    it("prints a single bucket as one column headed Total", () => {
        const single: PeriodReport = {
            buckets: ["2025-08-01..2026-07-08"],
            rows: [{account: "assets", depth: 1, values: [usd(5050)], total: usd(5050)}],
            totals: [usd(5050)],
            grandTotal: usd(5050),
        };
        render(ReportTable, {report: single, styles: STYLES});
        expect(headers()).toEqual(["Total"]);
        expect(cells("assets")).toEqual(["$50.50"]);
        expect(cells("Net")).toEqual(["$50.50"]);
        expect(screen.queryByText("2025-08-01..2026-07-08")).toBeNull();
    });

    it("leaves net worth, which carries no totals, without a Total column", () => {
        const worth: PeriodReport = {
            buckets: ["2025", "2026"],
            rows: [{account: "assets", depth: 1, values: [usd(100), usd(200)], kind: "asset"}],
            totals: [usd(100), usd(200)],
        };
        render(ReportTable, {report: worth, styles: STYLES});
        expect(headers()).toEqual(["2025", "2026"]);
        expect(cells("Net")).toEqual(["$1.00", "$2.00"]);
    });
});
