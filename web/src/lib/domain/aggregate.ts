// Posting aggregation (WP-02). Pure TS: no Svelte/DOM imports.

import {accountMatches} from "./accounts";
import {add, isZero, maAdd, type MixedAmount} from "./money";
import type {ISODate, Transaction, TxnStatus} from "./types";

export interface PostingFilter {
    /** Inclusive lower bound on the posting's effective date. */
    from?: ISODate;
    /** Inclusive upper bound on the posting's effective date. */
    to?: ISODate;
    /** Selected accounts (each matches itself + sub-accounts); empty/absent = all. */
    accounts?: string[];
    status?: TxnStatus;
}

/**
 * One pass over all postings, summing per FULL account name.
 * Effective posting date is `posting.date ?? txn.date`; effective status falls
 * back to the transaction's when the posting is unmarked (hledger semantics).
 */
export function accountTotals(txns: Transaction[], f?: PostingFilter): Map<string, MixedAmount> {
    const selected = f?.accounts !== undefined && f.accounts.length > 0 ? f.accounts : null;
    const totals = new Map<string, MixedAmount>();
    for (const txn of txns) {
        for (const posting of txn.postings) {
            const date = posting.date ?? txn.date;
            if (f?.from !== undefined && date < f.from) continue;
            if (f?.to !== undefined && date > f.to) continue;
            if (f?.status !== undefined) {
                const effective = posting.status === "unmarked" ? txn.status : posting.status;
                if (effective !== f.status) continue;
            }
            if (selected !== null && !selected.some((sel) => accountMatches(sel, posting.account))) continue;
            let ma = totals.get(posting.account);
            if (ma === undefined) {
                ma = new Map();
                totals.set(posting.account, ma);
            }
            for (const amount of posting.amounts) {
                const prev = ma.get(amount.commodity);
                ma.set(amount.commodity, prev === undefined ? amount.qty : add(prev, amount.qty));
            }
        }
    }
    for (const ma of totals.values()) {
        for (const [commodity, qty] of ma) {
            if (isZero(qty)) ma.delete(commodity);
        }
    }
    return totals;
}

/** Add each account's total into itself and all ancestors (inclusive balances). */
export function rollUp(totals: Map<string, MixedAmount>): Map<string, MixedAmount> {
    const out = new Map<string, MixedAmount>();
    for (const [account, ma] of totals) {
        let path = "";
        for (const segment of account.split(":")) {
            path = path === "" ? segment : `${path}:${segment}`;
            out.set(path, maAdd(out.get(path) ?? new Map(), ma));
        }
    }
    return out;
}

/** Keep only accounts with at most `depth` segments. */
export function atDepth(rolled: Map<string, MixedAmount>, depth: number): Map<string, MixedAmount> {
    const out = new Map<string, MixedAmount>();
    for (const [account, ma] of rolled) {
        if (account.split(":").length <= depth) out.set(account, ma);
    }
    return out;
}
