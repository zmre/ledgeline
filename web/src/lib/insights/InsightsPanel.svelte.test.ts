// The insights box's tab strip, mounted.
//
// The two views are tested in their own files (BalancesView.svelte.test.ts and
// series.test.ts); what is only testable here is the SWITCH — that exactly one
// view is mounted at a time, that the collapsed header's figure follows the
// chosen tab, and that the tab strip lives somewhere clicking it does not
// collapse the panel.
//
// That last one is not fussiness. daisyUI's `collapse` lays its toggle checkbox
// over the whole title row, so a tab button placed beside the heading would
// have swallowed its own click and shut the box instead.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {beforeEach, describe, expect, it} from "vitest";
import type {AccountDecl} from "$lib/domain/accountTypes";
import {dec} from "$lib/domain/money";
import type {Amount, AmountStyle, Posting, Transaction} from "$lib/domain/types";
import {settings} from "$lib/stores/settings.svelte";
import InsightsPanel from "./InsightsPanel.svelte";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: STYLE});

let nextIndex = 0;
function txn(date: string, postings: [string, Amount][]): Transaction {
    nextIndex += 1;
    const built: Posting[] = postings.map(([account, amount]) => ({account, amounts: [amount], status: "unmarked", comment: "", tags: []}));
    return {index: nextIndex, date, status: "cleared", description: "t", code: "", comment: "", tags: [], postings: built, haystack: "t"};
}

const DECLS: AccountDecl[] = [
    {name: "assets:bank:checking", type: "cash"},
    {name: "liabilities:card", type: "liability"},
];

const TXNS: Transaction[] = [
    txn("2026-01-02", [
        ["assets:bank:checking", usd(500_000)],
        ["equity:opening", usd(-500_000)],
    ]),
    txn("2026-02-10", [
        ["expenses:food", usd(20_000)],
        ["assets:bank:checking", usd(-20_000)],
    ]),
    txn("2026-02-11", [
        ["income:salary", usd(-300_000)],
        ["assets:bank:checking", usd(300_000)],
    ]),
];

const mount = () => render(InsightsPanel, {txns: TXNS, allTxns: TXNS, decls: DECLS});
const tab = (name: string): HTMLElement => screen.getByRole("tab", {name});

beforeEach(() => {
    settings.insightsOpen = true;
    settings.insightsTab = "activity";
});

describe("COMPONENT InsightsPanel tabs", () => {
    it("opens on Activity, the P&L view, as the default", () => {
        mount();
        expect(tab("Activity").getAttribute("aria-selected")).toBe("true");
        expect(tab("Balances").getAttribute("aria-selected")).toBe("false");
        expect(screen.queryByTestId("balances-view")).toBeNull();
        expect(screen.queryByLabelText("Chart mode")).not.toBeNull();
    });

    it("swaps to the balances view, unmounting the charts rather than hiding them", async () => {
        mount();
        tab("Balances").click();
        expect(await screen.findByTestId("balances-view")).not.toBeNull();
        // The chart and the depth slider are GONE, not merely off screen: their
        // whole-journal passes are what the panel's mount guard exists to avoid
        // paying for.
        expect(screen.queryByLabelText("Chart mode")).toBeNull();
        expect(screen.queryByLabelText("Account depth")).toBeNull();
    });

    it("remembers the tab across mounts", async () => {
        const first = mount();
        tab("Balances").click();
        await screen.findByTestId("balances-view");
        first.unmount();
        mount();
        expect(await screen.findByTestId("balances-view")).not.toBeNull();
        expect(settings.insightsTab).toBe("balances");
    });

    it("shows the active view's headline figure in the header, and keeps it when collapsed", async () => {
        mount();
        const header = (): HTMLElement => document.querySelector(".collapse-title") as HTMLElement;
        // Activity: income − expenses over the filtered period.
        expect(header().textContent).toContain("Net");
        expect(header().textContent).toContain("$2,800.00");

        tab("Balances").click();
        await screen.findByTestId("balances-view");
        // Balances: cash + short-term debt as of today, over the whole journal —
        // a different question, so deliberately a different number. $5,000
        // opening, less $200 spent, plus $3,000 of salary.
        expect(header().textContent).toContain("Net cash");
        expect(header().textContent).toContain("$7,800.00");

        settings.insightsOpen = false;
        await tick();
        // The body unmounts; the headline does not — that is the point of a box
        // that collapses rather than one that hides.
        expect(header().textContent).toContain("Net cash");
        expect(screen.queryByTestId("balances-view")).toBeNull();
    });

    it("keeps the tab strip out of the collapse title, whose checkbox would eat the click", () => {
        mount();
        const title = document.querySelector(".collapse-title") as HTMLElement;
        expect(title).not.toBeNull();
        expect(title.querySelector("[role='tab']")).toBeNull();
    });
});
