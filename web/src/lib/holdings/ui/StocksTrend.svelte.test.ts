// The Stocks value-over-time chart with its benchmark overlay, mounted against
// the REAL benchmark store and settings, fed by a stubbed `fetch` (the
// FAKE_ENGINE convention): what is asserted is what a click actually causes —
// a request for that benchmark's line over this chart's window, then a second,
// dashed line in the benchmark's own colour, named in the legend.

import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {dec} from "$lib/domain/money";
import {chartColors} from "$lib/format/chartColors.svelte";
import type {HoldingsScope, HoldingsSeries} from "$lib/holdings/types";
import {benchmarkLines} from "$lib/stores/benchmarks.svelte";
import {settings} from "$lib/stores/settings.svelte";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
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
    })),
    hasBasis: false,
};

const SCOPE: HoldingsScope = {accounts: new Set(), mode: "include", asOf: "2026-09-28", gainPeriod: "12mo"};

const lineOf = (symbol: string): unknown =>
    symbol === "GLD"
        ? {
              symbol,
              points: DATES.map((date) => ({date, value: null})),
              stale: false,
              error: "Could not fetch GLD history from Yahoo Finance (offline).",
          }
        : {
              symbol,
              points: DATES.map((date, i) => ({date, value: 1000 + i * 100})),
              stale: false,
              error: null,
          };

/** The server's answer to `symbols=A,B`: one line per symbol. */
const lineFor = (symbols: string): unknown => ({benchmarks: symbols.split(",").map(lineOf)});

let benchmarkRequests: URL[] = [];

beforeEach(async () => {
    await connectFakeEngine();
    benchmarkRequests = [];
    vi.stubGlobal("fetch", (input: unknown) => {
        const url = new URL(String(input));
        if (url.pathname === "/version") return Promise.resolve(new Response(JSON.stringify("1.52"), {status: 200}));
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
    vi.unstubAllGlobals();
});

const mount = () =>
    render(StocksTrend, {
        trend: TREND,
        scope: SCOPE,
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
        await fireEvent.click(screen.getByTestId("benchmark-SPY"));

        await waitFor(() => expect(lines()).toHaveLength(2));
        expect(benchmarkRequests).toHaveLength(1);
        const params = benchmarkRequests[0].searchParams;
        expect(params.get("symbols")).toBe("SPY");
        expect(params.get("interval")).toBe("monthly");
        expect(params.get("since")).toBe("2025-09-28");
        expect(params.get("asOf")).toBe("2026-09-28");

        const [portfolio, spy] = lines();
        expect(portfolio.getAttribute("stroke")).toBe(chartColors.flowNet);
        expect(spy.getAttribute("stroke")).toBe(chartColors.colorAt(0));
        expect(spy.getAttribute("class")).toContain("[stroke-dasharray:4_3]");

        const legend = document.querySelector('[data-testid="holdings-trend-legend"]')?.textContent ?? "";
        expect(legend).toContain("Your portfolio");
        expect(legend).toContain("S&P 500 (SPY)");
        expect(settings.benchmarks).toEqual(["SPY"]);
    });

    it("unticking removes the line without refetching, and re-ticking reuses it", async () => {
        mount();
        await fireEvent.click(screen.getByTestId("benchmark-SPY"));
        await waitFor(() => expect(lines()).toHaveLength(2));

        await fireEvent.click(screen.getByTestId("benchmark-SPY"));
        await waitFor(() => expect(lines()).toHaveLength(1));
        await fireEvent.click(screen.getByTestId("benchmark-SPY"));
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
        await fireEvent.click(screen.getByTestId("benchmark-GLD"));

        await waitFor(() => expect(screen.getByTestId("benchmark-GLD-error").textContent).toContain("Could not fetch GLD"));
        expect(lines()).toHaveLength(1);
    });
});
