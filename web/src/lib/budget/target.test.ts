// Which rule a new goal joins, and what the tab therefore asks the engine for.
//
// The regression this file exists for: the top-level "Add a budget goal" button
// cannot know which rule it is adding to, so it used to send `addRule` with a
// name it synthesised — `"monthly budget"`. The engine joins on period AND name,
// so a journal whose monthly rule is called `household budget` got a SECOND rule
// opened per goal, and only people whose rules happened to carry the default
// name were spared. `a_goal_joins_a_rule_whose_name_is_not_the_default` is that
// case, and it is the half of the fix the engine cannot make on its own.

import {describe, expect, test} from "vitest";
import {alreadyBudgeted, defaultRuleName, goalChange, joinableRule} from "#lib/budget/target.js";
import type {BudgetFile, BudgetGoal, BudgetListing, BudgetPeriod, BudgetRule, GoalDraft, GoalSubmission} from "#lib/budget/types.js";

function goal(index: number, account: string): BudgetGoal {
    return {index, line: index + 2, account, unbalanced: true, amount: null, entry: null, inverted: false, annual: null, locked: null};
}

/**
 * A rule of an OFFERED period. `simple` and `raw` agree here because a bare
 * interval is written with the word it is offered as; a rule where they differ
 * is `unofferedRule`.
 */
function rule(block: number, period: BudgetPeriod, description: string, extra: Partial<BudgetRule> = {}): BudgetRule {
    return {block, line: 1, period: {raw: period, simple: period}, description, locked: null, goals: [goal(block, "expenses:food")], ...extra};
}

/** A rule whose header says more than a bare interval — `~ every 2 weeks`. */
function unofferedRule(block: number, raw: string, description: string, extra: Partial<BudgetRule> = {}): BudgetRule {
    return {
        block,
        line: 1,
        period: {raw, simple: null},
        description,
        locked: "its period says more than a plain interval",
        goals: [goal(block, "expenses:food")],
        ...extra,
    };
}

function file(journalId: string, rules: BudgetRule[], extra: Partial<BudgetFile> = {}): BudgetFile {
    return {journalId, label: journalId, revision: `rev-${journalId}`, writable: true, rules, ...extra};
}

/**
 * A listing, with `joinable` as the ENGINE would send it for these files.
 *
 * A stand-in for `budget_api::joinable_targets` (first writable, unlocked rule
 * per period, in file order), so the cases below read as journals rather than
 * as hand-written targets. The rule itself is the engine's and is tested there
 * (`budget_endpoints.rs`); this module only looks its answer up.
 */
function listing(files: BudgetFile[], joinable?: BudgetListing["joinable"]): BudgetListing {
    const engine: BudgetListing["joinable"] = {};
    for (const f of files.filter((candidate) => candidate.writable)) {
        for (const r of f.rules) {
            if (r.period.simple !== null && r.locked === null && engine[r.period.simple] === undefined) {
                engine[r.period.simple] = {journalId: f.journalId, block: r.block};
            }
        }
    }
    return {
        editable: true,
        defaultTarget: files[0]?.journalId ?? null,
        canCreateFile: false,
        createFileName: "budget.journal",
        files,
        joinable: joinable ?? engine,
    };
}

/** A modal opened by the top-level button: no rule, defaulting to the main file. */
function adding(period: BudgetPeriod, journalId = "main.journal", from: BudgetRule | null = null): GoalDraft {
    return {goal: null, rule: from, journalId, period, account: "", amount: ""};
}

function submitted(period: BudgetPeriod, account = "expenses:bus"): GoalSubmission {
    return {period, account, value: {m: 2000n, p: 2}};
}

describe("joinableRule", () => {
    test("finds the rule the engine named, in the file it named", () => {
        const files = [file("a.journal", [rule(0, "monthly", "first")]), file("b.journal", [rule(0, "monthly", "second"), rule(1, "yearly", "annual")])];
        const found = joinableRule(listing(files, {monthly: {journalId: "b.journal", block: 0}}), "monthly");
        expect(found?.rule.description).toBe("second");
        expect(found?.file.journalId).toBe("b.journal");
    });

    test("is null when the engine names no rule for the period", () => {
        expect(joinableRule(listing([file("main.journal", [rule(0, "monthly", "monthly budget")])], {}), "monthly")).toBeNull();
    });

    test("is null for a period the editor does not model, and for a target the listing does not hold", () => {
        const files = [file("main.journal", [unofferedRule(0, "every 2 weeks", "paycheck")])];
        expect(joinableRule(listing(files), "every 2 weeks")).toBeNull();
        expect(joinableRule(listing(files, {monthly: {journalId: "gone.journal", block: 0}}), "monthly")).toBeNull();
    });
});

