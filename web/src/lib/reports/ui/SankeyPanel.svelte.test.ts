// One flow panel, mounted.
//
// jsdom has no layout engine, so the first half of this file never draws the
// chart: the wrapper measures 0px, and the panel declines to draw a diagram
// into a box it has no width for. What is left is exactly what a reader needs
// to be able to find without the picture, and that is what it asserts: the
// legend names every account, the collapsed state follows the prop, the header
// total appears only while the panel is shut, the shortfall note appears only
// when there is a shortfall, and each empty state gives its own reason.
//
// The second half GIVES the box a size, and only that. Every other number it
// reads is computed: d3-sankey lays the bars out in that box, `layoutLabels`
// places the labels against those bars, and the SVG attributes are what came
// out. The one fabricated input is the box itself, which is the same thing a
// browser would hand the panel, so the claims (labels clear of each other, a
// leader for each one that moved, a hover and a focus that say the figure) are
// about real geometry, not geometry the test chose.
//
// It also pins the reason `AsyncSection` moved INSIDE this component. Wrapped
// from outside, a loading or failed fetch erased the whole panel, so a user who
// had shut it got a spinner block where a shut panel belongs. The header, its
// title and its arrow now survive every branch.
//
// `sankeyModel.test.ts` covers the folding and the colours; this covers the
// wiring.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterAll, beforeAll, describe, expect, it, vi} from "vitest";
import {decodeFlowReport} from "#lib/api/nativeDecode.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {OTHER_LABEL} from "#lib/format/palette.js";
import type {FlowReport} from "#lib/reports/types.js";
import {EMPTY_FLOW_REPORT, FLOW_REPORT, UNPRICEABLE_FLOW_REPORT} from "#lib/testing/flowsFixture.js";
import SankeyPanel from "./SankeyPanel.svelte";
import type {FlowsPanel} from "./sankeyModel";

const STYLES: ReadonlyMap<string, AmountStyle> = new Map<string, AmountStyle>([
    ["$", {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}],
]);

const REPORT = decodeFlowReport(FLOW_REPORT);
const UNPRICEABLE = decodeFlowReport(UNPRICEABLE_FLOW_REPORT);
const EMPTY = decodeFlowReport(EMPTY_FLOW_REPORT);

const ready = (report: FlowReport, retry = () => {}): FlowsPanel => ({view: "data", report, error: null, retry});

interface MountOptions {
    title?: string;
    panel?: FlowsPanel;
    inbound?: boolean;
    open?: boolean;
    onToggle?: (open: boolean) => void;
}

function mount({title = "Money out", panel = ready(REPORT), inbound = false, open = true, onToggle = () => {}}: MountOptions = {}) {
    return render(SankeyPanel, {
        title,
        caption: "Which accounts paid, and what the period spent it on.",
        panel,
        inbound,
        styles: STYLES,
        open,
        onToggle,
    });
}

const toggle = (name: string): HTMLInputElement => screen.getByLabelText(name) as HTMLInputElement;

