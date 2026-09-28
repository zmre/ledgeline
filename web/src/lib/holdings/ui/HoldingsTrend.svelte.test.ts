// The holdings trend, mounted — a regression net around its port onto
// `$lib/components/PeriodLineChart`.
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
import {dec} from "$lib/domain/money";
import type {HoldingsPoint, HoldingsSeries} from "$lib/holdings/types";
import HoldingsTrend from "./HoldingsTrend.svelte";

const MONTHS = ["Aug 2025", "Sep 2025", "Oct 2025", "Nov 2025", "Dec 2025", "Jan 2026", "Feb 2026", "Mar 2026", "Apr 2026", "May 2026", "Jun 2026", "Jul 2026"];

const point = (label: string, cents: number): HoldingsPoint => ({
    date: "2026-07-31",
    bucket: label,
    label,
    marketValue: dec(cents, 2),
    basis: null,
});

const series = (cents: readonly number[]): HoldingsSeries => ({
    base: "$",
    points: MONTHS.map((label, i) => point(label, cents[i] ?? 0)),
    hasBasis: false,
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
            points: days.map((date, i) => ({date, bucket: date, label: date, marketValue: dec(100_000 + i, 2), basis: null})),
            hasBasis: false,
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
});
