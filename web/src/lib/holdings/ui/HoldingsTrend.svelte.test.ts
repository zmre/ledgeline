// The holdings trend, mounted — a regression net around its port onto
// `#lib/components/PeriodLineChart`.
//
// The chart it used to draw itself is now drawn by a shared component, and the
// only other thing that watches this file is `e2e/holdings.e2e.ts` /
// `e2e/otherHoldings.e2e.ts`, which look for `data-testid="holdings-trend"` and
// cannot be run in the sandbox. So the port's contract is pinned here instead:
// the same test hook, the same heading, the same empty sentence, and still one
// series with no legend box.
//
// Everything about HOW the line is drawn is `PeriodLineChart.svelte.test.ts`'s
// business. This file only checks that the adapter hands it the right things.

import {render} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {HoldingsPoint, HoldingsSeries} from "#lib/holdings/types.js";
import {chartColors} from "#lib/format/chartColors.svelte.js";
import HoldingsTrend from "./HoldingsTrend.svelte";

const MONTHS = ["Aug 2025", "Sep 2025", "Oct 2025", "Nov 2025", "Dec 2025", "Jan 2026", "Feb 2026", "Mar 2026", "Apr 2026", "May 2026", "Jun 2026", "Jul 2026"];

const point = (label: string, cents: number, flowCents = 0, returnPct: number | null = 0): HoldingsPoint => ({
    date: "2026-07-31",
    bucket: label,
    label,
    marketValue: dec(cents, 2),
    basis: null,
    flow: dec(flowCents, 2),
    returnPct,
});

const series = (cents: readonly number[], flows: readonly number[] = [], returns: readonly (number | null)[] = []): HoldingsSeries => ({
    base: "$",
    points: MONTHS.map((label, i) => point(label, cents[i] ?? 0, flows[i] ?? 0, returns[i] ?? 0)),
    hasBasis: false,
    anchor: null,
    returnAnchor: null,
    liveAt: null,
});

const money = (n: number): string => `$${n.toFixed(2)}`;
const compact = (n: number): string => `$${Math.round(n / 1000)}K`;

const mount = (trend: HoldingsSeries) => render(HoldingsTrend, {trend, formatValue: money, formatAxis: compact});