describe("goalChange", () => {
    // THE regression test: the rule is named by its handle, not by a guess at
    // what it is called, so a rule with any name at all is joined.
    test("a goal joins a rule whose name is not the default", () => {
        const held = listing([file("main.journal", [rule(0, "monthly", "household budget")])]);
        const {journalId, change} = goalChange(held, adding("monthly"), submitted("monthly"));
        expect(change).toEqual({kind: "add", block: 0, account: "expenses:bus", value: {mantissa: "2000", places: 2}});
        expect(journalId).toBe("main.journal");
    });

    test("only a period with no rule at all opens one", () => {
        const held = listing([file("main.journal", [rule(0, "monthly", "household budget")])]);
        const {change} = goalChange(held, adding("yearly"), submitted("yearly"));
        expect(change).toEqual({
            kind: "addRule",
            period: "yearly",
            description: "yearly budget",
            account: "expenses:bus",
            value: {mantissa: "2000", places: 2},
        });
    });

    test("a period whose only rule is locked opens a new one rather than appending to it", () => {
        const held = listing([file("main.journal", [rule(0, "monthly", "household budget", {locked: "it uses balanced-virtual postings"})])]);
        expect(goalChange(held, adding("monthly"), submitted("monthly")).change.kind).toBe("addRule");
    });

    test("the goal is saved to the file holding the rule it joins, not the default one", () => {
        const held = listing([file("main.journal", []), file("budget.journal", [rule(0, "monthly", "household budget")])]);
        const {journalId, change} = goalChange(held, adding("monthly", "main.journal"), submitted("monthly"));
        expect(journalId).toBe("budget.journal");
        expect(change.kind).toBe("add");
    });

    // The period is a live field for an ADD (`GoalModal`'s `periodFixed` covers
    // edits only), so the rule it lands in follows the form, not the button.
    test("switching the period in the modal moves the goal to that period's rule", () => {
        const monthly = rule(0, "monthly", "household budget");
        const held = listing([file("main.journal", [monthly, rule(1, "yearly", "annual budget")])]);
        const {change} = goalChange(held, adding("monthly", "main.journal", monthly), submitted("yearly"));
        expect(change).toMatchObject({kind: "add", block: 1});
    });

    test("a goal added from a rule still joins that rule", () => {
        const second = rule(1, "monthly", "second monthly rule");
        const held = listing([file("main.journal", [rule(0, "monthly", "first"), second])]);
        const {change} = goalChange(held, adding("monthly", "main.journal", second), submitted("monthly"));
        expect(change).toMatchObject({kind: "add", block: 1});
    });

    test("an existing goal is a set, in its own file, whatever rules exist elsewhere", () => {
        const held = listing([file("budget.journal", [rule(0, "monthly", "household budget")])]);
        const draft: GoalDraft = {...adding("monthly", "budget.journal"), goal: goal(3, "expenses:food")};
        const {journalId, change} = goalChange(held, draft, submitted("monthly", "expenses:food"));
        expect(change).toEqual({kind: "set", index: 3, value: {mantissa: "2000", places: 2}});
        expect(journalId).toBe("budget.journal");
    });

    test("a listing that has not loaded still produces a usable request", () => {
        expect(goalChange(null, adding("monthly"), submitted("monthly")).change.kind).toBe("addRule");
    });
});

describe("alreadyBudgeted", () => {
    const held = listing([file("main.journal", [rule(0, "monthly", "household budget"), rule(1, "yearly", "annual budget")])]);

    test("is true for an account the joined rule already states", () => {
        // `rule()` gives every rule one goal, on expenses:food.
        expect(alreadyBudgeted(held, adding("monthly"), "monthly", "expenses:food")).toBe(true);
    });

    test("is false for an account it does not", () => {
        expect(alreadyBudgeted(held, adding("monthly"), "monthly", "expenses:bus")).toBe(false);
    });

    test("follows the period, so a category budgeted monthly is free to budget yearly", () => {
        const monthlyOnly = listing([file("main.journal", [rule(0, "monthly", "household budget")])]);
        expect(alreadyBudgeted(monthlyOnly, adding("monthly"), "yearly", "expenses:food")).toBe(false);
    });

    test("is false when there is no rule to join, because nothing is being appended to", () => {
        expect(alreadyBudgeted(null, adding("monthly"), "monthly", "expenses:food")).toBe(false);
    });
});

test("a new rule is named after its period", () => {
    expect(defaultRuleName("quarterly")).toBe("quarterly budget");
});
