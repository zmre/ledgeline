// The statement of cash flows, mounted over a literal statement (plans/27):
// the section layout, the columns, folding, the cash detail kept open, and the
// warning that only appears when a posting date splits a transaction.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {CASH_FLOW, CASH_FLOW_TOTAL} from "#lib/testing/cashFlowFixture.js";
import CashFlowStatementView from "./CashFlowStatementView.svelte";

const openJournal = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("#lib/journal/openJournal.js", () => ({openJournal}));

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

afterEach(() => {
    openJournal.mockClear();
    keymap.reset();
});

const STYLES = new Map();
const mount = (report = CASH_FLOW) => render(CashFlowStatementView, {props: {report, styles: STYLES, base: "$"}});

/** The row labels of one block, in screen order. */
const rowLabels = (testid: string): string[] =>
    [...screen.getByTestId(testid).querySelectorAll("tbody tr th")].map((th) => th.textContent?.replace(/▶/g, "").trim() ?? "");

describe("COMPONENT CashFlowStatementView", () => {
    it("lays out operating, investing and financing, net income first", () => {
        mount();
        expect(screen.getByTestId("cf-section-operating")).toBeDefined();
        expect(screen.getByTestId("cf-section-investing")).toBeDefined();
        expect(screen.getByTestId("cf-section-financing")).toBeDefined();
        expect(rowLabels("cf-section-operating")[0]).toBe("Net income");
        expect(screen.getByTestId("cf-section-operating").textContent).toContain("Net cash from operating activities");
    });

    it("prints one column per period plus a Total, and a single Amount on Interval = Total", () => {
        const {unmount} = mount();
        const heads = [...screen.getByTestId("cf-section-operating").querySelectorAll("thead th")].map((th) => th.textContent?.trim());
        expect(heads.slice(1)).toEqual(["Jan 2026", "Feb 2026", "Mar 2026", "Total"]);
        unmount();

        mount(CASH_FLOW_TOTAL);
        const single = [...screen.getByTestId("cf-section-operating").querySelectorAll("thead th")].map((th) => th.textContent?.trim());
        expect(single.slice(1)).toEqual(["Amount"]);
    });

    it("reads the net change, and the beginning and ending cash with the detail open", () => {
        mount();
        expect(screen.getByTestId("cf-net-change").textContent).toContain("$9,500.00");
        expect(rowLabels("cf-cash-beginning")).toEqual([
            "Beginning cash",
            "Operating cash",
            "assets:bank:checking",
            "assets:bank:savings",
            "assets:petty-cash",
        ]);
        expect(screen.getByTestId("cf-cash-ending").textContent).toContain("$19,500.00");
    });

    it("opens a group line to its accounts", async () => {
        mount();
        expect(rowLabels("cf-section-operating")).not.toContain("liabilities:card");
        await fireEvent.click(screen.getByRole("button", {name: /Payables/}));
        expect(rowLabels("cf-section-operating")).toContain("liabilities:card");
    });

    it("folds the cash detail under both blocks at once", async () => {
        mount();
        const [beginning] = screen.getAllByRole("button", {name: /Operating cash/});
        await fireEvent.click(beginning);
        expect(rowLabels("cf-cash-beginning")).toEqual(["Beginning cash", "Operating cash", "assets:petty-cash"]);
        expect(rowLabels("cf-cash-ending")).toEqual(["Ending cash", "Operating cash", "assets:petty-cash"]);
    });

    it("warns only when the statement does not reconcile column by column", () => {
        const {unmount} = mount();
        expect(screen.queryByTestId("cf-unreconciled")).toBeNull();
        unmount();
        mount({...CASH_FLOW, reconciles: false});
        expect(screen.getByTestId("cf-unreconciled").textContent).toContain("posting date");
    });

    it("leaves out the exchange-rate line when there is none", () => {
        mount();
        expect(screen.queryByTestId("cf-exchange")).toBeNull();
    });

    it("walks rows with j, opens a line with Enter, and drills an account into the journal for the report's dates", async () => {
        mount();
        await press("j"); // Net income: nothing to open or drill
        await press("Enter");
        expect(openJournal).not.toHaveBeenCalled();

        await press("j"); // accumulated depreciation, a single account
        await press("Enter");
        expect(openJournal).toHaveBeenCalledWith({accounts: ["assets:equipment:accum-depreciation"], from: "2026-01-01", to: "2026-03-31"});

        await press("j");
        await press("j");
        await press("j"); // Payables, a group line
        await press("Enter");
        expect(rowLabels("cf-section-operating")).toContain("liabilities:payable");
    });
});
