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
// under node.

import {dec, mul, toNumber, type MixedAmount} from "$lib/domain/money";
import type {BudgetRule} from "$lib/budget/types";

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
 * working unchanged: `every 2 weeks` matches no offered period (so the rule
 * groups under "Other", as it must) and is absent from `ANNUAL_FACTOR` (so it
 * sorts unranked, which is the documented refusal to invent a position for a
 * rule Ledgeline will not model). Null would have needed both of those handled
 * again, separately.
 */
export function periodOf(rule: BudgetRule): string {
    return rule.period.simple ?? rule.period.raw;
}

/**
 * How many times a goal of each period recurs in a year.
 *
 * A period this map does not name has no factor, and is NOT guessed at: a
 * `~ every 2 weeks` rule (which plan 21 teaches the parser to read) is shown
 * read-only by the editor precisely because Ledgeline will not model it, and
 * inventing a sort position for it would be the same claim in another place.
 *
 * A `Map`, not an object literal, because the key is free text off the wire and
 * `get` is the lookup whose type says "this may not be there".
 */
const ANNUAL_FACTOR: ReadonlyMap<string, number> = new Map([
    ["daily", 365],
    ["weekly", 52],
    ["monthly", 12],
    ["quarterly", 4],
    ["yearly", 1],
]);

/**
 * A goal's magnitude normalised to a year, so periods compare: $100 weekly is a
 * bigger commitment than $300 monthly, and a list that sorted on the written
 * figures alone would say the opposite.
 *
 * An unrecognised period has no factor to apply, so the amount comes back
 * unscaled — `sortBudgetLines` sorts those rows to the end rather than pretending
 * the unscaled figure is comparable.
 */
export function annualised(amount: MixedAmount, period: string): MixedAmount {
    const factor = ANNUAL_FACTOR.get(period);
    if (factor === undefined) return amount;
    const scale = dec(factor, 0);
    const out: MixedAmount = new Map();
    for (const [commodity, qty] of amount) out.set(commodity, mul(qty, scale));
    return out;
}

/**
 * The number an amount sorts by, or null when it has none.
 *
 * Null for an unrecognised period (see `annualised`) and for a multi-commodity
 * goal, which has no single magnitude for the same reason `primaryValue` refuses
 * one — $100 and €100 do not add up without a price, and the sort must not
 * invent one. Both cases sort last.
 *
 * The magnitude, never the signed value: revenue is credit-normal, so a signed
 * descending sort would put the smallest earner at the top of the income list.
 */
function sortMagnitude(amount: MixedAmount, period: string): number | null {
    if (!ANNUAL_FACTOR.has(period)) return null;
    const scaled = annualised(amount, period);
    if (scaled.size === 0) return 0;
    if (scaled.size > 1) return null;
    const [qty] = scaled.values();
    return Math.abs(toNumber(qty));
}

/** Lexical account comparison — the same `<`/`>` string order every other list here uses. */
function compareAccount(a: string, b: string): number {
    return a < b ? -1 : a > b ? 1 : 0;
}

/** What `sortBudgetLines` needs to know about one item, whatever the item is. */
export interface SortFields {
    account: string;
    /** The goal amount, in whatever sign the source carries; only its magnitude is read. */
    amount: MixedAmount;
    /** The recurrence `amount` is stated per — see `annualised`. */
    period: string;
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
        const left = sortMagnitude(a.fields.amount, a.fields.period);
        const right = sortMagnitude(b.fields.amount, b.fields.period);
        if (left === null || right === null) {
            if (left !== null) return -1;
            if (right !== null) return 1;
            return a.index - b.index;
        }
        return left !== right ? sign * (left - right) : a.index - b.index;
    });
    return decorated.map((entry) => entry.item);
}
