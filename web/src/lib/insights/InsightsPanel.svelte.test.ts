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
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {Amount, AmountStyle, Posting, Transaction} from "#lib/domain/types.js";
import {filters} from "#lib/stores/filters.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {running} from "#lib/stores/running.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE, FAKE_JOURNAL_INFO} from "#lib/testing/fakeEngine.js";
import {balanceSeriesWire} from "#lib/testing/balanceSeriesFixture.js";
import {showRows, summaryBody} from "#lib/testing/journalViewFixture.js";
import InsightsPanel from "./InsightsPanel.svelte";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style: STYLE});

let nextIndex = 0;
function txn(date: string, postings: [string, Amount][]): Transaction {
    nextIndex += 1;
    const built: Posting[] = postings.map(([account, amount]) => ({account, amounts: [amount], status: "unmarked", comment: "", tags: []}));
    return {index: nextIndex, date, status: "cleared", description: "t", code: "", comment: "", tags: [], postings: built};
}

const TXNS: Transaction[] = [
    txn("2026-02-11", [
        ["income:salary", usd(-300_000)],
        ["assets:bank:checking", usd(300_000)],
    ]),
    txn("2026-02-10", [
        ["expenses:food", usd(20_000)],
        ["assets:bank:checking", usd(-20_000)],
    ]),
    txn("2026-01-02", [
        ["assets:bank:checking", usd(500_000)],
        ["equity:opening", usd(-500_000)],
    ]),
];

const STYLE_WIRE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
/** What the engine summarizes TXNS as: $3,000 of income, $200 spent, $2,800 net. */
const SUMMARY = summaryBody(3, {
    visibleNet: [{commodity: "$", qty: {m: "280000", p: 2}, style: STYLE_WIRE}],
    commodities: ["$"],
    perCommodity: [
        {
            commodity: "$",
            style: STYLE_WIRE,
            income: {m: "300000", p: 2},
            expenses: {m: "20000", p: 2},
            net: {m: "280000", p: 2},
            categories: ["expense", "revenue", "asset", "equity"],
        },
    ],
    maxDepth: 3,
    primary: "$",
});
const CHART = {generation: "fake-1", commodity: "$", groups: ["expenses:food"], pie: [{account: "expenses:food", value: 200, formatted: "$200.00"}]};

/** The line for the account the tests pin. */
const SERIES = balanceSeriesWire({account: "assets:bank:checking"});

const mount = () => render(InsightsPanel);
const tab = (name: string): HTMLElement => screen.getByRole("tab", {name});

/** What the engine answers for TXNS as of today: $5,000 opening, less $200 spent, plus $3,000 of salary. */
const CASH_VIEW = {
    asOf: "2026-09-13",
    commodities: ["$"],
    summaries: [
        {
            commodity: "$",
            style: STYLE_WIRE,
            cash: [{account: "assets:bank:checking", qty: {m: "780000", p: 2}, asOf: "2026-02-11", confirmed: false}],
            liabilities: [],
            totalCash: {m: "780000", p: 2},
            totalLiabilities: {m: "0", p: 0},
            net: {m: "780000", p: 2},
        },
    ],
    guessedLongTerm: [],
};

beforeEach(async () => {
    settings.insightsOpen = true;
    settings.insightsTab = "activity";
    // The Balances tab reads the engine, keyed on the journal generation, so
    // the journal round has to have run once against the fake engine.
    await connectFakeEngine({"/api/balances/cash": CASH_VIEW});
    await journal.refresh({force: true});
    // The Activity tab reads the journal VIEW: the engine's summary and chart.
    await showRows(TXNS, {
        "/version": "1.52",
        "/api/journal": FAKE_JOURNAL_INFO,
        "/api/journal/summary": SUMMARY,
        "/api/journal/chart": CHART,
        "/api/balances/cash": CASH_VIEW,
        "/api/journal/running/series": SERIES,
    });
});

afterEach(() => vi.unstubAllGlobals());

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
        // Activity: income − expenses over the filtered period (the engine's summary).
        expect(header().textContent).toContain("Net");
        await vi.waitFor(() => expect(header().textContent).toContain("$2,800.00"));

        tab("Balances").click();
        await screen.findByTestId("balances-view");
        // Balances: cash + short-term debt as of today, over the whole journal —
        // a different question, so deliberately a different number (the
        // engine's, see CASH_VIEW).
        expect(header().textContent).toContain("Net cash");
        await vi.waitFor(() => expect(header().textContent).toContain("$7,800.00"));

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

    describe("the pinned account's balance line (amendment A1)", () => {
        /** Every `/api/journal/running/series` URL fetched since the last reset. */
        let seriesCalls: string[] = [];
        beforeEach(() => {
            filters.reset();
            seriesCalls = [];
            const answer = globalThis.fetch;
            vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
                const url = String(input);
                if (url.includes("/api/journal/running/series")) seriesCalls.push(url);
                return answer(input, init);
            });
        });

        it("is fetched only while the Balances tab is open and an account is pinned", async () => {
            mount();
            filters.pin("assets:bank:checking");
            await tick();
            // Activity is the active tab: nothing reads the line, so nothing asks.
            expect(seriesCalls).toEqual([]);

            tab("Balances").click();
            await vi.waitFor(() => expect(seriesCalls).toHaveLength(1));
            const url = new URL(seriesCalls[0]);
            expect(url.searchParams.get("account")).toBe("assets:bank:checking");
            expect(url.searchParams.get("today")).toMatch(/^\d{4}-\d{2}-\d{2}$/);
            await vi.waitFor(() => expect(screen.getByTestId("balances-line").querySelector('[data-testid="balance-trend"]')).not.toBeNull());

            // Collapsing the box stops it asking, even when the range moves.
            settings.insightsOpen = false;
            await tick();
            filters.setRange("2026-01-01", "2026-03-31");
            await tick();
            expect(seriesCalls).toHaveLength(1);
        });

        it("follows the journal's date range but not its search", async () => {
            settings.insightsTab = "balances";
            filters.pin("assets:bank:checking", "$");
            mount();
            await vi.waitFor(() => expect(seriesCalls).toHaveLength(1));

            filters.setQuery("coffee");
            await tick();
            expect(seriesCalls).toHaveLength(1);

            filters.setRange("2026-01-01", "2026-03-31");
            await vi.waitFor(() => expect(seriesCalls).toHaveLength(2));
            const url = new URL(seriesCalls[1]);
            expect([url.searchParams.get("from"), url.searchParams.get("to"), url.searchParams.get("commodity")]).toEqual(["2026-01-01", "2026-03-31", "$"]);
        });

        it("does not ask about an account the engine refused, and says why instead of spinning", async () => {
            const answer = globalThis.fetch;
            vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) =>
                String(input).includes("/api/journal/running?")
                    ? Promise.resolve(new Response("no posting is in the account", {status: 400}))
                    : answer(input, init)
            );
            filters.pin("assets:gone");
            await running.load(FAKE_ENGINE, {account: "assets:gone", generation: "fake-1"});
            expect(running.refused("assets:gone")).toBe(true);
            settings.insightsTab = "balances";
            mount();
            await vi.waitFor(() => expect(screen.getByTestId("balances-line").textContent).toContain("Couldn't chart assets:gone"));
            expect(seriesCalls.filter((url) => url.includes("assets%3Agone"))).toEqual([]);
        });

        it("does not ask without a pin", async () => {
            settings.insightsTab = "balances";
            mount();
            await screen.findByTestId("balances-view");
            await tick();
            expect(seriesCalls).toEqual([]);
            expect(screen.queryByTestId("balances-line")).toBeNull();
        });
    });
});
