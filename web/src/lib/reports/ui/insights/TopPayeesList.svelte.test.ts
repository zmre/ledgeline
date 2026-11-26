// Top Payees (Box 11), mounted: each row shows the current amount, the
// previous one and the change, links into the journal filtered to the payee
// over the current period, and a payee new this period says so instead of
// showing a percent.

import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {AmountStyle, ISODate} from "#lib/domain/types.js";
import type {PayeeRow} from "#lib/reports/insightsTypes.js";
import {expectHelp} from "#lib/testing/helpTips.js";
import {METRIC_HELP} from "./metricHelp";
import TopPayeesList from "./TopPayeesList.svelte";

vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const styles = new Map([["$", STYLE]]);

const ROWS: PayeeRow[] = [
    {payee: "Wells Fargo", current: dec(88000, 2), previous: dec(90000, 2), delta: dec(-2000, 2), pct: -2.2222},
    {payee: "Costco", current: dec(70000, 2), previous: dec(50000, 2), delta: dec(20000, 2), pct: 40},
    {payee: "Trader Joe's", current: dec(6000, 2), previous: null, delta: null, pct: null},
];

function mount(rows: PayeeRow[]): void {
    render(TopPayeesList, {rows, base: "$", styles, from: "2025-07-01" as ISODate, to: "2025-12-31" as ISODate, testid: "insights-box-toppayees"});
}

describe("COMPONENT TopPayeesList", () => {
    it("lists payees in the engine's order with current, previous and change", () => {
        mount(ROWS);
        const rows = screen.getAllByTestId("top-payee-row");
        expect(rows.map((row) => within(row).getByRole("link").querySelector(".font-medium")?.textContent)).toEqual(["Wells Fargo", "Costco", "Trader Joe's"]);

        const costco = rows[1];
        expect(costco.textContent).toContain("$700.00");
        expect(costco.textContent).toContain("prev $500.00");
        expect(costco.textContent).toContain("▲ $200.00 (40.0%)");
        // Spending up is the bad direction.
        expect(within(costco).getByText("▲ $200.00 (40.0%)").className).toContain("text-error");
        expect(within(rows[0]).getByText("▼ $20.00 (2.2%)").className).toContain("text-success");
    });

    it("marks a payee new this period instead of inventing a percent", () => {
        mount(ROWS);
        const fresh = screen.getAllByTestId("top-payee-row")[2];
        expect(fresh.textContent).toContain("$60.00");
        expect(fresh.textContent).toContain("new");
        expect(fresh.textContent).not.toContain("%");
        expect(fresh.textContent).not.toContain("prev");
    });

    it("links each row to the journal, filtered to the payee over the current period", () => {
        mount(ROWS);
        const link = within(screen.getAllByTestId("top-payee-row")[2]).getByRole("link");
        const url = new URL(link.getAttribute("href") ?? "", "http://app");
        expect(url.pathname).toBe("/");
        expect(url.searchParams.get("q")).toBe("Trader Joe's");
        expect(url.searchParams.get("from")).toBe("2025-07-01");
        expect(url.searchParams.get("to")).toBe("2025-12-31");
    });

    it("has its help, and says so plainly when there is no spending", () => {
        mount([]);
        const box = screen.getByTestId("insights-box-toppayees");
        expectHelp(box, METRIC_HELP.toppayees);
        expect(box.textContent).toContain("No spending this period");
        expect(screen.queryAllByTestId("top-payee-row")).toHaveLength(0);
    });
});
