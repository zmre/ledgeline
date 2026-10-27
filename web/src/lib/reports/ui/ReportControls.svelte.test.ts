// The report controls bar, mounted per tab (plans/24): which controls each tab
// shows, which Interval options it offers, and which params its From/To edit.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {defaultReportParams, type ReportParams, type ReportTab} from "./params";
import ReportControls from "./ReportControls.svelte";

const mount = (tab: ReportTab, patch: Partial<ReportParams> = {}) => {
    const params: ReportParams = {...defaultReportParams("2026-07-08"), tab, ...patch};
    render(ReportControls, {params, maxDepth: 4});
    return params;
};

/** The Interval select's options, as labelled. */
const intervalOptions = (): string[] => [...screen.getByLabelText<HTMLSelectElement>("Interval").options].map((option) => option.textContent?.trim() ?? "");

describe("COMPONENT ReportControls", () => {
    it("gives the P&L From, To and an Interval that offers Total first", () => {
        mount("is", {interval: "total"});
        expect(screen.getByLabelText("From")).toBeDefined();
        expect(screen.getByLabelText("To")).toBeDefined();
        expect(intervalOptions()).toEqual(["Total", "Monthly", "Quarterly", "Yearly"]);
        expect(screen.getByLabelText<HTMLSelectElement>("Interval").value).toBe("total");
        expect(screen.queryByLabelText("Number of periods")).toBeNull();
    });

    it("gives cash flow From/To over its OWN range, an Interval with Total, and no End or Periods", () => {
        mount("cf");
        expect(screen.getByLabelText<HTMLInputElement>("From").value).toBe("2025-08-01");
        expect(screen.getByLabelText<HTMLInputElement>("To").value).toBe("2026-07-08");
        expect(intervalOptions()).toEqual(["Total", "Monthly", "Quarterly", "Yearly"]);
        expect(screen.queryByLabelText("End")).toBeNull();
        expect(screen.queryByLabelText("Number of periods")).toBeNull();
    });

    it("writes cash flow's dates to cfFrom/cfTo, leaving the P&L's range alone", async () => {
        const params = mount("cf");
        await fireEvent.change(screen.getByLabelText("From"), {target: {value: "2026-01-15"}});
        expect(params.cfFrom).toBe("2026-01-15");
        expect(params.from).toBe("2026-01-01");
    });

    it("opens the balance sheet on As of alone, with a Single date interval first", () => {
        mount("bs", {interval: "total"});
        expect(screen.getByLabelText<HTMLInputElement>("As of").value).toBe("2026-07-08");
        expect(intervalOptions()).toEqual(["Single date", "Monthly", "Quarterly", "Yearly"]);
        expect(screen.getByLabelText<HTMLSelectElement>("Interval").value).toBe("total");
        expect(screen.queryByLabelText("From")).toBeNull();
        expect(screen.queryByLabelText("To")).toBeNull();
        expect(screen.queryByLabelText("Number of periods")).toBeNull();
    });

    it("switches the balance sheet to multi-period and back, showing From only while it is on", async () => {
        // `$state`, so the bar re-renders on its own writes, as it does on the page.
        const params = $state<ReportParams>({...defaultReportParams("2026-07-08"), tab: "bs", interval: "total"});
        render(ReportControls, {params, maxDepth: 4});

        await fireEvent.change(screen.getByLabelText("Interval"), {target: {value: "quarterly"}});
        expect(params.interval).toBe("quarterly");
        const from = screen.getByLabelText<HTMLInputElement>("From");
        // The sheet's own start, the trailing year to As of; never later than As of.
        expect(from.value).toBe("2025-08-01");
        expect(from.max).toBe("2026-07-08");
        // From comes before As of, as a range reads.
        expect(from.compareDocumentPosition(screen.getByLabelText("As of")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

        await fireEvent.change(from, {target: {value: "2026-01-01"}});
        expect(params.bsFrom).toBe("2026-01-01");
        // The P&L's From is its own.
        expect(params.from).toBe("2026-01-01");
        expect(params.cfFrom).toBe("2025-08-01");

        await fireEvent.change(screen.getByLabelText("Interval"), {target: {value: "total"}});
        expect(params.interval).toBe("total");
        expect(screen.queryByLabelText("From")).toBeNull();
        // The chosen start is kept for the next switch.
        expect(params.bsFrom).toBe("2026-01-01");
    });

    it("leaves net worth on End + Periods with no Total on offer", () => {
        mount("nw", {interval: "yearly"});
        expect(screen.getByLabelText("End")).toBeDefined();
        expect(screen.getByLabelText("Number of periods")).toBeDefined();
        expect(screen.queryByLabelText("From")).toBeNull();
        expect(intervalOptions()).toEqual(["Monthly", "Quarterly", "Yearly"]);
    });
});
