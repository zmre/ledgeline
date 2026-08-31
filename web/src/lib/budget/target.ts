// Which `~` rule a new budget goal joins — the one question both Add paths on
// the Budget tab have to answer identically.
//
// There are two ways to add a goal. The per-period "+ Add monthly goal" button
// knows the rule it is adding to. The top-level "Add a budget goal" button
// cannot, because the period is not chosen until the modal is already open — so
// it used to send `addRule` carrying a name it made up (`"monthly budget"`), and
// a journal whose monthly rule was called anything else grew a second rule per
// goal:
//
//     ~ monthly  monthly budget
//         (expenses:whatever)    $5
//
//     ~ monthly  monthly budget
//         (expenses:other)    $10
//
// The engine joins on period AND name — `--budget=DESCPAT` filters on the name,
// so folding a goal into a differently-named rule would quietly change which
// filtered report it turns up in (`PeriodicDoc::joinable_block`). That is the
// right rule, and it means the remaining half of the fix has to be here: name
// the rule the goal is joining instead of guessing what it is called.
//
// So both paths ask this module, and a new rule is opened only when no rule
// states that period at all.

import {encodeDec} from "$lib/api/editMapping";
import type {BudgetChange} from "$lib/api/native";
import type {BudgetFile, BudgetListing, BudgetPeriod, BudgetRule, GoalDraft, GoalSubmission} from "$lib/budget/types";

/**
 * The rule a new goal of `period` joins, and the file it is written in: the
 * first writable, unlocked rule of that period, in file order.
 *
 * `null` means no rule states that period yet, and one has to be opened.
 *
 * The first, in file order, deliberately — it is the same rule the engine picks
 * on its own (`PeriodicDoc::joinable_block`), so the rule the tab says it will
 * add to is the rule that is actually added to. A locked rule is skipped for the
 * same reason the engine skips it: Ledgeline will not rewrite one, so a goal
 * appended to it would be refused.
 */
export function joinableRule(listing: BudgetListing, period: string): {file: BudgetFile; rule: BudgetRule} | null {
    return (
        listing.files
            .filter((file) => file.writable)
            .flatMap((file) => file.rules.map((rule) => ({file, rule})))
            .find(({rule}) => rule.period === period && rule.locked === null) ?? null
    );
}

/**
 * The one change a confirmed goal modal asks for, and the file to send it to.
 *
 * Three shapes, in the order they are decided:
 *
 * - an existing goal is a `set` of its amount, in the file it already lives in;
 * - a new goal joins a rule — the one its modal was opened from when that rule
 *   still states the period the user settled on, else the first joinable rule of
 *   that period, wherever it lives;
 * - and only a period with no rule at all gets `addRule`.
 *
 * Why the period and not the rule the modal was opened from: the period is a
 * live field in the modal for an ADD (`GoalModal`'s `periodFixed` is set for an
 * edit only), so a goal opened from the monthly group and then submitted as
 * yearly has to land in a yearly rule. Deciding from the rule it was opened from
 * would file it under monthly, which is not what the form says.
 */
export function goalChange(listing: BudgetListing | null, draft: GoalDraft, submission: GoalSubmission): {journalId: string; change: BudgetChange} {
    const value = encodeDec(submission.value);
    if (draft.goal !== null) {
        // An existing goal: its amount changes, and nothing else about it can.
        return {journalId: draft.journalId, change: {kind: "set", index: draft.goal.index, value}};
    }
    const target = joinTarget(listing, draft, submission.period);
    if (target !== null) {
        return {journalId: target.journalId, change: {kind: "add", block: target.rule.block, account: submission.account, value}};
    }
    return {
        journalId: draft.journalId,
        change: {kind: "addRule", period: submission.period, description: defaultRuleName(submission.period), account: submission.account, value},
    };
}

/**
 * Whether the rule this goal would join already states a goal for `account`.
 *
 * The engine refuses a second goal for one account in one rule — hledger adds
 * the two lines together, so it is not another goal but an unreadable way of
 * writing the first (`PeriodicError::DuplicateGoal`). This is the same question
 * asked one step earlier, so the modal can say so beside the field instead of
 * letting the user find out from a refused save.
 *
 * It resolves the rule exactly as {@link goalChange} does, so what is warned
 * about and what would be refused cannot come apart.
 */
export function alreadyBudgeted(listing: BudgetListing | null, draft: GoalDraft, period: BudgetPeriod, account: string): boolean {
    const target = joinTarget(listing, draft, period);
    return target !== null && target.rule.goals.some((existing) => existing.account === account);
}

/** The rule a new goal joins, preferring the one its modal was opened from. */
function joinTarget(listing: BudgetListing | null, draft: GoalDraft, period: BudgetPeriod): {journalId: string; rule: BudgetRule} | null {
    if (draft.rule !== null && draft.rule.period === period) return {journalId: draft.journalId, rule: draft.rule};
    const found = listing === null ? null : joinableRule(listing, period);
    return found === null ? null : {journalId: found.file.journalId, rule: found.rule};
}

/**
 * The name a brand-new rule is created with.
 *
 * `--budget=DESCPAT` matches on it, so it is worth being a real name rather than
 * blank — and naming it after its period is what makes "one rule per interval"
 * legible in the file itself.
 *
 * It is only ever used for a period that has no rule yet. A goal joining an
 * existing rule takes that rule's name, whatever it happens to be, because the
 * rule is named by its `block` handle rather than described.
 */
export function defaultRuleName(period: BudgetPeriod): string {
    return `${period} budget`;
}