describe("COMPONENT SankeyPanel", () => {
    it("names every account in the legend, with the folded tail as one entry", () => {
        mount();
        const legend = screen.getByTestId("sankey-legend-money-out");

        for (const label of [
            "Bank: Checking",
            "Credit cards: Visa",
            "Bank: Savings",
            "Bank: Wise: Eur",
            "Credit cards: Amex",
            "Cash: Wallet",
            "Vehicles: Car: Depreciation",
            "Bank: HSA",
            "Bank: Union",
            "Credit cards: Discover",
            "Credit cards: Citi",
            "Loan: Heloc",
            "Cash: Safe",
            OTHER_LABEL,
        ]) {
            expect(legend.textContent).toContain(label);
        }
        // The three folded accounts are named nowhere: they ARE the tail entry.
        expect(legend.textContent).not.toContain("Loan: Auto");
        expect(legend.querySelectorAll("li").length).toBe(14);
    });

    it("keeps the total in the header while shut, where it is the only figure there is", () => {
        mount({open: false});

        expect(toggle("Toggle Money out").checked).toBe(false);
        expect(screen.getByRole("heading", {name: "Money out"}).parentElement?.textContent).toContain("$10,350.00");
    });

    it("drops the header total once the panel is open, because the box below repeats it", () => {
        // The duplicate-total complaint plans/13 exists to fix: expanded, the
        // node labels hold the detail and the statement footer below holds the
        // same figure, so the header adds a second copy of it to one screen.
        // Guarded end to end by smoke.e2e.ts's one-and-only-one count on
        // $28,626.48 inside `income-statement`.
        mount({open: true});

        expect(screen.getByRole("heading", {name: "Money out"}).parentElement?.textContent?.trim()).toBe("Money out");
        expect(screen.getByTestId("sankey-legend-money-out")).toBeDefined();
    });

    it("follows the open prop and reports a toggle back to the caller", () => {
        const onToggle = vi.fn();
        mount({open: true, onToggle});
        const input = toggle("Toggle Money out");

        expect(input.checked).toBe(true);
        input.click();
        expect(onToggle).toHaveBeenCalledWith(false);
    });

    it("says what is drawn against what the box totals, but only when they differ", () => {
        mount({title: "Money in", inbound: true});
        expect(screen.getByTestId("sankey-incomplete-money-in").textContent?.replace(/\s+/g, " ").trim()).toBe("Showing $5,700.00 of $6,000.00");

        // The tied-out graph says nothing, rather than "Showing X of X".
        mount();
        expect(screen.queryByTestId("sankey-incomplete-money-out")).toBeNull();
    });

    it("blames the missing prices when there is no base commodity", () => {
        mount({panel: ready(UNPRICEABLE)});
        const empty = screen.getByTestId("sankey-empty-money-out");

        expect(empty.textContent).toContain("no prices between them");
        expect(screen.queryByTestId("sankey-legend-money-out")).toBeNull();
    });

    it("blames the range when there is a base and nothing happened in it", () => {
        mount({panel: ready(EMPTY)});
        expect(screen.getByTestId("sankey-empty-money-out").textContent).toContain("Nothing in this range");
    });

    it("keeps the header and the arrow while the report is still loading", () => {
        mount({panel: {view: "loading", report: null, error: null, retry: () => {}}});

        // The shell, in full: this is what wrapping the component in
        // <AsyncSection> from outside used to erase.
        expect(screen.getByRole("heading", {name: "Money out"})).toBeDefined();
        expect(toggle("Toggle Money out")).toBeDefined();
        expect(screen.getByLabelText("Loading Money out")).toBeDefined();
        // No figure at all, rather than a zero standing in for an unknown total.
        expect(screen.getByRole("heading", {name: "Money out"}).parentElement?.textContent?.trim()).toBe("Money out");
        expect(screen.queryByTestId("sankey-chart-money-out")).toBeNull();
    });

    it("keeps the header, says it failed, and offers a retry", async () => {
        const retry = vi.fn();
        mount({panel: {view: "error", report: null, error: new Error("connection refused"), retry}});

        expect(screen.getByRole("heading", {name: "Money out"})).toBeDefined();
        expect(toggle("Toggle Money out")).toBeDefined();
        expect(screen.getByTestId("flows-error-money-out").textContent).toContain("connection refused");
        screen.getByRole("button", {name: "Retry"}).click();
        expect(retry).toHaveBeenCalled();
    });

    it("shows the error branch even when a previous report is still held (FE-5)", () => {
        // `createResource` leaves the last good payload in place on failure, so
        // the error branch has to outrank it or a failed refetch silently keeps
        // serving the previous window's diagram.
        mount({panel: {view: "error", report: REPORT, error: new Error("boom"), retry: () => {}}});

        expect(screen.getByTestId("flows-error-money-out")).toBeDefined();
        expect(screen.queryByTestId("sankey-legend-money-out")).toBeNull();
    });
});

/**
 * Money out with ONE paying account and forty cost lines, halving in size down
 * the list: the long tail of bars a pixel tall that piled their labels on top
 * of each other. Statement lines never fold, so all forty are drawn.
 */
