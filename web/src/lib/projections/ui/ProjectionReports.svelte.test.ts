// The three report tabs, mounted.
//
// What only a mount can answer is what each surface was HANDED and whether it
// said it:
//
//   · The runway sentence is the single most load-bearing string this tab
//     produces, and it has to be on screen ABOVE the chart rather than left for
//     a reader to find by eye on a line. That is a claim about a mounted
//     component and about nothing else.
//   · The three readings are one report read three ways, so the chart's bucket
//     labels have to be the table's.
//   · The zero rule and the crossing marker belong to the cash tab alone.
//
// Which NUMBERS reach the charts is `projectionView.test.ts`'s — the depth-1
// split, the magnitudes, and the fact that revenue arrives positive and is not
// flipped again are all decisions in a pure function, and a pure function is
// tested by calling it (web/README.md).
//
// Nothing asserts on geometry: jsdom has no layout engine, and every coordinate
// a chart emits in a 0x0 box is meaningless.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import {DEFAULT_AMOUNT_STYLE} from "$lib/format/amounts";
import type {PeriodReport} from "$lib/reports/types";
import type {Projection} from "../types";
import ProjectionReports from "./ProjectionReports.svelte";

const usd = (n: number): MixedAmount => new Map([["$", dec(n, 0)]]);
const STYLES: ReadonlyMap<string, AmountStyle> = new Map([["$", DEFAULT_AMOUNT_STYLE]]);

const NET_INCOME: PeriodReport = {
    buckets: ["2026-08", "2026-09"],
    rows: [
        {account: "expenses", depth: 1, values: [usd(-4200), usd(-4200)]},
        {account: "expenses:rent", depth: 2, values: [usd(-4200), usd(-4200)]},
        {account: "income", depth: 1, values: [usd(9000), usd(9000)]},
        {account: "income:salary", depth: 2, values: [usd(9000), usd(9000)]},
    ],
    totals: [usd(4800), usd(4800)],
};

function projection(overrides: Partial<Projection> = {}): Projection {
    return {
        buckets: ["2026-08", "2026-09"],
        start: "2026-08-01",
        netIncome: NET_INCOME,
        cash: {opening: usd(10000), values: [usd(14800), usd(19600)]},
        netWorth: {opening: usd(50000), values: [usd(54800), usd(59600)]},
        runway: null,
        assets: [],
        warnings: [],
        ...overrides,
    };
}

function mount(tab: "net" | "cash" | "worth", value: Projection = projection()) {
    return render(ProjectionReports, {tab, projection: value, interval: "monthly" as const, styles: STYLES, fallbackCommodity: "$"});
}

describe("COMPONENT ProjectionReports — net income", () => {
    it("renders the period report through ReportTable, unchanged", () => {
        mount("net");
        const table = screen.getByTestId("projection-net").querySelector("table");
        expect(table?.textContent).toContain("expenses:rent");
        expect(table?.textContent).toContain("income:salary");
    });

    it("labels the chart's buckets the way the table above it does", () => {
        // The hand-off: the chart takes LABELS, and they come from the same
        // `bucketLabel` the report table's header uses. A chart headed with raw
        // bucket keys under a table headed with month names is two readings of
        // one report that do not look like one report.
        const {container} = mount("net");
        const ticks = [...container.querySelectorAll('[data-placement="bottom"] .lc-axis-tick-label')].map((t) => t.textContent?.trim());
        expect(ticks).toEqual(["Aug 2026", "Sep 2026"]);
    });

    it("names the three marks in a legend, so identity is never colour-alone", () => {
        mount("net");
        const legend = screen.getByTestId("projection-flow-chart-legend");
        expect(legend.textContent).toContain("Money in");
        expect(legend.textContent).toContain("Money out");
        expect(legend.textContent).toContain("Net");
    });
});

