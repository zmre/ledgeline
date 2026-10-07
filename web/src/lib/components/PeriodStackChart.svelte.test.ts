// The shared stacked-diverging period chart, mounted.
//
// jsdom has no layout engine (see PeriodFlowChart.svelte.test.ts), so nothing
// here asserts a coordinate (`stackBars.test.ts` covers the bar geometry). What
// it can check is structure: one bar per non-zero value, one area per series,
// one legend entry per ENTITY (not per drawn half), the net drawn twice, net
// labels only where asked, and the empty and too-short states instead of a
// broken plot.

import {render} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {chartColors} from "$lib/format/chartColors.svelte";
import PeriodStackChart from "./PeriodStackChart.svelte";

const LABELS = ["2022", "2023", "2024", "2025", "2026"];
const money = (n: number): string => `$${n.toFixed(2)}`;

/** An account that is overdrawn in 2023 — split into halves that share its identity. */
const SERIES = [
    {key: "checking|+", entity: "checking", label: "assets:checking", color: "#111111", values: [100, 0, 120, 130, 140]},
    {key: "checking|-", entity: "checking", label: "assets:checking", color: "#111111", values: [0, -20, 0, 0, 0]},
    {key: "home|+", entity: "home", label: "assets:home", color: "#222222", values: [500, 510, 520, 530, 540]},
    {key: "visa|-", entity: "visa", label: "liabilities:visa", color: "#333333", values: [-40, -50, -30, -20, -10]},
    // Two accounts past the palette's slots: the same muted colour, still two entries.
    {key: "car|+", entity: "car", label: "assets:car", color: "#999999", values: [5, 5, 5, 5, 5]},
    {key: "loan|-", entity: "loan", label: "liabilities:loan", color: "#999999", values: [-1, -1, -1, -1, -1]},
];
const NET = [564, 444, 614, 644, 674];

interface MountOptions {
    labels?: readonly string[];
    series?: typeof SERIES;
    net?: readonly number[];
    mark?: "bar" | "area";
    labelNet?: boolean;
    minBuckets?: number;
}

function mount({labels = LABELS, series = SERIES, net = NET, mark = "bar", labelNet, minBuckets}: MountOptions = {}) {
    return render(PeriodStackChart, {
        heading: "Net worth",
        labels,
        series,
        net,
        mark,
        labelNet,
        minBuckets,
        netLabel: "Net worth",
        formatValue: money,
        empty: "Nothing here.",
        testid: "stack",
    });
}

const legend = (): string[] => [...document.querySelectorAll('[data-testid="stack-legend"] li')].map((li) => li.textContent?.trim() ?? "");

describe("COMPONENT PeriodStackChart", () => {
    describe("bars", () => {
        it("draws one bar per non-zero value, in the series' own colours, with no outline", () => {
            const {container} = mount();
            const bars = [...container.querySelectorAll(".lc-bars-bar")];
            const nonZero = SERIES.flatMap((s) => s.values).filter((v) => v !== 0).length;

            expect(bars.length).toBe(nonZero);
            expect(new Set(bars.map((b) => b.getAttribute("fill")))).toEqual(new Set(SERIES.map((s) => s.color)));
            expect(bars.every((b) => (b.getAttribute("stroke") ?? "none") === "none" || b.getAttribute("stroke-width") === "0")).toBe(true);
        });

        it("draws the zero rule the stacks grow from", () => {
            const {container} = mount();

            expect(container.querySelector(".lc-rule-y-line")).not.toBeNull();
        });
    });

    describe("areas", () => {
        it("draws one filled area per series, and no top line", () => {
            const {container} = mount({mark: "area"});

            expect(container.querySelectorAll("path.lc-area-path")).toHaveLength(SERIES.length);
            expect(container.querySelectorAll(".lc-area-line")).toHaveLength(0);
            expect(container.querySelectorAll(".lc-bars-bar")).toHaveLength(0);
        });

        it("fills solid, separated by a hairline of surface", () => {
            const {container} = mount({mark: "area"});

            const paths = [...container.querySelectorAll("path.lc-area-path")];
            expect(paths.map((path) => path.getAttribute("fill-opacity"))).toEqual(paths.map(() => null));
            expect(paths.every((path) => path.classList.contains("stroke-base-200") && path.getAttribute("stroke-width") === "1")).toBe(true);
        });
    });

    describe("the net", () => {
        it("is drawn twice — a surface halo under dashed ink", () => {
            const {container} = mount({mark: "area"});
            const splines = [...container.querySelectorAll("path.lc-spline-path, path.lc-path")].filter((p) =>
                (p.getAttribute("class") ?? "").includes("[stroke-dasharray:5_3]")
            );

            expect(splines).toHaveLength(2);
            expect(splines[0].getAttribute("class")).toContain("stroke-base-200");
            expect(splines[1].getAttribute("stroke")).toBe(chartColors.flowNet);
        });

        // jsdom lays nothing out, so the plot has no width and the labels get
        // the axis's unfitted budget (about six) — every one of these five.
        it("labels the net with the formatted value where asked", () => {
            const {container} = mount({labelNet: true});
            const points = [...container.querySelectorAll("g.lc-net-point")];

            expect(points.map((p) => p.getAttribute("data-bucket"))).toEqual(["0", "1", "2", "3", "4"]);
            expect(points[4].querySelector("text")?.textContent).toBe("$674.00");
            expect(points[4].querySelector("circle")?.getAttribute("fill")).toBe(chartColors.flowNet);
        });

        it("puts no label on a bucket with nothing in it", () => {
            const series = SERIES.map((s) => ({...s, values: [0, ...s.values.slice(1)]}));
            const {container} = mount({series, net: [0, ...NET.slice(1)], labelNet: true});
            const buckets = [...container.querySelectorAll("g.lc-net-point")].map((p) => p.getAttribute("data-bucket"));

            expect(buckets).toEqual(["1", "2", "3", "4"]);
        });

        it("labels none by default — never a number on every point unasked", () => {
            const {container} = mount();

            expect(container.querySelectorAll("g.lc-net-point")).toHaveLength(0);
        });
    });

    describe("legend", () => {
        it("has one entry per entity, not per drawn half — two that share a colour included — and the net", () => {
            mount();

            expect(legend()).toEqual(["assets:checking", "assets:home", "liabilities:visa", "assets:car", "liabilities:loan", "Net worth"]);
        });
    });

    describe("nothing to draw", () => {
        it("says so when every value and the net are zero", () => {
            mount({series: SERIES.map((s) => ({...s, values: s.values.map(() => 0)})), net: LABELS.map(() => 0)});

            expect(document.querySelector('[data-testid="stack-empty"]')?.textContent).toBe("Nothing here.");
            expect(document.querySelector('[data-testid="stack"]')).toBeNull();
        });

        it("says so with no buckets at all", () => {
            mount({labels: [], net: []});

            expect(document.querySelector('[data-testid="stack-empty"]')).not.toBeNull();
        });

        it("refuses a single bucket when the caller asks for a trend", () => {
            mount({labels: ["2026"], net: [674], minBuckets: 2});

            expect(document.querySelector('[data-testid="stack-empty"]')?.textContent).toContain("Only one period");
            expect(document.querySelector('[data-testid="stack"]')).toBeNull();
        });
    });
});