function crowdedReport(lines: number): FlowReport {
    const usd = (cents: number) => ({mantissa: String(cents), places: 2});
    const sizes = Array.from({length: lines}, (_, i) => Math.max(100, Math.round(5_000_000 / 1.25 ** i)));
    const total = sizes.reduce((sum, cents) => sum + cents, 0);
    const empty = {nodes: [], links: [], total: usd(0), sectionTotal: usd(0)};
    return decodeFlowReport({
        from: "2026-01-01",
        to: "2026-12-31",
        base: "$",
        inflows: empty,
        outflows: {
            nodes: [
                {key: "a:assets:bank:checking", label: "Bank: Checking", side: "source", account: "assets:bank:checking", total: usd(total)},
                ...sizes.map((cents, i) => ({key: `g:Line ${i}`, label: `Line ${i}`, side: "target", account: null, total: usd(cents)})),
            ],
            links: sizes.map((cents, i) => ({source: "a:assets:bank:checking", target: `g:Line ${i}`, value: usd(cents)})),
            total: usd(total),
            sectionTotal: usd(total),
        },
        meta: {unpriced: []},
    });
}

/** The nearest box with a pixel height set in its style: the panel's chart frame, for anything inside it. */
function styledHeight(element: HTMLElement): number {
    for (let at: HTMLElement | null = element; at !== null; at = at.parentElement) {
        const px = /^(\d+(?:\.\d+)?)px$/.exec(at.style.height);
        if (px !== null) return Number(px[1]);
    }
    return 0;
}

