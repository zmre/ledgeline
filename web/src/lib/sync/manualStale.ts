// Manual downloads (plan 43 §4.2): what each manually imported account's row
// says, and when it turns amber — `manualRemindDays` (35 by default) with no
// new transactions, unless the nudge is switched off for that account.
//
// Pure — today comes in as an argument. Relative imports only.

import {daysBetween, shortDate} from "./format";
import type {WireManualAccount} from "./types";

import {MANUAL_REMIND_DAYS_DEFAULT} from "#lib/settings/uiPrefs.js";

export interface ManualView {
    stale: boolean;
    /** "Last imported Oct 2 (43 days ago) · last transaction Sep 30" */
    line: string;
}

const ago = (days: number): string => (days <= 0 ? "today" : days === 1 ? "yesterday" : `${days} days ago`);

/**
 * The row's line and colour.
 *
 * Stale is decided here, from the last transaction and `remindDays`, by the
 * same rule as the server's `stale` flag. The flag isn't read: this side
 * follows a changed setting at once, and keeps a long-open page honest as days
 * pass without a refetch.
 */
export function manualStale(account: WireManualAccount, today: string, locale?: string, remindDays = MANUAL_REMIND_DAYS_DEFAULT): ManualView {
    const sinceTxn = account.lastTransaction === null ? null : daysBetween(account.lastTransaction, today);
    const stale = account.nudge && sinceTxn !== null && sinceTxn > remindDays;
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
