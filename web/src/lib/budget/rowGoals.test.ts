// Which goals stand behind a budget-table row.
//
// The claims worth pinning are the ones a wrong answer would turn into a wrong
// EDIT: a row whose amount opens the wrong goal, or opens one goal when its
// figure is the sum of two. The engine's clip-then-hide-parents rule
// (`budget.rs` `clip`, `budget_summary.rs` `budget_leaves`) is what the match
// has to agree with, so every case below is phrased as a journal and the rows
// the engine would draw for it.

import {describe, expect, it} from "vitest";
import {budgetedCell, clipAccount, goalKey, goalRefs, lockReason, periodLabel, rowGoals} from "#lib/budget/rowGoals.js";
import type {BudgetFile, BudgetGoal, BudgetListing, BudgetPeriod, BudgetRule} from "#lib/budget/types.js";

function goal(index: number, account: string, locked: string | null = null): BudgetGoal {
    return {index, line: index + 2, account, unbalanced: true, amount: null, entry: null, inverted: false, annual: null, locked};
}

function rule(block: number, period: BudgetPeriod, goals: BudgetGoal[], locked: string | null = null): BudgetRule {
    return {block, line: 1, period: {raw: period, simple: period}, description: "", locked, goals};
}

function file(journalId: string, rules: BudgetRule[], writable = true): BudgetFile {
    return {journalId, label: journalId, revision: "r", writable, rules};
}

function listing(files: BudgetFile[], editable = true): BudgetListing {
    return {editable, defaultTarget: files[0]?.journalId ?? null, canCreateFile: false, createFileName: "budget.journal", files, joinable: {}};
}

/** The accounts of each row's backing goals, for terse assertions. */
function accountsBehind(result: ReturnType<typeof rowGoals>, row: string): string[] {
    return (result.byAccount.get(row) ?? []).map((ref) => `${ref.goal.account}@${ref.rule.period.raw}`);
}

describe("UNIT budget/rowGoals — clipAccount mirrors the engine's clip", () => {
    it("keeps at most `depth` segments", () => {
        expect(clipAccount("expenses:food:dining", 2)).toBe("expenses:food");
        expect(clipAccount("expenses:food:dining", 3)).toBe("expenses:food:dining");
        expect(clipAccount("expenses:food:dining", 9)).toBe("expenses:food:dining");
        expect(clipAccount("expenses:food", 1)).toBe("expenses");
    });

    it("treats a depth below one as one, as `depth.max(1)` does", () => {
        expect(clipAccount("expenses:food", 0)).toBe("expenses");
    });
});

describe("UNIT budget/rowGoals — matching goals to rows", () => {
    it("matches a goal written on the row's own account", () => {
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:food"), goal(1, "expenses:rent")])])]);
        const result = rowGoals(l, ["expenses:food", "expenses:rent"], 3);
        expect(accountsBehind(result, "expenses:food")).toEqual(["expenses:food@monthly"]);
        expect(accountsBehind(result, "expenses:rent")).toEqual(["expenses:rent@monthly"]);
        expect(result.unmatched).toEqual([]);
    });

    it("lists every period's goal on one account behind its row (multi-period)", () => {
        // hledger sums a monthly and a yearly goal on one account into ONE
        // figure, so the row's amount is both — and must offer both.
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:gifts")]), rule(1, "yearly", [goal(1, "expenses:gifts")])])]);
        const result = rowGoals(l, ["expenses:gifts"], 3);
        expect(accountsBehind(result, "expenses:gifts")).toEqual(["expenses:gifts@monthly", "expenses:gifts@yearly"]);
    });

    it("rolls a goal deeper than the depth up under its ancestor's row", () => {
        // Depth 2: the engine clips `expenses:food:dining` to `expenses:food`
        // and sums it with the goal written there.
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:food"), goal(1, "expenses:food:dining")])])]);
        const result = rowGoals(l, ["expenses:food"], 2);
        expect(accountsBehind(result, "expenses:food")).toEqual(["expenses:food@monthly", "expenses:food:dining@monthly"]);
        expect(result.unmatched).toEqual([]);
    });

    it("leaves a parent goal hidden by a budgeted child unmatched (depth mismatch)", () => {
        // Depth 3: the engine draws only the leaf `expenses:food:dining`; the
        // `expenses:food` goal has no row, so it must stay in the editor.
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:food"), goal(1, "expenses:food:dining")])])]);
        const result = rowGoals(l, ["expenses:food:dining"], 3);
        expect(accountsBehind(result, "expenses:food:dining")).toEqual(["expenses:food:dining@monthly"]);
        expect(result.unmatched.map((ref) => ref.goal.account)).toEqual(["expenses:food"]);
        expect(result.represented).toEqual(new Set([goalKey("main", goal(1, ""))]));
    });

    it("does not match a goal to a row that merely shares a name prefix", () => {
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:foodbank")])])]);
        const result = rowGoals(l, ["expenses:food"], 3);
        expect(result.byAccount.size).toBe(0);
        expect(result.unmatched).toHaveLength(1);
    });

    it("leaves goals on accounts with no row (other sections, transfers) unmatched", () => {
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:food"), goal(1, "assets:checking")])])]);
        const result = rowGoals(l, ["expenses:food"], 3);
        expect(result.unmatched.map((ref) => ref.goal.account)).toEqual(["assets:checking"]);
    });

    it("keys goals per file, since the engine's index is only unique within one", () => {
        const l = listing([file("a", [rule(0, "monthly", [goal(0, "expenses:food")])]), file("b", [rule(0, "yearly", [goal(0, "expenses:food")])])]);
        const result = rowGoals(l, ["expenses:food"], 3);
        expect(result.represented).toEqual(new Set(["a#0", "b#0"]));
    });
});

