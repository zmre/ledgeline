// The Insights dashboard's hide rules, mounted (plans/37 decision 4). The rule
// itself is `startup.ts` `boxVisibility` (node-tested); this asks that the
// dashboard applies it: which boxes reach the screen, and that Runway lands in
// Net Worth's slot. Both stores are stubbed with a held payload.

import {render, screen} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {InsightsReport} from "#lib/reports/insightsTypes.js";
import type {SaasReport} from "#lib/reports/saasTypes.js";
import {defaultReportParams} from "#lib/reports/ui/params.js";
import {expectEveryBoxHelped} from "#lib/testing/helpTips.js";
import {householdReport, startupReport} from "#lib/testing/saasFixture.js";
import InsightsDashboard from "./InsightsDashboard.svelte";

const SPAN = {start: "2024-07-01", end: "2026-06-30"};
const held = vi.hoisted(() => ({
    insights: null as unknown,
    saas: null as unknown,
    saasError: false,
    refreshing: false,
    insightsQuery: {start: "2024-07-01", end: "2026-06-30"} as {start: string; end: string},
    saasQuery: {start: "2024-07-01", end: "2026-06-30", asOf: "2026-07-08"} as {start: string; end: string; asOf: string},
}));

vi.mock("$app/paths", () => ({resolve: (path: string) => path}));
vi.mock("#lib/stores/journal.svelte.js", () => ({journal: {changes: 0}}));
vi.mock("#lib/stores/insights.svelte.js", () => ({
    insights: {
        get value() {
            return held.insights;
        },
        get query() {
            return held.insightsQuery;
        },
        view: "data",
        get refreshing() {
            return held.refreshing;
        },
        error: null,
        load: vi.fn(),
    },
}));
vi.mock("#lib/stores/saas.svelte.js", () => ({
    saas: {
        get value() {
            return held.saas;
        },
        get query() {
            return held.saas === null ? null : held.saasQuery;
        },
        get status() {
            return held.saasError ? "error" : held.saas === null ? "loading" : "ready";
        },
        get view() {
            return held.saasError ? "error" : held.saas === null ? "loading" : "data";
        },
        get error() {
            return held.saasError ? new Error("boom") : null;
        },
        load: vi.fn(),
    },
}));

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const styles = new Map([["$", STYLE]]);

const usd = (cents: number) => new Map([["$", dec(cents, 2)]]);
const metric = (cents: number) => ({current: usd(cents), previous: usd(cents), delta: usd(0), pct: 0});

/** A household's insights: positive net worth, holdings, one mover. */
function insightsReport(over: Partial<InsightsReport> = {}): InsightsReport {
    return {
        period: {
            start: "2024-07-01",
            mid: "2025-06-30",
            end: "2026-06-30",
            prevStart: "2024-07-01",
            prevEnd: "2025-06-30",
            currStart: "2025-07-01",
            currEnd: "2026-06-30",
        },
        base: "$",
        journalStart: "2024-07-01",
        revenue: metric(100000),
        expenses: metric(50000),
        netWorth: metric(2000000),
        costOfLiving: {currentTotal: usd(120000), previousTotal: usd(120000), monthsCurrent: 12, monthsPrevious: 12},
        investment: {current: {gain: dec(1000, 2), gainPct: 1}, previous: {gain: dec(500, 2), gainPct: 0.5}},
        cashBalance: metric(500000),
        expenseChanges: [],
        revenueChanges: [],
        movers: [{symbol: "VTI", name: "Vanguard Total", gain: dec(1000, 2), gainPct: 3.1, startEstimated: false}],
        topTxns: [],
        hasHoldings: true,
        ...over,
    };
}

function mount(insights: InsightsReport, saas: SaasReport | null): void {
    held.insights = insights;
    held.saas = saas;
    render(InsightsDashboard, {params: defaultReportParams(), serverUrl: "http://engine", styles});
}

beforeEach(() => {
    held.saasError = false;
    held.refreshing = false;
    held.insightsQuery = {...SPAN};
    held.saasQuery = {...SPAN, asOf: "2026-07-08"};
});

