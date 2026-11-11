import {describe, expect, it} from "vitest";
import {dec} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import type {InsightsReport} from "#lib/reports/insightsTypes.js";
import type {SaasAvailable} from "#lib/reports/saasTypes.js";
import {billingReport, bridge, emptyPeriod, householdReport, STARTUP_CURRENT, startupReport, usd} from "#lib/testing/saasFixture.js";
import {fmt} from "./format";
import {
    boxVisibility,
    bridgeModel,
    burnMultipleLine,
    closedThroughMonth,
    exportsStartup,
    hiddenNote,
    moreMetrics,
    runwayText,
    sameSpan,
    showsStartup,
    STARTUP_DOCS,
} from "./startup";

const USD: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const styles = new Map([["$", USD]]);

/** Just enough of an InsightsReport for the hide rules. */
function insights(netWorthCents: number, over: Partial<InsightsReport> = {}): InsightsReport {
    const worth = new Map([["$", dec(netWorthCents, 2)]]);
    return {
        base: "$",
        netWorth: {current: worth, previous: worth, delta: new Map(), pct: null},
        movers: [{symbol: "VTI", name: "VTI", gain: null, gainPct: 1, startEstimated: false}],
        hasHoldings: true,
        ...over,
    } as InsightsReport;
}

describe("UNIT startup boxVisibility (plans/37 decision 4)", () => {
    it("a household keeps every box, with no Runway", () => {
        expect(boxVisibility(insights(100), householdReport())).toEqual({netWorth: true, runway: false, investment: true, movers: true});
    });

    it("keeps Net Worth while /api/saas hasn't answered", () => {
        expect(boxVisibility(insights(100), null).netWorth).toBe(true);
    });

    it("a business swaps Net Worth for Runway", () => {
        const shows = boxVisibility(insights(100), startupReport());
        expect(shows.netWorth).toBe(false);
        expect(shows.runway).toBe(true);
    });

    it("a negative net worth hides Net Worth even for a household, and Runway takes the slot", () => {
        const shows = boxVisibility(insights(-1), householdReport());
        expect(shows).toMatchObject({netWorth: false, runway: true});
    });

    it("no Runway without a cash figure (a half with no complete month)", () => {
        const report = startupReport({current: emptyPeriod()});
        expect(boxVisibility(insights(100), report)).toMatchObject({netWorth: false, runway: false});
    });

    it("no holdings hides Investment Performance; no movers hides Biggest Movers", () => {
        const shows = boxVisibility(insights(100, {hasHoldings: false, movers: []}), null);
        expect(shows.investment).toBe(false);
        expect(shows.movers).toBe(false);
    });
});

describe("UNIT startup runwayText", () => {
    it("on adjusted burn: months, cash-out month, and both burns (Amendment A1.1)", () => {
        expect(runwayText(STARTUP_CURRENT, "$", styles)).toEqual({
            big: "117 months",
            small: "cash-out ~2036-03 · burn $3,693.33/mo adjusted for prepayments · $6,543.33/mo cash (3-mo avg)",
        });
    });

    it("on cash burn when the journal has no deferred revenue or receivable", () => {
        const current = {
            ...STARTUP_CURRENT,
            runwayBasis: "cash" as const,
            netBurnAdjustedAvg3: null,
            runwayMonths: 66.04584819154356,
            cashOutDate: "2032-01-01",
        };
        expect(runwayText(current, "$", styles)).toEqual({big: "66 months", small: "cash-out ~2032-01 · burn $6,543.33/mo (3-mo avg)"});
    });

    it("judges Not burning on the burn the runway used, not on cash burn", () => {
        // Cash burn positive, adjusted burn ≤ 0: the engine sends no runway.
        const current = {...STARTUP_CURRENT, netBurnAdjustedAvg3: usd(-10), runwayMonths: null, cashOutDate: null};
        expect(runwayText(current, "$", styles)).toEqual({big: "Not burning", small: "cash $432,160.00"});
    });

    it("says Not burning, with the cash, when there is no burn", () => {
        const current = {...STARTUP_CURRENT, runwayMonths: null, cashOutDate: null, netBurnAvg3: usd(-100)};
        expect(runwayText(current, "$", styles)).toEqual({big: "Not burning", small: "cash $432,160.00"});
    });

    it("leaves the date out past the engine's 100-year cap", () => {
        const current = {...STARTUP_CURRENT, runwayMonths: 5000, cashOutDate: null};
        expect(runwayText(current, "$", styles)?.small).toBe("burn $3,693.33/mo adjusted for prepayments · $6,543.33/mo cash (3-mo avg)");
    });

    it("is null without cash", () => {
        expect(runwayText(emptyPeriod(), "$", styles)).toBeNull();
    });
});