describe("COMPONENT SankeyPanel, drawn into an 800px box", () => {
    const WIDTH = 800;
    const PAD_Y = 8;
    const LINE_GAP = 6;
    const box = (lines: number) => (lines === 2 ? 26 : 14);
    const saved = {
        width: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth"),
        height: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight"),
    };

    beforeAll(() => {
        Object.defineProperty(HTMLElement.prototype, "clientWidth", {configurable: true, get: () => WIDTH});
        Object.defineProperty(HTMLElement.prototype, "clientHeight", {
            configurable: true,
            get(this: HTMLElement) {
                return styledHeight(this);
            },
        });
    });
    afterAll(() => {
        if (saved.width !== undefined) Object.defineProperty(HTMLElement.prototype, "clientWidth", saved.width);
        if (saved.height !== undefined) Object.defineProperty(HTMLElement.prototype, "clientHeight", saved.height);
    });

    interface DrawnLabel {
        key: string;
        lines: number;
        /** The middle of the label's box, read back from its name's `y`. */
        y: number;
        /** The middle of its bar. */
        bar: number;
    }

    /** Every drawn label, with its bar, read from the SVG the panel produced. */
    function drawn(): DrawnLabel[] {
        return [...document.querySelectorAll<SVGGElement>("g[data-label]")].map((group) => {
            const key = group.dataset.label ?? "";
            const lines = Number(group.dataset.lines);
            const name = Number(group.querySelector("text")?.getAttribute("y"));
            const rect = group.parentElement?.querySelector("rect");
            const bar = Number(rect?.getAttribute("y")) + Number(rect?.getAttribute("height")) / 2;
            return {key, lines, y: lines === 2 ? name + LINE_GAP : name, bar};
        });
    }

    it("draws the chart at all, which every assertion below depends on", () => {
        mount({panel: ready(crowdedReport(40))});
        expect(drawn().length).toBe(41);
    });

    it("keeps forty labels a box apart, in bar order, inside the chart", () => {
        mount({panel: ready(crowdedReport(40))});
        const height = styledHeight(screen.getByTestId("sankey-chart-money-out"));
        // The cost column: every label but the one account's.
        const column = drawn().filter((label) => label.key.startsWith("g:"));
        const byBar = column.slice().sort((a, b) => a.bar - b.bar);
        const byLabel = column.slice().sort((a, b) => a.y - b.y);

        expect(byLabel.map((label) => label.key)).toEqual(byBar.map((label) => label.key));
        for (let i = 1; i < byLabel.length; i++) {
            const [above, below] = [byLabel[i - 1], byLabel[i]];
            expect(below.y - above.y, `${above.key} / ${below.key}`).toBeGreaterThanOrEqual((box(above.lines) + box(below.lines)) / 2 - 1e-6);
        }
        for (const label of byLabel) {
            expect(label.y - box(label.lines) / 2).toBeGreaterThanOrEqual(-PAD_Y - 1e-6);
            expect(label.y + box(label.lines) / 2).toBeLessThanOrEqual(height - PAD_Y + 1e-6);
        }
        // Forty two-line labels need 1040px of the usual 900, so the smallest
        // lines gave up their figures, and the biggest kept theirs.
        expect(height).toBe(900);
        expect(column.find((label) => label.key === "g:Line 0")?.lines).toBe(2);
        expect(column.find((label) => label.key === "g:Line 39")?.lines).toBe(1);
    });

    it("grows past 900px only as far as one-line labels need, and stops at 1600", () => {
        mount({panel: ready(crowdedReport(80))});
        expect(styledHeight(screen.getByTestId("sankey-chart-money-out"))).toBe(80 * 14);
        expect(
            drawn()
                .filter((label) => label.key.startsWith("g:"))
                .every((label) => label.lines === 1)
        ).toBe(true);
    });

    it("stops growing at 1600px however many lines there are", () => {
        mount({panel: ready(crowdedReport(130))});
        expect(styledHeight(screen.getByTestId("sankey-chart-money-out"))).toBe(1600);
    });

    it("keeps an ordinary P&L at the height it always had", () => {
        // Money out's taller column has 14 bars: 14 × 30px, as before.
        mount();
        expect(styledHeight(screen.getByTestId("sankey-chart-money-out"))).toBe(14 * 30);
    });

    it("draws a leader line for every label that left its bar, and none for one that did not", () => {
        mount({panel: ready(crowdedReport(40))});
        const labels = drawn();
        const moved = labels.filter((label) => Math.abs(label.y - label.bar) > 2).map((label) => label.key);
        const leaders = [...document.querySelectorAll<SVGPathElement>("path[data-leader]")].map((path) => path.dataset.leader);

        // Forty bars this crowded cannot all keep their place.
        expect(moved.length).toBeGreaterThan(0);
        expect(leaders.sort()).toEqual(moved.sort());
    });

    it("leaves a roomy graph's labels on their bars, two lines each", () => {
        // Money in: two revenue lines and three accounts, nothing crowded.
        mount({title: "Money in", inbound: true});
        const labels = drawn();

        expect(labels.length).toBeGreaterThan(0);
        expect(document.querySelectorAll("path[data-leader]").length).toBe(0);
        expect(labels.every((label) => label.lines === 2)).toBe(true);
    });

    it("answers a hover over a bar with its figure and share, and drops it on leave", async () => {
        mount();
        const food = screen.getByRole("img", {name: /^Food:/});

        await fireEvent.pointerEnter(food);
        const tip = screen.getByRole("tooltip");
        expect(tip.textContent).toContain("Food");
        expect(tip.textContent).toContain("$2,700.00");
        expect(tip.textContent).toContain("26.1% of the total");

        await fireEvent.pointerLeave(food);
        expect(screen.queryByRole("tooltip")).toBeNull();
    });

    it("answers a hover over a ribbon with both ends and its share of the source", async () => {
        mount();
        const ribbon = document.querySelector<SVGGElement>("g[aria-hidden='true'] path")?.parentElement;
        if (ribbon === null || ribbon === undefined) throw new Error("no ribbon drawn");

        await fireEvent.pointerEnter(ribbon);
        expect(screen.getByRole("tooltip").textContent).toMatch(/→.*\$[\d,.]+.*% of /s);
    });

    it("shows the same tooltip on keyboard focus, and Escape or blur hides it", async () => {
        mount();
        const food = screen.getByRole("img", {name: "Food: $2,700.00, 26.1% of the total"});

        expect(food.getAttribute("tabindex")).toBe("0");
        food.focus();
        await Promise.resolve();
        expect(screen.getByRole("tooltip").textContent).toContain("$2,700.00");

        await fireEvent.keyDown(window, {key: "Escape"});
        expect(screen.queryByRole("tooltip")).toBeNull();

        food.blur();
        food.focus();
        await Promise.resolve();
        expect(screen.getByRole("tooltip")).toBeDefined();
        food.blur();
        await Promise.resolve();
        expect(screen.queryByRole("tooltip")).toBeNull();
    });

    it("leaves no native <title> to double up with the tooltip", () => {
        mount();
        expect(drawn().length).toBeGreaterThan(0);
        expect(document.querySelectorAll("svg title").length).toBe(0);
    });
});
