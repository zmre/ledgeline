// Settings › Automatic postings (plan 30 Phase 5), driven through the real
// store by a fake engine serving `GET /api/auto-rules`.

import {render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {AUTO_RULES_LISTING} from "#lib/testing/autoRow.js";
import AutoRulesPanel from "./AutoRulesPanel.svelte";

afterEach(() => {
    vi.unstubAllGlobals();
});

let generation = 0;
/** A fresh generation per test, so the store's (server, generation) latch re-reads. */
async function serve(listing: Record<string, unknown>): Promise<void> {
    generation += 1;
    await connectFakeEngine({
        "/api/auto-rules": listing,
        "/api/journal": {
            title: null,
            file: null,
            commodityStyles: {},
            reportStyles: {},
            dominantCommodity: "$",
            txnCount: 0,
            firstDate: null,
            lastDate: null,
            accounts: {},
            generation: `auto-${generation}`,
        },
    });
    const {journal} = await import("#lib/stores/journal.svelte.js");
    await journal.refresh({force: true});
}

describe("COMPONENT AutoRulesPanel", () => {
    it("lists every rule with its place, text, lock and generated count", async () => {
        await serve(AUTO_RULES_LISTING);
        render(AutoRulesPanel, {focus: 2});
        const rules = await screen.findAllByTestId("settings-auto-rule");
        expect(rules).toHaveLength(3);
        expect(rules[0].textContent).toContain("= revenue:sales");
        expect(rules[0].textContent).toContain("rules.journal:5");
        expect(rules[0].textContent).toContain("4 generated postings");
        expect(rules[0].querySelector("pre")?.textContent).toContain("liabilities:tax:sales      *0.1");
        expect(rules[1].querySelector("[data-testid=settings-auto-locked]")).not.toBeNull();
        expect(screen.getByTestId("settings-auto-in-force").textContent).toContain("hledger.conf");
    });

    it("outlines the rule a generated posting linked to", async () => {
        await serve(AUTO_RULES_LISTING);
        render(AutoRulesPanel, {focus: 2});
        const rules = await screen.findAllByTestId("settings-auto-rule");
        expect(rules.map((rule) => rule.getAttribute("aria-current"))).toEqual([null, null, "true"]);
    });

    it("folds a problem repeated across transactions into one line", async () => {
        await serve(AUTO_RULES_LISTING);
        render(AutoRulesPanel, {});
        const rules = await screen.findAllByTestId("settings-auto-rule");
        const problems = rules[2].querySelectorAll("[data-testid=settings-auto-problem]");
        expect(problems).toHaveLength(1);
        expect(problems[0].textContent).toContain("in 2 transactions");
    });

    it("says the rules are not applied, and where to turn them on, without --auto", async () => {
        await serve({...AUTO_RULES_LISTING, inForce: false, conf: null});
        render(AutoRulesPanel, {});
        const off = await screen.findByTestId("settings-auto-off");
        expect(off.textContent).toContain("no hledger.conf is in force");
        expect(off.textContent).toContain("hledger config");
        expect(screen.getAllByTestId("settings-auto-rule")[0].textContent).not.toContain("generated postings");
    });
});
