// The shared inflow/outflow/net chart, mounted.
//
// Same constraint as `PeriodLineChart.svelte.test.ts`: jsdom has no layout
// engine, the chart renders into a 0×0 box, and every coordinate it emits is
// meaningless. So nothing here asks whether the green bars are ABOVE the red
// ones — that is the one claim this chart makes that a mount cannot check, and
// pretending otherwise with a coordinate comparison would be asserting on
// jsdom's degenerate arithmetic rather than on the chart.
//
// Two things it can check, and they cover the props most likely to be misused:
//
//   · The sign is the chart's, not the caller's. `outflows` is documented as a
//     magnitude, and the same outflow handed over positive or negative has to
//     draw the same picture — asserted as an invariance between two renders, so
//     no particular geometry is pinned, only that the two agree.
//   · `net` defaults to inflow − outflow and is otherwise taken literally.
//
// The rest is structure: the bar colours are the diverging pair and not
// categorical slots, the zero rule exists, the net is drawn twice (halo then
// ink), no bar wears a stroke, and the legend names all three marks.

import {render} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {CATEGORICAL, FLOW_IN, FLOW_NET, FLOW_OUT} from "$lib/format/palette";
import PeriodFlowChart from "./PeriodFlowChart.svelte";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const money = (n: number): string => `$${n.toFixed(2)}`;

const fill = (labels: readonly string[], n: number): number[] => labels.map(() => n);

interface MountOptions {
    heading?: string;
    note?: string;
    labels?: readonly string[];
    inflows?: readonly number[];
    outflows?: readonly number[];
    net?: readonly number[];
    inflowLabel?: string;
    outflowLabel?: string;
    netLabel?: string;
    empty?: string;
}

function mount({heading = "Net income", note, labels = MONTHS, inflows, outflows, net, inflowLabel, outflowLabel, netLabel, empty}: MountOptions = {}) {
    return render(PeriodFlowChart, {
        heading,
        note,
        labels,
        inflows: inflows ?? fill(labels, 100),
        outflows: outflows ?? fill(labels, 60),
        net,
        inflowLabel,
        outflowLabel,
        netLabel,
        formatValue: money,
        empty,
        testid: "flow",
    });
}

/** Every bar's fill, in draw order. */
const barFills = (root: HTMLElement): string[] => [...root.querySelectorAll("path.lc-bars-bar")].map((p) => p.getAttribute("fill") ?? "");

/** The free-standing splines — the net line and its halo. */
const splines = (root: HTMLElement): {stroke: string | null; class: string}[] =>
    [...root.querySelectorAll("path.lc-path")].map((p) => ({stroke: p.getAttribute("stroke"), class: p.getAttribute("class") ?? ""}));

const xLabels = (root: HTMLElement): string[] =>
    [...root.querySelectorAll('[data-placement="bottom"] .lc-axis-tick-label')].map((t) => t.textContent?.trim() ?? "");

/**
 * The plot's markup, for comparing one render against another.
 *
 * Scoped to the plot so a legend or heading difference cannot pass for a data
 * difference, and with layerchart's clip-path ids renumbered: those carry a
 * counter that advances with every mount in the file, so two identical charts
 * differ by nothing else. That renumbering is the ONLY thing normalised —
 * everything a mark is made of still has to match exactly.
 */
const plot = (root: HTMLElement): string => (root.querySelector('[data-testid="flow"]')?.innerHTML ?? "").replaceAll(/clipPath--c\d+/g, "clipPath--id");