describe("COMPONENT InsightsDashboard — hide rules", () => {
    it("a household journal keeps all of today's boxes and shows no startup section", () => {
        mount(insightsReport(), householdReport());
        for (const box of ["revenue", "expenses", "networth", "costofliving", "investment", "cash", "expensechanges", "movers", "revenuechanges", "toptxns"]) {
            expect(screen.getByTestId(`insights-box-${box}`)).toBeTruthy();
        }
        expect(screen.queryByTestId("insights-box-runway")).toBeNull();
        expect(screen.queryByTestId("startup-metrics")).toBeNull();
    });

    it("a business hides Net Worth, shows Runway, and adds the startup section", () => {
        mount(insightsReport(), startupReport());
        expect(screen.queryByTestId("insights-box-networth")).toBeNull();
        expect(screen.getByTestId("insights-box-runway-big").textContent).toBe("117 months");
        expect(screen.getByTestId("insights-box-runway-small").textContent).toContain(
            "cash-out ~2036-03 · burn $3,693.33/mo adjusted for prepayments · $6,543.33/mo cash (3-mo avg)"
        );
        expect(screen.getByTestId("startup-metrics")).toBeTruthy();
    });

    it("a negative net worth hides Net Worth", () => {
        mount(insightsReport({netWorth: metric(-100)}), householdReport());
        expect(screen.queryByTestId("insights-box-networth")).toBeNull();
        // The household fixture has cash and no burn.
        expect(screen.getByTestId("insights-box-runway-big").textContent).toBe("Not burning");
        expect(screen.getByTestId("insights-box-runway-small").textContent).toBe("cash $50,277.56");
    });

    it("no holdings hides Investment Performance", () => {
        mount(insightsReport({hasHoldings: false}), householdReport());
        expect(screen.queryByTestId("insights-box-investment")).toBeNull();
        expect(screen.getByTestId("insights-box-networth")).toBeTruthy();
    });

    it("no movers hides Biggest Movers", () => {
        mount(insightsReport({movers: []}), householdReport());
        expect(screen.queryByTestId("insights-box-movers")).toBeNull();
        expect(screen.getByTestId("insights-box-toptxns")).toBeTruthy();
    });

    it("keeps Net Worth while the startup metrics are still loading", () => {
        mount(insightsReport(), null);
        expect(screen.getByTestId("insights-box-networth")).toBeTruthy();
        expect(screen.queryByTestId("startup-error")).toBeNull();
    });

    it("a failed /api/saas is a small banner, not a broken dashboard", () => {
        held.saasError = true;
        mount(insightsReport(), null);
        expect(screen.getByTestId("startup-error").textContent).toContain("Couldn't load startup metrics: boom");
        expect(screen.getByTestId("insights-box-networth")).toBeTruthy();
    });

    // The span moved: insights already answers the new one, saas still holds
    // the old one. The old report must not draw this span's boxes.
    it("ignores a startup report held from another span: no old boxes, no stale Runway", () => {
        held.insightsQuery = {start: "2025-07-01", end: "2026-06-30"};
        mount(insightsReport(), startupReport());
        expect(screen.queryByTestId("startup-metrics")).toBeNull();
        expect(screen.queryByTestId("insights-box-arr")).toBeNull();
        expect(screen.queryByTestId("insights-box-runway")).toBeNull();
        // Treated as "not loaded yet": a household reading, quietly.
        expect(screen.getByTestId("insights-box-networth")).toBeTruthy();
        expect(screen.queryByLabelText("Loading startup metrics")).toBeNull();
    });

    it("loads the startup metrics with the browser's local today as asOf", async () => {
        const {saas} = await import("#lib/stores/saas.svelte.js");
        const {today} = await import("#lib/reports/periods.js");
        vi.mocked(saas.load).mockClear();
        mount(insightsReport(), startupReport());
        const params = defaultReportParams();
        expect(saas.load).toHaveBeenCalledWith("http://engine", {start: params.insStart, end: params.insEnd, asOf: today()});
    });
});

describe("COMPONENT InsightsDashboard — (i) help", () => {
    it("every box and list of a household's dashboard has its help", () => {
        mount(insightsReport(), householdReport());
        expect(expectEveryBoxHelped().sort()).toEqual(
            ["cash", "costofliving", "expensechanges", "expenses", "investment", "movers", "networth", "revenue", "revenuechanges", "toptxns"].sort()
        );
    });

    it("a business's Runway and startup boxes have theirs too", () => {
        mount(insightsReport(), startupReport());
        const seen = expectEveryBoxHelped();
        for (const id of ["runway", "arr", "netnewarr", "customers", "nrr", "burnmultiple", "grossmargin"]) expect(seen).toContain(id);
    });
});

// TODO.md "a slow insights refetch shows stale numbers": while a new period
// loads, the previous period's boxes stay (never blank) but must not pass for
// the answer to the period the control now reads.
describe("COMPONENT InsightsDashboard — refreshing", () => {
    it("at rest: no spinner, not busy, not dimmed", () => {
        mount(insightsReport(), householdReport());
        const content = screen.getByTestId("insights-content");
        expect(screen.queryByTestId("insights-refreshing")).toBeNull();
        expect(content.getAttribute("aria-busy")).toBe("false");
        expect(content.classList.contains("opacity-60")).toBe(false);
    });

    it("keeps the old boxes on screen, dimmed and busy, with a status by the period control", () => {
        held.refreshing = true;
        mount(insightsReport(), householdReport());

        // Nothing blanks: the held numbers are still there...
        expect(screen.getByTestId("insights-box-revenue")).toBeTruthy();
        expect(screen.queryByLabelText("Loading insights")).toBeNull();
        // ...inside a region that says it is being replaced.
        const content = screen.getByTestId("insights-content");
        expect(content.contains(screen.getByTestId("insights-box-revenue"))).toBe(true);
        expect(content.getAttribute("aria-busy")).toBe("true");
        expect(content.classList.contains("opacity-60")).toBe(true);

        const status = screen.getByRole("status");
        expect(status.getAttribute("data-testid")).toBe("insights-refreshing");
        expect(status.textContent).toContain("Updating");
        // The spinner is decorative and drops out under reduced motion; the text stays.
        const spinner = status.querySelector(".loading-spinner");
        expect(spinner?.classList.contains("motion-reduce:hidden")).toBe(true);
        expect(spinner?.getAttribute("aria-hidden")).toBe("true");
        // Sits beside the period control, not inside the dimmed region.
        expect(content.contains(status)).toBe(false);
    });
});
