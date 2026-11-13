// Which written goals stand behind a row of the budget table (plan 41 WP-B).
//
// The Budget tab's table puts each rule on the same row as its progress, so the
// "Budgeted" figure on a row has to be able to say WHICH `~` lines it came
// from — that is what the edit button on it opens. The bars come from the
// engine's summary and the goals from the listing; nothing on the wire joins
// the two, so this module does, structurally, from account names alone.
//
// # Depth: how a goal finds its row
//
// The summary is computed at the page's depth. The engine CLIPS every goal's
// account to that many segments before it sums anything (`budget.rs`'s `clip`),
// then keeps only the leaf-most budgeted lines (`budget_leaves`). So a row's
// Budgeted figure is the sum of every goal whose account clips to the row's
// account, and that is the whole rule here:
//
//     a goal backs row R  ⇔  clip(goal.account, depth) === R.account
//
// Which gives three cases, all deliberate:
//
//   - A goal ON the row's account backs it (the common case).
//   - A goal DEEPER than the depth (`expenses:food:dining` at depth 2) rolls up
//     under its depth-`depth` ancestor's row (`expenses:food`), beside any goal
//     written on that ancestor itself. The row's figure is their sum, so the
//     row lists them all — a figure that cannot show its parts is one you cannot
//     edit honestly. That is the main source of "several goals behind one row".
//   - A goal SHALLOWER than a budgeted descendant (`expenses:food` while
//     `expenses:food:dining` also has a goal, at depth 3) backs NO row: the
//     engine hides that parent line because a deeper budgeted line exists. Its
//     clip is itself, which matches no row, so it stays in the goals editor
//     below the table — the only place it is visible at all.
//
// Period is NOT part of the match. Every period's goals feed the same row
// (hledger sums a monthly and a yearly goal on one account into one figure), so
// a monthly + yearly pair on one account is two backing goals, listed with
// their periods. A rule the editor cannot rewrite (`~ every 2 weeks`) still
// backs its row; it is listed read-only, with the engine's reason.
//
// Pure TS, no Svelte, no clock: unit-tested under node (`rowGoals.test.ts`).

import {periodOf} from "#lib/budget/sort.js";
import {BUDGET_PERIODS, type BudgetFile, type BudgetGoal, type BudgetListing, type BudgetRule} from "#lib/budget/types.js";
import type {MixedAmount} from "#lib/domain/money.js";
import type {AmountStyle} from "#lib/domain/types.js";
import {formatTotals} from "#lib/journal/rowModel.js";

/** One goal, carried with the file and rule it is written in, and whether it can be changed here. */
export interface GoalRef {
    file: BudgetFile;
    rule: BudgetRule;
    goal: BudgetGoal;
    /** Whether Edit/Remove may be offered for this goal — see {@link goalEditable}. */
    editable: boolean;
}

/**
 * The goal's identity on the page: the file plus the engine's per-FILE index.
 *
 * The same pair the page's two-step remove and the modal's `{#key}` already
 * identify a goal by. A scan ordinal, so only valid against one listing.
 */
export function goalKey(journalId: string, goal: BudgetGoal): string {
    return `${journalId}#${goal.index}`;
}

/** Whether a goal can be edited or removed here: the journal, the file, the rule and the line all have to allow it. */
export function goalEditable(listing: BudgetListing, file: BudgetFile, rule: BudgetRule, goal: BudgetGoal): boolean {
    return listing.editable && file.writable && rule.locked === null && goal.locked === null;
}

/** The sentence a lock shows, most specific first. Only meaningful for a goal that is not {@link goalEditable}. */
export function lockReason(listing: BudgetListing, ref: GoalRef): string {
    if (ref.rule.locked !== null) return ref.rule.locked;
    if (ref.goal.locked !== null) return ref.goal.locked;
    if (!ref.file.writable) return "This file is not a regular file inside the journal's directory.";
    if (!listing.editable) return "This journal is open read-only, so its budget can be viewed but not changed.";
    return "";
}

