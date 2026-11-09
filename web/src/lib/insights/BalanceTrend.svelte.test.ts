// The pinned account's balance line, mounted (plan 36 amendment A1).
//
// HOW the line is drawn is PeriodLineChart's business (its own test file); what
// is asked here is what this adapter adds: one line, the heading naming the
// account, commodity and window, and a marker per failing period (a shape,
// with a hairline to the asserted figure). jsdom has no layout, so positions
// are not asserted; the click is BalanceTrend.click.svelte.test.ts's.

import {render} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {balanceSeries} from "#lib/testing/balanceSeriesFixture.js";
import BalanceTrend from "./BalanceTrend.svelte";

const SVG = "http://www.w3.org/2000/svg";

describe("COMPONENT BalanceTrend", () => {
    it("draws one line and names the account, commodity and window", () => {
        render(BalanceTrend, {series: balanceSeries()});
        expect(document.querySelectorAll("path.lc-path")).toHaveLength(1);
        expect(document.querySelector('[data-testid="balance-trend-legend"]')).toBeNull();
        const heading = document.querySelector("h3")?.textContent ?? "";
        expect(heading).toContain("assets:bank");
        expect(heading).toContain("$ · Jan 1 – Feb 3, 2026");
    });

    it("keeps zero in frame, as a balance chart must", () => {
        render(BalanceTrend, {series: balanceSeries()});
        expect(document.querySelector('[data-rule="zero"]')).not.toBeNull();
    });

    it("marks each failing period with a shape, a hairline and the asserted point, as SVG", () => {
        render(BalanceTrend, {series: balanceSeries()});
        const groups = document.querySelectorAll("[data-failure]");
        expect(groups).toHaveLength(1);
        const group = groups[0];
        expect(group.getAttribute("data-failure")).toBe("4");
        for (const part of ["[data-failure-marker]", "[data-failure-gap]", "[data-failure-expected]"]) {
            const element = group.querySelector(part);
            expect(element, part).not.toBeNull();
            // Created in the SVG namespace, or a browser would not draw it at all.
            expect(element?.namespaceURI, part).toBe(SVG);
        }
        expect(group.querySelector("title")?.textContent).toContain("expected $300.00, found $220.25, off by $-79.75");
    });

    it("draws no markers when every assertion holds", () => {
        const healthy = balanceSeries();
        render(BalanceTrend, {series: {...healthy, points: healthy.points.map((point) => ({...point, failures: []}))}});
        expect(document.querySelectorAll("[data-failure]")).toHaveLength(0);
    });

    it("says the balance is zero rather than drawing a line along the axis", () => {
        const flat = balanceSeries();
        render(BalanceTrend, {series: {...flat, points: flat.points.map((point) => ({...point, balance: {m: 0n, p: 0}, failures: []}))}});
        expect(document.body.textContent).toContain("The balance is zero throughout this range.");
    });
});
