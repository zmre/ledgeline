// Settings › Automatic postings (plan 30 Phase 5), driven through the real
// store by a fake engine serving `GET /api/auto-rules`.

import {fireEvent, render, screen, waitFor, within} from "@testing-library/svelte";
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

    describe("editing (plan 30 amendment 22)", () => {
        /** Every PUT body; the PUT answers `answer` (the listing by default). */
        function capturePuts(answer?: Response): unknown[] {
            const puts: unknown[] = [];
            const inner = globalThis.fetch;
            vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
                if (init?.method === "PUT") {
                    puts.push(JSON.parse(String(init.body)));
                    if (answer !== undefined) return Promise.resolve(answer.clone());
                }
                return inner(url, init);
            });
            return puts;
        }

        it("edits a rule through its form and saves it as a replace of that block", async () => {
            await serve(AUTO_RULES_LISTING);
            const puts = capturePuts();
            render(AutoRulesPanel, {});
            const rules = await screen.findAllByTestId("settings-auto-rule");
            await fireEvent.click(within(rules[0]).getByTestId("settings-auto-edit"));
            const percent = screen.getByLabelText("Percent for posting 1 of rule 1");
            expect((percent as HTMLInputElement).value).toBe("10");
            await fireEvent.change(percent, {target: {value: "8"}});
            // A journal rule says whether it is for forecasts; this one is not.
            expect((screen.getByLabelText("Forecast transactions only for rule 1") as HTMLInputElement).checked).toBe(false);
            await fireEvent.click(screen.getByTestId("settings-auto-save"));
            await waitFor(() => expect(puts).toHaveLength(1));
            expect(puts[0]).toMatchObject({
                journalId: "rules.journal",
                revision: "rev-rules",
                change: {
                    kind: "replace",
                    block: 0,
                    was: AUTO_RULES_LISTING.rules[0].raw,
                    rule: {form: {matchAccount: "revenue:sales", forecastOnly: false, postings: [{percent: {mantissa: "8", places: 0}}]}},
                },
            });
            await waitFor(() => expect(screen.queryByTestId("settings-auto-editor")).toBeNull());
        });

        it("edits a rule a form cannot state as its text", async () => {
            await serve(AUTO_RULES_LISTING);
            const puts = capturePuts();
            render(AutoRulesPanel, {});
            const rules = await screen.findAllByTestId("settings-auto-rule");
            await fireEvent.click(within(rules[1]).getByTestId("settings-auto-edit"));
            const text = screen.getByTestId("settings-auto-text") as HTMLTextAreaElement;
            expect(text.value).toBe(AUTO_RULES_LISTING.rules[1].raw);
            await fireEvent.input(text, {target: {value: "= desc:food\n    (budget:food)  *-1\n"}});
            await fireEvent.click(screen.getByTestId("settings-auto-save"));
            await waitFor(() => expect(puts).toHaveLength(1));
            expect(puts[0]).toMatchObject({change: {kind: "replace", block: 1, rule: {raw: "= desc:food\n    (budget:food)  *-1\n"}}});
        });

        it("offers no edit on a rule whose file cannot be written", async () => {
            await serve(AUTO_RULES_LISTING);
            render(AutoRulesPanel, {});
            const rules = await screen.findAllByTestId("settings-auto-rule");
            expect(within(rules[2]).queryByTestId("settings-auto-edit")).toBeNull();
            expect(within(rules[2]).queryByTestId("settings-auto-delete")).toBeNull();
        });

        it("adds a rule to the main journal, and will not send a half-typed one", async () => {
            await serve(AUTO_RULES_LISTING);
            const puts = capturePuts();
            render(AutoRulesPanel, {});
            await fireEvent.click(await screen.findByTestId("settings-auto-add"));
            await fireEvent.click(screen.getByTestId("settings-auto-save"));
            expect(screen.getByTestId("settings-auto-save-error").textContent).toContain("account to match");
            expect(puts).toHaveLength(0);

            await fireEvent.click(screen.getByText("Edit as text"));
            await fireEvent.input(screen.getByTestId("settings-auto-text"), {target: {value: "= expenses:food\n    (budget:food)  *1"}});
            await fireEvent.click(screen.getByTestId("settings-auto-save"));
            await waitFor(() => expect(puts).toHaveLength(1));
            expect(puts[0]).toEqual({
                journalId: "main.journal",
                revision: "rev-main",
                change: {kind: "add", rule: {raw: "= expenses:food\n    (budget:food)  *1"}},
            });
        });

        it("deletes only once the delete is confirmed", async () => {
            await serve(AUTO_RULES_LISTING);
            const puts = capturePuts();
            render(AutoRulesPanel, {});
            const rules = await screen.findAllByTestId("settings-auto-rule");
            await fireEvent.click(within(rules[0]).getByTestId("settings-auto-delete"));
            expect(puts).toHaveLength(0);
            await fireEvent.click(within(rules[0]).getByTestId("settings-auto-confirm-delete"));
            await waitFor(() => expect(puts).toHaveLength(1));
            expect(puts[0]).toEqual({
                journalId: "rules.journal",
                revision: "rev-rules",
                change: {kind: "remove", block: 0, was: AUTO_RULES_LISTING.rules[0].raw},
            });
        });

        it("says so when the file moved underneath the editor, and keeps the edit open", async () => {
            await serve(AUTO_RULES_LISTING);
            capturePuts(new Response("rules.journal changed on disk", {status: 409}));
            render(AutoRulesPanel, {});
            const rules = await screen.findAllByTestId("settings-auto-rule");
            await fireEvent.click(within(rules[1]).getByTestId("settings-auto-edit"));
            await fireEvent.click(screen.getByTestId("settings-auto-save"));
            const error = await screen.findByTestId("settings-auto-save-error");
            expect(error.textContent).toContain("changed since this list was read");
            expect(screen.getByTestId("settings-auto-editor")).toBeTruthy();
        });

        it("offers no editing when the server does not write", async () => {
            await serve({...AUTO_RULES_LISTING, editable: false});
            render(AutoRulesPanel, {});
            await screen.findAllByTestId("settings-auto-rule");
            // The store is a module singleton: wait out the previous test's listing.
            await waitFor(() => expect(screen.queryAllByTestId("settings-auto-edit")).toHaveLength(0));
            expect(screen.getAllByTestId("settings-auto-rule")).toHaveLength(3);
            expect(screen.queryByTestId("settings-auto-add")).toBeNull();
        });
    });
});