describe("UNIT startup bridgeModel", () => {
    it("derives start MRR from the bridge identity and signs the losses", () => {
        const model = bridgeModel(bridge(), usd(5850));
        expect(fmt("$", model.start, styles)).toBe("$3,600.00");
        expect(model.lines.map((line) => [line.category, fmt("$", line.amount, styles), line.count])).toEqual([
            ["new", "$1,500.00", 1],
            ["expansion", "$1,100.00", null],
            ["contraction", "$-150.00", null],
            ["churn", "$-800.00", 3],
            ["reactivation", "$600.00", 2],
        ]);
    });

    it("shows Other only when it is non-zero, and drops zero categories", () => {
        const model = bridgeModel(bridge({other: usd(-150), expansion: usd(0)}), usd(4600));
        expect(model.lines.map((line) => line.category)).toEqual(["new", "contraction", "churn", "reactivation", "other"]);
        expect(fmt("$", model.start, styles)).toBe("$3,600.00");
    });
});

describe("UNIT startup moreMetrics", () => {
    it("lists every figure the engine sent, in the docs' order", () => {
        const rows = moreMetrics(STARTUP_CURRENT, "$", styles);
        expect(rows.map((row) => row.label)).toEqual([
            "ARPA",
            "ACV",
            "Logo churn",
            "Quick ratio",
            "Top customer share",
            "CAC",
            "CAC payback",
            "S&M %",
            "R&D %",
            "G&A %",
            "Rule of 40",
            "DSO",
            "DPO",
            "Deferred revenue",
            "Billings",
        ]);
        const value = (key: string) => rows.find((row) => row.key === key)?.value;
        expect(value("arpa")).toBe("$731.25");
        expect(value("top-customers")).toBe("top 1: 25.6% · top 5: 87.2% · top 10: 100.0%");
        expect(value("cac-payback")).toBe("186 months");
        expect(value("sm-pct")).toBe("49.6% of revenue");
        expect(value("dso")).toBe("25.5 days");
        expect(value("quick-ratio")).toBe("3.4×");
    });

    it("leaves out each figure that is absent", () => {
        expect(moreMetrics(emptyPeriod(), "$", styles)).toEqual([]);
        expect(moreMetrics(emptyPeriod({dsoDays: 10}), "$", styles).map((row) => row.key)).toEqual(["dso"]);
    });
});

describe("UNIT startup hiddenNote — one case per reason", () => {
    const available = (over: Partial<SaasAvailable>): SaasAvailable => ({...startupReport().available, customers: false, ...over});

    it("null when customer metrics show", () => {
        expect(hiddenNote(startupReport().available)).toBeNull();
    });

    it("low coverage: the share, floored, and the threshold", () => {
        const note = hiddenNote(available({customersHiddenReason: "low-coverage", coveragePct: 62.9}));
        expect(`${note?.before}customer:${note?.after}`).toBe("Customer metrics hidden: 62% of recurring revenue has a customer: tag (90% needed).");
        expect(hiddenNote(available({customersHiddenReason: "low-coverage", coveragePct: 89.9}))?.before).toContain("89%");
        expect(note?.link.href).toBe(`${STARTUP_DOCS}#customer-tags`);
    });

    it("no customer tags", () => {
        const note = hiddenNote(available({customersHiddenReason: "no-customer-tags", coveragePct: 0}));
        expect(`${note?.before}customer:${note?.after}`).toBe("Customer metrics hidden: no recurring revenue has a customer: tag.");
    });

    it("billing-clearing books link to the deferred-revenue section", () => {
        const note = hiddenNote(available({customersHiddenReason: "billing-clearing"}));
        expect(note?.tag).toBe(true);
        expect(`${note?.before}customer:${note?.after}`).toBe(
            "Customer metrics hidden: your customer: tags are on invoices, but the monthly revenue-recognition lines that make up MRR have none, so per-customer figures would be billings, not MRR. Tag the recognition lines to turn these on."
        );
        expect(note?.link).toEqual({text: "Deferred revenue books", href: `${STARTUP_DOCS}#deferred-revenue-books`});
    });
});

