// The Stocks value-over-time chart with its benchmark overlay, mounted against
// the REAL benchmark store and settings, fed by a stubbed `fetch` (the
// FAKE_ENGINE convention): what is asserted is what a click actually causes —
// a request for that benchmark's line over this chart's window, then a second,
// dashed line in the benchmark's own colour, named in the legend.
//
// Plan 47 A1: benchmarks are returns, drawn on the Performance chart only. The
// overlay suite therefore runs in Performance; the Value / Performance switch
// and Compare's gating have a suite of their own.

import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {flushSync} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {dec} from "#lib/domain/money.js";
import {chartColors} from "#lib/format/chartColors.svelte.js";
import type {HoldingsScope, HoldingsSeries} from "#lib/holdings/types.js";
import {benchmarkCatalog, benchmarkLines} from "#lib/stores/benchmarks.svelte.js";
import {holdingsChartView} from "#lib/stores/holdings.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {keymap} from "#ui/keys/keymap.svelte.js";
import StocksTrend from "./StocksTrend.svelte";

const DATES = ["2026-07-31", "2026-08-31", "2026-09-28"];

const TREND: HoldingsSeries = {
    base: "$",
    points: DATES.map((date, i) => ({
        date,
        bucket: date.slice(0, 7),
        label: ["Jul 2026", "Aug 2026", "Sep 2026"][i],
        marketValue: dec(100_000 + i * 5_000, 2),
        basis: null,
        flow: dec(0, 0),
        returnPct: [0, 0.02, 0.04][i],
    })),
    hasBasis: false,
    anchor: null,
    returnAnchor: null,
    liveAt: null,
};

const SCOPE: HoldingsScope = {accounts: new Set(), mode: "include", asOf: "2026-09-28", gainPeriod: "12mo", holdings: new Set()};

/** The engine's catalog (`GET /api/holdings/benchmarks/catalog`), as benchmarks_api.rs serves it. */
const CATALOG = [
    {symbol: "SPY", label: "S&P 500 (SPY)"},
    {symbol: "DIA", label: "Dow Jones (DIA)"},
    {symbol: "QQQ", label: "Nasdaq-100 (QQQ)"},
    {symbol: "VTI", label: "US total market (VTI)"},
    {symbol: "BND", label: "US bonds (BND)"},
    {symbol: "VXUS", label: "International (VXUS)"},
    {symbol: "IWM", label: "Small cap (IWM)"},
    {symbol: "GLD", label: "Gold (GLD)"},
];
const slotOf = (symbol: string): number => CATALOG.findIndex((b) => b.symbol === symbol);
const labelOf = (symbol: string): string => CATALOG[slotOf(symbol)]?.label ?? symbol;

const lineOf = (symbol: string): unknown =>
    symbol === "GLD"
        ? {
              symbol,
              label: labelOf(symbol),
              slot: slotOf(symbol),
              points: DATES.map((date) => ({date, value: null, returnPct: null})),
              stale: false,
              error: "Could not fetch GLD history from Yahoo Finance (offline).",
          }
        : {
              symbol,
              label: labelOf(symbol),
              slot: slotOf(symbol),
              points: DATES.map((date, i) => ({date, value: 1000 + i * 100, returnPct: i * 0.1})),
              stale: false,
              error: null,
          };

/** The server's answer to `symbols=A,B`: one line per symbol. */
const lineFor = (symbols: string): unknown => ({benchmarks: symbols.split(",").map(lineOf)});

let benchmarkRequests: URL[] = [];

