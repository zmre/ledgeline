// The Net Worth and Cash Flow chart panels, mounted over literal reports.
//
// Structure only (jsdom draws into a 0×0 box): the panel shell, the persisted
// collapse flag, the legend the model produced — exactly the table's displayed
// accounts — and the friendly empty and one-bucket states.

import {fireEvent, render} from "@testing-library/svelte";
import {afterEach, describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {CASH_FLOW, CASH_FLOW_TOTAL, flow, usd} from "#lib/testing/cashFlowFixture.js";
import type {CashFlowStatement, PeriodReport, PeriodRow, PeriodRowKind} from "../types";
import CashFlowChart from "./CashFlowChart.svelte";
import {compressPeriodRows} from "./displayRows";
import NetWorthChart from "./NetWorthChart.svelte";
import {periodsOf} from "#lib/reports/test-helpers.js";

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
    periods: periodsOf(BUCKETS),
    rows: [
        row("assets", [600, 610, 650, 700, 720], "asset"),
        row("assets:bank", [100, 90, 120, 150, 160], "asset"),
        row("assets:home", [500, 520, 530, 550, 560], "asset"),
        row("liabilities", [-400, -390, -380, -370, -360], "liability"),
        row("liabilities:mortgage", [-400, -390, -380, -370, -360], "liability"),
    ],
    totals: [200, 220, 270, 330, 360].map(usd),
};

/** The table's displayed rows with no displayed child, by the table's own function, as labelled there. */
function displayedLeaves(report: PeriodReport): string[] {
    const display = compressPeriodRows(report.rows);
    return display.filter((d) => !display.some((other) => other.row.account.startsWith(`${d.row.account}:`))).map((d) => d.label);
}

const legend = (testid: string): string[] => [...document.querySelectorAll(`[data-testid="${testid}-legend"] li`)].map((li) => li.textContent?.trim() ?? "");

afterEach(() => {
    settings.netWorthChartOpen = true;
    settings.cashFlowChartOpen = true;
});

describe("COMPONENT NetWorthChart", () => {
    it("stacks exactly the table's displayed leaves, assets then liabilities, under the table's labels", () => {
        render(NetWorthChart, {report: NET_WORTH, styles: STYLES});

        // The table shows assets › bank, home and the collapsed
        // liabilities:mortgage chain; no parent has postings of its own.
        expect(displayedLeaves(NET_WORTH)).toEqual(["bank", "home", "liabilities:mortgage"]);
        expect(legend("networth-chart")).toEqual([...displayedLeaves(NET_WORTH), "Net worth"]);
    });

    it("folds nothing: a parent's own postings are their own entry, named like the parent", () => {
        const own: PeriodReport = {
            ...NET_WORTH,
            rows: NET_WORTH.rows.map((r) => (r.account === "assets" ? row("assets", [650, 660, 700, 750, 770], "asset") : r)),
        };
        render(NetWorthChart, {report: {...own, totals: [250, 270, 320, 380, 410].map(usd)}, styles: STYLES});

        expect(legend("networth-chart")).toEqual(["assets", "bank", "home", "liabilities:mortgage", "Net worth"]);
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
        const empty: PeriodReport = {buckets: BUCKETS, periods: periodsOf(BUCKETS), rows: [], totals: BUCKETS.map(() => new Map())};
        render(NetWorthChart, {report: empty, styles: STYLES});

        expect(document.querySelector('[data-testid="networth-chart-empty"]')?.textContent).toContain("No assets or liabilities");
    });

    it("refuses a single bucket", () => {
        const one: PeriodReport = {buckets: ["2026"], periods: periodsOf(["2026"]), rows: [row("assets:bank", [5], "asset")], totals: [usd(5)]};
        render(NetWorthChart, {report: one, styles: STYLES});

        expect(document.querySelector('[data-testid="networth-chart-empty"]')?.textContent).toContain("Only one period");
    });

    it("names the commodities it could not chart", () => {
        const mixed: PeriodReport = {
            buckets: ["2025", "2026"],
            periods: periodsOf(["2025", "2026"]),
            rows: [row("assets:bank", [5, 6], "asset"), {account: "assets:wise", depth: 2, values: [eur(1), eur(2)], kind: "asset"}],
            totals: [new Map([...usd(5), ...eur(1)]), new Map([...usd(6), ...eur(2)])],
        };
        render(NetWorthChart, {report: mixed, styles: STYLES});

        expect(document.querySelector('[data-testid="networth-chart-omitted"]')?.textContent).toContain("EUR not shown");
    });
});

describe("COMPONENT CashFlowChart", () => {
    it("stacks one series per activity, with the net change in cash as the line", () => {
        render(CashFlowChart, {report: CASH_FLOW, styles: STYLES});

        expect(legend("cashflow-chart")).toEqual(["Operating", "Investing", "Financing", "Net change in cash"]);
    });

    it("adds the effect of exchange rates as its own series when there is one", () => {
        const withFx: CashFlowStatement = {...CASH_FLOW, exchange: flow([0, 10, 0])};
        render(CashFlowChart, {report: withFx, styles: STYLES});

        expect(legend("cashflow-chart")).toContain("Exchange rates");
    });

    it("has no breakdown toggle", () => {
        const {container} = render(CashFlowChart, {report: CASH_FLOW, styles: STYLES});

        expect(container.querySelector('[role="radiogroup"]')).toBeNull();
        expect(container.querySelector('[data-testid^="cashflow-chart-mode"]')).toBeNull();
    });

    it("draws bars, not areas", () => {
        const {container} = render(CashFlowChart, {report: CASH_FLOW, styles: STYLES});

        expect(container.querySelectorAll("path.lc-area-path")).toHaveLength(0);
    });

    it("names the commodities it could not chart", () => {
        const mixed: CashFlowStatement = {
            ...CASH_FLOW,
            sections: [{...CASH_FLOW.sections[0], total: {values: [usd(5), eur(1), usd(6)], total: usd(11)}}],
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
        const still: CashFlowStatement = {...CASH_FLOW, sections: [], netChange: flow([0, 0, 0])};
        render(CashFlowChart, {report: still, styles: STYLES});

        expect(document.querySelector('[data-testid="cashflow-chart-empty"]')?.textContent).toContain("No cash moved");
    });

    it("asks for a wider range on Interval = Total, whose one column is no trend", () => {
        render(CashFlowChart, {report: CASH_FLOW_TOTAL, styles: STYLES});

        expect(document.querySelector('[data-testid="cashflow-chart-empty"]')?.textContent).toContain("Only one period");
    });
});