describe("COMPONENT PeriodFlowChart", () => {
    describe("the caller supplies magnitudes; the chart supplies the sign", () => {
        it("draws an outflow the same whether it arrives positive or negative", () => {
            const positive = mount({outflows: fill(MONTHS, 60)});
            const negative = mount({outflows: fill(MONTHS, -60)});

            expect(plot(negative.container)).toBe(plot(positive.container));
            expect(plot(positive.container)).not.toBe("");
        });

        it("does the same for an inflow", () => {
            const positive = mount({inflows: fill(MONTHS, 100)});
            const negative = mount({inflows: fill(MONTHS, -100)});

            expect(plot(negative.container)).toBe(plot(positive.container));
        });

        it("draws one bar per bucket per direction", () => {
            const {container} = mount();
            const fills = barFills(container);

            expect(fills.filter((f) => f === FLOW_IN)).toHaveLength(MONTHS.length);
            expect(fills.filter((f) => f === FLOW_OUT)).toHaveLength(MONTHS.length);
        });
    });

    describe("net", () => {
        it("defaults to inflow − outflow", () => {
            const implicit = mount({inflows: fill(MONTHS, 100), outflows: fill(MONTHS, 60)});
            const explicit = mount({inflows: fill(MONTHS, 100), outflows: fill(MONTHS, 60), net: fill(MONTHS, 40)});

            expect(plot(explicit.container)).toBe(plot(implicit.container));
        });

        it("takes a supplied net literally rather than reconciling it with the bars", () => {
            const implicit = mount({inflows: fill(MONTHS, 100), outflows: fill(MONTHS, 60)});
            const supplied = mount({inflows: fill(MONTHS, 100), outflows: fill(MONTHS, 60), net: fill(MONTHS, -25)});

            expect(plot(supplied.container)).not.toBe(plot(implicit.container));
        });

        it("draws the line twice — a surface halo under the ink — so it stays legible over a bar", () => {
            const {container} = mount();
            const drawn = splines(container);

            expect(drawn).toHaveLength(2);
            expect(drawn[0].class).toContain("stroke-base-200");
            expect(drawn[1].stroke).toBe(FLOW_NET);
            expect(drawn.every((s) => s.class.includes("[stroke-dasharray:5_3]"))).toBe(true);
        });
    });

    describe("marks", () => {
        it("colours the bars from the diverging pair, never from a categorical slot", () => {
            const {container} = mount();
            const used = new Set(barFills(container));

            expect([...used].sort()).toEqual([FLOW_OUT, FLOW_IN].sort());
            expect(CATEGORICAL.some((slot) => used.has(slot))).toBe(false);
        });

        it("outlines no bar, because a border around a mark is ink that is not data", () => {
            const {container} = mount();
            const outlined = [...container.querySelectorAll("path.lc-bars-bar")].filter((p) => {
                const stroke = p.getAttribute("stroke");
                return stroke !== null && stroke !== "none" && p.getAttribute("strokeWidth") !== "0";
            });

            expect(outlined).toHaveLength(0);
        });

        it("draws the zero rule the bars grow from", () => {
            const {container} = mount();

            expect(container.querySelector(".lc-rule-y-line")).not.toBeNull();
        });
    });

    describe("x axis", () => {
        it("labels whole buckets only, about six of them", () => {
            const {container} = mount();

            expect(xLabels(container)).toEqual(["Jan", "Mar", "May", "Jul", "Sep", "Nov", "Dec"]);
        });
    });

    describe("legend", () => {
        it("is always present and names all three marks", () => {
            mount();
            const legend = document.querySelector('[data-testid="flow-legend"]');

            expect(legend?.textContent).toContain("Money in");
            expect(legend?.textContent).toContain("Money out");
            expect(legend?.textContent).toContain("Net");
        });

        it("uses the caller's words when it has some — the Cash Flow report says it differently", () => {
            mount({inflowLabel: "Cash in", outflowLabel: "Cash out", netLabel: "Net change"});
            const legend = document.querySelector('[data-testid="flow-legend"]');

            expect(legend?.textContent).toContain("Cash in");
            expect(legend?.textContent).toContain("Cash out");
            expect(legend?.textContent).toContain("Net change");
        });
    });

    describe("nothing to draw", () => {
        it("says so rather than drawing twelve empty bands", () => {
            mount({inflows: fill(MONTHS, 0), outflows: fill(MONTHS, 0), empty: "No activity in this range."});

            expect(document.body.textContent).toContain("No activity in this range.");
            expect(document.querySelector('[data-testid="flow"]')).toBeNull();
        });

        it("draws when the bars are flat but the net is not", () => {
            mount({inflows: fill(MONTHS, 0), outflows: fill(MONTHS, 0), net: fill(MONTHS, 5)});

            expect(document.querySelector('[data-testid="flow"]')).not.toBeNull();
        });
    });

    it("heads the chart, and sets the note off from it", () => {
        mount({heading: "Net income", note: "next 24 months"});
        const h3 = document.querySelector("h3");

        expect(h3?.textContent).toContain("Net income");
        expect(h3?.textContent).toContain("· next 24 months");
    });
});
