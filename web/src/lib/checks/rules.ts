// MVP check rules (WP-08). Pure functions over domain Transactions — relative
// imports only (same convention as lib/reports/). `today()` comes from
// lib/reports/periods.ts, the codebase's single sanctioned local-Date read.

import {isAccountType, NO_ACCOUNT_CLASSES, type AccountClasses} from "../domain/accountClass";
import type {Dec} from "../domain/money";
import type {Transaction} from "../domain/types";
import {accountBalances} from "../reports/cashBalances";
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
function isUncategorized(account: string, classes: AccountClasses): boolean {
    const segments = account.toLowerCase().split(":");
    if (UNCATEGORIZED_SEGMENTS.has(segments[segments.length - 1])) return true;
    if (segments.length !== 1) return false;
    // `isAccountType`: a bare root declared `type: G` is a revenue root with no
    // category under it, which is exactly the mistake this rule catches.
    return isAccountType(account, classes, "expense") || isAccountType(account, classes, "revenue");
}

const uncategorized: CheckRule = {
    id: "uncategorized",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        const problems: Problem[] = [];
        const classes = ctx.classes ?? NO_ACCOUNT_CLASSES;
        for (const txn of txns) {
            const seen = new Set<string>();
            for (const posting of txn.postings) {
                if (!isUncategorized(posting.account, classes) || seen.has(posting.account)) continue;
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

/**
 * A cash account whose balance is negative today — you cannot hold less than
 * nothing in a bank account, so this is either an overdraft or a booking
 * mistake, and both are worth knowing about.
 *
 * ANCHORED TO THE ACCOUNT, not to a transaction: no single entry is at fault,
 * the running total is. That makes this the second account-anchored rule after
 * the engine's `account-tag`, and the drawer already renders that shape (it
 * prints the account name where a transaction finding prints its date and
 * description, and offers no jump target, because there is no one row to jump
 * to).
 *
 * Scope matches the Balances view exactly — `accountBalances` is the same
 * memoized pass both use, so a red row there always has a finding here and vice
 * versa. That includes the current/non-current filter: a non-current account is
 * out of both. Liabilities are not checked at all; a negative balance is what
 * owing money LOOKS like.
 */
const negativeCash: CheckRule = {
    id: "negative-cash",
    run(txns: Transaction[], ctx: CheckContext): Problem[] {
        const asOf = today();
        return accountBalances(txns, ctx.classes ?? NO_ACCOUNT_CLASSES, asOf)
            .filter((balance) => balance.kind === "cash")
            .flatMap((balance) =>
                [...balance.amounts.entries()]
                    .filter(([, qty]) => qty.m < 0n)
                    .map(([commodity, qty]) => ({
                        txnIndex: null,
                        account: balance.account,
                        rule: "negative-cash",
                        severity: "warning" as const,
                        message: `cash balance is negative: ${commodity} ${decToString(qty)} as of ${balance.asOf ?? asOf}`,
                    }))
            );
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
 * (DRY-1). One engine now answers both. The SPA requires the native API, so
 * `/api/diagnostics` is always there to supply them.
 */
export const ALL_RULES: CheckRule[] = [pending, uncategorized, negativeCash, missingDescription, futureDate];
