// Manual downloads (plan 43 §4.2): what each manually imported account's row
// says, and when it turns amber — 35 days with no new transactions, unless
// the nudge is switched off for that account.
//
// Pure — today comes in as an argument. Relative imports only.

import {daysBetween, shortDate} from "./format";
import type {WireManualAccount} from "./types";

/** Days without a new transaction before a nudged account's row turns amber. */
export const STALE_AFTER_DAYS = 35;

export interface ManualView {
    stale: boolean;
    /** "Last imported Oct 2 (43 days ago) · last transaction Sep 30" */
    line: string;
}

const ago = (days: number): string => (days <= 0 ? "today" : days === 1 ? "yesterday" : `${days} days ago`);

/**
 * The row's line and colour.
 *
 * Stale is decided here from the last transaction, by the same rule the
 * server's `stale` flag states, and either one is enough: the server knows
 * import history this page does not, and this side keeps a long-open page
 * honest as days pass without a refetch.
 */
export function manualStale(account: WireManualAccount, today: string, locale?: string): ManualView {
    const sinceTxn = account.lastTransaction === null ? null : daysBetween(account.lastTransaction, today);
    const stale = account.nudge && (account.stale || (sinceTxn !== null && sinceTxn > STALE_AFTER_DAYS));
    const parts: string[] = [];
    if (account.lastImport !== null) {
        const sinceImport = daysBetween(account.lastImport, today);
        parts.push(`Last imported ${shortDate(account.lastImport, today, locale)}${sinceImport === null ? "" : ` (${ago(sinceImport)})`}`);
        if (account.lastTransaction !== null) parts.push(`last transaction ${shortDate(account.lastTransaction, today, locale)}`);
    } else if (account.lastTransaction !== null) {
        parts.push(`Last transaction ${shortDate(account.lastTransaction, today, locale)}${sinceTxn === null ? "" : ` (${ago(sinceTxn)})`}`);
    } else {
        parts.push("No transactions yet");
    }
    return {stale, line: parts.join(" · ")};
}