describe("UNIT budget/rowGoals — editability and the Budgeted cell", () => {
    it("marks a goal editable only when journal, file, rule and line all allow it", () => {
        const l = listing([
            file("main", [
                rule(0, "monthly", [goal(0, "expenses:a"), goal(1, "expenses:b", "an expression amount")]),
                rule(1, "yearly", [goal(2, "expenses:c")], "a locked rule"),
            ]),
            file("ro", [rule(0, "monthly", [goal(0, "expenses:d")])], false),
        ]);
        expect(goalRefs(l).map((ref) => [ref.goal.account, ref.editable])).toEqual([
            ["expenses:a", true],
            ["expenses:b", false],
            ["expenses:c", false],
            ["expenses:d", false],
        ]);
        expect(goalRefs(listing(l.files, false)).every((ref) => !ref.editable)).toBe(true);
    });

    it("offers a direct edit for exactly one editable goal", () => {
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:food")])])]);
        const cell = budgetedCell(l, rowGoals(l, ["expenses:food"], 3).byAccount.get("expenses:food"));
        expect(cell.kind).toBe("single");
    });

    it("offers a list when several goals back the row, even if some are locked", () => {
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:food")]), rule(1, "yearly", [goal(1, "expenses:food")], "locked")])]);
        const cell = budgetedCell(l, rowGoals(l, ["expenses:food"], 3).byAccount.get("expenses:food"));
        expect(cell.kind).toBe("several");
        if (cell.kind === "several") expect(cell.refs).toHaveLength(2);
    });

    it("is locked, with the engine's own reason, when no backing goal is editable", () => {
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:food", "the amount is an expression")])])]);
        const refs = rowGoals(l, ["expenses:food"], 3).byAccount.get("expenses:food");
        expect(budgetedCell(l, refs)).toEqual({kind: "locked", reason: "the amount is an expression"});
    });

    it("says why a read-only journal's goal is locked", () => {
        const l = listing([file("main", [rule(0, "monthly", [goal(0, "expenses:food")])])], false);
        const [ref] = goalRefs(l);
        expect(lockReason(l, ref)).toMatch(/read-only/);
    });

    it("is none when the listing has nothing for the row (still loading, or failed)", () => {
        const l = listing([]);
        expect(budgetedCell(l, undefined)).toEqual({kind: "none"});
    });
});

describe("UNIT budget/rowGoals — periodLabel", () => {
    it("uses the editor's word for an offered period and the raw text otherwise", () => {
        expect(periodLabel(rule(0, "yearly", []))).toBe("Annual");
        expect(periodLabel({...rule(0, "monthly", []), period: {raw: "every 2 weeks", simple: null}})).toBe("every 2 weeks");
    });
});