/** The rule's period as a person says it ("Monthly"), or its raw expression for one the editor does not offer. */
export function periodLabel(rule: BudgetRule): string {
    const period = periodOf(rule);
    return BUDGET_PERIODS.find((p) => p.id === period)?.label ?? rule.period.raw;
}

/**
 * A goal's amount as the editor lists it: the magnitude the user typed when
 * there is one, else the file's raw amount, else "—" (the inferred leg).
 */
export function goalAmountText(goal: BudgetGoal, styles: ReadonlyMap<string, AmountStyle>): string {
    const amount: MixedAmount | null = goal.entry === null ? goal.amount : new Map([[goal.entry.commodity, goal.entry.value]]);
    if (amount === null) return "—";
    const parts = formatTotals(amount, styles).map((line) => line.text);
    return parts.length > 0 ? parts.join(", ") : "—";
}

/** Every goal in the listing, in file → rule → line order. */
export function goalRefs(listing: BudgetListing): GoalRef[] {
    return listing.files.flatMap((file) =>
        file.rules.flatMap((rule) => rule.goals.map((goal) => ({file, rule, goal, editable: goalEditable(listing, file, rule, goal)})))
    );
}

/**
 * Clamp an account name to at most `depth` segments — the engine's `clip`,
 * byte for byte: a depth below 1 is treated as 1, and a name already that
 * short is returned whole.
 */
export function clipAccount(account: string, depth: number): string {
    const keep = Math.max(1, Math.floor(depth));
    let at = -1;
    for (let segment = 0; segment < keep; segment++) {
        at = account.indexOf(":", at + 1);
        if (at === -1) return account;
    }
    return account.slice(0, at);
}

/** The listing split by the rows of the table. */
export interface RowGoals {
    /** Row account → the goals behind it, in listing order. Rows with none are absent. */
    byAccount: Map<string, GoalRef[]>;
    /** Goals behind no row: a hidden parent, a non-income/expense account, an account with no row this span. */
    unmatched: GoalRef[];
    /** {@link goalKey}s of every goal in `byAccount`, for the editor to leave out. */
    represented: Set<string>;
}

/**
 * Map each table row to its backing goals (see the header for the rule).
 *
 * `accounts` are the rows' accounts exactly as the engine named them (already
 * clipped to `depth`); `depth` is the depth the summary was computed at.
 */
export function rowGoals(listing: BudgetListing, accounts: Iterable<string>, depth: number): RowGoals {
    const rows = new Set(accounts);
    const byAccount = new Map<string, GoalRef[]>();
    const unmatched: GoalRef[] = [];
    const represented = new Set<string>();
    for (const ref of goalRefs(listing)) {
        const row = clipAccount(ref.goal.account, depth);
        if (!rows.has(row)) {
            unmatched.push(ref);
            continue;
        }
        const list = byAccount.get(row);
        if (list === undefined) byAccount.set(row, [ref]);
        else list.push(ref);
        represented.add(goalKey(ref.file.journalId, ref.goal));
    }
    return {byAccount, unmatched, represented};
}

/**
 * What the Budgeted cell offers, from the goals behind it.
 *
 *   - `single`: exactly one goal and it is editable — the amount opens it.
 *   - `several`: more than one goal and at least one editable — the amount
 *     opens a list of them, each with its own Edit/Remove (locked ones shown
 *     read-only, so the list adds up to the figure).
 *   - `locked`: goals, none editable — plain text with the first reason.
 *   - `none`: no goal found (the listing is still loading, or failed).
 */
export type BudgetedCell = {kind: "single"; ref: GoalRef} | {kind: "several"; refs: GoalRef[]} | {kind: "locked"; reason: string} | {kind: "none"};

export function budgetedCell(listing: BudgetListing, refs: readonly GoalRef[] | undefined): BudgetedCell {
    if (refs === undefined || refs.length === 0) return {kind: "none"};
    const editable = refs.filter((ref) => ref.editable);
    if (editable.length === 0) return {kind: "locked", reason: lockReason(listing, refs[0])};
    if (refs.length === 1) return {kind: "single", ref: refs[0]};
    return {kind: "several", refs: [...refs]};
}
