// One flow panel, mounted.
//
// jsdom has no layout engine, so nothing here asks how anything LOOKS, and the
// chart itself never renders: the wrapper measures 0px, and the panel declines
// to draw a diagram into a box it has no width for. What is left is exactly
// what a reader needs to be able to find without the picture, and that is what
// this file asserts: the legend names every account, the collapsed state
// follows the prop, the shortfall note appears only when there is a shortfall,
// and each empty state gives its own reason.
//
// It also pins the reason `AsyncSection` moved INSIDE this component. Wrapped
// from outside, a loading or failed fetch erased the whole panel, so a user who
// had shut it got a spinner block where a shut panel belongs. The header, its
// title and its arrow now survive every branch.
//
// `sankeyModel.test.ts` covers the folding and the colours; this covers the
// wiring.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {decodeFlowReport} from "$lib/api/nativeDecode";
import type {AmountStyle} from "$lib/domain/types";
import {OTHER_LABEL} from "$lib/format/palette";
import type {FlowReport} from "$lib/reports/types";
import {EMPTY_FLOW_REPORT, FLOW_REPORT, UNPRICEABLE_FLOW_REPORT} from "$lib/testing/flowsFixture";
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
            OTHER_LABEL,
        ]) {
            expect(legend.textContent).toContain(label);
        }
        // The three folded accounts are named nowhere: they ARE the tail entry.
        expect(legend.textContent).not.toContain("Loan: Auto");
        expect(legend.querySelectorAll("li").length).toBe(8);
    });

    it("keeps the total in the header, where it stays visible when collapsed", () => {
        mount({open: false});

        expect(toggle("Toggle Money out").checked).toBe(false);
        expect(screen.getByRole("heading", {name: "Money out"}).parentElement?.textContent).toContain("$10,350.00");
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
