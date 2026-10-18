// How the Budget tab orders what it shows.
//
// Goals come out of the journal in whatever order they were written, which is
// the order they happened to be added in — `revenues:salary` at the top and
// `revenues:dividends` at the bottom, with nothing to scan by. ONE control
// governs both halves of the page: the bars above and the goals below are the
// same subject read two ways, and they must not disagree about order.
//
// Not purity-guarded (this is `lib/budget/`, not `lib/reports/`), but it still
// imports no Svelte and reads no clock, so the whole module is unit-testable
// under node. The annual factors that make goals of different periods
// comparable are the engine's (`BudgetGoal.annual`, plan 26 B6); this only
// orders what it is handed.

import {toNumber, type MixedAmount} from "#lib/domain/money.js";
import type {BudgetRule} from "#lib/budget/types.js";

/** Which column the goals are ordered by. */
export type BudgetSortKey = "account" | "amount";
export type SortDir = "asc" | "desc";

export const BUDGET_SORT_KEYS: readonly BudgetSortKey[] = ["account", "amount"];
export const SORT_DIRS: readonly SortDir[] = ["asc", "desc"];

/**
 * Read a rule's recurrence, as one string.
 *
 * `simple` when the editor models the period, else the raw expression. Falling
 * back to `raw` rather than to null is what makes the rest of this module keep
 * working unchanged: `every 2 weeks` matches no offered period, so the rule
 * groups under "Other", as it must.
 */
export function periodOf(rule: BudgetRule): string {
    return rule.period.simple ?? rule.period.raw;
}

/**
 * The number an amount sorts by, or null when it has none.
 *
 * Null for an amount the engine would not rank (a rule that is not a bare fixed
 * interval — see `BudgetGoal.annual`) and for a multi-commodity one, which has no
 * single magnitude for the same reason `primaryValue` refuses one — $100 and €100
 * do not add up without a price, and the sort must not invent one. Both sort
 * last.
 *
 * The magnitude, never the signed value: revenue is credit-normal, so a signed
 * descending sort would put the smallest earner at the top of the income list.
 * (Both callers already hand over magnitudes; `abs` keeps that a non-question.)
 */
function sortMagnitude(amount: MixedAmount | null): number | null {
    if (amount === null) return null;
    if (amount.size === 0) return 0;
    if (amount.size > 1) return null;
    const [qty] = amount.values();
    return Math.abs(toNumber(qty));
}

/** Lexical account comparison — the same `<`/`>` string order every other list here uses. */
function compareAccount(a: string, b: string): number {
    return a < b ? -1 : a > b ? 1 : 0;
}

/** What `sortBudgetLines` needs to know about one item, whatever the item is. */
export interface SortFields {
    account: string;
    /**
     * The amount to rank by, already comparable across items: a bar's span-total
     * goal, or a goal's engine-annualised `annual` (so $100 weekly outranks $300
     * monthly). Null ranks last. Only its magnitude is read.
     */
    amount: MixedAmount | null;
}

/**
 * Order `items` by `key`/`dir`, reading each one through `of`.
 *
 * Stable for equal keys: the original index is the final tiebreak, so two goals
 * of the same size keep the order the journal wrote them in rather than moving
 * about between renders. Rows with no comparable amount go LAST in both
 * directions — they are not "the smallest", they are unranked, and burying them
 * at the top of an ascending list would be a different lie.
 *
 * The input is never mutated (`sort` is in place, so the decoration is what gets
 * sorted): callers hand this `$derived` arrays.
 */
export function sortBudgetLines<T>(items: readonly T[], key: BudgetSortKey, dir: SortDir, of: (item: T) => SortFields): T[] {
    const sign = dir === "asc" ? 1 : -1;
    const decorated = items.map((item, index) => ({item, index, fields: of(item)}));
    decorated.sort((a, b) => {
        if (key === "account") {
            const order = compareAccount(a.fields.account, b.fields.account);
            return order !== 0 ? sign * order : a.index - b.index;
        }
        const left = sortMagnitude(a.fields.amount);
        const right = sortMagnitude(b.fields.amount);
        if (left === null || right === null) {
            if (left !== null) return -1;
            if (right !== null) return 1;
            return a.index - b.index;
        }
        return left !== right ? sign * (left - right) : a.index - b.index;
    });
    return decorated.map((entry) => entry.item);
}
