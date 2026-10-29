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
        /** Every PUT body; the PUT answers `answer` (the listing by default), after `onPut` (the file changing underneath). */
        function capturePuts(answer?: Response, onPut?: () => void): unknown[] {
            const puts: unknown[] = [];
            const inner = globalThis.fetch;
            vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
                if (init?.method === "PUT") {
                    puts.push(JSON.parse(String(init.body)));
                    onPut?.();
                    if (answer !== undefined) return Promise.resolve(answer.clone());
                }
                return inner(url, init);
            });
            return puts;
        }

        const conflict = (): Response => new Response("rules.journal changed on disk", {status: 409});

        /** A listing as served; its rules are whatever "the file" now holds. */
        type Listing = Omit<typeof AUTO_RULES_LISTING, "rules"> & {rules: {raw: string}[]};

        /** A mutable copy of the listing, so a test can change "the file" after the panel read it. */
        function servedListing(): Listing {
            return structuredClone(AUTO_RULES_LISTING);
        }

        /** `rules.journal` once someone else added a rule above its first: every rule of that file one block later, at a new revision. */
        function withRuleInsertedAbove(): Listing["rules"] {
            const inserted = {
                ...AUTO_RULES_LISTING.rules[1],
                index: 0,
                line: 3,
                query: "expenses:rent",
                raw: "= expenses:rent\n    (budget:rent)  *-1\n",
                unsupported: null,
                block: 0,
                revision: "rev-rules-2",
            };
            const shifted = AUTO_RULES_LISTING.rules.map((rule) =>
                rule.journalId === "rules.journal"
                    ? {...rule, index: rule.index + 1, block: rule.block + 1, revision: "rev-rules-2"}
                    : {...rule, index: rule.index + 1}
            );
            return [inserted, ...shifted];
        }

        /** The rows, once they show `listing` — the store is a module singleton, so the previous test's can linger. */
        async function rowsShowing(listing: Listing): Promise<HTMLElement[]> {
            await waitFor(() =>
                expect(screen.getAllByTestId("settings-auto-rule").map((row) => row.querySelector("pre")?.textContent)).toEqual(
                    listing.rules.map((rule) => rule.raw.trimEnd())
                )
            );
            return screen.getAllByTestId("settings-auto-rule");
        }

        /** Open the text editor on the `expr:` rule (index 1) and type a new block into it. */
        async function draftTheExprRule(): Promise<string> {
            const rules = await rowsShowing(AUTO_RULES_LISTING);
            await fireEvent.click(within(rules[1]).getByTestId("settings-auto-edit"));
            const draft = "= expr:'food and not dinner'\n    (budget:food)  *-1\n";
            await fireEvent.input(screen.getByTestId("settings-auto-text"), {target: {value: draft}});
            return draft;
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

        it("keeps an open edit on its own rule when a re-read moves it, and saves it there", async () => {
            await serve(AUTO_RULES_LISTING);
            render(AutoRulesPanel, {});
            const draft = await draftTheExprRule();

            // The watcher republishes: a rule was added above, so index 1 is now `= revenue:sales`.
            await serve({...AUTO_RULES_LISTING, rules: withRuleInsertedAbove()});
            await waitFor(() => expect(screen.getAllByTestId("settings-auto-rule")).toHaveLength(4));
            const rows = screen.getAllByTestId("settings-auto-rule");
            expect(within(rows[1]).queryByTestId("settings-auto-editor")).toBeNull();
            expect(within(rows[2]).getByTestId("settings-auto-text")).toBeTruthy();
            expect((screen.getByTestId("settings-auto-text") as HTMLTextAreaElement).value).toBe(draft);

            const puts = capturePuts();
            await fireEvent.click(screen.getByTestId("settings-auto-save"));
            await waitFor(() => expect(puts).toHaveLength(1));
            expect(puts[0]).toEqual({
                journalId: "rules.journal",
                revision: "rev-rules-2",
                change: {kind: "replace", block: 2, was: AUTO_RULES_LISTING.rules[1].raw, rule: {raw: draft}},
            });
        });

        it("closes an open edit, and says why, once a re-read shows its rule changed", async () => {
            await serve(AUTO_RULES_LISTING);
            render(AutoRulesPanel, {});
            await draftTheExprRule();

            const rules = AUTO_RULES_LISTING.rules.map((rule) =>
                rule.index === 1 ? {...rule, raw: "= expr:'food'\n    (budget:food)  *-2\n", revision: "rev-rules-2"} : rule
            );
            await serve({...AUTO_RULES_LISTING, rules});
            await waitFor(() => expect(screen.queryByTestId("settings-auto-editor")).toBeNull());
            expect(screen.getByTestId("settings-auto-notice").textContent).toContain("your edit was not saved");
            expect(screen.getAllByTestId("settings-auto-rule")[1].querySelector("pre")?.textContent).toContain("*-2");
        });

        it("after a 409 re-read moved the rule, saves the retry over that rule and not the one now at its old place", async () => {
            await serve(AUTO_RULES_LISTING);
            render(AutoRulesPanel, {});
            const draft = await draftTheExprRule();
            const listing = servedListing();
            await serve(listing);
            await waitFor(() => expect(screen.getByTestId("settings-auto-text")).toBeTruthy());
            // The first PUT finds the file moved: someone added a rule above.
            const puts = capturePuts(conflict(), () => {
                listing.rules = withRuleInsertedAbove();
            });
            await fireEvent.click(screen.getByTestId("settings-auto-save"));
            expect((await screen.findByTestId("settings-auto-save-error")).textContent).toContain("changed since this list was read");
            await waitFor(() => expect(screen.getAllByTestId("settings-auto-rule")).toHaveLength(4));
            expect(within(screen.getAllByTestId("settings-auto-rule")[2]).getByTestId("settings-auto-editor")).toBeTruthy();

            await fireEvent.click(screen.getByTestId("settings-auto-save"));
            await waitFor(() => expect(puts).toHaveLength(2));
            expect(puts[1]).toEqual({
                journalId: "rules.journal",
                revision: "rev-rules-2",
                change: {kind: "replace", block: 2, was: AUTO_RULES_LISTING.rules[1].raw, rule: {raw: draft}},
            });
        });

        it("a Delete the file moved under says so above the re-read list, and opens no editor on the old text", async () => {
            const listing = servedListing();
            await serve(listing);
            // The rule was already removed on disk: the file now starts at the `expr:` rule.
            const puts = capturePuts(conflict(), () => {
                listing.rules = AUTO_RULES_LISTING.rules
                    .slice(1)
                    .map((rule) =>
                        rule.journalId === "rules.journal"
                            ? {...rule, index: rule.index - 1, block: rule.block - 1, revision: "rev-rules-2"}
                            : {...rule, index: rule.index - 1}
                    );
            });
            render(AutoRulesPanel, {});
            const rules = await rowsShowing(AUTO_RULES_LISTING);
            await fireEvent.click(within(rules[0]).getByTestId("settings-auto-delete"));
            await fireEvent.click(within(rules[0]).getByTestId("settings-auto-confirm-delete"));
            await waitFor(() => expect(puts).toHaveLength(1));
            await waitFor(() => expect(screen.getAllByTestId("settings-auto-rule")).toHaveLength(2));
            expect(screen.queryByTestId("settings-auto-editor")).toBeNull();
            expect(screen.queryByTestId("settings-auto-save")).toBeNull();
            const notice = screen.getByTestId("settings-auto-notice");
            expect(notice.textContent).toContain("nothing was deleted");
            expect(notice.closest("[data-testid=settings-auto-rule]")).toBeNull();
            expect(screen.getAllByTestId("settings-auto-rule")[0].querySelector("pre")?.textContent).toContain("expr:");
        });

        it("a Delete that fails shows the error on that rule's row and leaves the row as listed", async () => {
            await serve(AUTO_RULES_LISTING);
            capturePuts(new Response("rules.journal could not be written: permission denied", {status: 500}));
            render(AutoRulesPanel, {});
            const rules = await rowsShowing(AUTO_RULES_LISTING);
            await fireEvent.click(within(rules[0]).getByTestId("settings-auto-delete"));
            await fireEvent.click(within(rules[0]).getByTestId("settings-auto-confirm-delete"));
            const notice = await within(rules[0]).findByTestId("settings-auto-notice");
            expect(notice.textContent).not.toBe("");
            expect(screen.queryByTestId("settings-auto-editor")).toBeNull();
            expect(within(rules[0]).getByTestId("settings-auto-edit")).toBeTruthy();
            expect(within(rules[0]).getByTestId("settings-auto-delete")).toBeTruthy();
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
