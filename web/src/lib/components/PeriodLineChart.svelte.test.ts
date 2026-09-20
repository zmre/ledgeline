// The shared period line chart, mounted.
//
// jsdom has no layout engine, so the chart renders into a 0×0 box and every
// coordinate it produces is meaningless — several are negative. Nothing here
// asks where anything is. What a 0×0 render still answers, and what this file
// asserts, is everything upstream of layout: how many lines there are, which
// slot colour each took, which one is dashed, how many x labels were asked for
// and which buckets they name, and whether the legend exists at all.
//
// Those are the conventions `PeriodLineChart`'s header promises and the ones a
// second chart copying it would otherwise quietly drop. The tick count is the
// load-bearing one: an index x-scale will put a tick at 2.5 unless the integers
// are named, and the symptom — a label belonging to neither adjacent bucket —
// looks like a data bug rather than an axis bug.
//
// The queries reach for layerchart's own class names (`lc-path`,
// `lc-axis-tick-label`). That couples this file to a dependency's internals on
// purpose: a layerchart upgrade that changes what these charts draw should stop
// the suite rather than land silently.

import {render} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {colorAt} from "$lib/format/palette";
import PeriodLineChart from "./PeriodLineChart.svelte";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const money = (n: number): string => `$${n.toFixed(2)}`;
const compact = (n: number): string => `$${n}`;

const ramp = (labels: readonly string[]): number[] => labels.map((_, i) => i + 1);

interface MountOptions {
    heading?: string;
    note?: string;
    labels?: readonly string[];
    series?: {name: string; values: readonly number[]; dashed?: boolean}[];
    empty?: string;
}

function mount({heading = "Value over time", note, labels = MONTHS, series, empty}: MountOptions = {}) {
    return render(PeriodLineChart, {
        heading,
        note,
        labels,
        series: series ?? [{name: "Market value", values: ramp(labels)}],
        formatValue: money,
        formatAxis: compact,
        empty,
        testid: "trend",
    });
}

/** One entry per drawn line, in the order the chart drew them. */
const lines = (root: HTMLElement): {stroke: string | null; class: string}[] =>
    [...root.querySelectorAll("path.lc-path")].map((p) => ({stroke: p.getAttribute("stroke"), class: p.getAttribute("class") ?? ""}));

/** The x-axis labels actually rendered, left to right. */
const xLabels = (root: HTMLElement): string[] =>
    [...root.querySelectorAll('[data-placement="bottom"] .lc-axis-tick-label')].map((t) => t.textContent?.trim() ?? "");

describe("COMPONENT PeriodLineChart", () => {
    describe("series", () => {
        it("draws one line per series, taking palette slots in order and never cycling", () => {
            const {container} = mount({
                series: [
                    {name: "Actual", values: ramp(MONTHS)},
                    {name: "Budget", values: ramp(MONTHS)},
                    {name: "Last year", values: ramp(MONTHS)},
                ],
            });

            expect(lines(container).map((l) => l.stroke)).toEqual([colorAt(0), colorAt(1), colorAt(2)]);
        });

        it("dashes only the series that asked for it, and keeps the 2px stroke on both", () => {
            const {container} = mount({
                series: [
                    {name: "Actual", values: ramp(MONTHS)},
                    {name: "Projected", values: ramp(MONTHS), dashed: true},
                ],
            });
            const drawn = lines(container);

            expect(drawn.map((l) => l.class.includes("[stroke-dasharray:4_3]"))).toEqual([false, true]);
            expect(drawn.every((l) => l.class.includes("stroke-2"))).toBe(true);
        });

        it("gives two series the same name different lines rather than collapsing them", () => {
            // Keyed by slot, not by label: the same metric under two scenarios is
            // a real caller, and a name-keyed series would silently merge them.
            const {container} = mount({
                series: [
                    {name: "Cash", values: ramp(MONTHS)},
                    {name: "Cash", values: ramp(MONTHS)},
                ],
            });

            expect(lines(container)).toHaveLength(2);
        });
    });

    describe("legend", () => {
        it("omits it for a single series, because the heading already names it", () => {
            mount();

            expect(document.querySelector('[data-testid="trend-legend"]')).toBeNull();
        });

        it("shows it from two series on, naming every one", () => {
            mount({
                series: [
                    {name: "Actual", values: ramp(MONTHS)},
                    {name: "Projected", values: ramp(MONTHS), dashed: true},
                ],
            });
            const legend = document.querySelector('[data-testid="trend-legend"]');

            expect(legend?.textContent).toContain("Actual");
            expect(legend?.textContent).toContain("Projected");
        });
    });

    describe("x axis", () => {
        it("labels about six buckets out of twelve, on whole buckets only", () => {
            const {container} = mount();

            // Stride 2 from index 0, so every label names a real bucket — there is
            // no label sitting between Feb and Mar.
            expect(xLabels(container)).toEqual(["Jan", "Mar", "May", "Jul", "Sep", "Nov", "Dec"]);
        });

        it("always labels the last bucket, even when the stride steps over it", () => {
            // 14 buckets ⇒ stride 3 ⇒ 0,3,6,9,12 … and 13, which 3 misses. An
            // unlabelled final bucket reads as if the series stopped early.
            const labels = [...MONTHS, "Jan+", "Feb+"];
            const {container} = mount({labels, series: [{name: "Market value", values: ramp(labels)}]});

            expect(xLabels(container)).toEqual(["Jan", "Apr", "Jul", "Oct", "Jan+", "Feb+"]);
        });

        it("labels every bucket when there are fewer than the target", () => {
            const labels = ["Q1", "Q2", "Q3", "Q4"];
            const {container} = mount({labels, series: [{name: "Market value", values: ramp(labels)}]});

            expect(xLabels(container)).toEqual(labels);
        });
    });

    describe("nothing to draw", () => {
        it("says so in words rather than drawing a flat line on the axis", () => {
            mount({labels: MONTHS, series: [{name: "Market value", values: MONTHS.map(() => 0)}], empty: "No priced holdings in the last 12 months."});

            expect(document.body.textContent).toContain("No priced holdings in the last 12 months.");
            expect(document.querySelector('[data-testid="trend"]')).toBeNull();
        });

        it("treats an empty bucket list the same way", () => {
            mount({labels: [], series: [{name: "Market value", values: []}], empty: "Nothing here."});

            expect(document.body.textContent).toContain("Nothing here.");
        });

        it("draws once any series has a non-zero bucket", () => {
            mount({
                series: [
                    {name: "A", values: MONTHS.map(() => 0)},
                    {name: "B", values: MONTHS.map((_, i) => (i === 11 ? 1 : 0))},
                ],
            });

            expect(document.querySelector('[data-testid="trend"]')).not.toBeNull();
        });
    });

    it("heads the chart, and sets the note off from it", () => {
        mount({heading: "Value over time", note: "last 12 months"});
        const h3 = document.querySelector("h3");

        expect(h3?.textContent).toContain("Value over time");
        expect(h3?.textContent).toContain("· last 12 months");
    });
});
