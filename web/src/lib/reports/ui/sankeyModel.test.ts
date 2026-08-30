// The Sankey display model.
//
// What is defended here is the small set of claims the diagrams rest on, and
// nothing else. Each of them is invisible in the wire body and invisible in the
// rendered SVG, so this is the only place they can be stated:
//
//   * one account is one colour, in BOTH graphs
//   * the tail past the last palette slot becomes ONE node, with its links
//     re-aggregated behind it
//   * statement lines never fold and never take a slot
//   * `complete` compares the exact figures, not the formatted ones
//   * every drawn column adds up to the graph's total

import {describe, expect, it} from "vitest";
import {decodeFlowReport} from "$lib/api/nativeDecode";
import type {AmountStyle} from "$lib/domain/types";
import {CATEGORICAL, OTHER_COLOR, OTHER_LABEL} from "$lib/format/palette";
import {FLOW_REPORT} from "$lib/testing/flowsFixture";
import {flowPalette, OTHER_KEY, sankeyView} from "./sankeyModel";

const STYLES: ReadonlyMap<string, AmountStyle> = new Map<string, AmountStyle>([
    ["$", {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}],
]);

const REPORT = decodeFlowReport(FLOW_REPORT);
const PALETTE = flowPalette(REPORT);

const IN = sankeyView(REPORT.inflows, PALETTE, REPORT.base, STYLES);
const OUT = sankeyView(REPORT.outflows, PALETTE, REPORT.base, STYLES);

describe("UNIT flowPalette", () => {
    it("gives one account the same colour in both graphs", () => {
        // `assets:bank:checking` is the biggest account by combined total, and
        // it appears on the TARGET side of Money in and the SOURCE side of Money
        // out. A per-graph palette would hand it two different hues on one page.
        const inbound = IN.nodes.find((node) => node.account === "assets:bank:checking");
        const outbound = OUT.nodes.find((node) => node.account === "assets:bank:checking");

        expect(inbound?.color).toBe(CATEGORICAL[0]);
        expect(outbound?.color).toBe(CATEGORICAL[0]);
    });

    it("ranks accounts by their COMBINED total, not by either graph's", () => {
        // Savings is third overall ($1,200 in + $900 out) though it is only the
        // eighth-largest node in Money out.
        expect(PALETTE.rank("a:assets:bank:checking")).toBe(0);
        expect(PALETTE.rank("a:liabilities:cc:visa")).toBe(1);
        expect(PALETTE.rank("a:assets:bank:savings")).toBe(2);
    });

    it("folds every account past the last slot instead of cycling the palette", () => {
        const folded = ["a:liabilities:loan:auto", "a:assets:bank:joint", "a:assets:prepaid:transit"];
        for (const key of folded) {
            expect(PALETTE.folded(key)).toBe(true);
            expect(PALETTE.color(key)).toBe(OTHER_COLOR);
        }
        // Everything with a slot keeps a distinct hue: no two accounts share one.
        const slotted = [...new Set(OUT.legend.filter((entry) => entry.key !== OTHER_KEY).map((entry) => entry.color))];
        expect(slotted.length).toBe(OUT.legend.length - 1);
        expect(slotted.length).toBeLessThanOrEqual(CATEGORICAL.length);
    });

    it("never gives a statement line a slot", () => {
        expect(PALETTE.folded("g:Housing")).toBe(false);
        expect(PALETTE.rank("g:Housing")).toBe(CATEGORICAL.length);
    });
});

describe("UNIT sankeyView", () => {
    it("folds the tail into one node and re-aggregates its links", () => {
        const other = OUT.nodes.filter((node) => node.key === OTHER_KEY);
        expect(other.length).toBe(1);
        expect(other[0].label).toBe(OTHER_LABEL);
        expect(other[0].color).toBe(OTHER_COLOR);
        // $150 + $100 + $50, the three accounts past the last slot.
        expect(other[0].amount).toBe("$300.00");

        // All three paid Utilities, so the three links become ONE ribbon.
        const toUtilities = OUT.links.filter((link) => link.source === OTHER_KEY);
        expect(toUtilities.length).toBe(1);
        expect(toUtilities[0].target).toBe("g:Utilities");
        expect(toUtilities[0].value).toBe(300);

        // And the three originals are gone.
        expect(OUT.nodes.some((node) => node.key === "a:liabilities:loan:auto")).toBe(false);
    });

    it("leaves the graph alone when nothing folds", () => {
        expect(IN.nodes.some((node) => node.key === OTHER_KEY)).toBe(false);
        expect(IN.nodes.length).toBe(REPORT.inflows.nodes.length);
        expect(IN.links.length).toBe(REPORT.inflows.links.length);
    });

    it("never folds a statement line and never colours one", () => {
        const statement = OUT.nodes.filter((node) => node.side === "target");
        // All six cost lines survive, though the graph names ten accounts.
        expect(statement.map((node) => node.label)).toEqual(["Housing", "Food", "Utilities", "Taxes", "Transport", "Depreciation"]);
        for (const node of statement) expect(node.color).toBeNull();
        expect(OUT.legend.some((entry) => entry.label === "Housing")).toBe(false);
    });

    it("colours a ribbon by its ACCOUNT end, whichever side that is", () => {
        // Money in: the account is the target. Money out: the account is the source.
        const inbound = IN.links.find((link) => link.target === "a:assets:bank:checking");
        const outbound = OUT.links.find((link) => link.source === "a:assets:bank:checking");

        expect(inbound?.color).toBe(CATEGORICAL[0]);
        expect(outbound?.color).toBe(CATEGORICAL[0]);
    });

    it("titles a ribbon with both ends and the amount", () => {
        const link = OUT.links.find((entry) => entry.source === OTHER_KEY);
        expect(link?.title).toBe(`${OTHER_LABEL} → Utilities: $300.00`);
    });

    it("lists the legend in palette order, tail last", () => {
        expect(OUT.legend.map((entry) => entry.label)).toEqual([
            "Bank: Checking",
            "Credit cards: Visa",
            "Bank: Savings",
            "Bank: Wise: Eur",
            "Credit cards: Amex",
            "Cash: Wallet",
            "Vehicles: Car: Depreciation",
            OTHER_LABEL,
        ]);
    });

    it("reports an incomplete graph as incomplete, and a tied-out one as complete", () => {
        expect(IN.complete).toBe(false);
        expect(IN.total).toBe("$5,700.00");
        expect(IN.sectionTotal).toBe("$6,000.00");
        expect(OUT.complete).toBe(true);
    });

    it("shares each column against the DRAWN total", () => {
        const sum = (side: string) => IN.nodes.filter((node) => node.side === side).reduce((total, node) => total + node.share, 0);
        // Both columns decompose the same $5,700, so each adds to 100% on its own.
        expect(sum("source")).toBeCloseTo(100, 6);
        expect(sum("target")).toBeCloseTo(100, 6);
    });

    it("emits nothing a structuredClone would choke on", () => {
        // LayerChart's Sankey clones the graph it is handed, and a `Dec` carries
        // a bigint. Cloning here is the assertion.
        expect(() => structuredClone({nodes: OUT.nodes, links: OUT.links})).not.toThrow();
    });
});
