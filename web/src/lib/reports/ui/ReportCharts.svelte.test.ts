// The Net Worth and Cash Flow chart panels, mounted over literal reports.
//
// Structure only (jsdom draws into a 0×0 box): the panel shell, the persisted
// collapse flag, the legend the model produced, the Cash Flow mode toggle and
// which report each mode draws, the sources' loading/error states, and the
// friendly empty and one-bucket states.

import {fireEvent, render} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {dec, type MixedAmount} from "$lib/domain/money";
import {settings} from "$lib/stores/settings.svelte";
import type {PeriodReport, PeriodRow, PeriodRowKind} from "../types";
import type {SourcesPanel} from "$lib/stores/cashFlowSources.svelte";
import CashFlowChart from "./CashFlowChart.svelte";
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

const CASH_FLOW: PeriodReport = {
    buckets: ["2026-01", "2026-02", "2026-03"],
    rows: [row("assets", [50, -30, 20]), row("assets:checking", [80, -60, 10]), row("assets:savings", [-30, 30, 10])],
    totals: [50, -30, 20].map(usd),
};

const SOURCES: PeriodReport = {
    buckets: ["2026-01", "2026-02", "2026-03"],
    rows: [
        row("expenses", [-100, -130, -130]),
        row("expenses:rent", [-100, -130, -130]),
        row("income", [150, 100, 150]),
        row("income:salary", [150, 100, 150]),
    ],
    totals: [50, -30, 20].map(usd),
};

const panel = (overrides: Partial<SourcesPanel> = {}): SourcesPanel => ({view: "data", report: SOURCES, error: null, retry: () => {}, ...overrides});

const legend = (testid: string): string[] => [...document.querySelectorAll(`[data-testid="${testid}-legend"] li`)].map((li) => li.textContent?.trim() ?? "");

afterEach(() => {
    settings.netWorthChartOpen = true;
    settings.cashFlowChartOpen = true;
    settings.cashFlowChartMode = "source";
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
    it("shows the breakdown by source by default, as areas", () => {
        const {container} = render(CashFlowChart, {report: CASH_FLOW, sources: panel(), styles: STYLES});

        expect(legend("cashflow-chart")).toEqual(["income:salary", "expenses:rent", "Net change"]);
        expect(container.querySelectorAll("path.lc-area-path").length).toBeGreaterThan(0);
        expect(container.querySelector('[data-testid="cashflow-chart-mode-source"]')?.getAttribute("aria-checked")).toBe("true");
    });

    it("switches to the table's own cash accounts, and remembers the choice", async () => {
        const {container} = render(CashFlowChart, {report: CASH_FLOW, sources: panel(), styles: STYLES});

        await fireEvent.click(container.querySelector('[data-testid="cashflow-chart-mode-account"]')!);

        expect(settings.cashFlowChartMode).toBe("account");
        // savings flips sign between buckets: it is split into two drawn halves
        // but stays ONE legend entry.
        expect(legend("cashflow-chart")).toEqual(["assets:checking", "assets:savings", "Net change"]);
        expect(container.querySelectorAll("path.lc-area-path")).toHaveLength(4);
    });

    it("needs no sources to draw By account", () => {
        settings.cashFlowChartMode = "account";
        render(CashFlowChart, {report: CASH_FLOW, sources: panel({view: "loading", report: null}), styles: STYLES});

        expect(legend("cashflow-chart")).toContain("assets:checking");
    });

    it("waits for the sources, and offers a retry when they fail", async () => {
        const retry = vi.fn();
        const loading = render(CashFlowChart, {report: CASH_FLOW, sources: panel({view: "loading", report: null}), styles: STYLES});
        expect(loading.container.querySelector('[aria-label="Loading cash-flow sources"]')).not.toBeNull();
        loading.unmount();

        const failed = render(CashFlowChart, {
            report: CASH_FLOW,
            sources: panel({view: "error", report: null, error: new Error("boom"), retry}),
            styles: STYLES,
        });
        const alert = failed.container.querySelector('[data-testid="cashflow-sources-error"]');
        expect(alert?.textContent).toContain("boom");
        await fireEvent.click(alert!.querySelector("button")!);
        expect(retry).toHaveBeenCalledOnce();
    });

    it("says so when no cash moved", () => {
        const still: PeriodReport = {buckets: CASH_FLOW.buckets, rows: [], totals: CASH_FLOW.buckets.map(() => new Map())};
        render(CashFlowChart, {report: CASH_FLOW, sources: panel({report: still}), styles: STYLES});

        expect(document.querySelector('[data-testid="cashflow-chart-empty"]')?.textContent).toContain("No cash moved");
    });
});
