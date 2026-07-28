// MVP check rules (WP-08). Pure functions over domain Transactions — relative
// imports only (same convention as lib/reports/). `today()` comes from
// lib/reports/periods.ts, the codebase's single sanctioned local-Date read.

import {declaredTypes, resolveAccountType, type AccountType} from "../domain/accountTypes";
import {add, isZero, mul, neg, type Dec} from "../domain/money";
import type {Amount, Transaction} from "../domain/types";
import {today} from "../reports/periods";
import type {CheckContext, CheckRule, Problem} from "./engine";

/** Plain decimal rendering for problem messages (no locale styling — messages are diagnostics). */
function decToString(d: Dec): string {
    const negative = d.m < 0n;
    const digits = (negative ? -d.m : d.m).toString().padStart(d.p + 1, "0");
    const whole = d.p === 0 ? digits : digits.slice(0, digits.length - d.p);
    const frac = d.p === 0 ? "" : `.${digits.slice(digits.length - d.p)}`;
    return `${negative ? "-" : ""}${whole}${frac}`;
}

/**
 * The value an amount contributes to transaction balancing: amounts carrying a
 * cost annotation balance in the COST commodity (hledger semantics — otherwise
 * every `10 AAPL @ $220.00` purchase would look unbalanced). `@` per-unit costs
 * multiply; `@@` total costs take the posting amount's sign.
 */
function balanceValue(amount: Amount): {commodity: string; qty: Dec} {
    const cost = amount.cost;
    if (cost === undefined) return {commodity: amount.commodity, qty: amount.qty};
    if (cost.per) return {commodity: cost.commodity, qty: mul(amount.qty, cost.qty)};
    return {commodity: cost.commodity, qty: amount.qty.m < 0n ? neg(cost.qty) : cost.qty};
}

/**
 * hledger's two INDEPENDENT balancing groups. Real postings balance among
 * themselves and balanced-virtual `[a]` postings balance among themselves, each
 * with its own one-posting elision budget (`[v] $10 / [v] / a $1 / b` is
 * accepted by hledger 1.52). Unbalanced-virtual `(a)` postings are in NEITHER:
 * being excluded from balancing is the whole meaning of the parentheses, so
 * summing them reported every envelope/budget journal as unbalanced.
 */
const BALANCING_GROUPS = [
    {type: "regular", label: "postings"},
    {type: "balancedVirtual", label: "balanced virtual postings"},
] as const;

/** One balancing group's finding, or none when it balances (or elides its remainder). */
function balanceGroup(txn: Transaction, postings: Transaction["postings"], label: string): Problem[] {
    const problem = (message: string): Problem[] => [{txnIndex: txn.index, rule: "unbalanced", severity: "error", message}];
    const elided = postings.filter((posting) => posting.amounts.length === 0).length;
    if (elided >= 2) return problem(`${elided} ${label} have no amount — at most one may be elided`);
    if (elided === 1) return []; // the amountless posting absorbs the remainder
    const residue = new Map<string, Dec>();
    for (const posting of postings) {
        for (const amount of posting.amounts) {
            const {commodity, qty} = balanceValue(amount);
            const prev = residue.get(commodity);
            residue.set(commodity, prev === undefined ? qty : add(prev, qty));
        }
    }
    const nonzero = [...residue.entries()].filter(([, qty]) => !isZero(qty));
    if (nonzero.length === 0) return [];
    const detail = nonzero.map(([commodity, qty]) => `${commodity} ${decToString(qty)}`).join(", ");
    return problem(`${label} do not sum to zero: ${detail} remaining`);
}

const unbalanced: CheckRule = {
    id: "unbalanced",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        // The engine already ran this check, more accurately — see
        // CheckContext.engineChecked. Deferring avoids both a duplicate finding
        // and this rule's false positives on journals hledger accepts.
        if (ctx.engineChecked === true) return [];
        return txns.flatMap((txn) =>
            BALANCING_GROUPS.flatMap((group) =>
                balanceGroup(
                    txn,
                    txn.postings.filter((posting) => (posting.type ?? "regular") === group.type),
                    group.label
                )
            )
        );
    },
};

const pending: CheckRule = {
    id: "pending",
    run(txns: Transaction[]): Problem[] {
        return txns
            .filter((txn) => txn.status === "pending")
            .map((txn) => ({txnIndex: txn.index, rule: "pending", severity: "warning" as const, message: "transaction is marked pending (!)"}));
    },
};

const UNCATEGORIZED_SEGMENTS = new Set(["unknown", "uncategorized"]);
/**
 * `*:unknown`, `*:uncategorized` (any depth, incl. bare), or a bare top-level
 * income/expense root with no subaccount.
 *
 * The bare-root case is decided by TYPE, not by the literal names
 * "expenses"/"income": the whole point of the rule is to catch a posting that
 * never got a category, and a chart of accounts rooted at `cogs:` or `gastos:`
 * has exactly the same mistake to catch.
 */
function isUncategorized(account: string, declared: ReadonlyMap<string, AccountType>): boolean {
    const segments = account.toLowerCase().split(":");
    if (UNCATEGORIZED_SEGMENTS.has(segments[segments.length - 1])) return true;
    if (segments.length !== 1) return false;
    const type = resolveAccountType(account, declared);
    return type === "expense" || type === "revenue";
}

const uncategorized: CheckRule = {
    id: "uncategorized",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        const problems: Problem[] = [];
        const declared = declaredTypes(ctx.decls ?? []);
        for (const txn of txns) {
            const seen = new Set<string>();
            for (const posting of txn.postings) {
                if (!isUncategorized(posting.account, declared) || seen.has(posting.account)) continue;
                seen.add(posting.account);
                problems.push({
                    txnIndex: txn.index,
                    rule: "uncategorized",
                    severity: "warning",
                    message: `posting to uncategorized account "${posting.account}"`,
                });
            }
        }
        return problems;
    },
};

const missingDescription: CheckRule = {
    id: "missing-description",
    run(txns: Transaction[]): Problem[] {
        return txns
            .filter((txn) => txn.description.trim() === "")
            .map((txn) => ({txnIndex: txn.index, rule: "missing-description", severity: "info" as const, message: "transaction has no description"}));
    },
};

const futureDate: CheckRule = {
    id: "future-date",
    run(txns: Transaction[]): Problem[] {
        const cutoff = today();
        return txns
            .filter((txn) => txn.date > cutoff)
            .map((txn) => ({txnIndex: txn.index, rule: "future-date", severity: "info" as const, message: `transaction is dated in the future (${txn.date})`}));
    },
};

/**
 * All rules, in report order. Adding a rule = one object here.
 *
 * The three WP-10 stock rules (`stock-missing-basis`, `stock-negative`,
 * `stock-unpriced`) are NOT here: they are computed by the Rust holdings engine
 * and arrive through `CheckContext.diagnostics` off `/api/diagnostics`, exactly
 * like `unbalanced` and `assertion`. They used to run here over a second,
 * TypeScript copy of the average-cost pools, and the two copies had drifted far
 * enough to give opposite answers for the same journal — a 2-for-1 split read as
 * a cost-less acquisition here while the Holdings page reported its real basis
 * (DRY-1). One engine now answers both.
 *
 * The visible consequence: against a plain `hledger-web` backend, which has no
 * `/api/diagnostics` route, there are no stock findings at all. That is the same
 * trade the engine-computed `unbalanced` diagnostics already make, and it is the
 * right one — a wrong finding is worse than no finding.
 */
export const ALL_RULES: CheckRule[] = [unbalanced, pending, uncategorized, missingDescription, futureDate];
