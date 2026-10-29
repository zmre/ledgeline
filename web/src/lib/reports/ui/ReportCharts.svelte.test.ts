// The Net Worth and Cash Flow chart panels, mounted over literal reports.
//
// Structure only (jsdom draws into a 0×0 box): the panel shell, the persisted
// collapse flag, the legend the model produced — exactly the table's displayed
// accounts — the friendly empty and one-bucket states, and the cash flow
// activities' pinned colours, which follow a theme switch.

import {fireEvent, render} from "@testing-library/svelte";
import {flushSync} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "#lib/domain/money.js";
import {DEFAULT_PALETTE} from "#lib/format/palette.js";
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

/** A cash flow with an exchange-rate effect, so all four pinned series draw. */
const WITH_FX: CashFlowStatement = {...CASH_FLOW, exchange: flow([0, 10, 0])};

/** Each legend entry's colour key, as jsdom serialises it (`background` or `border-color`, in `rgb()`). */
function swatches(container: HTMLElement): Map<string, string> {
    return new Map(
        [...container.querySelectorAll('[data-testid="cashflow-chart-legend"] li')].map((li) => {
            const key = li.querySelector("span") as HTMLElement | null;
            return [li.textContent?.trim() ?? "", key?.style.background || key?.style.borderColor || ""];
        })
    );
}

const barFills = (container: HTMLElement): Set<string | null> =>
    new Set([...container.querySelectorAll("path.lc-bars-bar")].map((bar) => bar.getAttribute("fill")));

/** `#rrggbb` as jsdom serialises it in a style. */
const rgb = (hex: string): string => `rgb(${[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(", ")})`;

/** Let the palette's `data-theme` observer run (a microtask), then flush the re-render. */
async function themeObserved(): Promise<void> {
    await new Promise<void>((resolve) => queueMicrotask(resolve));
    flushSync();
}

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

    it("draws each activity in its statement heading's colour, not by size, and exchange rates in grey", () => {
        // Financing is the biggest activity here by far (8,000 against 5,000 and
        // 3,000): biggest-first would have handed it the first palette slot.
        const {container} = render(CashFlowChart, {report: WITH_FX, styles: STYLES});

        // The bars take daisy dark's resolved strings, oklch and all, as their fill.
        expect(barFills(container)).toEqual(
            new Set([DEFAULT_PALETTE.theme.info, DEFAULT_PALETTE.theme.secondary, DEFAULT_PALETTE.theme.accent, DEFAULT_PALETTE.other])
        );
        expect(swatches(container).get("Exchange rates")).toBe(rgb(DEFAULT_PALETTE.other));
    });

    it("recolours the bars and legend when the theme's info, secondary and accent change", async () => {
        // jsdom's CSS parser drops oklch from an inline style (a browser keeps
        // it), so the legend is checked on a theme with hex values.
        const theme = {"--color-info": "#102030", "--color-secondary": "#405060", "--color-accent": "#708090"};
        const root = document.documentElement;
        const before = root.getAttribute("data-theme");
        const {container} = render(CashFlowChart, {report: WITH_FX, styles: STYLES});
        try {
            for (const [token, value] of Object.entries(theme)) root.style.setProperty(token, value);
            root.setAttribute("data-theme", "pinned-test");
            await themeObserved();

            expect([...swatches(container)]).toEqual([
                ["Operating", rgb("#102030")],
                ["Investing", rgb("#405060")],
                ["Financing", rgb("#708090")],
                ["Exchange rates", rgb(DEFAULT_PALETTE.other)],
                ["Net change in cash", rgb(DEFAULT_PALETTE.flowNet)],
            ]);
            expect(barFills(container)).toEqual(new Set(["#102030", "#405060", "#708090", DEFAULT_PALETTE.other]));
        } finally {
            for (const token of Object.keys(theme)) root.style.removeProperty(token);
            if (before === null) root.removeAttribute("data-theme");
            else root.setAttribute("data-theme", before);
            await themeObserved();
        }
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
