// The Net Worth and Cash Flow chart panels, mounted over literal reports.
//
// Structure only (jsdom draws into a 0×0 box): the panel shell, the persisted
// collapse flag, the legend the model produced — for Cash Flow, exactly the
// table's displayed accounts — and the friendly empty and one-bucket states.

import {fireEvent, render} from "@testing-library/svelte";
import {afterEach, describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "$lib/domain/money";
import {settings} from "$lib/stores/settings.svelte";
import type {PeriodReport, PeriodRow, PeriodRowKind} from "../types";
import CashFlowChart from "./CashFlowChart.svelte";
import {compressPeriodRows} from "./displayRows";
import NetWorthChart from "./NetWorthChart.svelte";

const usd = (n: number): MixedAmount => (n === 0 ? new Map() : new Map([["$", dec(Math.round(n * 100), 2)]]));
const eur = (n: number): MixedAmount => new Map([["EUR", dec(Math.round(n * 100), 2)]]);

function row(account: string, values: number[], kind?: PeriodRowKind): PeriodRow {
    const out: PeriodRow = {account, depth: account.split(":").length, values: values.map(usd)};
    if (kind !== undefined) out.kind = kind;
    return out;
}

const BUCKETS = ["2022", "2023", "2024", "2025", "2026"];
const STYLES = new Map();

const NET_WORTH: PeriodReport = {
    buckets: BUCKETS,
    rows: [
        row("assets", [600, 610, 650, 700, 720], "asset"),
        row("assets:bank", [100, 90, 120, 150, 160], "asset"),
        row("assets:home", [500, 520, 530, 550, 560], "asset"),
        row("liabilities", [-400, -390, -380, -370, -360], "liability"),
        row("liabilities:mortgage", [-400, -390, -380, -370, -360], "liability"),
    ],
    totals: [200, 220, 270, 330, 360].map(usd),
};

/**
 * The table shows `assets` › `bank` (with postings of its own) › `checking`,
 * `savings`, and `broker:cash` (a collapsed single-child chain).
 */
const CASH_FLOW: PeriodReport = {
    buckets: ["2026-01", "2026-02", "2026-03"],
    rows: [
        row("assets", [50, -30, 20]),
        row("assets:bank", [60, -20, 20]),
        row("assets:bank:checking", [80, -60, 10]),
        row("assets:bank:savings", [-30, 30, 10]),
        row("assets:broker", [-10, -10, 0]),
        row("assets:broker:cash", [-10, -10, 0]),
    ],
    totals: [50, -30, 20].map(usd),
};

const legend = (testid: string): string[] => [...document.querySelectorAll(`[data-testid="${testid}-legend"] li`)].map((li) => li.textContent?.trim() ?? "");

afterEach(() => {
    settings.netWorthChartOpen = true;
    settings.cashFlowChartOpen = true;
});

describe("COMPONENT NetWorthChart", () => {
    it("stacks the leaf accounts, assets then liabilities, with the net worth", () => {
        render(NetWorthChart, {report: NET_WORTH, styles: STYLES});

        expect(legend("networth-chart")).toEqual(["assets:home", "assets:bank", "liabilities:mortgage", "Net worth"]);
    });

    it("labels every net point on a five-year chart", () => {
        const {container} = render(NetWorthChart, {report: NET_WORTH, styles: STYLES});

        expect(container.querySelectorAll("g.lc-net-point")).toHaveLength(BUCKETS.length);
    });

    it("draws bars, not areas", () => {
        const {container} = render(NetWorthChart, {report: NET_WORTH, styles: STYLES});

        expect(container.querySelectorAll(".lc-bars-bar").length).toBeGreaterThan(0);
        expect(container.querySelectorAll("path.lc-area-path")).toHaveLength(0);
    });

    it("collapses, and persists that it did", async () => {
        const {container} = render(NetWorthChart, {report: NET_WORTH, styles: STYLES});
        const toggle = container.querySelector<HTMLInputElement>('[data-testid="networth-chart-panel"] input[type="checkbox"]');
        expect(toggle?.checked).toBe(true);

        await fireEvent.click(toggle!);

        expect(settings.netWorthChartOpen).toBe(false);
        expect(container.querySelector('[data-testid="networth-chart"]')).toBeNull();
        // The header stays, so it can be opened again.
        expect(container.textContent).toContain("Net worth over time");
    });

    it("says so, rather than drawing, when there is nothing to stack", () => {
        const empty: PeriodReport = {buckets: BUCKETS, rows: [], totals: BUCKETS.map(() => new Map())};
        render(NetWorthChart, {report: empty, styles: STYLES});

        expect(document.querySelector('[data-testid="networth-chart-empty"]')?.textContent).toContain("No assets or liabilities");
    });

    it("refuses a single bucket", () => {
        const one: PeriodReport = {buckets: ["2026"], rows: [row("assets:bank", [5], "asset")], totals: [usd(5)]};
        render(NetWorthChart, {report: one, styles: STYLES});

        expect(document.querySelector('[data-testid="networth-chart-empty"]')?.textContent).toContain("Only one period");
    });

    it("names the commodities it could not chart", () => {
        const mixed: PeriodReport = {
            buckets: ["2025", "2026"],
            rows: [row("assets:bank", [5, 6], "asset"), {account: "assets:wise", depth: 2, values: [eur(1), eur(2)], kind: "asset"}],
            totals: [new Map([...usd(5), ...eur(1)]), new Map([...usd(6), ...eur(2)])],
        };
        render(NetWorthChart, {report: mixed, styles: STYLES});

        expect(document.querySelector('[data-testid="networth-chart-omitted"]')?.textContent).toContain("EUR not shown");
    });
});

describe("COMPONENT CashFlowChart", () => {
    it("stacks exactly the table's displayed leaves, plus a parent's own postings, under the table's labels", () => {
        render(CashFlowChart, {report: CASH_FLOW, styles: STYLES});

        // The table's own rows, by the table's own function: every displayed
        // row with no displayed child, and `bank` again for its own postings.
        const display = compressPeriodRows(CASH_FLOW.rows);
        const leaves = display.filter((d) => !display.some((other) => other.row.account.startsWith(`${d.row.account}:`))).map((d) => d.label);
        expect(leaves).toEqual(["checking", "savings", "broker:cash"]);
        expect(legend("cashflow-chart")).toEqual(["bank", ...leaves, "Net change"]);
    });

    it("has no breakdown toggle", () => {
        const {container} = render(CashFlowChart, {report: CASH_FLOW, styles: STYLES});

        expect(container.querySelector('[role="radiogroup"]')).toBeNull();
        expect(container.querySelector('[data-testid^="cashflow-chart-mode"]')).toBeNull();
        expect(container.textContent).not.toMatch(/By (source|account)/);
    });

    it("draws areas, one per sign half", () => {
        const {container} = render(CashFlowChart, {report: CASH_FLOW, styles: STYLES});

        // bank's own +, checking ±, savings ±, broker:cash −: an account that
        // flips sign between buckets is two drawn halves but ONE legend entry.
        expect(container.querySelectorAll("path.lc-area-path")).toHaveLength(6);
    });

    it("names the commodities it could not chart", () => {
        const mixed: PeriodReport = {
            buckets: ["2026-01", "2026-02"],
            rows: [row("assets:bank", [5, 6]), {account: "assets:wise", depth: 2, values: [eur(1), eur(2)]}],
            totals: [new Map([...usd(5), ...eur(1)]), new Map([...usd(6), ...eur(2)])],
        };
        render(CashFlowChart, {report: mixed, styles: STYLES});

        expect(document.querySelector('[data-testid="cashflow-chart-omitted"]')?.textContent).toContain("EUR not shown");
    });

    it("collapses, and persists that it did", async () => {
        const {container} = render(CashFlowChart, {report: CASH_FLOW, styles: STYLES});

        await fireEvent.click(container.querySelector<HTMLInputElement>('[data-testid="cashflow-chart-panel"] input[type="checkbox"]')!);

        expect(settings.cashFlowChartOpen).toBe(false);
        expect(container.querySelector('[data-testid="cashflow-chart"]')).toBeNull();
    });

    it("says so when no cash moved", () => {
        const still: PeriodReport = {buckets: CASH_FLOW.buckets, rows: [], totals: CASH_FLOW.buckets.map(() => new Map())};
        render(CashFlowChart, {report: still, styles: STYLES});

        expect(document.querySelector('[data-testid="cashflow-chart-empty"]')?.textContent).toContain("No cash moved");
    });
});