describe("COMPONENT ProjectionReports — cash and runway", () => {
    it("states the crossing point in WORDS above the chart", () => {
        mount("cash", projection({runway: {bucket: 1, label: "Mar 2028", date: "2028-03-31", periods: 18}}));
        expect(screen.getByTestId("projection-runway").textContent).toContain("Cash turns negative in Mar 2028 — 18 months.");
        // A crossing is a warning, and is announced as one.
        expect(screen.getByRole("alert")).toBeDefined();
    });

    it("marks the crossing bucket on the chart as well as in the sentence", () => {
        const {container} = mount("cash", projection({runway: {bucket: 1, label: "Sep 2026", date: "2026-09-30", periods: 2}}));
        expect(container.querySelectorAll('[data-rule="mark"]')).toHaveLength(1);
    });

    it("says so plainly when it never crosses, and keeps the zero rule in frame anyway", () => {
        const {container} = mount("cash");
        expect(screen.getByTestId("projection-runway").textContent).toContain("Cash never turns negative over the 2 months projected.");
        expect(screen.getByTestId("projection-runway").textContent).toContain("builds by");
        expect(container.querySelectorAll('[data-rule="zero"]')).toHaveLength(1);
        expect(container.querySelectorAll('[data-rule="mark"]')).toHaveLength(0);
    });

    it("labels the opening balance beside the heading — it is a real figure, not a projected bucket", () => {
        mount("cash");
        expect(screen.getByTestId("projection-cash").textContent).toContain("opening $10,000.00");
    });
});

describe("COMPONENT ProjectionReports — net worth", () => {
    it("is one line with its opening balance named", () => {
        mount("worth");
        const panel = screen.getByTestId("projection-worth");
        expect(panel.textContent).toContain("opening $50,000.00");
        // One series gets no legend box — the heading names it.
        expect(screen.queryByTestId("projection-worth-chart-legend")).toBeNull();
    });

    it("says out loud that it is summary level, so nobody reads an amortisation into it", () => {
        mount("worth");
        expect(screen.getByTestId("projection-worth").textContent).toContain("held flat");
    });

    it("breaks the growth down per asset, so one line does not have to explain three", () => {
        mount(
            "worth",
            projection({
                assets: [
                    {group: "a", account: "assets:savings", journalOpening: usd(1000), opening: usd(1000), growth: usd(40)},
                    {group: "b", account: "assets:brokerage", journalOpening: usd(500000), opening: usd(500000), growth: usd(35000)},
                ],
            })
        );
        const rows = [...screen.getByTestId("projection-asset-breakdown").querySelectorAll("tbody tr")].map((tr) =>
            [...tr.querySelectorAll("td")].map((td) => td.textContent?.trim())
        );
        // Biggest contributor first, and the figures spelled the way every
        // other figure on the page is.
        expect(rows).toEqual([
            ["assets:brokerage", "$500,000.00", "$35,000.00"],
            ["assets:savings", "$1,000.00", "$40.00"],
        ]);
        // The total is the GROWTH column's. The balances are already inside the
        // opening figure above the chart.
        expect(screen.getByTestId("projection-asset-breakdown").querySelector("tfoot")?.textContent).toContain("$35,040.00");
        // Unrealised, and it says so where the numbers are.
        expect(screen.getByTestId("projection-asset-breakdown").textContent).toContain("never reaches cash or net income");
    });

    it("shows no breakdown at all for a scenario with no asset rows", () => {
        mount("worth");
        expect(screen.queryByTestId("projection-asset-breakdown")).toBeNull();
    });
});

describe("COMPONENT ProjectionReports — the tab strip", () => {
    it("switches which reading is on screen", async () => {
        mount("net");
        expect(screen.queryByTestId("projection-cash")).toBeNull();
        await fireEvent.click(screen.getByRole("tab", {name: "Cash & runway"}));
        expect(screen.getByTestId("projection-cash")).toBeDefined();
        expect(screen.queryByTestId("projection-net")).toBeNull();
    });

    it("marks the active tab for a screen reader, not only with a class", () => {
        mount("worth");
        expect(screen.getByRole("tab", {name: "Net worth"}).getAttribute("aria-selected")).toBe("true");
        expect(screen.getByRole("tab", {name: "Net income"}).getAttribute("aria-selected")).toBe("false");
    });
});

describe("COMPONENT ProjectionReports — a scenario in more than one commodity", () => {
    it("names what charting one commodity leaves out rather than dropping it silently", () => {
        mount(
            "cash",
            projection({
                cash: {
                    opening: new Map([
                        ["$", dec(10000, 0)],
                        ["EUR", dec(50, 0)],
                    ]),
                    values: [usd(14800), usd(19600)],
                },
            })
        );
        expect(screen.getByTestId("projection-uncharted").textContent).toContain("EUR");
    });

    it("says nothing when there is nothing left out", () => {
        mount("cash");
        expect(screen.queryByTestId("projection-uncharted")).toBeNull();
    });
});