beforeEach(async () => {
    await connectFakeEngine();
    keymap.reset();
    holdingsChartView.value = "perf";
    benchmarkRequests = [];
    vi.stubGlobal("fetch", (input: unknown) => {
        const url = new URL(String(input));
        if (url.pathname === "/version") return Promise.resolve(new Response(JSON.stringify("1.52"), {status: 200}));
        if (url.pathname === "/api/holdings/benchmarks/catalog") {
            return Promise.resolve(new Response(JSON.stringify({benchmarks: CATALOG}), {status: 200, headers: {"Content-Type": "application/json"}}));
        }
        if (url.pathname === "/api/holdings/benchmarks") {
            benchmarkRequests.push(url);
            return Promise.resolve(
                new Response(JSON.stringify(lineFor(url.searchParams.get("symbols") ?? "")), {status: 200, headers: {"Content-Type": "application/json"}})
            );
        }
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
});

afterEach(() => {
    for (const symbol of [...settings.benchmarks]) settings.toggleBenchmark(symbol, false);
    benchmarkLines.reset();
    benchmarkCatalog.reset();
    holdingsChartView.value = "value";
    vi.unstubAllGlobals();
});

const mount = (scope: HoldingsScope = SCOPE) =>
    render(StocksTrend, {
        trend: TREND,
        scope,
        serverUrl: FAKE_ENGINE,
        formatValue: (n: number) => `$${n.toFixed(2)}`,
        formatAxis: (n: number) => `$${n}`,
    });

const lines = (): Element[] => [...document.querySelectorAll("path.lc-path")];

describe("COMPONENT StocksTrend benchmark overlay", () => {
    it("draws the portfolio alone and asks for nothing until a box is ticked", async () => {
        mount();
        await Promise.resolve();

        expect(lines()).toHaveLength(1);
        expect(benchmarkRequests).toHaveLength(0);
        expect(document.querySelector('[data-testid="holdings-trend-legend"]')).toBeNull();
    });

    it("ticking a benchmark fetches it for this chart's window and adds its dashed line", async () => {
        mount();
        await fireEvent.click(await screen.findByTestId("benchmark-SPY"));

        await waitFor(() => expect(lines()).toHaveLength(2));
        expect(benchmarkRequests).toHaveLength(1);
        const params = benchmarkRequests[0].searchParams;
        expect(params.get("symbols")).toBe("SPY");
        expect(params.get("interval")).toBe("monthly");
        expect(params.get("since")).toBe("2025-09-28");
        expect(params.get("asOf")).toBe("2026-09-28");

        expect(params.get("holdings")).toBeNull(); // no holdings filter → every holding seeds the line

        const [portfolio, spy] = lines();
        // The portfolio rose over the window, so it is drawn up-green (plan 47 D4)…
        expect(portfolio.getAttribute("stroke")).toBe(chartColors.flowIn);
        expect(spy.getAttribute("stroke")).toBe(chartColors.colorAt(0));
        expect(spy.getAttribute("class")).toContain("[stroke-dasharray:4_3]");
        // …and only the portfolio is filled: the benchmark stays a bare dashed line.
        expect(document.querySelectorAll("path[data-trend-area]")).toHaveLength(1);

        const legend = document.querySelector('[data-testid="holdings-trend-legend"]')?.textContent ?? "";
        expect(legend).toContain("Your portfolio");
        expect(legend).toContain("S&P 500 (SPY)");
        expect(settings.benchmarks).toEqual(["SPY"]);
    });

    it("seeds the line from the filtered holdings' own flows under a holdings filter (plan 47 D8)", async () => {
        mount({...SCOPE, holdings: new Set(["VTI", "AAPL"])});
        await fireEvent.click(await screen.findByTestId("benchmark-SPY"));

        await waitFor(() => expect(benchmarkRequests).toHaveLength(1));
        const params = benchmarkRequests[0].searchParams;
        expect(params.get("symbols")).toBe("SPY");
        expect(params.get("holdings")).toBe("AAPL,VTI");
    });

    it("reads as a button: Compare is a primary button (plan 47 D5)", async () => {
        mount();
        const compare = await screen.findByTestId("benchmark-compare");

        expect(compare.className).toContain("btn-primary");
        expect(compare.className).not.toContain("btn-ghost");
        expect(compare.textContent).toContain("Compare");
    });

    it("unticking removes the line without refetching, and re-ticking reuses it", async () => {
        mount();
        await fireEvent.click(await screen.findByTestId("benchmark-SPY"));
        await waitFor(() => expect(lines()).toHaveLength(2));

        await fireEvent.click(await screen.findByTestId("benchmark-SPY"));
        await waitFor(() => expect(lines()).toHaveLength(1));
        await fireEvent.click(await screen.findByTestId("benchmark-SPY"));
        await waitFor(() => expect(lines()).toHaveLength(2));

        expect(benchmarkRequests).toHaveLength(1);
    });

    it("boxes already ticked go out as ONE request, which adds all their lines", async () => {
        settings.toggleBenchmark("SPY", true);
        settings.toggleBenchmark("QQQ", true);
        settings.toggleBenchmark("GLD", true);
        mount();

        await waitFor(() => expect(lines()).toHaveLength(3));
        expect(benchmarkRequests).toHaveLength(1);
        expect(benchmarkRequests[0].searchParams.get("symbols")).toBe("SPY,QQQ,GLD");
        // GLD's own failure stays beside its own box.
        await waitFor(() => expect(screen.getByTestId("benchmark-GLD-error").textContent).toContain("Could not fetch GLD"));
    });

    it("a reloaded base series (same window) refetches the lines seeded from it", async () => {
        settings.toggleBenchmark("SPY", true);
        const view = mount();
        await waitFor(() => expect(lines()).toHaveLength(2));
        expect(benchmarkRequests).toHaveLength(1);

        // Refresh / Update prices: the page hands down a NEW series for the same scope.
        await view.rerender({trend: {...TREND, points: [...TREND.points]}});
        await waitFor(() => expect(benchmarkRequests).toHaveLength(2));
        await waitFor(() => expect(lines()).toHaveLength(2));
    });

    it("a benchmark that cannot be fetched shows its hint and leaves the chart as it was", async () => {
        mount();
        await fireEvent.click(await screen.findByTestId("benchmark-GLD"));

        await waitFor(() => expect(screen.getByTestId("benchmark-GLD-error").textContent).toContain("Could not fetch GLD"));
        expect(lines()).toHaveLength(1);
    });
});

describe("COMPONENT StocksTrend Value / Performance (plan 47 A1)", () => {
    beforeEach(() => {
        holdingsChartView.value = "value";
    });

    const pressed = (view: string): string | null => screen.getByTestId(`chart-view-${view}`).getAttribute("aria-pressed");
    const heading = (): string => document.querySelector("h3")?.textContent ?? "";

    it("opens on Value: both buttons are tabbable buttons, Value pressed", async () => {
        mount();
        const value = await screen.findByTestId("chart-view-value");
        const perf = screen.getByTestId("chart-view-perf");

        expect(value.tagName).toBe("BUTTON");
        expect(perf.tagName).toBe("BUTTON");
        expect(value.getAttribute("tabindex")).not.toBe("-1");
        expect(perf.getAttribute("tabindex")).not.toBe("-1");
        expect(pressed("value")).toBe("true");
        expect(pressed("perf")).toBe("false");
        expect(heading()).toContain("Value over time");
    });

    it("clicking Performance switches the chart and the store the URL mirrors", async () => {
        mount();
        await fireEvent.click(await screen.findByTestId("chart-view-perf"));

        expect(holdingsChartView.value).toBe("perf");
        expect(pressed("perf")).toBe("true");
        expect(heading()).toContain("Performance");
        await fireEvent.click(screen.getByTestId("chart-view-value"));
        expect(holdingsChartView.value).toBe("value");
    });

    it("`v` flips the view", async () => {
        mount();
        await screen.findByTestId("chart-view-value");

        keymap.handle(new KeyboardEvent("keydown", {key: "v", cancelable: true}));
        flushSync();
        expect(holdingsChartView.value).toBe("perf");
        keymap.handle(new KeyboardEvent("keydown", {key: "v", cancelable: true}));
        flushSync();
        expect(holdingsChartView.value).toBe("value");
    });

    it("colours the Value line by the gain it is handed (A1a)", async () => {
        render(StocksTrend, {trend: TREND, scope: SCOPE, serverUrl: FAKE_ENGINE, formatValue: (n: number) => `$${n}`, gain: -10});
        await screen.findByTestId("chart-view-value");

        // The value rose, the gain did not: red.
        expect(lines()[0].getAttribute("stroke")).toBe(chartColors.flowOut);
    });

    it("draws no benchmark in Value mode and asks for none, even with boxes ticked (A1d)", async () => {
        settings.toggleBenchmark("SPY", true);
        mount();
        await screen.findByTestId("benchmark-SPY");
        await Promise.resolve();

        expect(lines()).toHaveLength(1);
        expect(benchmarkRequests).toHaveLength(0);
        expect(document.querySelector('[data-testid="holdings-trend-legend"]')).toBeNull();
    });

    it("Compare in Value mode switches to Performance and opens the picker, which then draws the ticked return", async () => {
        settings.toggleBenchmark("SPY", true);
        mount();
        const picker = (await screen.findByTestId("benchmark-picker")) as HTMLDetailsElement;
        expect(picker.open).toBe(false);

        await fireEvent.click(screen.getByTestId("benchmark-compare"));

        expect(holdingsChartView.value).toBe("perf");
        expect(picker.open).toBe(true);
        await waitFor(() => expect(lines()).toHaveLength(2));
        expect(benchmarkRequests).toHaveLength(1);
    });

    it("switching back to Value closes the picker and drops the overlay", async () => {
        settings.toggleBenchmark("SPY", true);
        mount();
        await fireEvent.click(await screen.findByTestId("benchmark-compare"));
        await waitFor(() => expect(lines()).toHaveLength(2));

        await fireEvent.click(screen.getByTestId("chart-view-value"));

        expect((screen.getByTestId("benchmark-picker") as HTMLDetailsElement).open).toBe(false);
        expect(lines()).toHaveLength(1);
    });

    it("explains the comparison as total return since the first point", async () => {
        mount();
        const picker = await screen.findByTestId("benchmark-picker");

        expect(picker.textContent).toContain("total return since the chart's first point");
        expect(picker.textContent).not.toContain("contributions and withdrawals");
    });
});