describe("COMPONENT HoldingsTrend", () => {
    it("keeps the test hook the holdings e2e specs look for", () => {
        mount(series(MONTHS.map((_, i) => 100_000 + i * 1_000)));

        expect(document.querySelector('[data-testid="holdings-trend"]')).not.toBeNull();
    });

    it("keeps its heading and its period note", () => {
        mount(series(MONTHS.map(() => 100_000)));
        const h3 = document.querySelector("h3");

        expect(h3?.textContent).toContain("Value over time");
        expect(h3?.textContent).toContain("· last 12 months");
    });

    it("draws one series and so carries no legend box — the heading names it", () => {
        mount(series(MONTHS.map((_, i) => 100_000 + i * 1_000)));

        expect(document.querySelectorAll("path.lc-path")).toHaveLength(1);
        expect(document.querySelector('[data-testid="holdings-trend-legend"]')).toBeNull();
    });

    it("paints the line by the window's direction: green up, red down, ink when unchanged (D4)", () => {
        const stroke = (): string | null => document.querySelector("path.lc-path")?.getAttribute("stroke") ?? null;
        const up = mount(series(MONTHS.map((_, i) => 100_000 + i * 1_000)));
        expect(stroke()).toBe(chartColors.flowIn);
        expect(document.querySelector("[data-trend-area]")?.getAttribute("data-trend-area")).toBe("up");
        up.unmount();

        const down = mount(series(MONTHS.map((_, i) => 100_000 - i * 1_000)));
        expect(stroke()).toBe(chartColors.flowOut);
        expect(document.querySelector("[data-trend-area]")?.getAttribute("data-trend-area")).toBe("down");
        down.unmount();

        mount(series(MONTHS.map(() => 100_000)));
        expect(stroke()).toBe(chartColors.flowNet);
        expect(chartColors.flowIn).not.toBe(chartColors.flowOut);
    });

    it("fills the area under the line with a gradient of the line's colour, fading to nothing", () => {
        mount(series(MONTHS.map((_, i) => 100_000 + i * 1_000)));
        const area = document.querySelector("path[data-trend-area]");
        const fill = area?.getAttribute("fill") ?? "";
        const id = /^url\(#(.+)\)$/.exec(fill)?.[1];
        const stops = [...document.querySelectorAll(`linearGradient[id="${id}"] stop`)];

        expect(id).toBeDefined();
        expect(stops.map((stop) => stop.getAttribute("stop-color"))).toEqual([chartColors.flowIn, chartColors.flowIn]);
        expect(stops.map((stop) => stop.getAttribute("stop-opacity"))).toEqual(["0.4", "0"]);
    });

    it("says when the last point is a live quote", () => {
        const trend = {...series(MONTHS.map((_, i) => 100_000 + i * 1_000)), liveAt: 1_795_000_000};
        render(HoldingsTrend, {trend, formatValue: money});

        expect(document.querySelector("h3")?.textContent).toMatch(/· last 12 months · live .+ \(delayed\)/);
    });

    it("says nothing is priced rather than drawing a flat line along zero", () => {
        mount(series(MONTHS.map(() => 0)));

        expect(document.body.textContent).toContain("No priced holdings in the last 12 months.");
        expect(document.querySelector('[data-testid="holdings-trend"]')).toBeNull();
    });

    it("labels the months the points carry", () => {
        mount(series(MONTHS.map((_, i) => 100_000 + i * 1_000)));
        const labels = [...document.querySelectorAll('[data-placement="bottom"] .lc-axis-tick-label')].map((t) => t.textContent?.trim());

        expect(labels).toEqual(["Aug 2025", "Oct 2025", "Dec 2025", "Feb 2026", "Apr 2026", "Jun 2026", "Jul 2026"]);
    });

    it("names the window it was fetched for, and labels daily points by date", () => {
        const days = ["2026-09-21", "2026-09-22", "2026-09-23", "2026-09-24", "2026-09-25", "2026-09-26", "2026-09-27", "2026-09-28"];
        const week: HoldingsSeries = {
            base: "$",
            points: days.map((date, i) => ({date, bucket: date, label: date, marketValue: dec(100_000 + i, 2), basis: null, flow: dec(0, 0), returnPct: 0})),
            hasBasis: false,
            anchor: null,
            returnAnchor: null,
            liveAt: null,
        };
        render(HoldingsTrend, {trend: week, period: "1wk", formatValue: money});
        const labels = [...document.querySelectorAll('[data-placement="bottom"] .lc-axis-tick-label')].map((t) => t.textContent?.trim());

        expect(document.querySelector("h3")?.textContent).toContain("· last 7 days");
        expect(labels[0]).toBe("Sep 21");
        expect(labels.at(-1)).toBe("Sep 28");
    });

    it("words its empty state for the window", () => {
        render(HoldingsTrend, {trend: series(MONTHS.map(() => 0)), period: "all", formatValue: money});

        expect(document.body.textContent).toContain("No priced holdings yet.");
    });

    it("colours the Value line by the period's gain, not by its value (plan 47 A1a)", () => {
        const stroke = (): string | null => document.querySelector("path.lc-path")?.getAttribute("stroke") ?? null;
        // A sale halves the value; the gain over the window was positive.
        const falling = series(MONTHS.map((_, i) => 100_000 - i * 4_000));
        const up = render(HoldingsTrend, {trend: falling, formatValue: money, gain: 1_200});
        expect(stroke()).toBe(chartColors.flowIn);
        up.unmount();

        const rising = series(MONTHS.map((_, i) => 100_000 + i * 4_000));
        const down = render(HoldingsTrend, {trend: rising, formatValue: money, gain: -50});
        expect(stroke()).toBe(chartColors.flowOut);
        down.unmount();

        // Unknown gain: value first-vs-last, as the Other tab always is.
        render(HoldingsTrend, {trend: falling, formatValue: money, gain: null});
        expect(stroke()).toBe(chartColors.flowOut);
    });

    it("marks net buying ▲ and selling ▼ where the flow shows on the axis, with a tooltip line each (plan 47 A1e)", () => {
        // Values $1,000–$1,110 (cents); flows: +$600 bought in Sep, −$480 sold in Nov, a $1 dividend in Jan.
        const values = MONTHS.map((_, i) => 100_000 + i * 1_000);
        const flows = [0, 60_000, 0, -48_000, 0, 100];
        const trend = {...series(values, flows), anchor: {min: 0, max: 2_000}};
        render(HoldingsTrend, {trend, formatValue: money});

        const marks = [...document.querySelectorAll("[data-flow-marker]")];
        expect(marks.map((m) => m.getAttribute("data-flow-marker"))).toEqual(["in", "out"]);
        expect(marks.map((m) => m.getAttribute("aria-label"))).toEqual(["Net bought $600.00", "Net sold $480.00"]);
        // Theme tokens, never a hard-coded colour.
        expect(marks[0].getAttribute("class")).toContain("fill-base-content");
        expect(marks[0].getAttribute("fill")).toBeNull();
    });

    it("draws no markers when no flow reaches the threshold", () => {
        mount(
            series(
                MONTHS.map((_, i) => 100_000 + i * 1_000),
                [0, 10, 10, 10]
            )
        );

        expect(document.querySelector("[data-flow-marker]")).toBeNull();
    });

    describe("Performance (plan 47 A1b/A1c)", () => {
        const RETURNS = [0, 0.01, 0.02, 0.05, 0.03, 0.04, 0.06, 0.08, 0.07, 0.09, 0.1, 0.123];

        it("plots the return, headed Performance, with a 0 % rule (the percent format is trendView.test.ts's)", () => {
            // Value falls (a sale) while the return rises: Performance is green.
            render(HoldingsTrend, {
                trend: series(
                    MONTHS.map((_, i) => 200_000 - i * 5_000),
                    [],
                    RETURNS
                ),
                formatValue: money,
                view: "perf",
            });

            expect(document.querySelector("h3")?.textContent).toContain("Performance");
            expect(document.querySelector("path.lc-path")?.getAttribute("stroke")).toBe(chartColors.flowIn);
            expect(document.querySelector('[data-rule="zero"]')).not.toBeNull();
        });

        it("is red when the last return is below zero, and draws no flow markers", () => {
            const returns = RETURNS.map((r) => -r);
            render(HoldingsTrend, {
                trend: series(
                    MONTHS.map(() => 100_000),
                    [0, 90_000],
                    returns
                ),
                formatValue: money,
                view: "perf",
            });

            expect(document.querySelector("path.lc-path")?.getAttribute("stroke")).toBe(chartColors.flowOut);
            expect(document.querySelector("[data-flow-marker]")).toBeNull();
        });

        it("draws a window where nothing moved as a flat 0 % line, not as the empty sentence", () => {
            render(HoldingsTrend, {trend: series(MONTHS.map(() => 100_000)), formatValue: money, view: "perf"});

            expect(document.querySelector('[data-testid="holdings-trend"]')).not.toBeNull();
            expect(document.querySelector("path.lc-path")?.getAttribute("stroke")).toBe(chartColors.flowNet);
        });
    });
});