describe("UNIT startup showsStartup", () => {
    it("needs recurring revenue or any kpi: tag", () => {
        expect(showsStartup(householdReport().available)).toBe(false);
        expect(showsStartup({...householdReport().available, business: true})).toBe(true);
        expect(showsStartup({...householdReport().available, recurring: true})).toBe(true);
    });
});

describe("UNIT startup sameSpan / exportsStartup — never act on another span's report", () => {
    const OLD = {start: "2024-07-01", end: "2026-06-30"};
    const NEW = {start: "2025-07-01", end: "2026-06-30"};

    it("sameSpan compares start and end, and nothing held is never the same", () => {
        expect(sameSpan(OLD, {...OLD})).toBe(true);
        expect(sameSpan(OLD, NEW)).toBe(false);
        expect(sameSpan(null, NEW)).toBe(false);
        // The saas key also carries asOf; only the span matters here.
        expect(sameSpan({...OLD, asOf: "2026-07-08"} as typeof OLD, OLD)).toBe(true);
    });

    it("offers the export only for a held report answering the span on screen, with recurring revenue", () => {
        expect(exportsStartup("data", OLD, startupReport(), OLD)).toBe(true);
        expect(exportsStartup("data", OLD, startupReport(), NEW)).toBe(false);
        expect(exportsStartup("loading", OLD, startupReport(), OLD)).toBe(false);
        expect(exportsStartup("data", OLD, householdReport(), OLD)).toBe(false);
        expect(exportsStartup("data", null, null, OLD)).toBe(false);
    });
});

describe("UNIT startup — customers from billings (Amendment A1.2)", () => {
    it("rewords the billing-clearing note: what is hidden, and where the count comes from", () => {
        const note = hiddenNote(billingReport().available);
        expect(note?.before).toBe("Per-customer MRR figures hidden: your ");
        expect(note?.after.endsWith(" Tag the recognition lines to turn these on. Customer counts are from billings.")).toBe(true);
        expect(note?.link.href).toBe(`${STARTUP_DOCS}#deferred-revenue-books`);
    });

    it("leaves the note alone when no customers were billed", () => {
        const available = {...billingReport().available, billingCustomers: false};
        expect(hiddenNote(available)?.before).toBe("Customer metrics hidden: your ");
    });

    it("adds ACV and logo churn from billings to More metrics", () => {
        const rows = moreMetrics(billingReport().current, "$", styles);
        expect(rows.find((row) => row.key === "acv-billings")).toEqual({key: "acv-billings", label: "ACV (from billings)", value: "$12,600.00"});
        expect(rows.find((row) => row.key === "logo-churn-billings")).toEqual({
            key: "logo-churn-billings",
            label: "Logo churn (from billings)",
            value: "10.0%",
        });
        expect(rows.find((row) => row.key === "acv")).toBeUndefined();
    });
});

describe("UNIT startup — data horizon (A2) and burn multiple basis (A3)", () => {
    it("names the closed-through month only when it is before the span's last month", () => {
        expect(closedThroughMonth(startupReport())).toBeNull();
        const later = startupReport({period: {...startupReport().period, end: "2026-10-31", currEnd: "2026-10-31"}});
        expect(closedThroughMonth(later)).toBe("2026-06");
        expect(closedThroughMonth(householdReport())).toBeNull();
    });

    it("the burn multiple's line shows the burn it divided", () => {
        expect(burnMultipleLine(STARTUP_CURRENT, "$", styles)).toBe("net burn $44,070.00 (adjusted for prepayments)");
        expect(burnMultipleLine({...STARTUP_CURRENT, burnMultipleBasis: "cash"}, "$", styles)).toBe("net burn $43,720.00");
        expect(burnMultipleLine({...STARTUP_CURRENT, burnMultipleBasis: null, netBurnAdjusted: null}, "$", styles)).toBe("net burn $43,720.00");
    });
});
