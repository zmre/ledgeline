// INTEGRATION: "Remember as a rule" from the Journal view (plan 32 D27)
// against a live engine. Skipped unless LEDGELINE_API_URL is set — `just
// test-integration` sets it, serving fixtures/sample.journal, whose directory
// is the rules discovery root: `fixtures/rules/simple/checking.csv.rules` (and
// its sibling `checking.csv`) is `rules/simple/checking.csv.rules` here, and
// `fixtures/review-rules/` is `review-rules/`.
//
// READ-ONLY: the rules-for route reads, and the rule preview writes nothing
// the user owns (temporary copies of the rules file). The save is pinned in
// Rust over temp copies; it is the rules PUT the import already uses.

import {describe, expect, it} from "vitest";
import {decodeJournalRulePreview} from "#lib/api/importReviewDecode.js";
import {decodeRowsPage} from "#lib/api/journalViewDecode.js";
import {LedgelineApi} from "#lib/api/native.js";
import {decodeRulesFor} from "#lib/api/reviewDecode.js";

const apiUrl = process.env.LEDGELINE_API_URL;
const TODAY = "2026-07-08";
const ALL = {from: null, to: null, accounts: new Set<string>(), query: ""};
const CHECKING_RULES = "rules/simple/checking.csv.rules";
const RULE = {
    matchers: [{field: "description", pattern: "oakview"}],
    assignments: [{field: "account2", value: "expenses:housing:rent"}],
};

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live journal rules (D27)", () => {
    const api = (): LedgelineApi => new LedgelineApi(apiUrl ?? "");

    /** The tindex of the first transaction paid from the checking account into an expense. */
    async function checkingIndex(): Promise<number> {
        const page = decodeRowsPage(await api().journalRows(ALL, {offset: 0, limit: 200, today: TODAY}));
        const row = page.rows.find(
            (row) =>
                row.txn.postings.some((posting) => posting.account === "assets:bank:checking") &&
                row.txn.postings.some((posting) => posting.account.startsWith("expenses:"))
        );
        expect(row).toBeDefined();
        return row!.txn.index;
    }

    it("infers the rules files that book to the transaction's account", async () => {
        const found = decodeRulesFor(await api().reviewRulesFor(await checkingIndex()));
        const ids = found.candidates.map((candidate) => candidate.rulesId);
        expect(ids).toContain(CHECKING_RULES);
        expect(found.candidates.every((candidate) => candidate.account1 === "assets:bank:checking")).toBe(true);
        // Several unrelated files book to it in the fixtures: a tie, so the user picks.
        expect(found.needsChoice).toBe(found.candidates.every((candidate) => !candidate.recommended));
        expect(found.candidates.filter((candidate) => candidate.recommended).length).toBeLessThanOrEqual(1);
    });

    it("checks a rule against the rules file's own statement", async () => {
        const preview = decodeJournalRulePreview(await api().reviewRulePreview({index: await checkingIndex(), rulesId: CHECKING_RULES, rule: RULE}));
        if (!preview.ok) throw new Error(preview.stderr);
        expect(preview.basis).toBe("csv");
        if (preview.basis !== "csv") return;
        expect(preview.file).toBe("checking.csv");
        expect(preview.total).toBeGreaterThan(0);
        // `checking.csv` has no Oakview row: the pattern meets nothing, and says so.
        expect(preview.matched).toBe(0);
        expect(preview.noCsvMatch).toBe(true);

        const coffee = decodeJournalRulePreview(
            await api().reviewRulePreview({
                index: await checkingIndex(),
                rulesId: CHECKING_RULES,
                rule: {matchers: [{field: "description", pattern: "starbucks"}], assignments: [{field: "account2", value: "expenses:food:cafe"}]},
            })
        );
        if (!coffee.ok || coffee.basis !== "csv") throw new Error("expected a CSV-basis preview");
        expect(coffee.matched).toBeGreaterThan(0);
        expect(coffee.alreadyCategorised).toBe(coffee.matched);
        expect(coffee.changedTotal).toBe(coffee.matched);
    });

    it("falls back to the journal with no statement, and refuses an include", async () => {
        const index = await checkingIndex();
        const fallback = decodeJournalRulePreview(await api().reviewRulePreview({index, rulesId: "review-rules/bank/2026/schwab.csv.rules", rule: RULE}));
        expect(fallback).toEqual({ok: true, basis: "journal", matched: 0, total: 0});
        const included = decodeJournalRulePreview(await api().reviewRulePreview({index, rulesId: "review-rules/import/paypal.csv.rules", rule: RULE}));
        expect(included.ok).toBe(false);
    });
});
