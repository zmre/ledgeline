// The (i) help content: no empty or overlong entries, acronyms spelled out, and
// an entry for every id the startup section builds at run time (the "More
// metrics" keys and the bridge categories). That each rendered box and row
// actually shows its entry is the component tests' job.

import {describe, expect, it} from "vitest";
import {STARTUP_CURRENT} from "#lib/testing/saasFixture.js";
import {METRIC_HELP, type MetricHelp} from "./metricHelp";
import {BRIDGE_LABELS, moreMetrics} from "./startup";

const entries = Object.entries(METRIC_HELP) as [string, MetricHelp][];
const words = (help: MetricHelp): number => [help.text, help.formula ?? ""].join(" ").split(/\s+/).filter(Boolean).length;

describe("UNIT metricHelp", () => {
    it.each(entries)("%s has a label, title and text, and no blank formula", (_id, help) => {
        expect(help.label.trim()).not.toBe("");
        expect(help.title.trim()).not.toBe("");
        expect(help.text.trim()).not.toBe("");
        if (help.formula !== undefined) expect(help.formula.trim()).not.toBe("");
    });

    it.each(entries)("%s stays short (text and formula under 50 words)", (_id, help) => {
        expect(words(help)).toBeLessThanOrEqual(50);
    });

    it("spells out every acronym in its title", () => {
        const spelled = {
            arr: "Annual Recurring Revenue",
            nrr: "Net Revenue Retention",
            arpa: "Average Revenue Per Account",
            acv: "Annual Contract Value",
            cac: "Customer Acquisition Cost",
            dso: "Days Sales Outstanding",
            dpo: "Days Payable Outstanding",
            "sm-pct": "Sales & Marketing",
            "rnd-pct": "Research & Development",
            "ga-pct": "General & Administrative",
            "segment-arpa": "Average Revenue Per Account",
            "segment-nrr": "Net Revenue Retention",
        } as const;
        for (const [id, words] of Object.entries(spelled)) {
            expect(METRIC_HELP[id as keyof typeof spelled].title).toContain(`— ${words}`);
        }
        // The acronyms that only appear inside another box's text.
        expect(METRIC_HELP.arr.text).toContain("MRR (Monthly Recurring Revenue)");
        expect(METRIC_HELP.nrr.text).toContain("GRR (Gross Revenue Retention)");
        expect(METRIC_HELP.netnewarr.text).toContain("CMGR (Compounded Monthly Growth Rate)");
    });

    it("gives the burn multiple's bands", () => {
        expect(METRIC_HELP.burnmultiple.text).toContain("Under 1 is very efficient, 1 to 2 good, over 2 to 3 suspect, over 3 bad.");
    });

    it("covers every More metrics row", () => {
        const all = {...STARTUP_CURRENT, dsoDays: 30, dpoDays: 20, ruleOf40: 41, cac: STARTUP_CURRENT.arpa, cacPaybackMonths: 9};
        for (const metric of moreMetrics(all, "$", new Map())) {
            expect(Object.hasOwn(METRIC_HELP, metric.key), metric.key).toBe(true);
        }
    });

    it("covers every bridge category, and Other", () => {
        for (const category of [...Object.keys(BRIDGE_LABELS), "other"]) {
            expect(Object.hasOwn(METRIC_HELP, `bridge-${category}`), category).toBe(true);
        }
    });
});

describe("UNIT metricHelp — Amendment A1", () => {
    it("runway explains adjusted burn", () => {
        expect(METRIC_HELP.runway.formula).toContain("adjusted burn = cash burn + change in deferred revenue − change in receivables");
        expect(METRIC_HELP.runway.text).toContain("as if customers paid when served");
        expect(METRIC_HELP.runway.text).not.toContain("cash burn");
    });

    it("burn multiple uses adjusted burn when the books have deferred revenue or receivables (A3)", () => {
        expect(METRIC_HELP.burnmultiple.text).toContain("Uses adjusted burn with deferred revenue or receivables, else cash burn.");
        expect(METRIC_HELP.burnmultiple.formula).toContain("adjusted burn = cash burn + Δ deferred revenue − Δ receivables");
    });

    it("explains the closed-through horizon (A2)", () => {
        expect(METRIC_HELP["closed-through"].text).toContain("stop at the last month with recurring revenue recorded");
        expect(METRIC_HELP["closed-through"].text).toContain("doesn't read as churn");
    });

    it("customers from billings say what they count and why", () => {
        expect(METRIC_HELP["customers-billings"].text).toContain("billed in the last 12 months");
        expect(METRIC_HELP["customers-billings"].text).toContain("per-customer MRR isn't available");
        expect(METRIC_HELP["acv-billings"].formula).toContain("billings over the last 12 months");
        expect(METRIC_HELP["logo-churn-billings"].text).toContain("12 months without a bill");
    });

    it("Top Transactions are ranked by income and expense impact", () => {
        expect(METRIC_HELP.toptxns.text).toContain("Payments, transfers, moves to deferred revenue and closing entries drop out");
        expect(METRIC_HELP.toptxns.formula).toContain("the larger of the debit and credit sides of its revenue and expense postings");
    });
});
